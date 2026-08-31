use crate::egui::{Color32, RichText};
use qsonaut_radio::models::{find_model, Manufacturer, Protocol};

pub(super) struct RadioHelp {
    pub title: &'static str,
    pub blurb: &'static str,
    pub manufacturer_faq: &'static str,
    pub manufacturer_guide: &'static str,
    pub model_faq: &'static str,
    pub model_guide: &'static str,
}

const ICOM_FAQ: &str = include_str!("../../../docs/radios/manufacturers/icom/faq.md");
const ICOM_GUIDE: &str = include_str!("../../../docs/radios/manufacturers/icom/guide.md");
const IC7300_FAQ: &str = include_str!("../../../docs/radios/models/ic-7300/faq.md");
const IC7300_GUIDE: &str = include_str!("../../../docs/radios/models/ic-7300/guide.md");
const YAESU_CLASSIC_FAQ: &str =
    include_str!("../../../docs/radios/manufacturers/yaesu-classic/faq.md");
const YAESU_CLASSIC_GUIDE: &str =
    include_str!("../../../docs/radios/manufacturers/yaesu-classic/guide.md");
const YAESU_FAQ: &str = include_str!("../../../docs/radios/manufacturers/yaesu/faq.md");
const YAESU_GUIDE: &str = include_str!("../../../docs/radios/manufacturers/yaesu/guide.md");
const KENWOOD_FAQ: &str = include_str!("../../../docs/radios/manufacturers/kenwood/faq.md");
const KENWOOD_GUIDE: &str = include_str!("../../../docs/radios/manufacturers/kenwood/guide.md");

macro_rules! model_docs {
    ($slug:literal) => {
        (
            include_str!(concat!("../../../docs/radios/models/", $slug, "/faq.md")),
            include_str!(concat!("../../../docs/radios/models/", $slug, "/guide.md")),
        )
    };
}

fn model_docs_for(model: &str) -> (&'static str, &'static str) {
    macro_rules! choose {
        ($($name:literal => $slug:literal),+ $(,)?) => {
            $(if model.eq_ignore_ascii_case($name) { return model_docs!($slug); })+
        };
    }
    choose!(
        "CI-V (generic)" => "ci-v-generic", "IC-7300" => "ic-7300", "IC-705" => "ic-705",
        "IC-7610" => "ic-7610", "IC-9700" => "ic-9700", "classic CAT (generic)" => "yaesu-classic-generic",
        "FT-817ND" => "ft-817nd", "FT-818" => "ft-818", "FT-857D" => "ft-857d", "FT-897D" => "ft-897d",
        "CAT (generic)" => "yaesu-generic", "FT-710" => "ft-710", "FTDX10" => "ftdx10",
        "FTDX101D" => "ftdx101d", "FTDX101MP" => "ftdx101mp", "FT-991A" => "ft-991a",
        "PC control (generic)" => "kenwood-generic", "TS-590SG" => "ts-590sg", "TS-890S" => "ts-890s",
        "TS-2000" => "ts-2000",
    );
    model_docs!("unknown")
}

pub(super) fn help_for_model(model: &str) -> RadioHelp {
    let model = model.trim();
    let catalog_profile = find_model(model);
    let (title, blurb, manufacturer_faq, manufacturer_guide, model_faq, model_guide) = if model
        .eq_ignore_ascii_case("IC-7300")
    {
        ("Icom IC-7300", "Start with the USB CI-V device and Auto baud; it is the normal recommendation. For the scope, the radio must be able to emit scope data, the scope must be visible, and 115200 may be required by the radio firmware.", ICOM_FAQ, ICOM_GUIDE, IC7300_FAQ, IC7300_GUIDE)
    } else if catalog_profile.is_some_and(|profile| profile.manufacturer == Manufacturer::Icom)
        || model.eq_ignore_ascii_case("CI-V (generic)")
    {
        let (faq, guide) = model_docs_for(model);
        ("Icom CI-V", "Start with the radio's USB/serial CI-V device and Auto baud. Select the exact model when available; use a fixed matching baud only when the radio or a troubleshooting step calls for it.", ICOM_FAQ, ICOM_GUIDE, faq, guide)
    } else if catalog_profile.is_some_and(|profile| {
        profile.manufacturer == Manufacturer::Yaesu && profile.protocol == Protocol::YaesuLegacyCat
    }) || model.eq_ignore_ascii_case("classic CAT (generic)")
    {
        let (faq, guide) = model_docs_for(model);
        ("Yaesu classic CAT", "Select the CAT device and start with Auto baud when supported. If CAT is unreliable, choose a fixed rate supported by the radio and match it in QSONaut.", YAESU_CLASSIC_FAQ, YAESU_CLASSIC_GUIDE, faq, guide)
    } else if catalog_profile.is_some_and(|profile| {
        profile.manufacturer == Manufacturer::Yaesu && profile.protocol == Protocol::YaesuCat
    }) || model.eq_ignore_ascii_case("CAT (generic)")
    {
        let (faq, guide) = model_docs_for(model);
        ("Yaesu CAT", "Select the radio's CAT/USB device and start with Auto baud. Select the exact model when available; use a fixed matching rate if CAT needs troubleshooting.", YAESU_FAQ, YAESU_GUIDE, faq, guide)
    } else if catalog_profile.is_some_and(|profile| profile.manufacturer == Manufacturer::Kenwood)
        || model.eq_ignore_ascii_case("PC control (generic)")
    {
        let (faq, guide) = model_docs_for(model);
        ("Kenwood PC control", "Select the PC control device and start with Auto baud when supported. Select the exact model when available; use a fixed matching rate if CAT needs troubleshooting.", KENWOOD_FAQ, KENWOOD_GUIDE, faq, guide)
    } else {
        let (faq, guide) = model_docs!("unknown");
        ("Radio connection", "Select the exact radio model when available, choose its USB/serial device, and start with Auto baud. Switch to a fixed matching rate only when needed.", ICOM_FAQ, ICOM_GUIDE, faq, guide)
    };
    RadioHelp {
        title,
        blurb,
        manufacturer_faq,
        manufacturer_guide,
        model_faq,
        model_guide,
    }
}

pub(super) fn render_document(ui: &mut crate::egui::Ui, markdown: &str) {
    for raw_line in markdown.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            ui.add_space(4.0);
        } else if let Some(heading) = line.strip_prefix("# ") {
            ui.heading(heading);
        } else if let Some(heading) = line.strip_prefix("## ") {
            ui.add_space(6.0);
            ui.label(RichText::new(heading).strong());
        } else if let Some(heading) = line.strip_prefix("### ") {
            ui.add_space(4.0);
            ui.label(RichText::new(heading).strong());
        } else if let Some(item) = line.strip_prefix("- ") {
            ui.horizontal_wrapped(|ui| {
                ui.label("•");
                ui.label(item);
            });
        } else {
            ui.add(
                crate::egui::Label::new(RichText::new(line).color(Color32::from_gray(190))).wrap(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_catalog_choice_has_help() {
        for model in [
            "CI-V (generic)",
            "IC-7300",
            "IC-705",
            "IC-7610",
            "IC-9700",
            "classic CAT (generic)",
            "FT-817ND",
            "FT-818",
            "FT-857D",
            "FT-897D",
            "CAT (generic)",
            "FT-710",
            "FTDX10",
            "FTDX101D",
            "FTDX101MP",
            "FT-991A",
            "PC control (generic)",
            "TS-590SG",
            "TS-890S",
            "TS-2000",
        ] {
            let help = help_for_model(model);
            assert!(!help.blurb.is_empty());
            assert!(!help.manufacturer_faq.is_empty());
            assert!(!help.manufacturer_guide.is_empty());
            assert!(!help.model_faq.is_empty());
            assert!(!help.model_guide.is_empty());
        }
    }
}
