use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

static PLANNER: LazyLock<Mutex<RealFftPlanner<f64>>> =
    LazyLock::new(|| Mutex::new(RealFftPlanner::new()));

/// Real transform of one length, working on the start of larger buffers. The
/// planner keeps what it has worked out, which transforms of the same length
/// then share.
#[derive(Clone)]
pub(crate) struct RealFft {
    len: usize,
    forward: Arc<dyn RealToComplex<f64>>,
    inverse: Arc<dyn ComplexToReal<f64>>,
}

impl RealFft {
    pub(crate) fn new(len: usize) -> Self {
        let mut planner = PLANNER.lock().unwrap_or_else(PoisonError::into_inner);
        Self {
            len,
            forward: planner.plan_fft_forward(len),
            inverse: planner.plan_fft_inverse(len),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Least length of the `scratch` that both directions take
    pub(crate) fn scratch_len(&self) -> usize {
        self.forward
            .get_scratch_len()
            .max(self.inverse.get_scratch_len())
    }

    /// Leaves `time` overwritten.
    pub(crate) fn forward(
        &self,
        time: &mut [f64],
        spectrum: &mut [Complex<f64>],
        scratch: &mut [Complex<f64>],
    ) {
        self.forward
            .process_with_scratch(
                &mut time[..self.len],
                &mut spectrum[..self.len / 2 + 1],
                scratch,
            )
            .expect("the buffers are cut to the length of the transform");
    }

    /// Leaves `spectrum` overwritten and `time` multiplied by the length of the
    /// transform, which [`Self::inverse_scale`] undoes.
    pub(crate) fn inverse(
        &self,
        spectrum: &mut [Complex<f64>],
        time: &mut [f64],
        scratch: &mut [Complex<f64>],
    ) {
        let last = self.len / 2;
        // A NaN in the signal gets here too, and would otherwise fail the
        // transform's check that these parts are zero
        spectrum[0].im = 0.0;
        spectrum[last].im = 0.0;
        self.inverse
            .process_with_scratch(&mut spectrum[..=last], &mut time[..self.len], scratch)
            .expect("the buffers are cut to the length of the transform");
    }

    pub(crate) fn inverse_scale(&self) -> f64 {
        1.0 / self.len as f64
    }
}
