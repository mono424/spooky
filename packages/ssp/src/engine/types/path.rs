use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Path(pub Vec<SmolStr>);

impl Path {
    pub fn new(s: &str) -> Self {
        if s.is_empty() {
            Path(vec![])
        } else {
            Path(s.split('.').map(SmolStr::new).collect())
        }
    }

    pub fn as_str(&self) -> String {
        self.0
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn segments(&self) -> &[SmolStr] {
        &self.0
    }
}

impl Serialize for Path {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.0.is_empty() {
            serializer.serialize_str("")
        } else {
            serializer.serialize_str(&self.as_str())
        }
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_path_new_empty() {
        let path = Path::new("");
        assert_eq!(path, Path(vec![]));
    }

    #[test]
    fn test_path_new_single() {
        let path = Path::new("id");
        assert_eq!(path, Path(vec!["id".into()]));
    }

    #[test]
    fn test_path_new_nested() {
        let path = Path::new("a.b.c");
        let res: Vec<SmolStr> = vec!["a", "b", "c"]
            .into_iter()
            .map(|v| SmolStr::new(v))
            .collect();
        assert_eq!(path, Path(res));
    }

    #[test]
    fn test_path_as_str() {
        let path = Path::new("a.b");
        let path_as_str = path.as_str();
        assert_eq!(path_as_str, "a.b");
    }

    #[test]
    fn test_path_is_empty() {
        let path = Path(vec![]);
        assert!(path.is_empty());
    }

    #[test]
    fn test_path_is_not_empty() {
        let path = Path::new("id");
        assert!(!path.is_empty());
    }

    #[test]
    fn test_path_segments() {
        let path = Path::new("user.name");
        let res: &[SmolStr] = &["user".into(), "name".into()];
        assert_eq!(path.segments(), res);
    }

    #[test]
    fn test_path_serialize_deserialize_empty() {
        let path = Path::new("");
        let json_str = serde_json::to_string(&path).unwrap();
        assert_eq!(json_str, "\"\"");
        let restored: Path = serde_json::from_str(&json_str).unwrap();
        assert!(restored.is_empty());
    }

    #[test]
    fn test_path_serialize_deserialize() {
        let path = Path::new("payload.record.id");
        let json_str = serde_json::to_string(&path).unwrap();
        assert_eq!(json_str, "\"payload.record.id\"");

        let restored: Path = serde_json::from_str(&json_str).unwrap();

        assert_eq!(restored.as_str(), "payload.record.id");
        assert_eq!(restored.0.len(), 3);
        assert_eq!(restored.0[0], "payload");
        assert_eq!(restored.0[1], "record");
        assert_eq!(restored.0[2], "id");
    }
}
