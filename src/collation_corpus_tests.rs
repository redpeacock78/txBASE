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

fn assert_expected_order(name: &str, collation: Collation, input: &str) -> (usize, usize) {
    let mut previous: Option<&str> = None;
    let mut count = 0;
    let mut sequences = 0;
    let mut in_sequence = false;

    for line in input.lines() {
        let sequence = line.split('#').next().unwrap_or("").trim();
        if sequence.is_empty() {
            previous = None;
            in_sequence = false;
            continue;
        }
        if !in_sequence {
            sequences += 1;
            in_sequence = true;
        }

        for (start, character) in sequence.char_indices() {
            if character.is_whitespace() {
                continue;
            }
            let current = &sequence[start..start + character.len_utf8()];
            if let Some(previous) = previous {
                assert_eq!(
                    collation.compare(previous, current),
                    Ordering::Less,
                    "{name}: {:?} must sort before {:?}",
                    previous,
                    current
                );
            }
            previous = Some(current);
            count += 1;
        }
    }

    assert!(count > 1, "{name}: expected-order fixture is too small");
    (count, sequences)
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

#[test]
fn cjk_locale_collators_follow_cldr_48_expected_order_sentinels() {
    for (name, collation, input) in [
        (
            "Japanese",
            Collation::Icu4x211Ja,
            include_str!("../tests/fixtures/collation/cldr48-ja.txt"),
        ),
        (
            "Chinese",
            Collation::Icu4x211Zh,
            include_str!("../tests/fixtures/collation/cldr48-zh.txt"),
        ),
        (
            "Korean",
            Collation::Icu4x211Ko,
            include_str!("../tests/fixtures/collation/cldr48-ko.txt"),
        ),
    ] {
        assert_expected_order(name, collation, input);
    }
}

#[test]
fn cjk_locale_collators_follow_cldr_48_starred_ordered_relations() {
    for (name, collation, input, expected) in [
        (
            "Japanese",
            Collation::Icu4x211Ja,
            include_str!("../tests/fixtures/collation/cldr48-ja-starred.txt"),
            (6_361, 2),
        ),
        (
            "Chinese",
            Collation::Icu4x211Zh,
            include_str!("../tests/fixtures/collation/cldr48-zh-pinyin-long.txt"),
            (44_470, 1),
        ),
        (
            "Korean",
            Collation::Icu4x211Ko,
            include_str!("../tests/fixtures/collation/cldr48-ko-starred.txt"),
            (7_871, 436),
        ),
    ] {
        let actual = assert_expected_order(name, collation, input);
        assert_eq!(
            actual, expected,
            "{name}: unexpected CLDR 48 corpus size or reset boundaries"
        );
    }
}
