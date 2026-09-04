use eframe::egui;
use eframe::egui::Color32;

use qsonaut_radio::{models::find_model, MeterId};

/// Paint the AI tab icon with egui primitives so it does not depend on an
/// emoji or a platform font containing a particular Unicode glyph.
pub(super) fn draw_ai_icon(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let stroke = egui::Stroke::new(1.4_f32, color);
    let center = rect.center();
    let body = egui::Rect::from_center_size(center, egui::vec2(10.0, 9.0));
    painter.rect_stroke(body, 2.0, stroke, egui::StrokeKind::Inside);
    painter.circle_filled(egui::pos2(center.x - 2.5, center.y), 1.1, color);
    painter.circle_filled(egui::pos2(center.x + 2.5, center.y), 1.1, color);
    painter.line_segment(
        [
            egui::pos2(body.left() - 2.0, body.top() + 2.0),
            egui::pos2(body.left(), body.top() + 2.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(body.right(), body.top() + 2.0),
            egui::pos2(body.right() + 2.0, body.top() + 2.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(body.left() - 2.0, body.bottom() - 2.0),
            egui::pos2(body.left(), body.bottom() - 2.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(body.right(), body.bottom() - 2.0),
            egui::pos2(body.right() + 2.0, body.bottom() - 2.0),
        ],
        stroke,
    );
}

pub(super) fn styled_selection_button(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    color: Color32,
    enabled: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(48.0, 27.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let fill = if selected {
        color.gamma_multiply(0.35)
    } else if response.hovered() && enabled {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    let stroke = if selected {
        egui::Stroke::new(1.0_f32, color)
    } else {
        egui::Stroke::new(1.0_f32, color.gamma_multiply(0.45))
    };
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter()
        .rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(12.0),
        if enabled {
            color
        } else {
            color.gamma_multiply(0.45)
        },
    );
    response
}

pub(super) fn draw_speaker_icon(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let center = rect.center();
    painter.rect_filled(
        egui::Rect::from_center_size(egui::pos2(center.x - 5.0, center.y), egui::vec2(4.0, 9.0)),
        1.0,
        color,
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(center.x - 3.0, center.y - 5.0),
            egui::pos2(center.x + 3.0, center.y - 9.0),
            egui::pos2(center.x + 3.0, center.y + 9.0),
            egui::pos2(center.x - 3.0, center.y + 5.0),
        ],
        color,
        egui::Stroke::NONE,
    ));
    painter.line_segment(
        [
            egui::pos2(center.x + 6.0, center.y - 5.0),
            egui::pos2(center.x + 9.0, center.y - 2.0),
        ],
        egui::Stroke::new(1.5_f32, color),
    );
    painter.line_segment(
        [
            egui::pos2(center.x + 6.0, center.y + 5.0),
            egui::pos2(center.x + 9.0, center.y + 2.0),
        ],
        egui::Stroke::new(1.5_f32, color),
    );
}

/// Paint a small radio/antenna mark for the About entry without relying on
/// platform fonts or emoji glyph coverage.
pub(super) fn draw_radio_about_icon(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let center = rect.center();
    let body =
        egui::Rect::from_center_size(egui::pos2(center.x, center.y + 3.0), egui::vec2(13.0, 11.0));
    painter.rect_stroke(
        body,
        2.0,
        egui::Stroke::new(1.5_f32, color),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(center, 2.0, color);
    painter.line_segment(
        [
            egui::pos2(center.x, body.top()),
            egui::pos2(center.x + 4.0, center.y - 10.0),
        ],
        egui::Stroke::new(1.5_f32, color),
    );
    painter.circle_stroke(
        egui::pos2(center.x + 4.0, center.y - 10.0),
        1.5,
        egui::Stroke::new(1.2_f32, color),
    );
}

pub(super) fn native_radio_profile(
    backend: &str,
    model: &str,
) -> Option<&'static qsonaut_radio::models::RadioModelProfile> {
    matches!(
        backend.trim().to_ascii_lowercase().as_str(),
        "native" | "hostbridge"
    )
    .then(|| find_model(model))
    .flatten()
}

pub(super) fn radio_control_max(
    model: &str,
    control: qsonaut_radio::ControlId,
    fallback: u8,
) -> u8 {
    native_radio_profile("native", model)
        .and_then(|profile| profile.control_max(control))
        .unwrap_or(fallback)
}

pub(super) fn radio_baud_rates(model: &str) -> &'static [u32] {
    let Some(profile) = find_model(model) else {
        return &[];
    };
    profile.supported_baud_rates()
}

pub(super) fn radio_supports_band(
    profile: Option<&qsonaut_radio::models::RadioModelProfile>,
    band: &str,
) -> bool {
    let Some(profile) = profile else {
        // Unknown/external radio connections must not be narrowed based on a
        // guess. The driver/model can opt into precise filtering when known.
        return true;
    };

    match band {
        "2m" | "70cm" => profile.capabilities.vhf_uhf,
        _ => profile.capabilities.hf,
    }
}

pub(super) fn format_swr_display(model: &str, normalized: Option<u8>) -> String {
    let Some(level) = normalized else {
        return "unavailable".to_string();
    };
    let meter_percent = (f32::from(level) * 100.0 / 255.0).round();
    if let Some(profile) = find_model(model) {
        // The IC-7300 manual documents these CI-V meter anchors. Interpolate
        // only between known points; do not invent a ratio above the documented
        // 3.0:1 anchor.
        if profile
            .calibrated_meter_value(MeterId::Swr, level)
            .is_some()
        {
            let ratio = profile
                .calibrated_meter_value(MeterId::Swr, level)
                .unwrap_or(3.0);
            if level > 120 {
                return format!(">3.00:1 ({meter_percent:.0}% meter)");
            }
            return format!("{ratio:.2}:1 ({meter_percent:.0}% meter)");
        }
    }
    format!("SWR meter {meter_percent:.0}%")
}

pub(super) fn swr_chart_value(model: &str, normalized: u8) -> f32 {
    if let Some(profile) = find_model(model) {
        if let Some(value) = profile.calibrated_meter_value(MeterId::Swr, normalized) {
            return value;
        }
    }
    f32::from(normalized) * 100.0 / 255.0
}

#[cfg(test)]
mod tests {
    use super::{
        draw_ai_icon, draw_radio_about_icon, draw_speaker_icon, format_swr_display,
        native_radio_profile, radio_baud_rates, radio_supports_band, styled_selection_button,
        swr_chart_value,
    };
    use eframe::egui::{self, Color32};

    #[test]
    fn baud_rates_follow_the_selected_radio_profile() {
        assert_eq!(radio_baud_rates("FTDX10"), &[4_800, 9_600, 19_200, 38_400]);
        assert!(radio_baud_rates("FT-710").contains(&115_200));
        assert_eq!(radio_baud_rates("FT-857D"), &[4_800, 9_600, 38_400]);
        assert!(!radio_baud_rates("TS-2000").contains(&115_200));
        assert!(radio_baud_rates("IC-7300").contains(&115_200));
    }

    #[test]
    fn swr_display_uses_model_calibration_and_safe_generic_fallbacks() {
        assert_eq!(format_swr_display("IC-7300", None), "unavailable");
        assert_eq!(format_swr_display("unknown", Some(128)), "SWR meter 50%");
        assert!(format_swr_display("IC-7300", Some(80)).ends_with("meter)"));
        assert!(format_swr_display("IC-7300", Some(121)).starts_with(">3.00:1"));
        assert_eq!(swr_chart_value("unknown", 128), 128.0 * 100.0 / 255.0);
        assert!(swr_chart_value("IC-7300", 80) > 1.0);
    }

    #[test]
    fn band_capability_filtering_is_conservative_for_unknown_models() {
        assert!(radio_supports_band(None, "20m"));
        assert!(radio_supports_band(None, "2m"));
        assert!(radio_supports_band(
            super::native_radio_profile("native", "IC-7300"),
            "20m"
        ));
        assert!(!radio_supports_band(
            super::native_radio_profile("native", "IC-7300"),
            "2m"
        ));
    }

    #[test]
    fn native_profile_selection_is_backend_and_model_aware() {
        assert!(native_radio_profile("native", "IC-7300").is_some());
        assert!(native_radio_profile(" NATIVE ", "IC-7300").is_some());
        assert!(native_radio_profile("rigctld", "IC-7300").is_none());
        assert!(native_radio_profile("native", "not-a-model").is_none());
        assert!(radio_baud_rates("not-a-model").is_empty());
    }

    #[test]
    fn widget_painters_and_selection_states_render_without_panicking() {
        let context = egui::Context::default();
        let color = Color32::from_rgb(20, 200, 240);
        let _ = context.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect = ui.allocate_space(egui::vec2(32.0, 32.0)).1;
                draw_ai_icon(ui.painter(), rect, color);
                draw_speaker_icon(ui.painter(), rect, color);
                draw_radio_about_icon(ui.painter(), rect, color);
                let _ = styled_selection_button(ui, "ON", true, color, true);
                let _ = styled_selection_button(ui, "OFF", false, color, false);
            });
        });
    }
}
