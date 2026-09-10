//! Catalog-driven local validation. Scores remain provisional until a supported
//! scoring strategy and an event configuration are available.

use crate::ContestDefinition;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateRule {
    None,
    Band,
    BandMode,
}

impl DuplicateRule {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "none" => Ok(Self::None),
            "band" => Ok(Self::Band),
            "band-mode" => Ok(Self::BandMode),
            _ => Err(format!("Unsupported duplicate rule: {value}")),
        }
    }

    #[must_use]
    pub fn matches(self, band: &str, mode: &str, previous_band: &str, previous_mode: &str) -> bool {
        match self {
            Self::None => false,
            Self::Band => normalized_band(band) == normalized_band(previous_band),
            Self::BandMode => {
                normalized_band(band) == normalized_band(previous_band)
                    && mode_category(mode) == mode_category(previous_mode)
            }
        }
    }
}

#[must_use]
pub fn normalized_band(value: &str) -> String {
    let band = value.trim().to_ascii_uppercase();
    if band.ends_with("CM") {
        band
    } else {
        band.strip_suffix('M').unwrap_or(&band).to_owned()
    }
}

#[must_use]
pub fn mode_category(value: &str) -> String {
    match value.trim().to_ascii_uppercase().as_str() {
        "SSB" | "USB" | "LSB" | "AM" | "FM" | "VOICE" | "PHONE" => "PHONE".into(),
        "FT8" | "FT4" | "RTTY" | "PSK31" | "PSK63" | "JS8" | "JT65" | "JT9" | "Q65" | "FST4"
        | "MSK144" | "DIGITAL" => "DIGITAL".into(),
        other => other.to_owned(),
    }
}

fn value_for<'a>(values: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    values
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

impl ContestDefinition {
    #[must_use]
    pub fn validate_setup(&self, values: &BTreeMap<String, String>) -> Vec<String> {
        self.fields
            .iter()
            .filter_map(|field| match value_for(values, &field.key) {
                None => Some(format!("{} is required", field.label)),
                Some(value)
                    if !field.options.is_empty()
                        && !field
                            .options
                            .iter()
                            .any(|option| option.eq_ignore_ascii_case(value)) =>
                {
                    Some(format!(
                        "{} must be one of: {}",
                        field.label,
                        field.options.join(", ")
                    ))
                }
                Some(_) => None,
            })
            .collect()
    }

    #[must_use]
    pub fn validate_exchange(
        &self,
        values: &BTreeMap<String, String>,
        direction: &str,
    ) -> Vec<String> {
        self.exchange
            .iter()
            .filter(|key| value_for(values, key).is_none())
            .map(|key| format!("{direction} {key} is required"))
            .collect()
    }

    #[must_use]
    pub fn validate_band_mode(&self, band: &str, mode: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if !self
            .bands
            .iter()
            .any(|allowed| normalized_band(allowed) == normalized_band(band))
        {
            errors.push(format!("Band {band} is not allowed in {}", self.name));
        }
        let mode = mode.trim().to_ascii_uppercase();
        let category = mode_category(&mode);
        if !self.modes.iter().any(|allowed| {
            allowed.eq_ignore_ascii_case(&mode)
                || allowed.eq_ignore_ascii_case(&category)
                || (allowed == "DIGITAL_NO_RTTY" && category == "DIGITAL" && mode != "RTTY")
                || (allowed == "MIXED" && matches!(category.as_str(), "CW" | "PHONE" | "DIGITAL"))
        }) {
            errors.push(format!("Mode {mode} is not allowed in {}", self.name));
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_and_mode_rules_use_operator_and_catalog_notation() {
        let fd = crate::find("ARRL_FD").unwrap();
        for mode in ["CW", "SSB", "USB", "FT8", "FT4"] {
            assert!(fd.validate_band_mode("20m", mode).is_empty());
        }
        assert!(fd.validate_band_mode("70cm", "FM").is_empty());
        assert_eq!(fd.validate_band_mode("30m", "WSPR").len(), 2);
        let digital = crate::find("ARRL_INTL_DIGITAL").unwrap();
        assert!(digital.validate_band_mode("20M", "ft8").is_empty());
        assert!(!digital.validate_band_mode("20m", "RTTY").is_empty());
    }

    #[test]
    fn setup_and_exchange_are_distinct_and_case_insensitive() {
        let fd = crate::find("ARRL_FD").unwrap();
        let values = BTreeMap::from([
            ("CLASS".into(), "1a".into()),
            ("SECTION".into(), "WMA".into()),
        ]);
        assert!(fd.validate_exchange(&values, "Received").is_empty());
        assert_eq!(fd.validate_setup(&values), ["Power is required"]);
        let mut values = values;
        values.insert("power".into(), "invalid".into());
        assert!(fd.validate_setup(&values)[0].contains("must be one of"));
        values.insert("power".into(), " QRP ".into());
        assert!(fd.validate_setup(&values).is_empty());
        values.insert("SECTION".into(), " ".into());
        assert_eq!(
            fd.validate_exchange(&values, "Received"),
            ["Received section is required"]
        );
    }

    #[test]
    fn duplicates_follow_definition_and_group_phone_and_digital_modes() {
        assert!(!DuplicateRule::None.matches("20m", "CW", "20", "CW"));
        assert!(DuplicateRule::Band.matches("20m", "CW", "20", "SSB"));
        assert!(!DuplicateRule::BandMode.matches("20m", "CW", "20", "SSB"));
        assert!(DuplicateRule::BandMode.matches("20m", "FT8", "20", "FT4"));
        assert!(DuplicateRule::BandMode.matches("20m", "USB", "20", "SSB"));
        assert!(!DuplicateRule::Band.matches("40m", "CW", "20", "CW"));
        for definition in crate::builtin_definitions() {
            assert!(DuplicateRule::parse(&definition.duplicate_rule).is_ok());
        }
        assert!(DuplicateRule::parse("unknown").is_err());
    }
}
