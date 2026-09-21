use super::IndexError;
use serde::de::Error as DeError;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexDefinition {
    pub(super) name: String,
    pub(super) fields: Vec<String>,
    pub(super) directions: Vec<i8>,
}

impl Serialize for IndexDefinition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let has_directions = self.directions.iter().any(|direction| *direction != 1);
        let mut state =
            serializer.serialize_struct("IndexDefinition", if has_directions { 3 } else { 2 })?;
        state.serialize_field("name", &self.name)?;
        if self.fields.len() == 1 {
            state.serialize_field("field", &self.fields[0])?;
        } else {
            state.serialize_field("fields", &self.fields)?;
        }
        if has_directions {
            state.serialize_field("directions", &self.directions)?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for IndexDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            name: String,
            #[serde(default)]
            field: Option<String>,
            #[serde(default)]
            fields: Option<Vec<String>>,
            #[serde(default)]
            directions: Option<Vec<i8>>,
        }

        let wire = Wire::deserialize(deserializer)?;
        let fields = match (wire.field, wire.fields) {
            (Some(_), Some(_)) => {
                return Err(DeError::custom(
                    "index definition has both field and fields",
                ));
            }
            (Some(field), None) => vec![field],
            (None, Some(fields)) => fields,
            (None, None) => {
                return Err(DeError::custom("index definition requires field or fields"));
            }
        };
        let directions = wire.directions.unwrap_or_else(|| vec![1; fields.len()]);
        if directions.len() != fields.len() {
            return Err(DeError::custom(
                "index definition directions must match field count",
            ));
        }
        if directions
            .iter()
            .any(|direction| !matches!(direction, -1 | 1))
        {
            return Err(DeError::custom(
                "index definition directions must be 1 or -1",
            ));
        }
        Ok(Self {
            name: wire.name,
            fields,
            directions,
        })
    }
}

impl IndexDefinition {
    pub fn for_field(field: impl Into<String>) -> Self {
        let field = field.into();
        Self {
            name: field.clone(),
            fields: vec![field],
            directions: vec![1],
        }
    }

    pub fn named(name: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: vec![field.into()],
            directions: vec![1],
        }
    }

    pub fn named_fields(name: impl Into<String>, fields: Vec<String>) -> Self {
        Self {
            name: name.into(),
            directions: vec![1; fields.len()],
            fields,
        }
    }

    pub fn named_fields_with_directions(
        name: impl Into<String>,
        fields: Vec<String>,
        directions: Vec<i8>,
    ) -> Self {
        Self {
            name: name.into(),
            fields,
            directions,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn field(&self) -> &str {
        self.fields.first().map(String::as_str).unwrap_or_default()
    }

    pub fn fields(&self) -> &[String] {
        &self.fields
    }

    pub fn directions(&self) -> &[i8] {
        &self.directions
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum IndexKey {
    Missing,
    Null,
    Scalar(Value),
    Compound(Vec<Self>),
}

impl IndexKey {
    pub(super) fn from_value(value: Option<&Value>) -> Result<Self, IndexError> {
        let Some(value) = value else {
            return Ok(Self::Missing);
        };
        match value {
            Value::Null => Ok(Self::Null),
            Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(Self::Scalar(value.clone())),
            Value::Array(_) | Value::Object(_) => Err(IndexError::Invalid(
                "only scalar values can be indexed".into(),
            )),
        }
    }

    pub(super) fn from_values(values: Vec<Option<&Value>>) -> Result<Self, IndexError> {
        if values.len() == 1 {
            return Self::from_value(values[0]);
        }
        Ok(Self::Compound(
            values
                .into_iter()
                .map(Self::from_value)
                .collect::<Result<_, _>>()?,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexEntry {
    pub(super) key: IndexKey,
    pub(super) records: Vec<usize>,
}

impl IndexEntry {
    pub fn key(&self) -> &IndexKey {
        &self.key
    }

    pub fn records(&self) -> &[usize] {
        &self.records
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecondaryIndex {
    pub(super) definition: IndexDefinition,
    pub(super) entries: Vec<IndexEntry>,
}

impl SecondaryIndex {
    pub fn definition(&self) -> &IndexDefinition {
        &self.definition
    }

    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileFingerprint {
    pub(super) length: u64,
    pub(super) hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceFingerprint {
    pub(super) dbf: FileFingerprint,
    pub(super) memo: Option<FileFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexFile {
    pub(super) format: String,
    pub(super) version: u8,
    pub(super) source: SourceFingerprint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) statistics: Option<super::statistics::CollectionStatistics>,
    pub(super) indexes: Vec<SecondaryIndex>,
}
