use super::super::*;

/// Keep warning text readable in both egui visual modes.
pub(crate) fn theme_warning(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::YELLOW
    } else {
        Color32::from_rgb(146, 92, 0)
    }
}

pub(crate) fn theme_muted(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::GRAY
    } else {
        Color32::from_rgb(75, 85, 99)
    }
}

pub(crate) fn theme_accent(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::LIGHT_BLUE
    } else {
        Color32::from_rgb(29, 78, 121)
    }
}

pub(crate) fn theme_success(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::LIGHT_GREEN
    } else {
        Color32::from_rgb(21, 128, 61)
    }
}

pub(crate) fn status_color(ui: &egui::Ui, status: &str) -> Color32 {
    if status.contains('🔥') {
        Color32::from_rgb(255, 92, 48)
    } else if status.contains('⚠') {
        theme_warning(ui)
    } else if status.contains('🏁') || status.contains('✅') {
        theme_success(ui)
    } else if status.contains('🔒') {
        theme_accent(ui)
    } else {
        theme_muted(ui)
    }
}
