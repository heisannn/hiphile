/// `44.1 kHz`, `96 kHz`.
pub fn format_sample_rate(rate: u32) -> String {
    let khz = f64::from(rate) / 1000.0;
    if rate.is_multiple_of(1000) {
        format!("{khz:.0} kHz")
    } else {
        format!("{khz:.1} kHz")
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
