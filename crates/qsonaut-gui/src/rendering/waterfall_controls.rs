use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn draw_waterfall_buttons(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let audio_button = ui
            .button(RichText::new("〰").size(16.0).color(Color32::LIGHT_BLUE))
            .on_hover_text("Audio waterfall controls");
        egui::Popup::menu(&audio_button)
            // Keep the drawer alive while nested combo boxes and sliders are
            // interacting with it. The banner redraws it every frame.
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(250.0);
                ui.label(RichText::new("AUDIO WATERFALL").strong());
                ui.label(RichText::new("Live audio spectrum display").small());
                ui.separator();
                ui.label(RichText::new("Theme").strong());
                ui.horizontal_wrapped(|ui| {
                    for theme in [
                        WaterfallTheme::RadioBlue,
                        WaterfallTheme::Inferno,
                        WaterfallTheme::Phosphor,
                        WaterfallTheme::Monochrome,
                    ] {
                        if ui
                            .selectable_label(self.waterfall_theme == theme, theme.label())
                            .clicked()
                        {
                            self.waterfall_theme = theme;
                            self.profile_dirty = true;
                            self.persist_profile("Waterfall theme saved to");
                        }
                    }
                });
                ui.label(RichText::new("Audio display speed").strong());
                {
                    let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
                    let selected = if tuning.audio_auto_visual {
                        "Auto"
                    } else {
                        tuning.audio_waterfall_speed.label()
                    };
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .selectable_label(tuning.audio_auto_visual, selected)
                            .clicked()
                        {
                            tuning.audio_auto_visual = true;
                        }
                        for speed in [
                            WaterfallSpeed::Fast,
                            WaterfallSpeed::Mid,
                            WaterfallSpeed::Slow,
                        ] {
                            let selected =
                                !tuning.audio_auto_visual && tuning.audio_waterfall_speed == speed;
                            if ui.selectable_label(selected, speed.label()).clicked() {
                                tuning.audio_auto_visual = false;
                                tuning.audio_waterfall_speed = speed;
                            }
                        }
                    });
                }
            });

        let radio_scope_available =
            self.config.radio.enabled && self.driver_metadata.scope.is_some();
        if radio_scope_available {
            let radio_button = ui
                .button(
                    RichText::new("🌈")
                        .size(15.0)
                        .color(Color32::from_rgb(180, 220, 255)),
                )
                .on_hover_text("Native CI-V waterfall controls");
            egui::Popup::menu(&radio_button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(280.0);
                    ui.label(RichText::new("RADIO WATERFALL").strong());
                    ui.label(RichText::new("Native scope stream controls").small());
                    ui.separator();
                    if ui
                        .checkbox(&mut self.civ_spectrum_on, "Enable radio waterfall")
                        .changed()
                    {
                        self.profile_dirty = true;
                        self.persist_profile("Radio waterfall setting saved to");
                    }
                    ui.label(RichText::new("Radio waterfall theme").strong());
                    ui.horizontal_wrapped(|ui| {
                        for theme in [
                            WaterfallTheme::RadioBlue,
                            WaterfallTheme::Inferno,
                            WaterfallTheme::Phosphor,
                            WaterfallTheme::Monochrome,
                        ] {
                            if ui
                                .selectable_label(
                                    self.radio_waterfall_theme == theme,
                                    theme.label(),
                                )
                                .clicked()
                            {
                                self.radio_waterfall_theme = theme;
                                self.profile_dirty = true;
                                self.persist_profile("Radio waterfall theme saved to");
                            }
                        }
                    });
                    ui.label(RichText::new("Native sweep speed").strong());
                    let mut visual_changed = false;
                    {
                        let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
                        ui.horizontal_wrapped(|ui| {
                            ui.label(if tuning.radio_auto_visual {
                                "Auto (mode-driven)"
                            } else {
                                "Native speed"
                            });
                            for speed in [
                                WaterfallSpeed::Fast,
                                WaterfallSpeed::Mid,
                                WaterfallSpeed::Slow,
                            ] {
                                let selected = !tuning.radio_auto_visual
                                    && tuning.radio_waterfall_speed == speed;
                                if ui.selectable_label(selected, speed.label()).clicked() {
                                    tuning.radio_auto_visual = false;
                                    tuning.radio_waterfall_speed = speed;
                                    visual_changed = true;
                                }
                            }
                        });
                    }
                    if visual_changed {
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .radio_scope_settings_dirty = true;
                    }
                    let mut scope_changed = false;
                    ui.horizontal(|ui| {
                        scope_changed |= ui
                            .selectable_value(
                                &mut self.radio_scope_view,
                                RadioScopeView::Narrow,
                                "Narrow",
                            )
                            .changed();
                        scope_changed |= ui
                            .selectable_value(
                                &mut self.radio_scope_view,
                                RadioScopeView::Overview,
                                "Overview",
                            )
                            .changed();
                    });
                    scope_changed |= ui
                        .checkbox(&mut self.radio_scope_vbw_wide, "Wide VBW")
                        .changed();
                    ui.checkbox(&mut self.radio_scope_lock_if_to_filter, "Match span to FIL");
                    if self.radio_scope_lock_if_to_filter {
                        self.radio_scope_span_code = scope_span_for_filter_with_options(
                            &snapshot.mode,
                            self.driver_metadata.filter_bandwidth_hz(
                                &snapshot.mode,
                                snapshot.filter.unwrap_or_default(),
                            ),
                            self.driver_metadata
                                .scope
                                .map(|metadata| metadata.span_options_hz)
                                .unwrap_or(&[]),
                        );
                        ui.label(format!(
                            "Automatic span: {}",
                            scope_span_label_for(
                                self.driver_metadata.scope,
                                self.radio_scope_span_code
                            )
                        ));
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            let spans = self
                                .driver_metadata
                                .scope
                                .map(|metadata| metadata.span_options_hz)
                                .unwrap_or(&[]);
                            for (code, span_hz) in spans.iter().copied().enumerate() {
                                let code = code as u8;
                                let label = format!("±{} kHz", span_hz / 1_000);
                                if ui
                                    .selectable_label(self.radio_scope_span_code == code, label)
                                    .clicked()
                                {
                                    self.radio_scope_span_code = code;
                                    scope_changed = true;
                                }
                            }
                        });
                    }
                    scope_changed |= ui.checkbox(&mut self.radio_scope_hold, "Hold").changed();
                    scope_changed |= ui
                        .add(
                            egui::Slider::new(
                                &mut self.radio_scope_reference_tenths_db,
                                -200..=200,
                            )
                            .step_by(5.0)
                            .custom_formatter(|value, _| format!("{:.1} dB", value / 10.0))
                            .text("Reference"),
                        )
                        .changed();
                    if scope_changed {
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .radio_scope_settings_dirty = true;
                    }
                });
        }

        ui.add_enabled(
            false,
            egui::Button::new(
                RichText::new("📡")
                    .size(13.0)
                    .color(Color32::from_gray(145)),
            ),
        )
        .on_disabled_hover_text(
            "IQ/SDR waterfall support is not enabled yet — development needs a radio that offers a supported IQ/SDR stream 😢",
        );
    }
}
