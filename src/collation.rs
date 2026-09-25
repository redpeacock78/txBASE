use icu_collator::{Collator, CollatorBorrowed, options::CollatorOptions};
use icu_locale::locale;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

/// Versioned sort collations shared by queries and indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Collation {
    UnicodeLowercase,
    UnicodeNfkcLowercase,
    #[serde(rename = "icu4x-2.1.1-ja")]
    Icu4x211Ja,
    #[serde(rename = "icu4x-2.1.1-zh")]
    Icu4x211Zh,
    #[serde(rename = "icu4x-2.1.1-ko")]
    Icu4x211Ko,
}

impl Collation {
    pub(crate) fn key(self, value: &str) -> String {
        match self {
            Self::UnicodeLowercase => value.chars().flat_map(char::to_lowercase).collect(),
            Self::UnicodeNfkcLowercase => value.nfkc().flat_map(char::to_lowercase).collect(),
            Self::Icu4x211Ja | Self::Icu4x211Zh | Self::Icu4x211Ko => value.to_owned(),
        }
    }

    pub(crate) fn compare(self, left: &str, right: &str) -> Ordering {
        match self {
            Self::UnicodeLowercase | Self::UnicodeNfkcLowercase => {
                self.key(left).cmp(&self.key(right))
            }
            Self::Icu4x211Ja => japanese_collator().compare(left, right),
            Self::Icu4x211Zh => chinese_collator().compare(left, right),
            Self::Icu4x211Ko => korean_collator().compare(left, right),
        }
    }
}

fn japanese_collator() -> &'static CollatorBorrowed<'static> {
    static COLLATOR: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();
    COLLATOR.get_or_init(|| {
        Collator::try_new(locale!("ja").into(), CollatorOptions::default())
            .expect("ICU4X compiled Japanese collation data")
    })
}

fn chinese_collator() -> &'static CollatorBorrowed<'static> {
    static COLLATOR: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();
    COLLATOR.get_or_init(|| {
        Collator::try_new(locale!("zh").into(), CollatorOptions::default())
            .expect("ICU4X compiled Chinese collation data")
    })
}

fn korean_collator() -> &'static CollatorBorrowed<'static> {
    static COLLATOR: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();
    COLLATOR.get_or_init(|| {
        Collator::try_new(locale!("ko").into(), CollatorOptions::default())
            .expect("ICU4X compiled Korean collation data")
    })
}

#[cfg(test)]
mod tests {
    use super::Collation;
    use std::cmp::Ordering;

    #[test]
    fn locale_collations_use_icu4x_211_defaults() {
        assert_eq!(Collation::Icu4x211Ja.compare("あ", "い"), Ordering::Less);
        assert_eq!(Collation::Icu4x211Zh.compare("阿", "中"), Ordering::Less);
        assert_eq!(Collation::Icu4x211Ko.compare("가", "나"), Ordering::Less);
        assert_eq!(Collation::Icu4x211Zh.key("阿"), "阿");
    }
}
