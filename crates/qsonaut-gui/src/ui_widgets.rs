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

#[derive(Clone, Copy)]
pub(super) enum OperatingModeIcon {
    Digital,
    Wspr,
    Cw,
    Sstv,
    Msk144,
    Voice,
    VaraAc,
    Rade,
    Text,
}

/// Paint voice-mode icons without depending on emoji coverage in the selected
/// system font.
pub(super) fn draw_operating_mode_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: OperatingModeIcon,
    color: Color32,
) {
    let center = rect.center();
    let stroke = egui::Stroke::new(1.4, color);
    match icon {
        OperatingModeIcon::Digital => {
            painter.circle_filled(egui::pos2(center.x - 5.0, center.y), 1.5, color);
            painter.circle_filled(center, 1.5, color);
            painter.circle_filled(egui::pos2(center.x + 5.0, center.y), 1.5, color);
            painter.line_segment(
                [
                    egui::pos2(center.x - 7.0, center.y - 5.0),
                    egui::pos2(center.x + 7.0, center.y - 5.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 7.0, center.y + 5.0),
                    egui::pos2(center.x + 7.0, center.y + 5.0),
                ],
                stroke,
            );
        }
        OperatingModeIcon::Wspr => {
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y + 8.0),
                    egui::pos2(center.x, center.y - 6.0),
                ],
                stroke,
            );
            painter.circle_stroke(egui::pos2(center.x, center.y - 5.0), 3.0, stroke);
            painter.circle_stroke(egui::pos2(center.x, center.y - 5.0), 6.0, stroke);
            painter.line_segment(
                [
                    egui::pos2(center.x - 5.0, center.y + 8.0),
                    egui::pos2(center.x + 5.0, center.y + 8.0),
                ],
                stroke,
            );
        }
        OperatingModeIcon::Cw => {
            painter.circle_filled(egui::pos2(center.x - 6.0, center.y), 1.5, color);
            painter.line_segment(
                [
                    egui::pos2(center.x - 2.0, center.y),
                    egui::pos2(center.x + 7.0, center.y),
                ],
                stroke,
            );
            painter.circle_filled(egui::pos2(center.x - 4.0, center.y + 6.0), 1.5, color);
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y + 6.0),
                    egui::pos2(center.x + 7.0, center.y + 6.0),
                ],
                stroke,
            );
        }
        OperatingModeIcon::Sstv => {
            let frame = egui::Rect::from_center_size(center, egui::vec2(15.0, 13.0));
            painter.rect_stroke(frame, 1.5, stroke, egui::StrokeKind::Inside);
            painter.circle_filled(
                egui::pos2(frame.right() - 4.0, frame.top() + 4.0),
                1.2,
                color,
            );
            painter.line(
                vec![
                    egui::pos2(frame.left() + 2.0, frame.bottom() - 2.0),
                    egui::pos2(center.x - 1.0, center.y),
                    egui::pos2(center.x + 2.0, frame.bottom() - 3.0),
                    egui::pos2(frame.right() - 2.0, frame.bottom() - 2.0),
                ],
                stroke,
            );
        }
        OperatingModeIcon::Msk144 => {
            for (index, height) in [4.0, 7.0, 10.0].into_iter().enumerate() {
                let x = center.x - 6.0 + index as f32 * 6.0;
                painter.line_segment(
                    [
                        egui::pos2(x, center.y + 5.0),
                        egui::pos2(x, center.y + 5.0 - height),
                    ],
                    stroke,
                );
            }
        }
        OperatingModeIcon::Voice => {
            let capsule = egui::Rect::from_center_size(
                egui::pos2(center.x, center.y - 1.0),
                egui::vec2(7.0, 11.0),
            );
            painter.rect_stroke(capsule, 3.5, stroke, egui::StrokeKind::Inside);
            painter.circle_stroke(egui::pos2(center.x, center.y + 1.0), 6.0, stroke);
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y + 7.0),
                    egui::pos2(center.x, center.y + 10.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 4.0, center.y + 10.0),
                    egui::pos2(center.x + 4.0, center.y + 10.0),
                ],
                stroke,
            );
        }
        OperatingModeIcon::VaraAc => {
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 1.0, center.y),
                    egui::pos2(center.x - 5.0, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 5.0, center.y),
                    egui::pos2(center.x - 2.0, center.y - 5.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 2.0, center.y - 5.0),
                    egui::pos2(center.x + 1.0, center.y + 5.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 1.0, center.y + 5.0),
                    egui::pos2(center.x + 4.0, center.y - 4.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 4.0, center.y - 4.0),
                    egui::pos2(rect.right() - 1.0, center.y - 4.0),
                ],
                stroke,
            );
            painter.circle_stroke(egui::pos2(rect.right() - 2.0, center.y + 4.0), 2.5, stroke);
        }
        OperatingModeIcon::Rade => {
            let bubble = egui::Rect::from_center_size(
                egui::pos2(center.x, center.y - 1.0),
                egui::vec2(14.0, 10.0),
            );
            painter.rect_stroke(bubble, 2.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(center.x - 3.0, bubble.bottom()),
                    egui::pos2(center.x - 5.0, bubble.bottom() + 4.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 5.0, bubble.bottom() + 4.0),
                    egui::pos2(center.x + 1.0, bubble.bottom()),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 4.0, center.y - 1.0),
                    egui::pos2(center.x + 4.0, center.y - 1.0),
                ],
                stroke,
            );
        }
        OperatingModeIcon::Text => {
            let keyboard = egui::Rect::from_center_size(center, egui::vec2(15.0, 11.0));
            painter.rect_stroke(keyboard, 1.5, stroke, egui::StrokeKind::Inside);
            for x in [-4.0_f32, 0.0, 4.0] {
                painter.circle_filled(egui::pos2(center.x + x, center.y - 2.0), 1.0, color);
            }
            painter.line_segment(
                [
                    egui::pos2(center.x - 5.0, center.y + 3.0),
                    egui::pos2(center.x + 5.0, center.y + 3.0),
                ],
                stroke,
            );
        }
    }
}

pub(super) fn operating_mode_button(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    icon: OperatingModeIcon,
    enabled: bool,
) -> egui::Response {
    let label_width = ui
        .painter()
        .layout_no_wrap(
            label.to_owned(),
            egui::TextStyle::Button.resolve(ui.style()),
            Color32::WHITE,
        )
        .size()
        .x;
    let width = 31.0 + label_width + 8.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(62.0), 24.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let fill = if selected {
        ui.visuals().selection.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    let shadow_color = ui
        .visuals()
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.8);
    if !selected && enabled {
        ui.painter()
            .rect_filled(rect.translate(egui::vec2(0.0, 2.0)), 3.0, shadow_color);
    }
    ui.painter().rect_filled(
        if selected {
            rect.translate(egui::vec2(0.0, 1.0))
        } else {
            rect
        },
        3.0,
        fill,
    );
    ui.painter().rect_stroke(
        if selected {
            rect.translate(egui::vec2(0.0, 1.0))
        } else {
            rect
        },
        3.0,
        if selected {
            ui.visuals().selection.stroke
        } else {
            ui.visuals().widgets.inactive.bg_stroke
        },
        egui::StrokeKind::Inside,
    );
    let color = if enabled {
        ui.visuals().widgets.inactive.fg_stroke.color
    } else {
        ui.visuals().widgets.noninteractive.fg_stroke.color
    };
    draw_operating_mode_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 13.0, rect.center().y),
            egui::vec2(18.0, 18.0),
        ),
        icon,
        color,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 25.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        color,
    );
    response
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

pub(super) fn format_swr_display(
    presentation: Option<qsonaut_radio::MeterPresentation>,
    normalized: Option<u8>,
) -> String {
    let Some(level) = normalized else {
        return "unavailable".to_string();
    };
    let meter_percent = (f32::from(level) * 100.0 / 255.0).round();
    if let Some(presentation) = presentation {
        if presentation
            .upper_bound
            .is_some_and(|bound| presentation.value >= bound && level > 120)
        {
            return format!(
                ">{:.2}{} ({}% meter)",
                presentation.upper_bound.unwrap_or(presentation.value),
                presentation.unit,
                meter_percent as u8
            );
        }
        return format!(
            "{:.precision$}{} ({}% meter)",
            presentation.value,
            presentation.unit,
            meter_percent as u8,
            precision = usize::from(presentation.precision)
        );
    }
    format!("SWR meter {meter_percent:.0}%")
}

pub(super) fn swr_chart_value(
    presentation: Option<qsonaut_radio::MeterPresentation>,
    normalized: u8,
) -> f32 {
    if let Some(presentation) = presentation {
        return presentation.value;
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
        assert_eq!(radio_baud_rates("IC-7300"), &[4_800, 9_600, 19_200]);
    }

    #[test]
    fn swr_display_uses_model_calibration_and_safe_generic_fallbacks() {
        assert_eq!(format_swr_display(None, None), "unavailable");
        assert_eq!(format_swr_display(None, Some(128)), "SWR meter 50%");
        assert_eq!(swr_chart_value(None, 128), 128.0 * 100.0 / 255.0);
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
