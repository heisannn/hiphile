use audio::PlaybackFormat;
use ui::format_sample_rate;

/// How the current file reaches the device, e.g. `FLAC · 96 kHz` or, when
/// the device runs at another rate, `FLAC · 96 → 48 kHz`.
pub(crate) fn describe(format: &PlaybackFormat, suffix: Option<&str>) -> String {
    let rate = if format.is_resampled() {
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
