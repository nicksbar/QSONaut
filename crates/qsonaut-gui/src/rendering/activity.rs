use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn draw_activity_selector(&mut self, ui: &mut egui::Ui) {
        let selected_activity = self.activity;
        let server_context = self.server_client.as_ref().map(ServerClient::status);
        let activity_button_label = self
            .server_active_event
            .as_ref()
            .map(|(_, name)| {
                self.server_active_identity
                    .as_ref()
                    .map(|(_, callsign)| format!("🏁 {callsign} · {name}"))
                    .unwrap_or_else(|| format!("🏁 Contest · {name}"))
            })
            .or_else(|| {
                self.server_active_club
                    .as_ref()
                    .map(|(_, name)| format!("🌐 {} · {name}", selected_activity.label()))
            })
            .unwrap_or_else(|| format!("📻 {}", selected_activity.label()));
        let previous_interact_height = ui.spacing().interact_size.y;
        ui.spacing_mut().interact_size.y = 28.0;
        let activity_menu = ui.menu_button(
            RichText::new(activity_button_label)
                .strong()
                .color(Color32::from_rgb(255, 190, 105)),
            |ui| {
                ui.label(RichText::new("OPERATING ACTIVITY").strong());
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    for activity in OperatingActivity::ALL {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(78.0, 52.0), egui::Sense::click());
                        let fill = if selected_activity == activity {
                            ui.visuals().selection.bg_fill
                        } else if response.hovered() {
                            ui.visuals().widgets.hovered.bg_fill
                        } else {
                            ui.visuals().widgets.inactive.bg_fill
                        };
                        ui.painter().rect_filled(rect, 4.0, fill);
                        draw_activity_icon(
                            ui.painter(),
                            activity,
                            rect.center_top() + egui::vec2(0.0, 14.0),
                            ui.visuals().text_color(),
                        );
                        ui.painter().text(
                            rect.center_bottom() - egui::vec2(0.0, 8.0),
                            egui::Align2::CENTER_CENTER,
                            activity.label(),
                            egui::FontId::proportional(12.0),
                            ui.visuals().text_color(),
                        );
                        if response.clicked() {
                            info!(activity = %activity.label(), "Operating activity changed");
                            self.disarm_all_tx_with_persistence(
                                "Operating activity changed",
                                false,
                            );
                            self.activity = activity;
                            self.server_active_event = None;
                            self.server_active_club = None;
                            self.server_active_identity = None;
                            self.contest_enabled = matches!(
                                activity,
                                OperatingActivity::Contest | OperatingActivity::FieldDay
                            );
                            if activity == OperatingActivity::FieldDay {
                                self.contest_type = "ARRL_FD".to_string();
                            }
                            self.contest_exchange_fields.clear();
                            self.cw_qso_exchange_received.clear();
                            self.profile_dirty = true;
                            self.persist_profile("Operating activity saved");
                            ui.close();
                        }
                    }
                });
                if let Some(server_context) = &server_context {
                    if server_context.state != ServerConnectionState::Connected {
                        ui.label("Server activities unavailable while disconnected");
                        return;
                    }
                    if !server_context.clubs.is_empty() || !server_context.active_events.is_empty()
                    {
                        ui.separator();
                        ui.label(
                            RichText::new("🌐 SERVER ACTIVITIES")
                                .strong()
                                .color(Color32::from_rgb(110, 220, 255)),
                        );
                        if !server_context.clubs.is_empty() {
                            ui.label(RichText::new("🌐 ACTIVE CLUB").small().strong());
                            for club in &server_context.clubs {
                                let selected = self
                                    .server_active_club
                                    .as_ref()
                                    .is_some_and(|(id, _)| id == &club.id);
                                let label = club
                                    .callsign
                                    .as_deref()
                                    .map(|callsign| format!("{} · {}", club.name, callsign))
                                    .unwrap_or_else(|| club.name.clone());
                                if ui.selectable_label(selected, label).clicked() {
                                    self.server_active_club =
                                        Some((club.id.clone(), club.name.clone()));
                                    self.disarm_all_tx_with_persistence(
                                        "Club context changed",
                                        false,
                                    );
                                    self.server_active_event = None;
                                    ui.close();
                                }
                            }
                        }
                        if !server_context.active_events.is_empty() {
                            ui.label(RichText::new("🏁 ACTIVE CONTESTS").small().strong());
                            for contest in &server_context.active_events {
                                let selected = self
                                    .server_active_event
                                    .as_ref()
                                    .is_some_and(|(id, _)| id == &contest.id);
                                let label = if contest.contest_name.is_empty() {
                                    format!("{} · {}", contest.name, contest.club_name)
                                } else {
                                    format!(
                                        "{} · {} · {}",
                                        contest.name, contest.contest_name, contest.club_name
                                    )
                                };
                                ui.horizontal(|ui| {
                                    if ui.selectable_label(selected, label).clicked() {
                                        self.disarm_all_tx_with_persistence(
                                            "Server event changed",
                                            false,
                                        );
                                        self.activity = OperatingActivity::Contest;
                                        self.contest_enabled = true;
                                        self.contest_exchange_fields.clear();
                                        self.cw_qso_exchange_received.clear();
                                        self.server_active_club = Some((
                                            contest.club_id.clone(),
                                            contest.club_name.clone(),
                                        ));
                                        self.server_active_event =
                                            Some((contest.id.clone(), contest.name.clone()));
                                        self.server_active_identity = None;
                                        ui.close();
                                    }
                                    let starts = contest
                                        .starts_at
                                        .get(0..16)
                                        .unwrap_or(&contest.starts_at)
                                        .replace('T', " ");
                                    let ends = contest
                                        .ends_at
                                        .get(0..16)
                                        .unwrap_or(&contest.ends_at)
                                        .replace('T', " ");
                                    ui.label(
                                        RichText::new(format!(
                                            "{starts} → {ends} · {} op",
                                            contest.participant_count
                                        ))
                                        .small()
                                        .color(Color32::GRAY),
                                    );
                                });
                            }
                        }
                        if self.server_active_event.is_none() {
                            if let Some((club_id, _)) = &self.server_active_club {
                                let identities = server_context
                                    .identities
                                    .iter()
                                    .filter(|identity| {
                                        identity.club_id.as_deref() == Some(club_id)
                                            && identity.event_id.is_none()
                                            && identity.status == "active"
                                            && identity.verification_status == "verified"
                                    })
                                    .collect::<Vec<_>>();
                                if !identities.is_empty() {
                                    ui.label(RichText::new("OPERATING IDENTITY").small().strong());
                                    for identity in identities {
                                        let selected = self
                                            .server_active_identity
                                            .as_ref()
                                            .is_some_and(|(id, _)| id == &identity.id);
                                        if ui
                                            .selectable_label(
                                                selected,
                                                format!(
                                                    "{} · {}",
                                                    identity.callsign, identity.identity_type
                                                ),
                                            )
                                            .clicked()
                                        {
                                            self.disarm_all_tx_with_persistence(
                                                "Operating identity changed",
                                                false,
                                            );
                                            self.server_active_identity = Some((
                                                identity.id.clone(),
                                                identity.callsign.clone(),
                                            ));
                                            ui.close();
                                        }
                                    }
                                }
                            }
                        }
                        if let Some((event_id, _)) = &self.server_active_event {
                            ui.label(RichText::new("OPERATING IDENTITY").small().strong());
                            let assignments = server_context
                                .participants
                                .iter()
                                .filter(|participant| {
                                    participant.event_id == *event_id
                                        && participant.status == "active"
                                })
                                .collect::<Vec<_>>();
                            if assignments.is_empty() {
                                ui.colored_label(
                                    theme_warning(ui),
                                    "No active station assignment for this event",
                                );
                            } else {
                                for participant in assignments {
                                    let selected = self
                                        .server_active_identity
                                        .as_ref()
                                        .is_some_and(|(id, _)| id == &participant.callsign_id);
                                    if ui
                                        .selectable_label(
                                            selected,
                                            format!(
                                                "{} · operator {}",
                                                participant.operating_callsign,
                                                participant.operator_callsign
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.disarm_all_tx_with_persistence(
                                            "Operating identity changed",
                                            false,
                                        );
                                        self.server_active_identity = Some((
                                            participant.callsign_id.clone(),
                                            participant.operating_callsign.clone(),
                                        ));
                                        ui.close();
                                    }
                                }
                            }
                        }
                        if (self.server_active_club.is_some() || self.server_active_event.is_some())
                            && ui.small_button("✕ CLEAR SERVER ACTIVITY").clicked()
                        {
                            self.server_active_club = None;
                            self.server_active_event = None;
                            self.server_active_identity = None;
                            self.disarm_all_tx_with_persistence("Server context cleared", false);
                            ui.close();
                        }
                    }
                }
                let profile = self.activity.profile();
                let band_summary = if profile.bands.is_unrestricted() {
                    "all core".to_string()
                } else {
                    profile.bands.labels().join(", ")
                };
                let mode_summary = if profile.modes.is_unrestricted() {
                    "all core".to_string()
                } else {
                    profile
                        .modes
                        .modes()
                        .iter()
                        .map(|mode| mode.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "{}  ·  Bands {}  ·  Modes {}",
                        profile.tx_cq, band_summary, mode_summary,
                    ))
                    .small()
                    .color(Color32::GRAY),
                );
            },
        );
        activity_menu
            .response
            .on_hover_text("Choose the operating activity and any active server event");
        ui.spacing_mut().interact_size.y = previous_interact_height;
    }
}
