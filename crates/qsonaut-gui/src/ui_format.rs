use std::time::{SystemTime, UNIX_EPOCH};

use super::modes::exchange::QsoStage;

pub(super) fn format_signal_report(report: i8) -> String {
    format!("{:+03}", report.clamp(-50, 49))
}

pub(super) fn qso_stage_label(stage: QsoStage) -> &'static str {
    match stage {
        QsoStage::Calling => "Calling",
        QsoStage::GridSent => "Grid sent",
        QsoStage::ReportSent => "Report sent",
        QsoStage::RogerReportSent => "Roger/report sent",
        QsoStage::FinalSent => "Final sent",
        QsoStage::Complete => "Complete",
    }
}

pub(super) fn utc_hhmmss_millis(epoch_s: f64) -> String {
    let day_s = epoch_s.max(0.0).rem_euclid(86_400.0);
    let h = (day_s / 3600.0).floor() as u64;
    let m = ((day_s % 3600.0) / 60.0).floor() as u64;
    let sec_f = day_s % 60.0;
    let s = sec_f.floor() as u64;
    let mut ms = ((sec_f - s as f64) * 1000.0).round() as u64;
    let mut sec = s;
    let mut min = m;
    let mut hour = h;

    if ms == 1000 {
        ms = 0;
        sec += 1;
        if sec == 60 {
            sec = 0;
            min += 1;
            if min == 60 {
                min = 0;
                hour = (hour + 1) % 24;
            }
        }
    }

    format!("{hour:02}:{min:02}:{sec:02}.{ms:03}")
}

pub(super) fn ft8_period_progress() -> f32 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    ((seconds % 15.0) / 15.0) as f32
}

pub(super) fn fmt_opt_u8(value: Option<u8>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "?".to_string())
}

pub(super) fn fmt_civ_level_percent(value: Option<u8>) -> String {
    value
        .map(|raw| {
            let percent = (raw as f32 * 100.0 / 255.0).round() as u8;
            format!("{percent}%")
        })
        .unwrap_or_else(|| "?".to_string())
}
