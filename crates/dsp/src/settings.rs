use std::ops::RangeInclusive;

/// bs2b's crossfeed: the frequency below which each channel is fed to the
/// other, and how much lower that bass is than the channel's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Crossfeed {
    cutoff_hz: u16,
    /// In tenths of a dB
    separation: u16,
}

const SEPARATION_TENTHS: RangeInclusive<u16> = 10..=150;

impl Crossfeed {
    /// bs2b's range
    pub const CUTOFFS_HZ: RangeInclusive<u16> = 300..=2000;
    /// bs2b's range
    pub const SEPARATIONS_DB: RangeInclusive<f32> = 1.0..=15.0;

    /// `None` outside the ranges. The separation is rounded to a tenth of a dB.
    pub fn new(cutoff_hz: u16, separation_db: f32) -> Option<Self> {
        (Self::CUTOFFS_HZ.contains(&cutoff_hz) && Self::SEPARATIONS_DB.contains(&separation_db))
            .then(|| Self {
                cutoff_hz,
                separation: (separation_db * 10.0).round() as u16,
            })
    }

    pub fn cutoff_hz(self) -> u16 {
        self.cutoff_hz
    }

    /// The higher it is, the weaker the crossfeed.
    pub fn separation_db(self) -> f32 {
        f32::from(self.separation) / 10.0
    }
}

/// The level of the left channel against the right, in tenths of a dB. A
/// negative value lowers the right channel and a positive one the left; the
/// other one stays as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Balance(i16);

impl Balance {
    pub const CENTER: Self = Self(0);
    pub const MAX_DB: f32 = 12.0;

    /// Rounded to a tenth of a dB and kept within [`Self::MAX_DB`] either way.
    /// NaN is the center.
    pub fn from_db(db: f32) -> Self {
        if db.is_nan() {
            return Self::CENTER;
        }
        Self((db.clamp(-Self::MAX_DB, Self::MAX_DB) * 10.0).round() as i16)
    }

    pub fn db(self) -> f32 {
        f32::from(self.0) / 10.0
    }

    pub fn is_centered(self) -> bool {
        self.0 == 0
    }

    /// Of the left and the right channel, never above 1.
    pub(crate) fn gains(self) -> [f64; 2] {
        let lowered = 10f64.powf(-f64::from(self.0.unsigned_abs()) / 200.0);
        match self.0.signum() {
            1 => [lowered, 1.0],
            -1 => [1.0, lowered],
            _ => [1.0, 1.0],
        }
    }

    fn from_tenths(tenths: i16) -> Self {
        let limit = (Self::MAX_DB * 10.0) as i16;
        Self(tenths.clamp(-limit, limit))
    }
}

/// What a [`Processor`](crate::Processor) does. The default does nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Settings {
    crossfeed: Option<Crossfeed>,
    balance: Balance,
}

impl Settings {
    pub fn new(crossfeed: Option<Crossfeed>, balance: Balance) -> Self {
        Self { crossfeed, balance }
    }

    pub fn crossfeed(&self) -> Option<Crossfeed> {
        self.crossfeed
    }

    pub fn balance(&self) -> Balance {
        self.balance
    }

    pub fn is_neutral(&self) -> bool {
        self.crossfeed.is_none() && self.balance.is_centered()
    }

    /// For an atomic that whatever runs the processor loads. The default
    /// comes to `0`.
    pub fn to_bits(self) -> u64 {
        let crossfeed = self.crossfeed.map_or(0, |crossfeed| {
            (1 << 48)
                | (u64::from(crossfeed.cutoff_hz) << 32)
                | (u64::from(crossfeed.separation) << 16)
        });
        crossfeed | u64::from(self.balance.0 as u16)
    }

    /// Bits that [`Self::to_bits`] cannot have made come to the nearest
    /// settings it can.
    pub fn from_bits(bits: u64) -> Self {
        let crossfeed = ((bits >> 48) & 1 == 1).then(|| Crossfeed {
            cutoff_hz: ((bits >> 32) as u16)
                .clamp(*Crossfeed::CUTOFFS_HZ.start(), *Crossfeed::CUTOFFS_HZ.end()),
            separation: ((bits >> 16) as u16)
                .clamp(*SEPARATION_TENTHS.start(), *SEPARATION_TENTHS.end()),
        });
        Self {
            crossfeed,
            balance: Balance::from_tenths(bits as u16 as i16),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crossfeed_keeps_to_bs2bs_ranges() {
        assert!(Crossfeed::new(299, 4.5).is_none());
        assert!(Crossfeed::new(2001, 4.5).is_none());
        assert!(Crossfeed::new(700, 0.9).is_none());
        assert!(Crossfeed::new(700, 15.1).is_none());
        assert!(Crossfeed::new(700, f32::NAN).is_none());
        let crossfeed = Crossfeed::new(650, 9.54).unwrap();
        assert_eq!(crossfeed.cutoff_hz(), 650);
        assert_eq!(crossfeed.separation_db(), 9.5);
    }

    #[test]
    fn a_balance_rounds_to_a_tenth_and_stops_at_12_db() {
        assert_eq!(Balance::from_db(-1.54).db(), -1.5);
        assert_eq!(Balance::from_db(30.0).db(), 12.0);
        assert_eq!(Balance::from_db(-30.0).db(), -12.0);
        assert_eq!(Balance::from_db(f32::NAN), Balance::CENTER);
        assert!(Balance::from_db(0.04).is_centered());
    }

    #[test]
    fn a_balance_lowers_one_side_and_leaves_the_other() {
        let [left, right] = Balance::from_db(-6.0).gains();
        assert_eq!(left, 1.0);
        assert!((right - 10f64.powf(-6.0 / 20.0)).abs() < 1e-12);
        let [left, right] = Balance::from_db(12.0).gains();
        assert!((left - 10f64.powf(-12.0 / 20.0)).abs() < 1e-12);
        assert_eq!(right, 1.0);
        assert_eq!(Balance::CENTER.gains(), [1.0, 1.0]);
    }

    #[test]
    fn settings_go_through_bits_unchanged() {
        let all = [
            Settings::default(),
            Settings::new(Crossfeed::new(700, 4.5), Balance::CENTER),
            Settings::new(None, Balance::from_db(-12.0)),
            Settings::new(Crossfeed::new(2000, 15.0), Balance::from_db(12.0)),
            Settings::new(Crossfeed::new(300, 1.0), Balance::from_db(-0.1)),
        ];
        for settings in all {
            assert_eq!(Settings::from_bits(settings.to_bits()), settings);
        }
        assert_eq!(Settings::default().to_bits(), 0);
    }

    #[test]
    fn bits_out_of_range_come_to_the_nearest_settings() {
        let settings = Settings::from_bits((1 << 48) | (5000 << 32) | (2 << 16) | 0x7fff);
        let crossfeed = settings.crossfeed().unwrap();
        assert_eq!(crossfeed.cutoff_hz(), 2000);
        assert_eq!(crossfeed.separation_db(), 1.0);
        assert_eq!(settings.balance().db(), 12.0);
    }
}
