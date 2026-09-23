use super::super::super::super::{join_index, join_nested, join_strategy};
use super::super::super::JoinError;
use super::super::{emit, encoded_key};
use super::DirectJoinContext;
use crate::dbf::DbfRecord;
use std::collections::BTreeMap;

pub(super) fn execute(
    context: &DirectJoinContext<'_>,
) -> Result<Vec<serde_json::Value>, JoinError> {
    let left_index = if context.large_join {
        context.current_catalog.and_then(|catalog| {
            join_index::load_fields(catalog, &context.request.from, context.local_fields)
        })
    } else {
        None
    };
    if matches!(
        join_strategy::choose_with_costs(
            context.right_records.len(),
            context.left_records.len(),
            left_index.as_ref().and_then(|index| {
                join_index::equality_probe_cost(
                    index,
                    context.left_records.len(),
                    context.local_fields,
                )
            }),
            None,
            context.cost_input,
        ),
        join_strategy::JoinStrategy::IndexNestedLoop
    ) {
        if let Some(index) = left_index.as_ref() {
            if let Some(output) = join_index::execute_right_join(
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
        return join_nested::execute_right_join(
            context.left_records,
            context.right_records,
            context.request,
            context.local_fields,
            context.foreign_fields,
        );
    }

    let mut left_by_key = BTreeMap::<String, Vec<&DbfRecord>>::new();
    for &record in context.left_records {
        let Some(key) = encoded_key(&record.values, context.local_fields)? else {
            continue;
        };
        left_by_key.entry(key).or_default().push(record);
    }

    let mut output = Vec::new();
    for &right_record in context.right_records {
        let matches = encoded_key(&right_record.values, context.foreign_fields)?
            .and_then(|key| left_by_key.get(&key));
        if let Some(matches) = matches {
            for left_record in matches {
                emit(
                    &mut output,
                    context.request,
                    Some(left_record),
                    Some(right_record),
                )?;
            }
        } else {
            emit(&mut output, context.request, None, Some(right_record))?;
        }
    }
    Ok(output)
}
