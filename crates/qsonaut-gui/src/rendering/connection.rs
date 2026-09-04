use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn draw_header_identity_and_activity(&self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.horizontal_wrapped(|ui| {
            let activity_profile = self.activity.profile();
            ui.label(
                RichText::new(format!("CALL · {}", activity_profile.tx_cq))
                    .size(18.0)
                    .monospace()
                    .color(Color32::from_rgb(255, 190, 105)),
            )
            .on_hover_text(format!(
                "Call behavior\nActivity: {}\nTransmit calling text: {}",
                self.activity.label(),
                activity_profile.tx_cq
            ));
            ui.label(RichText::new("·").color(Color32::DARK_GRAY));
            ui.label(
                RichText::new(format!(
                    "📍 {} · {}",
                    self.station_callsign_or_default(),
                    self.station_grid_or_default()
                ))
                .strong()
                .size(18.0)
                .color(Color32::from_rgb(255, 210, 110)),
            );
            if let Some(client) = &self.server_client {
                let server_status = client.status();
                if server_status.state == ServerConnectionState::Connected
                    && server_status.active_event_count > 0
                {
                    let label = if self.activity == OperatingActivity::Contest
                        && self.contest_enabled
                    {
                        "✅ SERVER CONTEST · PARTICIPATING".to_string()
                    } else {
                        format!("✅ SERVER CONTEST · {} ACTIVE", server_status.active_event_count)
                    };
                    ui.label(
                        RichText::new(label)
                            .size(15.0)
                            .strong()
                            .color(Color32::from_rgb(125, 225, 150)),
                    )
                    .on_hover_text(format!(
                        "Server contest status\nConnected server events: {}\nThis indicator reflects shared contest activity.",
                        server_status.active_event_count
                    ));
                }
            }
        });
    }

    pub(crate) fn draw_about_button(&self, ui: &mut egui::Ui) {
        let (about_rect, about_button) =
            ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
        let about_color = Color32::from_rgb(120, 210, 235);
        draw_radio_about_icon(&ui.painter_at(about_rect), about_rect, about_color);
        let about_button = about_button.on_hover_text("About QSONaut");
        egui::Popup::menu(&about_button).show(|ui| {
            ui.set_min_width(300.0);
            ui.heading("QSONaut");
            ui.label("Amateur Radio Mission Control");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.separator();
            ui.label("Original author");
            ui.label(RichText::new("N7UF").strong());
            ui.label("Copyright © 2026 N7UF and contributors");
            ui.label("Released under the MIT License.");
            ui.separator();
            ui.label(RichText::new("Contributors").strong());
            ui.label(qsonaut_contributors());
            ui.label(RichText::new("Testers").strong());
            ui.label(qsonaut_testers());
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to("GitHub", QSONAUT_GITHUB_URL);
                ui.hyperlink_to("File an issue", QSONAUT_ISSUES_URL);
                ui.hyperlink_to("qsonaut.com", QSONAUT_WEBSITE_URL);
            });
        });
    }

    pub(crate) fn draw_connection_status(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Connections").strong());
            ui.separator();
            ui.label(
                RichText::new(if snapshot.frequency_hz.is_some() {
                    "Radio CONNECTED"
                } else {
                    "Radio OFFLINE"
                })
                .color(if snapshot.frequency_hz.is_some() {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::GRAY
                }),
            );
            let (server_label, server_color) = self
                .server_client
                .as_ref()
                .map(|client| match client.status().state {
                    ServerConnectionState::Connected => {
                        ("QSONaut Server CONNECTED", Color32::LIGHT_GREEN)
                    }
                    ServerConnectionState::Connecting | ServerConnectionState::Reconnecting => {
                        ("QSONaut Server CONNECTING", theme_warning(ui))
                    }
                    ServerConnectionState::Disabled | ServerConnectionState::Stopped => {
                        ("QSONaut Server OFFLINE", Color32::GRAY)
                    }
                })
                .unwrap_or(("QSONaut Server DISABLED", Color32::GRAY));
            ui.separator();
            ui.label(RichText::new(server_label).color(server_color));
            ui.separator();
            let pota_activators = self
                .pota_spots
                .iter()
                .map(|spot| spot.activator.as_str())
                .collect::<HashSet<_>>()
                .len();
            let pota_label = if !self.pota_enabled {
                "🌲 POTA OFF".to_string()
            } else if self.pota_lookup_rx.is_some() {
                "🌲 POTA …".to_string()
            } else {
                format!("🌲 POTA {pota_activators}")
            };
            let pota_button = ui
                .selectable_label(self.pota_enabled && !self.pota_spots.is_empty(), pota_label)
                .on_hover_text("Show live POTA activator statistics and spots");
            egui::Popup::menu(&pota_button).show(|ui| self.draw_pota_panel(ui));
            ui.separator();
            if !self.psk_reporter_enabled {
                ui.label(RichText::new("PSK OFF").color(Color32::GRAY))
                    .on_hover_text(
                        "Enable in the Reporting panel to batch decoded stations to PSK Reporter",
                    );
            } else if let Some(reporter) = &self.psk_reporter {
                let status = reporter.status();
                let (label, color) = if status.last_error.is_some() {
                    ("PSK ERROR".to_string(), Color32::from_rgb(255, 110, 100))
                } else if !status.active {
                    ("PSK STOPPED".to_string(), theme_warning(ui))
                } else {
                    (
                        format!("PSK {}q · {} sent", status.queued, status.sent),
                        Color32::LIGHT_GREEN,
                    )
                };
                ui.label(RichText::new(label).color(color)).on_hover_text(
                    status
                        .last_error
                        .as_deref()
                        .map(|error| format!("network error: {error}"))
                        .unwrap_or_else(|| {
                            format!(
                                "PSK Reporter batching every ~{} s · same callsign re-reported after {} s · {} max pending",
                                self.psk_batch_interval_secs,
                                self.psk_repeat_cache_secs,
                                self.psk_max_pending
                            )
                        }),
                );
            } else {
                ui.label(RichText::new("PSK WAITING").color(theme_warning(ui)))
                    .on_hover_text("Set a real callsign and grid before reporting");
            }
            ui.separator();
            ui.label(
                RichText::new(format!("Compute {}", self.acceleration_report.summary()))
                    .color(Color32::from_rgb(180, 150, 255)),
            )
            .on_hover_text(self.acceleration_report.hardware_detail());
            ui.separator();
            for label in ["IRC", "Discord"] {
                ui.label(RichText::new(format!("{label} N/A")).color(Color32::GRAY));
                ui.separator();
            }
            if let Some(error) = &snapshot.last_error {
                ui.label(RichText::new("⚠ NEEDS ATTENTION").color(theme_warning(ui)))
                    .on_hover_text(error);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.draw_about_button(ui);
            });
        });
    }
}
