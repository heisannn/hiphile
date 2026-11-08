//! libsoxr's numeric constants, for [`crate::Recipe::from_bits`],
//! [`crate::QualitySpec::from_bits`] and [`crate::RuntimeSpec`]'s environment
//! variables. The rest of the API takes enums and fields instead.

pub const QQ: u32 = 0;
pub const LQ: u32 = 1;
pub const MQ: u32 = 2;
pub const BITS_16: u32 = 3;
pub const BITS_20: u32 = 4;
pub const HQ: u32 = BITS_20;
pub const BITS_24: u32 = 5;
pub const BITS_28: u32 = 6;
pub const VHQ: u32 = BITS_28;
pub const BITS_32: u32 = 7;

pub const LINEAR_PHASE: u32 = 0x00;
pub const INTERMEDIATE_PHASE: u32 = 0x10;
/// libsoxr has no name for this one
pub const MAXIMUM_PHASE: u32 = 0x20;
pub const MINIMUM_PHASE: u32 = 0x30;
pub const STEEP_FILTER: u32 = 0x40;

pub const ROLLOFF_SMALL: u32 = 0;
pub const ROLLOFF_MEDIUM: u32 = 1;
pub const ROLLOFF_NONE: u32 = 2;
pub const HI_PREC_CLOCK: u32 = 8;
pub const DOUBLE_PRECISION: u32 = 16;
pub const VR: u32 = 32;
/// libsoxr has no name for this one. Its presets of quality numbers 0 to 7
/// and 11 to 15 have it.
pub const KEEP_RATIO_ON_CLEAR: u32 = 0x8000_0000;

pub const COEF_INTERP_AUTO: u32 = 0;
pub const COEF_INTERP_LOW: u32 = 2;
pub const COEF_INTERP_HIGH: u32 = 3;
