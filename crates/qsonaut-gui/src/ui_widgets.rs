use eframe::egui;
use eframe::egui::Color32;

use qsonaut_radio::models::find_model;

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
    backend
        .trim()
        .eq_ignore_ascii_case("native")
        .then(|| find_model(model))
        .flatten()
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
    if model.eq_ignore_ascii_case("IC-7300") {
        // The IC-7300 manual documents these CI-V meter anchors. Interpolate
        // only between known points; do not invent a ratio above the documented
        // 3.0:1 anchor.
        let anchors = [(0_u8, 1.0_f32), (48, 1.5), (80, 2.0), (120, 3.0)];
        if let Some(window) = anchors.windows(2).find(|window| level <= window[1].0) {
            let (low_level, low_ratio) = window[0];
            let (high_level, high_ratio) = window[1];
            let fraction =
                f32::from(level.saturating_sub(low_level)) / f32::from(high_level - low_level);
            return format!(
                "{:.2}:1 ({meter_percent:.0}% meter)",
                low_ratio + fraction * (high_ratio - low_ratio)
            );
        }
        return format!(">3.00:1 ({meter_percent:.0}% meter)");
    }
    format!("SWR meter {meter_percent:.0}%")
}

pub(super) fn swr_chart_value(model: &str, normalized: u8) -> f32 {
    if !model.eq_ignore_ascii_case("IC-7300") {
        return f32::from(normalized) * 100.0 / 255.0;
    }
    let anchors = [(0_u8, 1.0_f32), (48, 1.5), (80, 2.0), (120, 3.0)];
    if let Some(window) = anchors.windows(2).find(|window| normalized <= window[1].0) {
        let (low_level, low_ratio) = window[0];
        let (high_level, high_ratio) = window[1];
        let fraction =
            f32::from(normalized.saturating_sub(low_level)) / f32::from(high_level - low_level);
        return low_ratio + fraction * (high_ratio - low_ratio);
    }
    3.0
}
