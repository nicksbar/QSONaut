//! External, packaged contest definitions for the QSONaut desktop client.
//!
//! The JSON catalog is intentionally reviewable and editable without changing
//! Rust code. The server catalog remains authoritative when connected.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

pub mod rules;

pub const CATALOG_VERSION: u32 = 1;
const CATALOG: &str = include_str!("../assets/contest-catalog.json");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDefinition {
    pub key: String,
    pub label: String,
    pub description: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContestDefinition {
    pub id: String,
    #[serde(rename = "contestType")]
    pub contest_type: String,
    pub name: String,
    pub description: String,
    pub organization: String,
    pub icon: String,
    #[serde(rename = "rulesUrl")]
    pub rules_url: String,
    pub formula: String,
    #[serde(rename = "pointsPerQso")]
    pub points_per_qso: u8,
    pub multiplier: Option<String>,
    pub fields: Vec<FieldDefinition>,
    pub bands: Vec<String>,
    pub modes: Vec<String>,
    #[serde(rename = "duplicateRule")]
    pub duplicate_rule: String,
    pub exchange: Vec<String>,
    pub schedule: String,
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    #[serde(rename = "catalogVersion")]
    catalog_version: u32,
    contests: Vec<ContestDefinition>,
}

fn catalog() -> &'static CatalogFile {
    static PARSED: OnceLock<CatalogFile> = OnceLock::new();
    PARSED.get_or_init(|| {
        let catalog: CatalogFile =
            serde_json::from_str(CATALOG).expect("packaged contest catalog must be valid JSON");
        assert_eq!(
            catalog.catalog_version, CATALOG_VERSION,
            "packaged contest catalog version must match client"
        );
        catalog
    })
}

#[must_use]
pub fn builtin_definitions() -> &'static [ContestDefinition] {
    &catalog().contests
}

#[must_use]
pub fn find(contest_type: &str) -> Option<&'static ContestDefinition> {
    builtin_definitions()
        .iter()
        .find(|definition| definition.contest_type.eq_ignore_ascii_case(contest_type))
}

#[must_use]
pub fn as_json(definition: &ContestDefinition) -> Value {
    serde_json::to_value(definition).expect("contest definition is serializable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn packaged_catalog_matches_server_catalog_shape() {
        assert_eq!(builtin_definitions().len(), 19);
        assert_eq!(
            builtin_definitions()
                .iter()
                .map(|item| &item.id)
                .collect::<HashSet<_>>()
                .len(),
            19
        );
        assert_eq!(
            find("ARRL_FD")
                .unwrap()
                .fields
                .iter()
                .map(|field| field.key.as_str())
                .collect::<Vec<_>>(),
            ["class", "section", "power"]
        );
    }

    #[test]
    fn field_day_definition_contains_rules_and_exchange() {
        let field_day = find("ARRL_FD").unwrap();
        assert!(field_day.rules_url.starts_with("https://"));
        assert_eq!(field_day.duplicate_rule, "band-mode");
        assert_eq!(field_day.exchange, ["class", "section"]);
        assert_eq!(field_day.fields[0].options[0], "1A");
    }
}
