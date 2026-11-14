//! How agents refer to lists and tags: by name, or by numeric ID.

use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NameOrId {
    Id(i64),
    Name(String),
}

/// Join names for an error message, capped so a huge store can't flood context.
pub fn summarize(names: &[String], max: usize) -> String {
    let shown: Vec<String> = names.iter().take(max).map(|n| format!("'{n}'")).collect();
    let mut out = shown.join(", ");
    if names.len() > max {
        out.push_str(&format!(", and {} more", names.len() - max));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_caps_long_lists() {
        let names: Vec<String> = (1..=5).map(|i| format!("n{i}")).collect();
        assert_eq!(summarize(&names, 3), "'n1', 'n2', 'n3', and 2 more");
        assert_eq!(summarize(&names[..2], 3), "'n1', 'n2'");
    }

    #[test]
    fn deserializes_numbers_and_strings() {
        let id: NameOrId = serde_json::from_str("12").unwrap();
        let name: NameOrId = serde_json::from_str("\"Groceries\"").unwrap();
        assert_eq!(id, NameOrId::Id(12));
        assert_eq!(name, NameOrId::Name("Groceries".into()));
    }
}
