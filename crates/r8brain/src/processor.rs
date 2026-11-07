/// One step of a [`crate::Resampler`]'s chain
pub(crate) trait Processor: Send {
    /// How many input samples it takes from a cleared state to reach the output
    /// sample `position`.
    fn input_before_output(&self, position: i64) -> i64;

    /// Latency left in the output, in samples. Only minimum-phase filters leave
    /// any, and the whole part of it goes on the next step.
    fn latency_frac(&self) -> f64;

    /// The most output that a call with up to `input_len` samples brings.
    fn max_output_len(&self, input_len: usize) -> usize;

    fn clear(&mut self);

    /// Writes to `output`, which comes empty, and returns where in it the
    /// samples start that are past the latency it removes.
    fn process(&mut self, input: &[f64], output: &mut Vec<f64>) -> usize;
}
