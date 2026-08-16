use super::super::*;

impl QsonautGuiApp {
    pub(in super::super) fn draw_status(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.heading("📡 Station Health");
        ui.label(
            RichText::new("What matters right now—not a wall of driver diagnostics.")
                .small()
                .color(Color32::GRAY),
        );
        ui.add_space(4.0);

        let update_age = snapshot
            .last_update
            .map(|last| last.elapsed().as_secs_f32());
        let (radio_value, radio_detail, radio_color) = match update_age {
            Some(age) if age < 3.0 => (
                "CONNECTED".to_string(),
                format!("Radio answered {:.1}s ago", age),
                Color32::LIGHT_GREEN,
            ),
            Some(age) => (
                "STALE".to_string(),
                format!("No fresh radio state for {:.1}s", age),
                Color32::YELLOW,
            ),
            None => (
                "WAITING".to_string(),
                "Waiting for the first radio update".to_string(),
                Color32::GRAY,
            ),
        };
        operator_status_card(
            ui,
            "📻 Radio link",
            &radio_value,
            &radio_detail,
            radio_color,
        );

        let (audio_value, audio_detail, audio_color) = snapshot.audio_level_dbfs.map_or_else(
            || {
                (
                    "NO LEVEL".to_string(),
                    snapshot.audio_spectrum_status.clone(),
                    Color32::YELLOW,
                )
            },
            |level| {
                let clipped = snapshot.audio_clip_percent > 0.1;
                (
                    if clipped {
                        "CLIPPING".to_string()
                    } else {
                        format!("{level:.0} dBFS")
                    },
                    format!(
                        "{} · {:.1}% clipped",
                        snapshot.audio_spectrum_status, snapshot.audio_clip_percent
                    ),
                    if clipped {
                        Color32::from_rgb(255, 110, 100)
                    } else if level < -45.0 {
                        Color32::YELLOW
                    } else {
                        Color32::LIGHT_GREEN
                    },
                )
            },
        );
        operator_status_card(
            ui,
            "🎧 Audio input",
            &audio_value,
            &audio_detail,
            audio_color,
        );

        let decode_status = match self.workspace_mode {
            WorkspaceMode::Ft8 => snapshot.ft8_decode_status.as_str(),
            _ => snapshot.digital_decode_status.as_str(),
        };
        let decode_detail = format!("mfsk-core · {decode_status}");
        operator_status_card(
            ui,
            "🔬 Decode engine",
            self.workspace_mode.label(),
            &decode_detail,
            if decode_status.contains("failed") || decode_status.contains("NO INPUT") {
                Color32::YELLOW
            } else {
                Color32::LIGHT_BLUE
            },
        );

        let (psk_value, psk_detail, psk_color) = if !self.psk_reporter_enabled {
            (
                "OFF",
                "Private by default · enable in Operator Profile".to_string(),
                Color32::GRAY,
            )
        } else if let Some(reporter) = &self.psk_reporter {
            let status = reporter.status();
            if let Some(error) = status.last_error {
                ("ERROR", error, Color32::from_rgb(255, 110, 100))
            } else {
                (
                    "ARMED",
                    format!(
                        "{} queued · {} sent · randomized five-minute batches",
                        status.queued, status.sent
                    ),
                    Color32::LIGHT_GREEN,
                )
            }
        } else {
            (
                "WAITING",
                "Set a real callsign and grid in Operator Profile".to_string(),
                Color32::YELLOW,
            )
        };
        operator_status_card(ui, "📡 PSK Reporter", psk_value, &psk_detail, psk_color);

        ui.horizontal(|ui| {
            ui.label(RichText::new("⚙ Compute policy").strong());
            let previous = self.compute_preference;
            egui::ComboBox::from_id_salt("compute_preference")
                .selected_text(self.compute_preference.label())
                .show_ui(ui, |ui| {
                    for preference in ComputePreference::ALL {
                        ui.selectable_value(
                            &mut self.compute_preference,
                            preference,
                            preference.label(),
                        );
                    }
                });
            if self.compute_preference != previous {
                self.acceleration_report = AccelerationReport::probe(self.compute_preference);
                self.profile_dirty = true;
                self.persist_profile("Compute policy saved to");
            }
        });
        let compute_detail = self
            .acceleration_report
            .fallback_reason
            .as_deref()
            .map(|reason| format!("{} · {reason}", self.acceleration_report.hardware_detail()))
            .unwrap_or_else(|| self.acceleration_report.hardware_detail());
        operator_status_card(
            ui,
            "🚀 Compute backend",
            &self.acceleration_report.summary(),
            &compute_detail,
            if self.acceleration_report.active == ActiveBackend::GpuCompute {
                Color32::from_rgb(210, 120, 255)
            } else {
                Color32::from_rgb(120, 210, 255)
            },
        );

        let gui_driver = std::env::var("GALLIUM_DRIVER").unwrap_or_default();
        let gui_adapter = std::env::var("MESA_D3D12_DEFAULT_ADAPTER_NAME")
            .unwrap_or_else(|_| "automatic adapter".to_string());
        let gui_renderer_detail = if gui_driver.eq_ignore_ascii_case("d3d12") {
            format!("{gui_adapter} preference · Mesa/WSLg")
        } else {
            "Override with GALLIUM_DRIVER; software rendering raises CPU load".to_string()
        };
        operator_status_card(
            ui,
            "🎨 GUI renderer",
            if gui_driver.eq_ignore_ascii_case("d3d12") {
                "D3D12 HARDWARE"
            } else if gui_driver.is_empty() {
                "SYSTEM DEFAULT"
            } else {
                &gui_driver
            },
            &gui_renderer_detail,
            if gui_driver.eq_ignore_ascii_case("d3d12") {
                Color32::LIGHT_GREEN
            } else {
                Color32::YELLOW
            },
        );

        let telemetry = if self.workspace_mode == WorkspaceMode::Ft8 {
            snapshot.ft8_compute_telemetry.as_ref()
        } else {
            snapshot.digital_compute_telemetry.as_ref()
        };
        if let Some(telemetry) = telemetry {
            operator_status_card(
                ui,
                "⏱ Last decode",
                &telemetry.concise(),
                &telemetry.stage_detail(),
                if telemetry.realtime_percent() > 80.0 {
                    Color32::YELLOW
                } else {
                    Color32::LIGHT_GREEN
                },
            );
        }

        operator_status_card(
            ui,
            "📊 Radio levels",
            &format!(
                "AF {} · RF {} · PWR {}",
                fmt_civ_level_percent(snapshot.af_gain),
                fmt_civ_level_percent(snapshot.rf_gain),
                fmt_civ_level_percent(snapshot.rf_power)
            ),
            &format!(
                "{} · {} · raw AF/RF/PWR: {}/{}/{}",
                snapshot
                    .filter
                    .map(|value| format!("FIL{value}"))
                    .unwrap_or_else(|| "filter unknown".to_string()),
                if snapshot.data_mode == Some(true) {
                    "data mode"
                } else {
                    "voice/CW mode"
                },
                fmt_opt_u8(snapshot.af_gain),
                fmt_opt_u8(snapshot.rf_gain),
                fmt_opt_u8(snapshot.rf_power)
            ),
            Color32::from_rgb(210, 190, 110),
        );

        if let Some(err) = &snapshot.last_error {
            ui.add_space(6.0);
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(70, 42, 20))
                .stroke(egui::Stroke::new(1.5_f32, Color32::YELLOW))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("⚠ NEEDS ATTENTION")
                            .strong()
                            .color(Color32::YELLOW),
                    );
                    ui.label(err);
                });
        }
    }
}
