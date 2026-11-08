use std::hash::{BuildHasher, Hasher, RandomState};

/// The seed of every resampler's noise after a clear, unless it was given one
const CLEARED_SEED: u64 = 0;

/// Where a resampler's noise starts on creation and after each clear
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seeds {
    first: u64,
    cleared: u64,
}

impl Seeds {
    /// With no `seed`, the first noise is the resampler's own and the noise
    /// after a clear the same for every resampler
    pub(crate) fn new(seed: Option<u64>) -> Self {
        match seed {
            Some(seed) => Self {
                first: seed,
                cleared: seed,
            },
            None => Self {
                first: random_seed(),
                cleared: CLEARED_SEED,
            },
        }
    }
}

/// Triangular noise from ±1, in units of the output's least significant bit
#[derive(Clone, Debug)]
pub(crate) struct Tpdf {
    cleared: u64,
    state: u64,
}

impl Tpdf {
    /// Each channel has its own sequence, so that one channel comes out as it
    /// would alone
    pub(crate) fn new(seeds: Seeds, channel: usize) -> Self {
        let of_channel = |seed: u64| {
            let mut mixer = seed ^ (channel as u64).wrapping_mul(0xd1b5_4a32_d192_ed03);
            split_mix(&mut mixer)
        };
        Self {
            cleared: of_channel(seeds.cleared),
            state: of_channel(seeds.first),
        }
    }

    pub(crate) fn restart(&mut self) {
        self.state = self.cleared;
    }

    pub(crate) fn next_value(&mut self) -> f64 {
        uniform(split_mix(&mut self.state)) - uniform(split_mix(&mut self.state))
    }
}

/// A seed of its own for each resampler, as the standard library draws the keys
/// of its hash maps
fn random_seed() -> u64 {
    RandomState::new().build_hasher().finish()
}

fn split_mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// In `[0, 1)`
fn uniform(bits: u64) -> f64 {
    (bits >> 11) as f64 / (1u64 << 53) as f64
}
