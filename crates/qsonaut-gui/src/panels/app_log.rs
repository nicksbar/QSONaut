use super::super::*;

const APP_LOG_VIEW_BYTES: usize = 256 * 1024;
const APP_LOG_REFRESH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AppLogLineLevel {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
}

fn app_log_line_level(line: &str) -> AppLogLineLevel {
    let uppercase = line.to_ascii_uppercase();
    if uppercase.contains(" ERROR ") || uppercase.contains("PANIC") || uppercase.contains("FATAL") {
        AppLogLineLevel::Error
    } else if uppercase.contains(" WARN ") || uppercase.contains("WARNING") {
        AppLogLineLevel::Warning
    } else if uppercase.contains(" DEBUG ") {
        AppLogLineLevel::Debug
    } else if uppercase.contains(" TRACE ") {
        AppLogLineLevel::Trace
    } else {
        AppLogLineLevel::Info
    }
}

fn app_log_level_matches(level: AppLogLineLevel, filter: AppLogLevelFilter) -> bool {
    match filter {
        AppLogLevelFilter::All => true,
        AppLogLevelFilter::Info => level >= AppLogLineLevel::Info,
        AppLogLevelFilter::Warning => level >= AppLogLineLevel::Warning,
        AppLogLevelFilter::Error => level == AppLogLineLevel::Error,
    }
}

fn app_log_line_color(ui: &egui::Ui, line: &str) -> Color32 {
    match app_log_line_level(line) {
        AppLogLineLevel::Error => Color32::from_rgb(255, 105, 100),
        AppLogLineLevel::Warning => theme_warning(ui),
        AppLogLineLevel::Trace | AppLogLineLevel::Debug => theme_muted(ui),
        AppLogLineLevel::Info => {
            let uppercase = line.to_ascii_uppercase();
            if uppercase.contains("PTT")
                || uppercase.contains(" TX ")
                || uppercase.contains("TRANSMIT")
            {
                Color32::from_rgb(255, 172, 92)
            } else if uppercase.contains(" RX ")
                || uppercase.contains("DECODE")
                || uppercase.contains("RECEIVE")
            {
                Color32::from_rgb(108, 220, 224)
            } else {
                Color32::LIGHT_GRAY
            }
        }
    }
}

impl QsonautGuiApp {
    fn refresh_app_log(&mut self) {
        self.app_log_last_refresh = Instant::now();
        match read_log_tail(APP_LOG_VIEW_BYTES) {
            Ok(text) => {
                self.app_log_status = format!(
                    "Newest {} KiB · {}",
                    APP_LOG_VIEW_BYTES / 1024,
                    log_file_path().display()
                );
                self.app_log_text = text;
            }
            Err(error) => {
                self.app_log_text.clear();
                self.app_log_status = format!("Could not read application log: {error}");
            }
        }
    }

    pub(in super::super) fn draw_app_log_panel(&mut self, ui: &mut egui::Ui) {
        if self.app_log_status.is_empty()
            || self.app_log_last_refresh.elapsed() >= APP_LOG_REFRESH_INTERVAL
        {
            self.refresh_app_log();
        }
        ui.ctx().request_repaint_after(APP_LOG_REFRESH_INTERVAL);

        ui.horizontal_wrapped(|ui| {
            ui.heading("Application Log");
            ui.label(
                RichText::new("● LIVE")
                    .small()
                    .strong()
                    .color(Color32::LIGHT_GREEN),
            );
            if ui.small_button("Refresh").clicked() {
                self.refresh_app_log();
            }
            if ui.small_button("Clear log").clicked() {
                match clear_log() {
                    Ok(()) => {
                        self.app_log_text.clear();
                        self.app_log_last_refresh = Instant::now();
                        self.app_log_status =
                            format!("Log cleared · {}", log_file_path().display());
                    }
                    Err(error) => {
                        self.app_log_status = format!("Could not clear application log: {error}");
                    }
                }
            }
            ui.checkbox(&mut self.app_log_follow, "Follow bottom");
            if ui.small_button("Bottom").clicked() {
                self.app_log_follow = true;
            }
            if ui.small_button("Server privacy controls").clicked() {
                self.signal_panel_tab = SignalPanelTab::Server;
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.app_log_filter)
                    .desired_width(170.0)
                    .hint_text("text, target, error…"),
            );
            if ui.small_button("SSTV").clicked() {
                self.app_log_filter = "SSTV".to_string();
                self.app_log_level_filter = AppLogLevelFilter::All;
            }
            for component in [
                "Radio",
                "Voice",
                "FT8",
                "FT4",
                "HamDB",
                "Contest",
                "Activity",
                "Audio",
                "Device",
                "PSK",
                "Automation",
                "Server",
                "Ingress",
            ] {
                if ui.small_button(component).clicked() {
                    self.app_log_filter = component.to_string();
                    self.app_log_level_filter = AppLogLevelFilter::All;
                }
            }
            egui::ComboBox::from_id_salt("app_log_level_filter")
                .selected_text(self.app_log_level_filter.label())
                .show_ui(ui, |ui| {
                    for level in AppLogLevelFilter::ALL {
                        ui.selectable_value(&mut self.app_log_level_filter, level, level.label());
                    }
                });
            if ui
                .add_enabled(!self.app_log_filter.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                self.app_log_filter.clear();
            }
        });

        let query = self.app_log_filter.trim().to_ascii_lowercase();
        let total_lines = self.app_log_text.lines().count();
        let visible_lines = self
            .app_log_text
            .lines()
            .filter(|line| {
                app_log_level_matches(app_log_line_level(line), self.app_log_level_filter)
                    && (query.is_empty() || line.to_ascii_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();

        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} of {} lines · {}",
                    visible_lines.len(),
                    total_lines,
                    self.app_log_status
                ))
                .small()
                .color(theme_muted(ui)),
            );
            if ui
                .add_enabled(!visible_lines.is_empty(), egui::Button::new("Copy visible"))
                .clicked()
            {
                ui.ctx().copy_text(visible_lines.join("\n"));
            }
        });
        ui.label(
            RichText::new(
                "Local view only. Server upload requires a manual snapshot and the separate redacted-log option.",
            )
            .small()
            .color(theme_muted(ui)),
        );
        ui.separator();

        if visible_lines.is_empty() {
            ui.label(
                RichText::new(if self.app_log_text.is_empty() {
                    "The application log is currently empty."
                } else {
                    "No log lines match the current filters."
                })
                .italics()
                .color(theme_muted(ui)),
            );
            return;
        }

        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::both()
            .id_salt("app_log_stream")
            .auto_shrink([false, false])
            .stick_to_bottom(self.app_log_follow)
            .max_height(ui.available_height())
            .show_rows(ui, row_height, visible_lines.len(), |ui, rows| {
                for line in &visible_lines[rows] {
                    ui.add(
                        egui::Label::new(
                            RichText::new(*line)
                                .monospace()
                                .small()
                                .color(app_log_line_color(ui, line)),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_log_severity_for_filtering_and_highlighting() {
        assert_eq!(
            app_log_line_level("2026 ERROR radio failed"),
            AppLogLineLevel::Error
        );
        assert_eq!(
            app_log_line_level("2026 WARN retrying"),
            AppLogLineLevel::Warning
        );
        assert_eq!(
            app_log_line_level("2026 DEBUG poll"),
            AppLogLineLevel::Debug
        );
        assert_eq!(app_log_line_level("2026 INFO ready"), AppLogLineLevel::Info);
        assert!(app_log_level_matches(
            AppLogLineLevel::Error,
            AppLogLevelFilter::Warning
        ));
        assert!(!app_log_level_matches(
            AppLogLineLevel::Info,
            AppLogLevelFilter::Warning
        ));
    }
}
