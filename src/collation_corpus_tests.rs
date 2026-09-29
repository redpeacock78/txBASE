use super::Collation;
use std::cmp::Ordering;

fn assert_total_preorder(name: &str, collation: Collation, input: &str) {
    let values = input
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 50, "unexpected {name} corpus size");

    let comparisons = values
        .iter()
        .map(|left| {
            values
                .iter()
                .map(|right| collation.compare(left, right))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    for left in 0..values.len() {
        assert_eq!(
            comparisons[left][left],
            Ordering::Equal,
            "{name}: reflexivity"
        );
        for right in 0..values.len() {
            assert_eq!(
                comparisons[left][right],
                comparisons[right][left].reverse(),
                "{name}: comparison reversal symmetry for {:?} and {:?}",
                values[left],
                values[right]
            );
            for third in 0..values.len() {
                if comparisons[left][right] != Ordering::Greater
                    && comparisons[right][third] != Ordering::Greater
                {
                    assert_ne!(
                        comparisons[left][third],
                        Ordering::Greater,
                        "{name}: transitivity for {:?}, {:?}, and {:?}",
                        values[left],
                        values[right],
                        values[third]
                    );
                }
                if comparisons[left][right] == Ordering::Equal {
                    assert_eq!(
                        comparisons[left][third], comparisons[right][third],
                        "{name}: equal values must compare identically to {:?}",
                        values[third]
                    );
                }
            }
        }
    }
}

#[test]
fn cjk_locale_collators_obey_total_preorder_for_icu4x_benchmark_inputs() {
    for (name, collation, input) in [
        (
            "Chinese",
            Collation::Icu4x211Zh,
            include_str!("../tests/fixtures/collation/icu4x/TestNames_Chinese.txt"),
        ),
        (
            "Japanese Hiragana",
            Collation::Icu4x211Ja,
            include_str!("../tests/fixtures/collation/icu4x/TestNames_Japanese_h.txt"),
        ),
        (
            "Japanese Katakana",
            Collation::Icu4x211Ja,
            include_str!("../tests/fixtures/collation/icu4x/TestNames_Japanese_k.txt"),
        ),
        (
            "Korean",
            Collation::Icu4x211Ko,
            include_str!("../tests/fixtures/collation/icu4x/TestNames_Korean.txt"),
        ),
    ] {
        assert_total_preorder(name, collation, input);
    }
}
