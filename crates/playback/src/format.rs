use audio::PlaybackFormat;

/// `44.1 kHz`, `96 kHz`.
pub fn format_sample_rate(rate: u32) -> String {
    let khz = f64::from(rate) / 1000.0;
    if rate.is_multiple_of(1000) {
        format!("{khz:.0} kHz")
    } else {
        format!("{khz:.1} kHz")
    }
}

/// How the current file reaches the device, e.g. `FLAC · 96 kHz` or, when
/// the device runs at another rate, `FLAC · 96 → 48 kHz`, or
/// `FLAC · 96 kHz → DSD256`.
pub fn describe_format(format: &PlaybackFormat, suffix: Option<&str>) -> String {
    let rate = if let Some(dop_rate) = format.dop_rate() {
        format!("{} → {dop_rate}", format_sample_rate(format.source_rate()))
    } else if format.is_resampled() {
        let source = format_sample_rate(format.source_rate());
        let source = source.trim_end_matches(" kHz");
        format!("{source} → {}", format_sample_rate(format.output_rate()))
    } else {
        format_sample_rate(format.output_rate())
    };
    match suffix {
        Some(suffix) => format!("{} · {rate}", suffix.to_uppercase()),
        None => rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_common_rates() {
        assert_eq!(format_sample_rate(44_100), "44.1 kHz");
        assert_eq!(format_sample_rate(48_000), "48 kHz");
        assert_eq!(format_sample_rate(88_200), "88.2 kHz");
        assert_eq!(format_sample_rate(192_000), "192 kHz");
    }
}
