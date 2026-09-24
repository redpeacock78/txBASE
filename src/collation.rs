use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// Bounded, locale-independent sort-key transformations shared by queries and indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Collation {
    UnicodeLowercase,
    UnicodeNfkcLowercase,
}

impl Collation {
    pub(crate) fn key(self, value: &str) -> String {
        match self {
            Self::UnicodeLowercase => value.chars().flat_map(char::to_lowercase).collect(),
            Self::UnicodeNfkcLowercase => value.nfkc().flat_map(char::to_lowercase).collect(),
        }
    }
}
