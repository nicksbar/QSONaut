use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn read_radio_profile(&self, name: &str, snapshot: &GuiState) -> RadioProfile {
        RadioProfile {
            name: name.to_string(),
            mode: Some(snapshot.mode.clone()),
            data_mode: snapshot.data_mode,
            filter: snapshot.filter,
            af_gain: snapshot.af_gain,
            rf_gain: snapshot.rf_gain,
            rf_power: snapshot.rf_power,
            preamp: None,
            attenuator: None,
            noise_blank: None,
            noise_reduction: None,
            agc: None,
        }
    }

    pub(crate) fn apply_radio_profile(&mut self, profile: RadioProfile) {
        let Some(tx) = &self.command_tx else {
            self.profile_io_status = "Radio tuning unavailable: radio is not connected".to_string();
            return;
        };
        if let Some(mode) = profile.mode.as_deref() {
            if let Some(workspace_mode) = WORKSPACE_MODES
                .iter()
                .copied()
                .find(|candidate| candidate.label().eq_ignore_ascii_case(mode))
            {
                let frequency_hz = self.state.lock().ok().and_then(|state| state.frequency_hz);
                if let Some(frequency_hz) = frequency_hz {
                    let _ = tx.send(GuiCommand::ApplyWorkspace {
                        mode: workspace_mode,
                        frequency_hz,
                    });
                }
            }
        }
        if let Some(filter) = profile.filter {
            let _ = tx.send(GuiCommand::SetFilter(filter));
        }
        for (control, value) in [
            (ControlId::AfGain, profile.af_gain.map(ControlValue::U8)),
            (ControlId::RfGain, profile.rf_gain.map(ControlValue::U8)),
            (ControlId::RfPower, profile.rf_power.map(ControlValue::U8)),
            (ControlId::Preamp, profile.preamp.map(ControlValue::Bool)),
            (
                ControlId::Attenuator,
                profile.attenuator.map(ControlValue::Bool),
            ),
            (
                ControlId::NoiseBlanker,
                profile.noise_blank.map(ControlValue::Bool),
            ),
            (
                ControlId::NoiseReduction,
                profile.noise_reduction.map(ControlValue::Bool),
            ),
            (ControlId::Agc, profile.agc.map(ControlValue::U8)),
        ] {
            if let Some(value) = value {
                let _ = tx.send(GuiCommand::SetControl(control, value));
            }
        }
        self.profile_io_status = format!("Applied radio profile {}", profile.name);
    }
}
