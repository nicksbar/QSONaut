use super::super::*;

pub(crate) const VOLTAGE_HISTORY_CAPACITY: usize = 180;
pub(crate) const METER_LABEL_WIDTH: f32 = 88.0;

pub(crate) fn record_voltage_sample(history: &mut VecDeque<u8>, value: u8) {
    history.push_back(value);
    while history.len() > VOLTAGE_HISTORY_CAPACITY {
        history.pop_front();
    }
}

pub(crate) fn draw_voltage_graph(ui: &mut egui::Ui, history: &VecDeque<u8>, reading: &str) {
    let desired_size = egui::vec2(ui.available_width().max(100.0), 28.0);
    let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    let painter = ui.painter();
    let outer = rect.expand(1.0);
    painter.rect_filled(
        outer,
        egui::CornerRadius::same(7),
        Color32::from_rgb(10, 20, 29),
    );
    painter.rect_stroke(
        outer,
        egui::CornerRadius::same(7),
        egui::Stroke::new(1.0_f32, Color32::from_rgb(45, 75, 88)),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(5),
        Color32::from_rgb(7, 18, 25),
    );
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(5),
        egui::Stroke::new(1.0_f32, Color32::from_rgb(28, 45, 57)),
        egui::StrokeKind::Inside,
    );
    if !history.is_empty() {
        let graph_rect = rect.shrink2(egui::vec2(3.0, 3.0));
        let capacity = VOLTAGE_HISTORY_CAPACITY.max(history.len());
        let points: Vec<egui::Pos2> = history
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let x = egui::lerp(
                    graph_rect.left()..=graph_rect.right(),
                    (index + 1) as f32 / capacity as f32,
                );
                let y = egui::lerp(
                    graph_rect.bottom()..=graph_rect.top(),
                    meter_percent(*value),
                );
                egui::pos2(x, y)
            })
            .collect();
        painter.add(egui::Shape::line(
            points.clone(),
            egui::Stroke::new(2.0_f32, Color32::from_rgb(100, 225, 165)),
        ));
        if let Some(last) = points.last() {
            painter.circle_filled(*last, 3.0, Color32::from_rgb(150, 255, 205));
        }
    }
    let reading_width = 90.0;
    let reading_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - reading_width, rect.top() + 2.0),
        egui::pos2(rect.right() - 3.0, rect.bottom() - 2.0),
    );
    painter.rect_filled(
        reading_rect,
        egui::CornerRadius::same(3),
        Color32::from_rgba_unmultiplied(10, 20, 29, 225),
    );
    painter.text(
        reading_rect.right_center() - egui::vec2(5.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        reading,
        egui::FontId::monospace(11.0),
        Color32::WHITE,
    );
}

pub(crate) fn draw_primary_meter(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    reading: &str,
    fraction: f32,
    color: Color32,
) {
    let painter = ui.painter();
    let outer = rect.expand(1.0);
    painter.rect_filled(
        outer,
        egui::CornerRadius::same(7),
        Color32::from_rgb(10, 20, 29),
    );
    painter.rect_stroke(
        outer,
        egui::CornerRadius::same(7),
        egui::Stroke::new(1.0_f32, color.gamma_multiply(0.65)),
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink2(egui::vec2(5.0, 6.0));
    let segments = 30;
    let gap = 2.0;
    let segment_width = ((inner.width() - gap * (segments - 1) as f32) / segments as f32).max(1.0);
    let lit = (fraction.clamp(0.0, 1.0) * segments as f32).ceil() as usize;
    for index in 0..segments {
        let left = inner.left() + index as f32 * (segment_width + gap);
        let segment = egui::Rect::from_min_max(
            egui::pos2(left, inner.top()),
            egui::pos2(left + segment_width, inner.bottom()),
        );
        let fill = if index < lit {
            color
        } else {
            Color32::from_rgb(28, 45, 57)
        };
        painter.rect_filled(segment, egui::CornerRadius::same(2), fill);
    }
    if !label.is_empty() {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 4.0, rect.top() + 2.0),
                egui::pos2(rect.left() + 57.0, rect.bottom() - 2.0),
            ),
            egui::CornerRadius::same(3),
            Color32::from_rgba_unmultiplied(10, 20, 29, 225),
        );
        painter.text(
            egui::pos2(rect.left() + 8.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::monospace(11.0),
            Color32::WHITE,
        );
    }
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(rect.right() - 143.0, rect.top() + 2.0),
            egui::pos2(rect.right() - 4.0, rect.bottom() - 2.0),
        ),
        egui::CornerRadius::same(3),
        Color32::from_rgba_unmultiplied(10, 20, 29, 225),
    );
    painter.text(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        reading,
        egui::FontId::monospace(11.0),
        Color32::WHITE,
    );
}

pub(crate) fn general_meter_order() -> [MeterId; 8] {
    [
        MeterId::Voltage,
        MeterId::Current,
        MeterId::Signal,
        MeterId::Power,
        MeterId::Swr,
        MeterId::Alc,
        MeterId::Compression,
        MeterId::Temperature,
    ]
}

pub(crate) fn mode_meter_order(transmitting: bool) -> [MeterId; 8] {
    if transmitting {
        [
            MeterId::Voltage,
            MeterId::Current,
            MeterId::Power,
            MeterId::Swr,
            MeterId::Alc,
            MeterId::Compression,
            MeterId::Temperature,
            MeterId::Signal,
        ]
    } else {
        general_meter_order()
    }
}

pub(crate) fn meter_value(snapshot: &GuiState, id: MeterId) -> Option<u8> {
    match id {
        MeterId::Signal => snapshot.signal_meter,
        MeterId::Power => snapshot.power_meter,
        MeterId::Swr => snapshot.swr,
        MeterId::Alc => snapshot.alc_meter,
        MeterId::Compression => snapshot.compression_meter,
        MeterId::Current => snapshot.current_meter,
        MeterId::Voltage => snapshot.voltage_meter,
        MeterId::Temperature => snapshot.temperature_meter,
    }
}

pub(crate) fn meter_percent(value: u8) -> f32 {
    f32::from(value) / 255.0
}

pub(crate) fn meter_color_for_context(
    id: MeterId,
    value: Option<u8>,
    transmitting: bool,
) -> Color32 {
    if id == MeterId::Current && transmitting && value.is_some() {
        return Color32::from_rgb(110, 245, 215);
    }
    meter_color(id, value)
}

pub(crate) fn meter_label(id: MeterId) -> &'static str {
    match id {
        MeterId::Signal => "S-METER",
        MeterId::Power => "POWER",
        MeterId::Swr => "SWR",
        MeterId::Alc => "ALC",
        MeterId::Compression => "COMP",
        MeterId::Current => "CURRENT",
        MeterId::Voltage => "VOLTAGE",
        MeterId::Temperature => "TEMP",
    }
}

pub(crate) fn meter_reading(id: MeterId, value: Option<u8>) -> String {
    let Some(value) = value else {
        return "—".to_string();
    };
    if id == MeterId::Signal {
        let s_units = u16::from(value) / 12;
        if s_units <= 9 {
            format!("S{s_units} · {}%", u16::from(value) * 100 / 255)
        } else {
            format!(
                "S9 +{} dB · {}%",
                (s_units - 9) * 6,
                u16::from(value) * 100 / 255
            )
        }
    } else if id == MeterId::Voltage {
        format!("REL {value}/255")
    } else {
        format!("{}%", u16::from(value) * 100 / 255)
    }
}

pub(crate) fn meter_reading_for_presentation(
    id: MeterId,
    value: Option<u8>,
    presentation: Option<qsonaut_radio::MeterPresentation>,
) -> String {
    if let (Some(presentation), Some(_raw)) = (presentation, value) {
        return format!(
            "{:.precision$} {}",
            presentation.value,
            presentation.unit,
            precision = usize::from(presentation.precision)
        );
    }
    meter_reading(id, value)
}

pub(crate) fn meter_tooltip(id: MeterId) -> &'static str {
    match id {
        MeterId::Signal => {
            "Receive signal level; S-unit display is derived from the normalized driver level"
        }
        MeterId::Power => "Measured relative RF output level",
        MeterId::Swr => "Transmit SWR meter level; exact ratio is model-specific",
        MeterId::Alc => "Transmit ALC level",
        MeterId::Compression => "Transmit speech/data compression level",
        MeterId::Current => "PA drain/current meter level",
        MeterId::Voltage => {
            "PA voltage level; physical units are shown only when documented by the driver"
        }
        MeterId::Temperature => "PA temperature meter; exact units depend on the driver",
    }
}

pub(crate) fn meter_color(id: MeterId, value: Option<u8>) -> Color32 {
    if value.is_none() {
        return Color32::GRAY;
    }
    let value = value.unwrap_or_default();
    match id {
        MeterId::Swr if value >= 190 => Color32::RED,
        MeterId::Swr if value >= 130 => Color32::YELLOW,
        MeterId::Power | MeterId::Alc | MeterId::Compression if value >= 220 => {
            Color32::from_rgb(255, 145, 100)
        }
        _ => Color32::from_rgb(100, 210, 150),
    }
}
