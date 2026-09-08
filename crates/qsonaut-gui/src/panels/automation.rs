use super::super::*;
use crate::automation_hunter::AUTOMATION_CONTROL_CATALOG;
use qsonaut_automation::{Capability, EventKind};

fn capability_label(capability: Capability) -> &'static str {
    match capability {
        Capability::UiNotification => "UI notifications",
        Capability::ExternalSend => "External send",
        Capability::ServerRead => "Server read",
        Capability::ServerPublish => "Server publish",
        Capability::SetCompose => "Set compose text",
        Capability::RadioControl => "Radio controls",
        Capability::Transmit => "Transmit",
    }
}

fn event_label(event: EventKind) -> &'static str {
    match event {
        EventKind::Decode => "decode",
        EventKind::CallsignHit => "callsign_hit",
        EventKind::QsoLogged => "qso_logged",
        EventKind::RadioState => "radio_state",
        EventKind::ContestState => "contest_state",
        EventKind::OperatorProfile => "operator_profile",
        EventKind::Command => "command",
        EventKind::ComponentState => "component_state",
        EventKind::ExternalMessage => "external_message",
        EventKind::ServerMessage => "server_message",
        EventKind::Timer => "timer",
        EventKind::ControlRead => "control_read",
    }
}

impl QsonautGuiApp {
    pub(in super::super) fn draw_automation_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("⚡ Automation cockpit");
        ui.label(
            RichText::new(
                "Understand what scripts can observe, what they can request, and how actions are safety-gated.",
            )
            .small()
            .color(Color32::GRAY),
        );

        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label(RichText::new("Runtime status").strong());
            ui.label(
                RichText::new(&self.automation_status)
                    .small()
                    .color(Color32::from_rgb(158, 217, 255)),
            );
            ui.label(
                RichText::new(format!(
                    "TX automation unlock: {}",
                    if self.automation_unlocked {
                        "unlocked"
                    } else {
                        "locked (safe default)"
                    }
                ))
                .small()
                .color(if self.automation_unlocked {
                    Color32::from_rgb(255, 201, 92)
                } else {
                    Color32::GRAY
                }),
            );
            ui.label(
                RichText::new(
                    "Radio control actions never access serial devices directly; they enter the GUI-owned worker queue.",
                )
                .small()
                .color(Color32::GRAY),
            );
        });

        ui.add_space(6.0);
        for component in self.automation_host.component_overview() {
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&component.name).strong());
                    ui.label(
                        RichText::new(&component.id)
                            .small()
                            .monospace()
                            .color(Color32::GRAY),
                    );
                });
                ui.label(
                    RichText::new(format!(
                        "Listens for: {}",
                        component
                            .subscriptions
                            .iter()
                            .map(|event| event_label(*event))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .small(),
                );
                ui.label(
                    RichText::new(format!(
                        "Requested: {}",
                        component
                            .requested
                            .iter()
                            .map(|capability| capability_label(*capability))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .small()
                    .color(Color32::GRAY),
                );
                ui.label(
                    RichText::new(format!(
                        "Granted: {}",
                        if component.granted.is_empty() {
                            "none".to_string()
                        } else {
                            component
                                .granted
                                .iter()
                                .map(|capability| capability_label(*capability))
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    ))
                    .small()
                    .color(Color32::from_rgb(132, 228, 255)),
                );
            });
        }

        ui.add_space(6.0);
        ui.collapsing("How to think about automation", |ui| {
            for (number, text) in [
                ("1", "An event happens: a decode, QSO, contest change, radio state change, or external message."),
                ("2", "A rule matches that event and renders templates such as ${call} or ${saved_power}."),
                ("3", "The host checks the component request and operator grant before approving each action."),
                ("4", "The GUI executes approved actions through normal safety and HAL boundaries."),
            ] {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(number).strong().color(Color32::from_rgb(126, 220, 180)));
                    ui.label(RichText::new(text).small());
                });
            }
        });

        ui.add_space(4.0);
        ui.collapsing("Capabilities", |ui| {
            for capability in [
                Capability::UiNotification,
                Capability::ExternalSend,
                Capability::ServerRead,
                Capability::ServerPublish,
                Capability::SetCompose,
                Capability::RadioControl,
                Capability::Transmit,
            ] {
                ui.label(
                    RichText::new(format!("• {}", capability_label(capability)))
                        .small()
                        .color(Color32::GRAY),
                );
            }
            ui.label(
                RichText::new(
                    "A script may request a capability, but the operator grant is the second lock. Transmit stays separate from generic radio controls.",
                )
                .small()
                .color(theme_warning(ui)),
            );
        });

        ui.add_space(4.0);
        ui.collapsing("Generic HAL controls · click Read to inspect", |ui| {
            ui.label(
                RichText::new(
                    "These stable names are what scripts use. Unsupported controls report unavailable instead of bypassing the selected driver.",
                )
                .small()
                .color(Color32::GRAY),
            );
            egui::Grid::new("automation_control_catalog")
                .num_columns(4)
                .spacing(egui::vec2(8.0, 4.0))
                .striped(true)
                .show(ui, |ui| {
                    for &(name, description, value_type) in AUTOMATION_CONTROL_CATALOG {
                        ui.label(RichText::new(description).small());
                        ui.label(RichText::new(name).small().monospace().color(Color32::from_rgb(158, 217, 255)));
                        ui.label(RichText::new(value_type).small().color(Color32::GRAY));
                        if ui.small_button("Read").clicked() {
                            self.automation_status = self.execute_automation_control_read(
                                name,
                                &format!("ui_{name}"),
                            );
                        }
                        ui.end_row();
                    }
                });
        });

        ui.add_space(4.0);
        ui.collapsing("Script recipe", |ui| {
            ui.label(
                RichText::new(
                    "Read a value, save it, then react to the resulting control_read event:",
                )
                .small(),
            );
            for line in [
                "action = \"read_control\"",
                "control = \"rf_power\"",
                "result_key = \"saved_power\"",
                "# later: ${saved_power}",
            ] {
                ui.label(
                    RichText::new(line)
                        .monospace()
                        .small()
                        .color(Color32::from_rgb(180, 220, 190)),
                );
            }
            ui.label(
                RichText::new("The full starter configuration is in automation.example.toml.")
                    .small()
                    .color(Color32::GRAY),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_labels_cover_the_automation_contract() {
        let labels = [
            (Capability::UiNotification, "UI notifications"),
            (Capability::ExternalSend, "External send"),
            (Capability::ServerRead, "Server read"),
            (Capability::ServerPublish, "Server publish"),
            (Capability::SetCompose, "Set compose text"),
            (Capability::RadioControl, "Radio controls"),
            (Capability::Transmit, "Transmit"),
        ];

        for (capability, expected) in labels {
            assert_eq!(capability_label(capability), expected);
        }
    }

    #[test]
    fn event_labels_cover_the_automation_contract() {
        let labels = [
            (EventKind::Decode, "decode"),
            (EventKind::CallsignHit, "callsign_hit"),
            (EventKind::QsoLogged, "qso_logged"),
            (EventKind::RadioState, "radio_state"),
            (EventKind::ContestState, "contest_state"),
            (EventKind::OperatorProfile, "operator_profile"),
            (EventKind::Command, "command"),
            (EventKind::ExternalMessage, "external_message"),
            (EventKind::ServerMessage, "server_message"),
            (EventKind::Timer, "timer"),
            (EventKind::ControlRead, "control_read"),
        ];

        for (event, expected) in labels {
            assert_eq!(event_label(event), expected);
        }
    }
}
