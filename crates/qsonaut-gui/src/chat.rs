use super::*;
use crate::third_party::{ThirdPartyChatEvent, ThirdPartyUserSource};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ChatSource {
    #[default]
    Js8,
    N3fjp,
    Lan,
    Server,
}

impl ChatSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Js8 => "JS8",
            Self::N3fjp => "N3FJP",
            Self::Lan => "LAN",
            Self::Server => "SERVER",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UnifiedChatMessage {
    pub(crate) source: ChatSource,
    pub(crate) author: String,
    pub(crate) message: String,
    pub(crate) utc: String,
    pub(crate) outgoing: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ChatUser {
    pub(crate) callsign: String,
    pub(crate) source: ChatSource,
    pub(crate) last_seen: String,
    pub(crate) last_seen_epoch: u64,
}

impl QsonautGuiApp {
    pub(crate) fn poll_third_party_chat(&mut self) {
        let Some(bridge) = self.third_party_bridge.as_ref() else {
            return;
        };
        for event in bridge.poll_chat_events() {
            match event {
                ThirdPartyChatEvent::Message { source, from, text } => {
                    let source = match source {
                        ThirdPartyUserSource::N3fjp => ChatSource::N3fjp,
                        ThirdPartyUserSource::Lan => ChatSource::Lan,
                    };
                    self.chat_users.insert(
                        from.clone(),
                        ChatUser {
                            callsign: from.clone(),
                            source,
                            last_seen: chat_now(),
                            last_seen_epoch: chat_now_epoch(),
                        },
                    );
                    if self.signal_panel_tab != SignalPanelTab::Chat {
                        self.chat_unread = self.chat_unread.saturating_add(1);
                    }
                    self.chat_messages.push_back(UnifiedChatMessage {
                        source,
                        author: from,
                        message: text,
                        utc: chat_now(),
                        outgoing: false,
                    });
                }
                ThirdPartyChatEvent::Users { source, users } => {
                    let source = match source {
                        ThirdPartyUserSource::N3fjp => ChatSource::N3fjp,
                        ThirdPartyUserSource::Lan => ChatSource::Lan,
                    };
                    for callsign in users {
                        self.chat_users.insert(
                            callsign.clone(),
                            ChatUser {
                                callsign,
                                source,
                                last_seen: chat_now(),
                                last_seen_epoch: chat_now_epoch(),
                            },
                        );
                    }
                }
            }
        }
        let stale_before = chat_now_epoch().saturating_sub(45);
        self.chat_users.retain(|_, user| {
            user.source != ChatSource::Lan || user.last_seen_epoch >= stale_before
        });
    }

    pub(crate) fn poll_server_chat(&mut self) {
        let Some(client) = self.server_client.as_ref() else {
            return;
        };
        for message in client.drain_channel_messages() {
            if !self.chat_seen_server.insert(message.id) {
                continue;
            }
            self.chat_users.insert(
                message.author_callsign.clone(),
                ChatUser {
                    callsign: message.author_callsign.clone(),
                    source: ChatSource::Server,
                    last_seen: message.created_at.clone(),
                    last_seen_epoch: chat_now_epoch(),
                },
            );
            let outgoing = message
                .author_callsign
                .eq_ignore_ascii_case(self.station_callsign_or_default());
            let display_message = format!("#{} · {}", message.channel, message.message);
            if outgoing && is_duplicate_server_echo(&self.chat_messages, &display_message) {
                continue;
            }
            if !outgoing && self.signal_panel_tab != SignalPanelTab::Chat {
                self.chat_unread = self.chat_unread.saturating_add(1);
            }
            self.chat_messages.push_back(UnifiedChatMessage {
                source: ChatSource::Server,
                author: message.author_callsign,
                message: display_message,
                utc: message.created_at,
                outgoing,
            });
        }
        while self.chat_messages.len() > 300 {
            self.chat_messages.pop_front();
        }
        if self.chat_seen_server.len() > 600 {
            self.chat_seen_server.clear();
        }
    }

    pub(crate) fn sync_js8_chat(&mut self, snapshot: &GuiState) {
        for entry in &snapshot.digital_decodes {
            if entry.mode != WorkspaceMode::Js8 {
                continue;
            }
            let key = (entry.period, entry.message.clone());
            if !self.chat_seen_js8.insert(key) {
                continue;
            }
            let author = entry
                .message
                .split_whitespace()
                .next()
                .unwrap_or("UNKNOWN")
                .to_ascii_uppercase();
            if author != self.station_callsign_or_default() {
                self.chat_users.insert(
                    author.clone(),
                    ChatUser {
                        callsign: author.clone(),
                        source: ChatSource::Js8,
                        last_seen: entry.utc.clone(),
                        last_seen_epoch: chat_now_epoch(),
                    },
                );
                if self.signal_panel_tab != SignalPanelTab::Chat {
                    self.chat_unread = self.chat_unread.saturating_add(1);
                }
            }
            self.chat_messages.push_back(UnifiedChatMessage {
                source: ChatSource::Js8,
                author,
                message: entry.message.clone(),
                utc: entry.utc.clone(),
                outgoing: false,
            });
        }
        for entry in &self.digital_tx_chat {
            let key = (entry.period, entry.message.clone());
            if !self.chat_seen_js8.insert(key) {
                continue;
            }
            self.chat_messages.push_back(UnifiedChatMessage {
                source: ChatSource::Js8,
                author: self.station_callsign_or_default().to_string(),
                message: entry.message.clone(),
                utc: entry.utc.clone(),
                outgoing: true,
            });
        }
        while self.chat_messages.len() > 300 {
            self.chat_messages.pop_front();
        }
        if self.chat_seen_js8.len() > 600 {
            self.chat_seen_js8.clear();
        }
    }

    pub(crate) fn draw_chat_panel(&mut self, ui: &mut egui::Ui) {
        let third_party_status = self
            .third_party_bridge
            .as_ref()
            .map(|bridge| bridge.status())
            .unwrap_or_default();
        let lan_available = third_party_status.lan == "READY";
        let n3fjp_available = third_party_status.network == "CONNECTED";
        let server_available = self
            .server_client
            .as_ref()
            .is_some_and(|client| client.status().state == ServerConnectionState::Connected);

        ui.horizontal(|ui| {
            ui.heading("Chat");
            if ui
                .small_button("?")
                .on_hover_text("Chat help and commands")
                .clicked()
            {
                self.chat_help_open = !self.chat_help_open;
            }
            if self.chat_unread > 0 {
                ui.label(
                    RichText::new(format!("{} unread", self.chat_unread))
                        .small()
                        .color(theme_accent(ui)),
                );
            }
        });
        if self.chat_help_open {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(RichText::new("Chat help").strong());
                ui.label("Plain messages are sent to every selected available destination.");
                ui.label("LAN sends to all trusted peers when no target is specified; Server uses the selected channel; N3FJP uses its configured target.");
                ui.label("Commands:");
                ui.label("/lan CALLSIGN message — send only to a trusted LAN peer");
                ui.label("/server message — send only to the current Server channel");
                ui.label("/server CHANNEL message — send only to a named Server channel");
                ui.label("/n3fjp CALLSIGN message — send only through N3FJP");
                ui.label("/users — show heard users · /clear — clear the stream");
                ui.label("JS8 decodes appear in the stream, but Chat does not transmit through JS8.");
            });
            ui.add_space(4.0);
        }
        let lan_before = self.config.third_party.lan_discovery.clone();
        ui.collapsing("LAN discovery", |ui| {
            ui.checkbox(
                &mut self.config.third_party.lan_discovery.enabled,
                "Announce this station and discover nearby QSONaut users",
            );
            ui.horizontal(|ui| {
                ui.label("Discovery port");
                ui.add_enabled(
                    self.config.third_party.lan_discovery.enabled,
                    egui::DragValue::new(&mut self.config.third_party.lan_discovery.port)
                        .range(1..=u16::MAX),
                );
            });
            ui.label(
                "Discovery is opt-in. It shows nearby stations here; approve peers before allowing LAN chat.",
            );
            if self.config.third_party.lan_discovery != lan_before {
                if ui.button("Apply LAN settings").clicked() {
                    self.persist_profile("LAN settings saved to");
                    self.restart_third_party_bridge();
                }
            }
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("Server channel");
            ui.add(
                egui::TextEdit::singleline(&mut self.chat_server_channel)
                    .desired_width(150.0)
                    .hint_text("general"),
            );
        });
        let routing_before = (
            self.chat_route_lan,
            self.chat_route_server,
            self.chat_route_n3fjp,
            self.chat_lan_target.clone(),
            self.chat_n3fjp_target.clone(),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Default destinations");
            ui.add_enabled_ui(lan_available, |ui| {
                ui.checkbox(&mut self.chat_route_lan, "LAN");
            });
            ui.add_enabled_ui(server_available, |ui| {
                ui.checkbox(&mut self.chat_route_server, "Server");
            });
            ui.add_enabled_ui(n3fjp_available, |ui| {
                ui.checkbox(&mut self.chat_route_n3fjp, "N3FJP");
            });
            ui.label(
                RichText::new(format!(
                    "LAN {} · Server {} · N3FJP {}",
                    if lan_available { "ready" } else { "offline" },
                    if server_available {
                        "connected"
                    } else {
                        "offline"
                    },
                    if n3fjp_available {
                        "connected"
                    } else {
                        "offline"
                    },
                ))
                .small()
                .color(theme_muted(ui)),
            );
        });
        if !lan_available {
            self.chat_route_lan = false;
        }
        if !server_available {
            self.chat_route_server = false;
        }
        if !n3fjp_available {
            self.chat_route_n3fjp = false;
        }
        if self.chat_route_lan {
            let lan_target = self.chat_lan_target.trim().to_string();
            let lan_target_trusted = lan_target.is_empty()
                || self
                    .config
                    .third_party
                    .lan_discovery
                    .trusted_callsigns
                    .iter()
                    .any(|peer| peer.eq_ignore_ascii_case(&lan_target));
            ui.horizontal(|ui| {
                ui.label("LAN target");
                ui.add(
                    egui::TextEdit::singleline(&mut self.chat_lan_target)
                        .desired_width(150.0)
                        .hint_text("blank = all trusted peers"),
                );
                if ui.small_button("Clear").clicked() {
                    self.chat_lan_target.clear();
                }
                if lan_target.is_empty() {
                    ui.label(
                        RichText::new("All trusted peers")
                            .small()
                            .color(theme_muted(ui)),
                    );
                } else if lan_target_trusted {
                    ui.label(
                        RichText::new("Trusted target")
                            .small()
                            .color(Color32::from_rgb(126, 220, 142)),
                    );
                } else {
                    ui.label(
                        RichText::new("Not trusted — clear or trust this peer below")
                            .small()
                            .color(Color32::from_rgb(230, 170, 90)),
                    );
                }
            });
        }
        if self.chat_route_n3fjp {
            ui.horizontal(|ui| {
                ui.label("N3FJP target");
                ui.add(
                    egui::TextEdit::singleline(&mut self.chat_n3fjp_target)
                        .desired_width(150.0)
                        .hint_text("* or callsign"),
                );
            });
        }
        if (
            self.chat_route_lan,
            self.chat_route_server,
            self.chat_route_n3fjp,
            self.chat_lan_target.clone(),
            self.chat_n3fjp_target.clone(),
        ) != routing_before
        {
            self.persist_profile("Chat routing defaults saved to");
        }
        ui.add_space(3.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("UNIFIED STREAM")
                    .strong()
                    .color(theme_accent(ui)),
            );
            ui.label(
                RichText::new("JS8 active · N3FJP/LAN/Server unified")
                    .small()
                    .color(theme_muted(ui)),
            );
            if ui.small_button("Mark read").clicked() {
                self.chat_unread = 0;
            }
        });
        ui.separator();
        ui.columns(2, |columns| {
            columns[0].heading("Users");
            columns[0].label(
                RichText::new(format!("{} heard", self.chat_users.len()))
                    .small()
                    .color(theme_muted(&columns[0])),
            );
            egui::ScrollArea::vertical()
                .id_salt("chat-users")
                .show(&mut columns[0], |ui| {
                    let users = self
                        .chat_users
                        .values()
                        .map(|user| {
                            (
                                user.callsign.clone(),
                                user.source,
                                user.last_seen.clone(),
                                user.last_seen_epoch,
                            )
                        })
                        .collect::<Vec<_>>();
                    let now = chat_now_epoch();
                    for (callsign, source, last_seen, last_seen_epoch) in users {
                        ui.horizontal(|ui| {
                            let online = source != ChatSource::Lan
                                || now.saturating_sub(last_seen_epoch) <= 45;
                            let marker = if online { "●" } else { "○" };
                            ui.label(format!("{} {} · {}", marker, callsign, source.label()))
                                .on_hover_text(format!("Last seen {}", last_seen));
                            if source == ChatSource::Lan {
                                let trusted = self
                                    .config
                                    .third_party
                                    .lan_discovery
                                    .trusted_callsigns
                                    .iter()
                                    .any(|peer| peer.eq_ignore_ascii_case(&callsign));
                                let blocked = self
                                    .config
                                    .third_party
                                    .lan_discovery
                                    .blocked_callsigns
                                    .iter()
                                    .any(|peer| peer.eq_ignore_ascii_case(&callsign));
                                if !trusted && !blocked && ui.small_button("Trust").clicked() {
                                    let peers = &mut self
                                        .config
                                        .third_party
                                        .lan_discovery
                                        .trusted_callsigns;
                                    if !trusted {
                                        peers.push(callsign.clone());
                                    }
                                    self.config
                                        .third_party
                                        .lan_discovery
                                        .blocked_callsigns
                                        .retain(|peer| !peer.eq_ignore_ascii_case(&callsign));
                                    if let Some(bridge) = self.third_party_bridge.as_ref() {
                                        bridge.set_lan_trust(callsign.clone(), true);
                                    }
                                    self.profile_dirty = true;
                                    self.persist_profile("LAN peer trust saved to");
                                }
                                if trusted && ui.small_button("Untrust").clicked() {
                                    self.config
                                        .third_party
                                        .lan_discovery
                                        .trusted_callsigns
                                        .retain(|peer| !peer.eq_ignore_ascii_case(&callsign));
                                    if let Some(bridge) = self.third_party_bridge.as_ref() {
                                        bridge.clear_lan_trust(callsign.clone());
                                    }
                                    self.profile_dirty = true;
                                    self.persist_profile("LAN peer trust removed from");
                                }
                                if !blocked && !trusted && ui.small_button("Block").clicked() {
                                    self.config
                                        .third_party
                                        .lan_discovery
                                        .trusted_callsigns
                                        .retain(|peer| !peer.eq_ignore_ascii_case(&callsign));
                                    if !blocked {
                                        self.config
                                            .third_party
                                            .lan_discovery
                                            .blocked_callsigns
                                            .push(callsign.clone());
                                    }
                                    if let Some(bridge) = self.third_party_bridge.as_ref() {
                                        bridge.set_lan_trust(callsign.clone(), false);
                                    }
                                    self.profile_dirty = true;
                                    self.persist_profile("LAN peer block saved to");
                                }
                                if blocked && ui.small_button("Unblock").clicked() {
                                    self.config
                                        .third_party
                                        .lan_discovery
                                        .blocked_callsigns
                                        .retain(|peer| !peer.eq_ignore_ascii_case(&callsign));
                                    if let Some(bridge) = self.third_party_bridge.as_ref() {
                                        bridge.clear_lan_trust(callsign.clone());
                                    }
                                    self.profile_dirty = true;
                                    self.persist_profile("LAN peer block removed from");
                                }
                            }
                        });
                    }
                });
            columns[1].heading("Messages");
            egui::ScrollArea::vertical()
                .id_salt("unified-chat")
                .stick_to_bottom(true)
                .show(&mut columns[1], |ui| {
                    if self.chat_messages.is_empty() {
                        ui.label(
                            RichText::new(
                                "No chat yet. JS8 messages will appear here as they are decoded.",
                            )
                            .color(theme_muted(ui)),
                        );
                    }
                    for message in &self.chat_messages {
                        let outgoing = message.outgoing;
                        let fill = if outgoing {
                            Color32::from_rgb(53, 43, 25)
                        } else {
                            Color32::from_rgb(25, 49, 38)
                        };
                        egui::Frame::group(ui.style()).fill(fill).show(ui, |ui| {
                            let layout = if outgoing {
                                egui::Layout::right_to_left(egui::Align::TOP)
                            } else {
                                egui::Layout::left_to_right(egui::Align::TOP)
                            };
                            ui.with_layout(layout, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    if outgoing {
                                        ui.label(
                                            RichText::new("✓")
                                                .strong()
                                                .color(Color32::from_rgb(126, 220, 142)),
                                        )
                                        .on_hover_text("Sent to the selected destination");
                                    }
                                    ui.label(&message.message);
                                    if message.author == "SYSTEM" {
                                        ui.label(
                                            RichText::new("Notice")
                                                .small()
                                                .italics()
                                                .color(theme_muted(ui)),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new(format!(
                                                "[{}] {}",
                                                message.source.label(),
                                                message.author
                                            ))
                                            .strong()
                                            .color(theme_accent(ui)),
                                        );
                                    }
                                    ui.label(
                                        RichText::new(&message.utc).small().color(theme_muted(ui)),
                                    );
                                });
                            });
                        });
                    }
                });
        });
        ui.separator();
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.chat_compose)
                    .desired_width(ui.available_width() - 75.0)
                    .hint_text("Message or /lan CALLSIGN message /server ops message"),
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
                || ui.small_button("Send").clicked()
            {
                self.submit_chat_command();
            }
        });
    }

    fn submit_chat_command(&mut self) {
        let input = self.chat_compose.trim().to_string();
        if input.is_empty() {
            return;
        }
        if input == "/clear" {
            self.chat_messages.clear();
        } else if input == "/users" {
            self.chat_messages.push_back(UnifiedChatMessage {
                source: ChatSource::Js8,
                author: "SYSTEM".to_string(),
                message: format!("{} users heard on JS8", self.chat_users.len()),
                utc: "now".to_string(),
                outgoing: false,
            });
        } else if input == "/help" {
            self.chat_messages.push_back(UnifiedChatMessage {
                source: ChatSource::Js8,
                author: "SYSTEM".to_string(),
                message:
                    "/users /source js8 /clear · JS8 transmit remains controlled by the JS8 panel"
                        .to_string(),
                utc: "now".to_string(),
                outgoing: false,
            });
        } else if let Some(payload) = input.strip_prefix("/n3fjp ") {
            let mut parts = payload.splitn(2, char::is_whitespace);
            let to = parts.next().unwrap_or("*").trim();
            let text = parts.next().unwrap_or_default().trim();
            if to.is_empty() || text.is_empty() {
                self.chat_messages.push_back(UnifiedChatMessage {
                    source: ChatSource::N3fjp,
                    author: "SYSTEM".to_string(),
                    message: "Usage: /n3fjp CALLSIGN message".to_string(),
                    utc: chat_now(),
                    outgoing: false,
                });
            } else if let Some(bridge) = self.third_party_bridge.as_ref() {
                bridge.send_chat(to, text);
                self.chat_messages.push_back(UnifiedChatMessage {
                    source: ChatSource::N3fjp,
                    author: self.station_callsign_or_default().to_string(),
                    message: text.to_string(),
                    utc: chat_now(),
                    outgoing: true,
                });
            } else {
                self.chat_messages.push_back(UnifiedChatMessage {
                    source: ChatSource::N3fjp,
                    author: "SYSTEM".to_string(),
                    message: "N3FJP station network is not connected".to_string(),
                    utc: chat_now(),
                    outgoing: false,
                });
            }
        } else if let Some(payload) = input.strip_prefix("/server ") {
            let mut parts = payload.splitn(2, char::is_whitespace);
            let first = parts.next().unwrap_or_default().trim();
            let second = parts.next().unwrap_or_default().trim();
            let (channel, text) = if second.is_empty() {
                (
                    self.chat_server_channel.trim().to_string(),
                    first.to_string(),
                )
            } else {
                (first.to_string(), second.to_string())
            };
            if channel.is_empty() || text.is_empty() {
                self.chat_messages.push_back(UnifiedChatMessage {
                    source: ChatSource::Server,
                    author: "SYSTEM".to_string(),
                    message: "Usage: /server CHANNEL message".to_string(),
                    utc: chat_now(),
                    outgoing: false,
                });
            } else if let Some(client) = self.server_client.as_ref() {
                if client.status().state == ServerConnectionState::Connected {
                    client.publish_channel_message(&channel, &text);
                    let author = self.station_callsign_or_default().to_string();
                    self.push_outgoing_server_chat(&channel, &author, &text);
                } else {
                    self.chat_messages.push_back(UnifiedChatMessage {
                        source: ChatSource::Server,
                        author: "SYSTEM".to_string(),
                        message: "QSONaut Server is not connected".to_string(),
                        utc: chat_now(),
                        outgoing: false,
                    });
                }
            } else {
                self.chat_messages.push_back(UnifiedChatMessage {
                    source: ChatSource::Server,
                    author: "SYSTEM".to_string(),
                    message: "QSONaut Server is disabled".to_string(),
                    utc: chat_now(),
                    outgoing: false,
                });
            }
        } else if let Some(payload) = input.strip_prefix("/lan ") {
            let mut parts = payload.splitn(2, char::is_whitespace);
            let to = parts.next().unwrap_or_default().trim();
            let text = parts.next().unwrap_or_default().trim();
            let trusted = self
                .config
                .third_party
                .lan_discovery
                .trusted_callsigns
                .iter()
                .any(|peer| peer.eq_ignore_ascii_case(to));
            if to.is_empty() || text.is_empty() {
                self.chat_messages.push_back(UnifiedChatMessage {
                    source: ChatSource::Lan,
                    author: "SYSTEM".to_string(),
                    message: "Usage: /lan CALLSIGN message".to_string(),
                    utc: chat_now(),
                    outgoing: false,
                });
            } else if !trusted {
                self.chat_messages.push_back(UnifiedChatMessage {
                    source: ChatSource::Lan,
                    author: "SYSTEM".to_string(),
                    message: "Trust this LAN peer before sending messages".to_string(),
                    utc: chat_now(),
                    outgoing: false,
                });
            } else if let Some(bridge) = self.third_party_bridge.as_ref() {
                bridge.send_lan_chat(to, text);
                self.chat_messages.push_back(UnifiedChatMessage {
                    source: ChatSource::Lan,
                    author: self.station_callsign_or_default().to_string(),
                    message: text.to_string(),
                    utc: chat_now(),
                    outgoing: true,
                });
            }
        } else if input.starts_with('/') {
            self.chat_messages.push_back(UnifiedChatMessage {
                source: ChatSource::Js8,
                author: "SYSTEM".to_string(),
                message: format!("Unknown command: {input}"),
                utc: "now".to_string(),
                outgoing: false,
            });
        } else {
            self.submit_default_routes(&input);
        }
        self.chat_compose.clear();
    }

    fn submit_default_routes(&mut self, text: &str) {
        let mut sent = false;
        let author = self.station_callsign_or_default().to_string();
        if self.chat_route_lan {
            let target = self.chat_lan_target.trim();
            let trusted = lan_default_target_allowed(
                target,
                &self.config.third_party.lan_discovery.trusted_callsigns,
            );
            if !trusted {
                self.push_chat_system(
                    "LAN target is not trusted; clear the target to broadcast to trusted peers",
                );
            } else if let Some(bridge) = self.third_party_bridge.as_ref() {
                bridge.send_lan_chat(target, text);
                self.push_outgoing_chat(ChatSource::Lan, &author, text);
                sent = true;
            } else {
                self.push_chat_system("LAN integration is not connected");
            }
        }
        if self.chat_route_server {
            let channel = self.chat_server_channel.trim().to_string();
            if channel.is_empty() {
                self.push_chat_system("Server default requires a channel");
            } else if let Some(client) = self.server_client.as_ref() {
                if client.status().state == ServerConnectionState::Connected {
                    client.publish_channel_message(&channel, text);
                    self.push_outgoing_server_chat(&channel, &author, text);
                    sent = true;
                } else {
                    self.push_chat_system("QSONaut Server is not connected");
                }
            } else {
                self.push_chat_system("QSONaut Server is disabled");
            }
        }
        if self.chat_route_n3fjp {
            let target = self.chat_n3fjp_target.trim();
            if target.is_empty() {
                self.push_chat_system("N3FJP default requires a target");
            } else if let Some(bridge) = self.third_party_bridge.as_ref() {
                bridge.send_chat(target, text);
                self.push_outgoing_chat(ChatSource::N3fjp, &author, text);
                sent = true;
            } else {
                self.push_chat_system("N3FJP station network is not connected");
            }
        }
        if !sent && !self.chat_route_lan && !self.chat_route_server && !self.chat_route_n3fjp {
            self.push_chat_system("Select at least one default destination");
        }
    }

    fn push_outgoing_chat(&mut self, source: ChatSource, author: &str, text: &str) {
        self.chat_messages.push_back(UnifiedChatMessage {
            source,
            author: author.to_string(),
            message: text.to_string(),
            utc: chat_now(),
            outgoing: true,
        });
    }

    fn push_outgoing_server_chat(&mut self, channel: &str, author: &str, text: &str) {
        self.chat_messages.push_back(UnifiedChatMessage {
            source: ChatSource::Server,
            author: author.to_string(),
            message: format!("#{channel} · {text}"),
            utc: chat_now(),
            outgoing: true,
        });
    }

    fn push_chat_system(&mut self, message: &str) {
        self.chat_messages.push_back(UnifiedChatMessage {
            source: ChatSource::Js8,
            author: "SYSTEM".to_string(),
            message: message.to_string(),
            utc: chat_now(),
            outgoing: false,
        });
    }
}

fn lan_default_target_allowed(target: &str, trusted_callsigns: &[String]) -> bool {
    target.is_empty()
        || trusted_callsigns
            .iter()
            .any(|peer| peer.eq_ignore_ascii_case(target))
}

fn is_duplicate_server_echo(
    messages: &VecDeque<UnifiedChatMessage>,
    display_message: &str,
) -> bool {
    messages.iter().any(|existing| {
        existing.outgoing
            && existing.source == ChatSource::Server
            && existing.message == display_message
    })
}

fn chat_now() -> String {
    chat_now_epoch().to_string()
}

fn chat_now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        is_duplicate_server_echo, lan_default_target_allowed, ChatSource, UnifiedChatMessage,
    };
    use std::collections::VecDeque;

    #[test]
    fn blank_lan_target_means_all_trusted_peers() {
        assert!(lan_default_target_allowed(
            "",
            &["N7UF".to_string(), "W1AW".to_string()]
        ));
    }

    #[test]
    fn lan_target_must_be_trusted_when_specific() {
        let trusted = vec!["N7UF".to_string()];
        assert!(lan_default_target_allowed("n7uf", &trusted));
        assert!(!lan_default_target_allowed("W1AW", &trusted));
    }

    #[test]
    fn only_matching_outgoing_server_echo_is_suppressed() {
        let mut messages = VecDeque::new();
        messages.push_back(UnifiedChatMessage {
            source: ChatSource::Server,
            author: "N7UF".to_string(),
            message: "#general · hello".to_string(),
            utc: "local".to_string(),
            outgoing: true,
        });

        assert!(is_duplicate_server_echo(&messages, "#general · hello"));
        assert!(!is_duplicate_server_echo(&messages, "#general · different"));
    }

    #[test]
    fn incoming_server_message_is_not_suppressed_as_echo() {
        let mut messages = VecDeque::new();
        messages.push_back(UnifiedChatMessage {
            source: ChatSource::Server,
            author: "N7UF".to_string(),
            message: "#general · hello".to_string(),
            utc: "local".to_string(),
            outgoing: false,
        });

        assert!(!is_duplicate_server_echo(&messages, "#general · hello"));
    }
}
