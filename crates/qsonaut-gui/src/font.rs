use std::sync::Arc;

use eframe::egui;
use font_kit::family_name::FamilyName;
use font_kit::properties::Properties;
use font_kit::source::SystemSource;

const SYSTEM_FONT_ID: &str = "qsonaut-system-font";

pub(super) fn available_font_families() -> Vec<String> {
    let mut families = SystemSource::new().all_families().unwrap_or_default();
    families.sort_by_key(|family| family.to_ascii_lowercase());
    families.dedup();
    families
}

pub(super) fn apply_font_family(ctx: &egui::Context, family: Option<&str>) -> bool {
    let source = SystemSource::new();
    let requested = family
        .filter(|family| !family.trim().is_empty())
        .map(|family| FamilyName::Title(family.to_string()))
        .unwrap_or(FamilyName::SansSerif);
    let Ok(handle) = source.select_best_match(&[requested], &Properties::new()) else {
        return false;
    };
    let Ok(font) = handle.load() else {
        return false;
    };
    let Some(font_data) = font.copy_font_data() else {
        return false;
    };

    let mut definitions = egui::FontDefinitions::default();
    definitions.font_data.insert(
        SYSTEM_FONT_ID.to_owned(),
        Arc::new(egui::FontData::from_owned((*font_data).clone())),
    );
    if let Some(family_fonts) = definitions
        .families
        .get_mut(&egui::FontFamily::Proportional)
    {
        family_fonts.insert(0, SYSTEM_FONT_ID.to_owned());
    }
    if let Some(family_fonts) = definitions.families.get_mut(&egui::FontFamily::Monospace) {
        family_fonts.push(SYSTEM_FONT_ID.to_owned());
    }
    ctx.set_fonts(definitions);
    true
}
