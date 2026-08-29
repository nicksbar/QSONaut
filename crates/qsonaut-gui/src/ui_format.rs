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

#[cfg(test)]
mod tests {
    use super::{format_signal_report, ft8_period_progress, qso_stage_label, utc_hhmmss_millis};
    use crate::modes::exchange::QsoStage;

    #[test]
    fn formats_signal_reports_with_clamping_and_signs() {
        assert_eq!(format_signal_report(-100), "-50");
        assert_eq!(format_signal_report(-7), "-07");
        assert_eq!(format_signal_report(0), "+00");
        assert_eq!(format_signal_report(9), "+09");
        assert_eq!(format_signal_report(100), "+49");
    }

    #[test]
    fn labels_every_qso_stage() {
        assert_eq!(qso_stage_label(QsoStage::Calling), "Calling");
        assert_eq!(qso_stage_label(QsoStage::GridSent), "Grid sent");
        assert_eq!(qso_stage_label(QsoStage::ReportSent), "Report sent");
        assert_eq!(
            qso_stage_label(QsoStage::RogerReportSent),
            "Roger/report sent"
        );
        assert_eq!(qso_stage_label(QsoStage::FinalSent), "Final sent");
        assert_eq!(qso_stage_label(QsoStage::Complete), "Complete");
    }

    #[test]
    fn formats_utc_time_and_rolls_milliseconds_into_the_next_day() {
        assert_eq!(utc_hhmmss_millis(-1.0), "00:00:00.000");
        assert_eq!(utc_hhmmss_millis(3_723.456), "01:02:03.456");
        assert_eq!(utc_hhmmss_millis(86_399.9996), "00:00:00.000");
        assert_eq!(utc_hhmmss_millis(86_400.25), "00:00:00.250");
    }

    #[test]
    fn reports_current_ft8_period_as_a_unit_interval() {
        let progress = ft8_period_progress();
        assert!((0.0..1.0).contains(&progress));
    }
}
