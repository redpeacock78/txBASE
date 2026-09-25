use super::super::super::super::{join_index, join_nested, join_strategy};
use super::super::super::{JoinError, JoinType};
use super::super::{emit, encoded_key};
use super::DirectJoinContext;
use crate::dbf::DbfRecord;
use std::collections::BTreeMap;

pub(super) fn execute(
    context: &DirectJoinContext<'_>,
) -> Result<Vec<serde_json::Value>, JoinError> {
    let right_index = if context.large_join {
        context.current_catalog.and_then(|catalog| {
            join_index::load_fields(
                catalog,
                &context.request.join.table,
                context.right_table,
                context.foreign_fields,
            )
        })
    } else {
        None
    };
    if matches!(
        join_strategy::choose_with_costs(
            context.left_records.len(),
            context.right_records.len(),
            right_index.as_ref().and_then(|index| {
                join_index::equality_probe_cost(
                    index,
                    context.right_records.len(),
                    context.foreign_fields,
                )
            }),
            None,
            context.cost_input,
        ),
        join_strategy::JoinStrategy::IndexNestedLoop
    ) {
        if let Some(index) = right_index.as_ref() {
            if let Some(output) = join_index::execute_join(
                context.left_records,
                context.right_records,
                context.request,
                index,
                context.local_fields,
                context.foreign_fields,
            )? {
                return Ok(output);
            }
        }
    }

    if matches!(
        join_strategy::choose(
            context.left_records.len(),
            context.right_records.len(),
            false,
        ),
        join_strategy::JoinStrategy::NestedLoop
    ) {
        return join_nested::execute_join(
            context.left_records,
            context.right_records,
            context.request,
            context.local_fields,
            context.foreign_fields,
        );
    }

    let mut right_by_key = BTreeMap::<String, Vec<&DbfRecord>>::new();
    for record in context.right_records {
        let Some(key) = encoded_key(&record.values, context.foreign_fields)? else {
            continue;
        };
        right_by_key.entry(key).or_default().push(record);
    }

    let mut output = Vec::new();
    for left_record in context.left_records {
        let matches = encoded_key(&left_record.values, context.local_fields)?
            .and_then(|key| right_by_key.get(&key));
        let had_matches = matches.is_some_and(|records| !records.is_empty());
        match &context.request.join.kind {
            JoinType::Inner => {
                if let Some(matches) = matches {
                    for right_record in matches {
                        emit(
                            &mut output,
                            context.request,
                            Some(left_record),
                            Some(right_record),
                        )?;
                    }
                }
            }
            JoinType::Left => {
                if let Some(matches) = matches {
                    for right_record in matches {
                        emit(
                            &mut output,
                            context.request,
                            Some(left_record),
                            Some(right_record),
                        )?;
                    }
                }
                if !had_matches {
                    emit(&mut output, context.request, Some(left_record), None)?;
                }
            }
            JoinType::Semi if had_matches => {
                emit(&mut output, context.request, Some(left_record), None)?
            }
            JoinType::Anti if !had_matches => {
                emit(&mut output, context.request, Some(left_record), None)?
            }
            JoinType::Semi | JoinType::Anti => {}
            JoinType::Right | JoinType::Full | JoinType::Cross => {
                unreachable!("join type handled above")
            }
        }
    }
    Ok(output)
}
