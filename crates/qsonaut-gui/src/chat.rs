use super::*;
use crate::third_party::ThirdPartyChatEvent;

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
}

impl QsonautGuiApp {
    pub(crate) fn poll_third_party_chat(&mut self) {
        let Some(bridge) = self.third_party_bridge.as_ref() else {
            return;
        };
        for event in bridge.poll_chat_events() {
            match event {
                ThirdPartyChatEvent::Message { from, text } => {
                    self.chat_users.insert(
                        from.clone(),
                        ChatUser {
                            callsign: from.clone(),
                            source: ChatSource::N3fjp,
                            last_seen: chat_now(),
                        },
                    );
                    if self.signal_panel_tab != SignalPanelTab::Chat {
                        self.chat_unread = self.chat_unread.saturating_add(1);
                    }
                    self.chat_messages.push_back(UnifiedChatMessage {
                        source: ChatSource::N3fjp,
                        author: from,
                        message: text,
                        utc: chat_now(),
                        outgoing: false,
                    });
                }
                ThirdPartyChatEvent::Users(users) => {
                    for callsign in users {
                        self.chat_users.insert(
                            callsign.clone(),
                            ChatUser {
                                callsign,
                                source: ChatSource::N3fjp,
                                last_seen: chat_now(),
                            },
                        );
                    }
                }
            }
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
        ui.heading("Chat");
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("UNIFIED STREAM")
                    .strong()
                    .color(theme_accent(ui)),
            );
            ui.label(
                RichText::new("JS8 active · N3FJP/LAN/Server ready to join")
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
                    for user in self.chat_users.values() {
                        ui.label(format!("● {} · {}", user.callsign, user.source.label()))
                            .on_hover_text(format!("Last seen {}", user.last_seen));
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
                        let fill = if message.outgoing {
                            Color32::from_rgb(53, 43, 25)
                        } else {
                            Color32::from_rgb(25, 49, 38)
                        };
                        egui::Frame::group(ui.style()).fill(fill).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "[{}] {}",
                                        message.source.label(),
                                        message.author
                                    ))
                                    .strong()
                                    .color(theme_accent(ui)),
                                );
                                ui.label(&message.message);
                                ui.label(
                                    RichText::new(&message.utc).small().color(theme_muted(ui)),
                                );
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
                    .hint_text("Message or /users /help /source js8"),
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
                || ui.small_button("Send").clicked()
            {
                self.submit_chat_command();
            }
        });
        ui.label(
            RichText::new("Commands: /help · /users · /source js8 · /clear")
                .small()
                .color(theme_muted(ui)),
        );
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
        } else if input.starts_with('/') {
            self.chat_messages.push_back(UnifiedChatMessage {
                source: ChatSource::Js8,
                author: "SYSTEM".to_string(),
                message: format!("Unknown command: {input}"),
                utc: "now".to_string(),
                outgoing: false,
            });
        } else {
            self.digital_compose = input;
            self.signal_panel_tab = SignalPanelTab::Chat;
            self.chat_messages.push_back(UnifiedChatMessage {
                source: ChatSource::Js8,
                author: self.station_callsign_or_default().to_string(),
                message: format!("QUEUED FOR JS8: {}", self.digital_compose),
                utc: "now".to_string(),
                outgoing: true,
            });
        }
        self.chat_compose.clear();
    }
}

fn chat_now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "now".to_string())
}
