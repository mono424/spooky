use serde::{Deserialize, Serialize};

/// A dot-separated field path for accessing nested values in SpookyValue.
///
/// Example: `Path::new("address.city")` represents the path `["address", "city"]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Path(pub Vec<String>);

impl Path {
    pub fn new(s: &str) -> Self {
        if s.is_empty() {
            Path(vec![])
        } else {
            Path(s.split('.').map(String::from).collect())
        }
    }

    pub fn as_str(&self) -> String {
        self.0.join(".")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

impl Serialize for Path {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        Ok(Path::new(&s))
    }
}
