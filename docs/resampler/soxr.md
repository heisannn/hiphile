# soxr

`soxr` converts sample rates with the functions of the
[SoX Resampler library](https://sourceforge.net/projects/soxr/) (libsoxr) 0.1.3. It was written in
a clean room, from a description of what libsoxr does and from observations of its behaviour,
without reading libsoxr's code or headers. This document explains how it converts, what it chose
where libsoxr's behaviour leaves the choice open, how Hiphile runs it and how it measured. The other
two resamplers are [Rubato](rubato.md) and [r8brain](r8brain.md).

| | |
| --- | --- |
| Code | The `soxr` crate. No I/O and no GPUI. |
| Dependencies | `realfft` for every real DFT, `thiserror` for the error type |
| Unsafe code | Only in `simd.rs`, for the vector instructions (see [Vector instructions](#vector-instructions)) |
| In Hiphile | 32-bit floating point in and out, for PCM only, at VHQ by default (see [In Hiphile](#in-hiphile)) |
| Agreement with libsoxr | The same functions, presets to the bit, the same messages and lengths. Output values agree to the precision asked for, not bit for bit, as the sums run in another order. Where behaviour differs, it follows libsoxr unless libsoxr's is a bug or one of four irregular behaviours (see [Following libsoxr](#following-libsoxr)). |

## Where the code lives

| File | Responsibility |
| --- | --- |
| `quality.rs` | `QualitySpec`, `Recipe` and the presets; their checks |
| `runtime.rs` | `RuntimeSpec` and the `SOXR_*` environment variables |
| `bits.rs` | libsoxr's numeric constants, for decoding recipes and flags |
| `io.rs` | `Sample`, `IoSpec`, `Dither` and the buffer layouts `Input` and `Output` |
| `source.rs` | `Source`, which `Resampler::output` pulls its input from |
| `error.rs` | `Error`, with libsoxr's messages |
| `resampler.rs` | `Resampler` and its `Builder`: the counts, the end of the input, the delay |
| `lanes.rs` | The channels of a constant ratio: conversion from and to the sample types, dither, the queue of output; the threads |
| `variable.rs` | A variable ratio: its filter, its clock and slews, and its channels |
| `chain.rs` | Which stages a ratio takes and their filters, and one channel's run through them |
| `fft_stage.rs` | Overlap-save convolution with the steep filter, doubling or halving the rate |
| `phase.rs` | The steep filter at minimum, intermediate or maximum phase |
| `poly.rs` | Interpolation at any time by a short filter, and the cubic interpolation of QQ |
| `clock.rs` | The ratio as a fraction, and time as a fraction or in 64.64 fixed point |
| `design.rs` | Kaiser-windowed sinc filters |
| `dither.rs` | The TPDF noise |
| `real.rs` | `f32` and `f64` behind one trait, the shared FFT planners, a fixed-order dot product |
| `simd.rs` | The inner loops of the filters, in NEON or SSE2 where the target has them |

## Following libsoxr

The user decided that **where this crate behaves otherwise than libsoxr, it follows libsoxr,
unless libsoxr's behaviour is a bug**. That comes before doing better than libsoxr where its
behaviour is merely odd. So integer output rounds ties away from zero, as libsoxr did on arm64,
rather than to even.

What follows libsoxr where another choice was open:

| Item | Now |
| --- | --- |
| Ties of integer rounding | Away from zero on every platform, so −32768.5 LSB rounds to −32769 and clips |
| Dither after `clear` | Every resampler not given a seed starts the same noise; one given a seed starts it again |
| LSR0 to LSR2 after `clear` | `set_io_ratio` may set a new ratio once before the next processing. Without one, the ratio from before the clear stays, where libsoxr crashed. |
| A ratio of 1 with a gain other than 1 | The last 2 frames wait for the end of the input, and the delay counts them. Dither has no part in it. |
| Runtime spec out of range | No errors. DFT sizes out of range are the defaults; a `coef_size_kbytes` of 0 or 1 interpolates; `SOXR_COEF_INTERP` 1 is auto |
| Variable ratios | Any ratio over 0, at creation and in `set_io_ratio` |
| LSR0 to LSR2 with `steep` | libsoxr's pass-band ends, which the filter follows |
| `CoefInterpolation::Low` and `High` | Interpolate the coefficients at any ratio, a short fraction too, as a table too small for the phases does, within 1.25 times the larger of libsoxr's error and that of `Auto` |

What does not follow libsoxr:

- **libsoxr's bugs**, which this crate fixes or which its API cannot reach:
  - `soxr_set_error` cannot set an error while none is recorded. Errors here are returned, so it
    has no counterpart.
  - After `clear`, a pull source is asked for 0 frames, so the input ends at once. Here the source
    keeps its largest request.
  - Input after the end crashes. Here it is `Error::InputAfterEnd`.
  - Processing a handle that has no ratio yet, and setting its channels, which reports "I/O ratio
    out-of-range". Here everything is set when a resampler is made.
  - A variable rate's `delay` is always 100. Here it is the real delay.
  - A variable rate runs about 2 samples late, a lack of precision. Here it is on time.
  - Without `HI_PREC_CLOCK`, a ratio with no short fraction drifts in time until the precision falls
    short of the quality asked for. Here the clock never drifts.
  - LSR0 to LSR2 crash when resampling after `clear` with no new ratio. Here the ratio from before
    the clear stays.
- **Four behaviours that change irregularly with the ratio or the quality**, so that following them
  would mean inferring libsoxr's internals from observations. The user chose to leave them:
  - **The rolloff**. libsoxr's rolloff takes effect at some ratios and not others, by amounts that
    change with the quality and the direction, with no rule found. This crate keeps the pass-band
    flat with any rolloff, which meets the bound that each rolloff sets.
  - **The timing of minimum and intermediate phase**. libsoxr's peak moves by −1.33 to +0.47 samples
    with the quality and the ratio, and phases 0 and 100 are not each other's reverse. This crate
    aligns at the filter's peak (see [Phase](#phase)).
  - **The shape of the variable rate's transition band**. This crate's prototype is its own (see
    [Filter](#filter)).
  - **The latency**, the input taken before the first output. Only the output's timing and length
    need to match libsoxr's. The latency follows from the transforms' lengths.

## The quality spec

### Presets

`QualitySpec::new(recipe)` returns libsoxr's values for each quality number to the bit, including
the steep ones. `Recipe::from_bits` and `QualitySpec::from_bits` decode libsoxr's numeric recipes
and flags, and `soxr::bits` holds the numbers, with `MAXIMUM_PHASE` for the phase bits 0x20 that
libsoxr leaves unnamed.

- **Bit 31**, which libsoxr returns for every quality number but 8 to 10, is
  `QualitySpec::keep_ratio_on_clear`, and `bits::KEEP_RATIO_ON_CLEAR` for `from_bits`. As in
  libsoxr, it decides what `clear` does to the ratio, not the preset (see [The API](#the-api)).
- **LSR2's rolloff bits of 3 are `Rolloff::None`**, and its bit 0x40 is ignored, as in libsoxr.
  Neither changes the output here, as the rolloff does not (see [Rolloff](#rolloff)).
- **The LSR presets take `steep`** with libsoxr's pass-band ends, 0.9825253148143992,
  0.98283977491453212 and 0.98331294043370743.

### Checks

`QualitySpec::validate` and `Builder::build` check in libsoxr's order and return the first error,
with libsoxr's message. The limits themselves pass. The tolerances past them follow what was
observed of libsoxr:

| Limit | Passes up to |
| --- | --- |
| The transition band's width, 0.002 to 0.5 | 1e-5 relative past either end |
| The pass-band's end, at least 0.5 | 1e-5 relative below |
| The stop-band's beginning, at most 1.5 | 1e-5 relative above |
| Imaging, when upsampling: `stopband_begin − 1 ≤ 1 − passband_end` | 5e-5 of `1 − passband_end` over |
| Precision, 15 to 33, and phase, 0 to 100 | None |

Without the tolerance on imaging, the observed case of `passband_end` 0.9 and `stopband_begin` 1.1
would fail, as `1.1 − 1` is a little more than `1 − 0.9` in floating point.

`Builder::build` checks the rates first, then the channels, then the quality spec and the scale. A
variable rate skips the checks of the quality spec, as in libsoxr. The runtime spec takes no
checks, as in libsoxr.

### Percentages

`passband_end` and `stopband_begin` over 2 are percentages of the API before libsoxr 0.1.1: the
pass-band's end is divided by 100, and the stop-band's beginning is `2 − value / 100`. Valid
fractions are at most 1.5 and valid percentages over 50, so the two never meet. The division is
rounded correctly, so 91.3 becomes the double just under 0.913, one ulp from the double nearest to
0.913. The output need not equal that of 0.913 to the bit: libsoxr's did only because its output
does not change with the last bit of the band.

### Rolloff

The rolloff is a bound on the fall in gain at the pass-band's end. The resampler keeps the whole
pass-band flat to the precision with any of the three, which meets every bound, so the choice does
not change the output. libsoxr's rolloff is not followed, as it takes effect irregularly.

## The runtime spec

`RuntimeSpec` has libsoxr's defaults, and no value of it is an error, as in libsoxr:

- **DFT sizes outside the ranges**, as base-2 logarithms 8 to 15 for the least and 8 to 20 for the
  large one, are taken as the defaults, 10 and 17, so the output is the default's. `RuntimeSpec`
  keeps the values as given.
- **Sizes within the ranges are used as given**, a least size over the large one too. Then the
  large one wins, and the output is not the default's, where libsoxr's was.
- **A `coef_size_kbytes` of 0 or 1** holds no exact table, so the coefficients are interpolated, as
  in libsoxr. The output stays within the precision of the default's.
- **`CoefInterpolation::Low` and `High`** interpolate the coefficients at any ratio (see
  [Interpolation](#interpolation)).
- **`SOXR_COEF_INTERP` 1** is `CoefInterpolation::Auto`, as 0 is.

The environment variables are read only by `RuntimeSpec::with_env`, so that a program that
embeds the crate is not surprised by them. The SIMD variables have no effect: the vector
instructions are chosen when the crate is compiled (see [Vector instructions](#vector-instructions)).

## How it converts at a constant ratio

The design follows the idea that libsoxr's README gives, of FFT oversampling with Smith's
band-limited interpolation, but its details are this crate's own. The steep filter always runs at
twice the lower of the two rates, where the signal fills only half the band. Interpolation between
that rate and the other then needs only a short filter, as images and aliases lie far from the
band.

```text
Upsampling, 44.1 → 48 kHz
44.1 kHz ──▶ steep filter, ×2 by FFT ──▶ 88.2 kHz ──▶ interpolation, 147 in : 80 out ──▶ 48 kHz

Downsampling, 192 → 44.1 kHz
192 kHz ──▶ ÷2, short filter ──▶ 96 kHz ──▶ interpolation, 160 in : 147 out ──▶ 88.2 kHz
        ──▶ steep filter, ÷2 by FFT ──▶ 44.1 kHz
```

| Ratio | Stages |
| --- | --- |
| 1 | None. The input passes in 64-bit floating point with the gain, whatever the quality. With a gain other than 1 overall, the last 2 frames wait for the end of the input, as in libsoxr. |
| QQ (precision 0) | Cubic Lagrange interpolation alone |
| Upsampling | The steep filter doubling the rate, then interpolation, unless the output is exactly twice the input |
| Downsampling | Halving by short filters while the ratio is over 4, then interpolation to twice the output rate unless the rate is already that, then the steep filter halving it |

### Steep filter

A Kaiser-windowed sinc with odd length and linear phase. Its cutoff is the middle of the
transition band, `(passband_end + stopband_begin) / 2` of the lower Nyquist frequency, and its
window reaches `(precision + 1) × 6.02` dB down over the transition band. The length is Kaiser's
fit, made longer by `max(0, A − 140) / 1000` of itself at an attenuation of `A` dB: without that,
Kaiser's fit falls up to 9 dB short at the precisions of VHQ and 32-bit. With it, every preset is
within 1 dB of its attenuation, and every −3 dB point within 0.0015 of what was observed of libsoxr
at 1 to 2.

`fft_stage.rs` runs it by overlap-save in one transform at the filter's rate, of the power
of two nearest to 4 times the filter in `f32` and 8 times in `f64`, within the runtime spec's DFT
sizes and at least twice the filter. In `f64`, 8 times costs 3 to 10 % less than 4 at VHQ and
32-bit, and takes more latency: from 44.1 to 48 kHz at VHQ, 1777 input frames before the first
output against 753. In `f32` it would round more, up to 0.6 dB at HQ, where the
transforms' rounding lies near the precision. Doubling
transforms only the input block and repeats its spectrum, which is the spectrum of the input with a
zero after each sample. Halving adds the two halves of the filtered
spectrum and inverts at half the length, which gives every other sample. The filter's spectrum is
computed once in 64-bit floating point, scaled by `1/N` and shared by the channels. The DC and
Nyquist bins of every inverse transform have their imaginary parts set to 0, the transforms take
preallocated scratch, and two buffers take turns, so that the history of each block goes into the
next one's before the transform overwrites it.

### Phase

Only the steep filter takes the phase response. The interpolating filters are short and keep
linear phase. At 50 the steep filter is as above; otherwise `phase.rs` makes it from that filter,
with the same magnitude:

1. Centred on 0, the filter has a real spectrum `A(ω)`, which changes sign at each zero of its
   stop-band. Those zeros lie on the unit circle.
2. The zeros come from the changes of sign on a grid of 8 times the filter's length, refined by
   Newton's method, in double-double arithmetic once `f64` rounding takes over.
3. `log |Q| = log |A| − Σ log |2 (cos ωᵢ − cos ω)|` takes them out. Their logarithm diverges, so
   with them the cepstrum aliases at any length of transform; `Q` has no zeros on the circle.
4. The real cepstrum of `log |Q|`, made causal, gives the phase `φ` of the minimum-phase `Q`. The
   fold tapers to 0 over the second quarter of the transform, where what is left is rounding.
5. The filter is the inverse transform of `A · e^(jαφ)`, with `α = 1 − p / 50` for `p ≤ 50`. At
   `α = 1` that is the minimum-phase filter, at `α = 0` the linear-phase one, and between them the
   phase blends. At every `α` the magnitude at each bin is `|A|`.
6. The ends are cut while what is cut changes the spectrum by less than 12 dB under the stop-band,
   checked on a grid 4 times finer, and the filter is scaled to a gain of 1 at 0 Hz.
7. Over 50, the filter is that of `100 − p` reversed in time. The phase bits are 25, 100 and 0.

| Decision | Choice |
| --- | --- |
| Alignment | At the filter's peak. An impulse peaks within half an output sample of its time; libsoxr's minimum phase peaked one sample early. |
| Between minimum and linear phase | A share `α` of the minimum phase, as the shape between them is free. At HQ from 1 to 2, the response over 1e-7 reaches −190/+267 samples from the peak at 25 against libsoxr's −194/+272, and −7/+347 at 0 against −8/+348. |
| Magnitude | The pass-band, the −3 dB point and the stop-band meet at every phase the bounds that linear phase meets. At HQ the pass-band deviates 4e-6 dB at minimum phase, where libsoxr's deviated 8.5e-4 dB. |
| Deep stop-bands | A transform rounds each bin by about 2e-16 of the pass-band, a large part of a 33-bit stop-band, and that rounding makes the phase rough enough to spread the filter. The spectrum is the mean over transforms of the filter at several shifts, which round independently: 1 up to 29 bits, 5 at 31 bits, 74 at 33. The zeros are refined in double-double, as Clenshaw's sum in `f64` rounds to about 1e-14. |

The phase costs time once, when a resampler is made: about 0.1 ms at LQ, 2 ms at HQ and VHQ and
3 ms at 32-bit, and with the steep filter 10, 33 and 45 ms. The latency hardly changes, as the
transform's block sets it, but the delay once output starts does, as with libsoxr:

| From 44.1 to 48 kHz | Input frames before the first output | Delay then, in output frames | libsoxr |
| --- | --- | --- | --- |
| HQ, linear phase | 832 | 109.6 | 922 (114.5) |
| HQ, minimum phase | 837 | 9.0 | 824 (7.9) |
| HQ, phase 25 | 756 | 135.9 | 1866 (152.0) |
| HQ, maximum phase | 837 | 205.0 | — |

### Interpolation

`poly.rs` computes each output sample from the input around its time, through a Kaiser-windowed
sinc that keeps the whole band the steep filter lets through, `[0, stopband_begin]` of the lower
Nyquist frequency, and removes whatever would fold into it. It reaches 10 dB further down than the
steep filter, so that it adds little to its error, and it is normalized to a gain of 1 at 0 Hz.
At HQ it has 10 to 18 taps, at VHQ 14 to 28 and at 32-bit 18 to 34.

| Ratio of the stage | Taps |
| --- | --- |
| A fraction whose table of rows fits in `coef_size_kbytes`, with `CoefInterpolation::Auto` | A row for each numerator of the fraction, exact |
| Any other, and any with `Low` or `High` | Polynomials in the position between rows, for `2^n` rows a sample |

The polynomials are cubic through four rows by default. `CoefInterpolation::Low` makes them linear,
with more rows, up to a table of 8 MiB. Past that, which happens from VHQ on, they are cubic as with
`High`, whose table is far smaller for the same error. `CoefInterpolation::Auto` takes linear ones
when their table fits in `coef_size_kbytes`, which happens at low precisions. The number of rows
keeps the error of interpolation under `2^-(precision + 4)` of the peak of the filter, under the
rounding of the `f32` engine at HQ. The coefficients stay in `f64`, and each output's taps are
worked out in `f64` and rounded once, as exact rows are, before one dot product. So the
interpolation adds no rounding of its own. These choices change the output only within the
precision.

As in libsoxr, `Low` and `High` interpolate at a ratio with a short fraction too. The error is the
exact rows' own: the pass-band's ripple, and at HQ mostly the rounding of the `f32` engine, as the
`f64` engine's error is 8.9e-8 there. That rounding moves with the frequency and the outputs taken,
by up to 15 % in libsoxr too, so each choice must err at most 1.25 times the larger of libsoxr's
error for the same choice and the error of `Auto` in the same build. `tests/runtime.rs` measures as
libsoxr was measured: a sine of 0.05 radians per sample, about 350.94 Hz, 40 000 inputs from 44.1
to 48 kHz, and all but 2000 outputs at each end. It takes 100 Hz, 1 kHz and 5 kHz too, but not
15 kHz, where the edge of the pass-band decides the error. The closest to the bound is 0.81 of it
with NEON and 0.80 with SSE2, at LQ and MQ at 5 kHz, where `Auto` errs more than libsoxr.

| Max error at 350.94 Hz, amplitude 0.5 | Auto | Low | High | `coef_size_kbytes` 1 | libsoxr: Low / High / 1 |
| --- | --- | --- | --- | --- | --- |
| LQ, NEON | 2.04e-6 | 2.02e-6 | 2.04e-6 | 2.04e-6 | 7.05e-6 / 6.87e-6 / 6.87e-6 |
| LQ, SSE2 | 2.04e-6 | 2.08e-6 | 2.04e-6 | 2.04e-6 | |
| MQ, NEON | 1.86e-6 | 1.83e-6 | 1.83e-6 | 1.83e-6 | 2.98e-6 / 3.01e-6 / 3.01e-6 |
| MQ, SSE2 | 1.84e-6 | 1.84e-6 | 1.83e-6 | 1.83e-6 | |
| HQ, NEON | 3.42e-7 | 3.42e-7 | 3.42e-7 | 3.42e-7 | 2.92e-7 / 3.04e-7 / 3.04e-7 |
| HQ, SSE2 | 2.77e-7 | 2.77e-7 | 2.77e-7 | 2.77e-7 | |
| VHQ | 1.67e-10 | 1.67e-10 | 1.67e-10 | 1.67e-10 | 4.83e-8 for each |

QQ has no filter: each output sample is the four-point Lagrange cubic through the input samples
around its time, at the clock's exact time. Its impulse response is the cubic's basis, and halving
takes every other sample.

### Clock

Output sample `k` lies at input time `k × ratio` of each stage, which the clock keeps without drift:

- **A fraction**, in lowest terms, for whole rates and for other rates whose ratio divides from a
  fraction with a denominator up to 2^20, such as 44100 to 47999.5. Time is a whole sample and a
  numerator.
- **64.64 fixed point** for any other ratio, such as 44100 to 50000.123. Time adds the double of the
  ratio exactly, so it is `k` times that double to the bit.

So `high_precision_clock` changes nothing. A longer search of fractions would find one for almost
any double, as near a denominator of 2^27 a convergent's error falls under the double's spacing.

### Timing and length

Each stage removes its own delay: the steep filter drops the samples before its peak, and the
interpolation is centred on the time it computes. So output sample `k` is the input's signal at
`k × io_ratio`, and an impulse peaks at the nearest output sample.

`Resampler` counts the frames taken and given, in `u64`:

- **Length**: `floor(input frames / io_ratio + 0.5)` in 64-bit floating point, as in libsoxr, which
  can be one off the exact count at a boundary.
  Before the end of the input it gives no output past that count for the input so far, which only
  binds QQ at large ratios, whose cubic needs little input ahead.
- **Delay**: before the end, `input frames / io_ratio − output frames`, and after it the output
  still to come. Both are exactly libsoxr's formulas.
- **The end**: zeros follow the input until the output is complete. Once a stage holds nothing but
  zeros, zeros go straight to the next one: its output would be +0.0 to the bit, as every sum starts
  at +0.0 and a zero of either sign added to it leaves it alone. At 10⁶ to 1 this takes the flush
  from 4 s to 20 ms.
- **The last interpolation is lazy.** It keeps its input and computes output only as far as the
  room given, as one block of the steep filter can make 10⁹ samples at 1 to 10⁶.

### Splitting into calls

The output does not depend on how the input and output are split into calls, as in libsoxr:
blocks of the steep filter fall where the stream puts them, the interpolation's sums run in a fixed
order whichever loop computes them (see [Vector instructions](#vector-instructions)), and the end
feeds the same zeros in the same order however the output is drained.
`tests/streaming.rs` checks this to the bit for 11 qualities and 7 ratios, as libsoxr was checked,
with 200 000 samples in calls of 1 to 4097 frames and room for 1 to 3000.

## How it converts at a variable ratio

With `variable_rate`, the engine is band-limited interpolation at the time of each output frame,
as Smith describes it [Smith, *Digital Audio Resampling*], through one prototype filter for every
ratio. Nothing of a constant ratio's stages is used, and the quality spec has no other effect and
takes no checks, as in libsoxr. The engine computes in 64-bit floating point, and reports
`Engine::Variable`, "vr32".

### Filter

| | |
| --- | --- |
| Prototype | Kaiser-windowed sinc at the lower rate: pass-band to 0.9 of its Nyquist frequency, stop-band from 1.0, at precision 20 (126 dB). 165 taps. |
| Downsampling | The prototype is stretched by the ratio in time and shrunk by it in gain, so that its band follows the output's Nyquist frequency. Each output frame then takes about 165 × ratio taps. |
| Taps | Worked out at each frame's own time from a table of 128 cubics per sample (Lagrange through four points, as `poly.rs` fits them). Their error is 1.3e-9 within the table and 4e-8 near its ends, where the window's slope is unbounded: both more than 12 dB under the stop-band. When upsampling, every tap lies as far along its segment, and a second copy of the table ordered by that place lets the taps be read in order. |
| Channels | Each frame's taps are worked out once and applied to every channel. |

Measured from sines in `tests/variable.rs`, against what was observed of libsoxr:

| | This crate | libsoxr |
| --- | --- | --- |
| Pass-band to 0.9 of the lower Nyquist frequency | Within 0.00001 dB at 0.5, 1, 1.0884, 2 and 3.9 | ±0.002 dB, and to 0.98 at 1.0884 and 3.9 |
| At 0.95 and 0.98 | −6.0 dB and −38.3 dB at every ratio | −10.2 dB and −43.3 dB at 0.5 and 2 |
| Above the lower Nyquist frequency | −128 dB at 1.02 of it, −141 dB or less at 1.1 and −146 dB or less at 1.5 | −116 dB to −136 dB |
| Time | Within 1e-6 of the ideal sine at 1/64 to 64: no delay | 1.4 to 2.9 output frames late |

### Ratio and slews

- **The clock** keeps the time of the next output frame in input frames, in 64.64 fixed point, and
  adds each step rounded to 2^-64.
- **Any ratio over 0 is taken**, as in libsoxr, at creation and in `set_io_ratio`, whatever the
  creation's, and the output is as long as a constant ratio's count. The filter widens with
  the ratio up to 64 and no more, so that each frame takes at most about 10 600 taps; over 64 the
  output aliases, as quality there is not required. The buffers keep the input that the widest
  filter reaches, 5283 frames a channel, as a ratio set later can widen the filter onto input that
  is otherwise gone. Over 64 the input between two output frames can be more than that, and is
  dropped.
- **A new ratio applies from the next output frame**. libsoxr's observed slew began some 3 frames
  after the call.
- **A slew of L frames** makes step j of them, from 0, `from + (to − from)(j + 0.5)/L`: the ratio
  halfway through the step, so the slew moves the time by `L(from + to)/2`, 1500 frames for 1 to 2
  over 1000. libsoxr's moved it 1507.4, with its start some 7 frames off.
- **A ratio set during a slew** starts from the step that the slew has reached, so the ratio stays
  continuous. With a slew of 0 it jumps, and the filter's band with it.
- **Clearing** goes back to the ratio of creation, as in libsoxr.
- `io_ratio` is the step from the next output frame to the one after, which within a slew lies
  between the two ratios.

### Length and delay

- **The end** is counted from the output frame and the time from which the ratio last stayed
  constant, as a constant ratio counts it from the start:
  `floor((input frames − time) / ratio + 0.5)` more frames, in 64-bit floating point. A ratio that
  never changes thus ends exactly as a constant one does, as in libsoxr. Within a slew, the output
  ends at the last frame whose time lies within the input by at least half of its step, which
  agrees with the count at a constant ratio. libsoxr agreed with this rule in 5 of 6 cases and gave
  1 frame fewer in the other, and as its timing is off by about 2 samples, the total need not match.
- **The delay** is the real one, not libsoxr's constant 100: before the end,
  how far the input reaches past the time of the next output frame, in steps of the current ratio,
  and after it the frames still to come.
- **Speed**, stereo, on one core of an Apple M5: 44.1 to 48 kHz in 90 ms for 10 s of input, 112×
  real time, against 10 ms for HQ at a constant ratio; 44.1 to 96 kHz in 178 ms, and 48 to
  44.1 kHz in 112 ms for 10 s. Downsampling costs about the same per input frame at any ratio.

## Threads

`num_threads` other than 1 shares out the work of a call over scoped threads
(`std::thread::scope`) that end with the call. A constant ratio gives each thread some of the
channels, and a variable one some of the output frames, for every channel, so that each frame's
taps are still worked out once. Each channel or frame takes the same steps on any thread, so the
output does not change a bit with the number of threads.

A call takes threads only when it has about 2^19 multiply-adds of work or more, some 0.15 ms,
counted as 32 for a sample of a constant ratio and as the taps times the channels and 4 at a
variable one. Below that, making the threads costs more than they save. With 8 channels from 44.1
to 48 kHz, 10 s of input:

| Threads | HQ, calls of up to 8192 frames | HQ, one call | VR, calls of up to 8192 frames |
| --- | --- | --- | --- |
| 1 | 42 ms | 42 ms | 217 ms |
| 2 | 32 ms | 23 ms | 121 ms |
| 4 | 25 ms | 13 ms | 71 ms |
| 8 | 28 ms | 11 ms | 67 ms |

Small calls gain less, as the threads are made for each call: a pool would keep them, but needs
either a dependency or more code than the gain is worth here.

## Making it fast

The user asked for at least 0.9 of libsoxr's speed at every rate pair, type and quality at which
libsoxr's speed was observed, `f32` first. [Against libsoxr](#against-libsoxr) has the result, and
[Speed](#speed) the measurements against the other resamplers. The time goes to the
interpolation's dot products and the steep filter's transforms, and these keep them short:

| Part | How |
| --- | --- |
| Interpolation at a fraction | The rows in the order that the outputs take them, from the clock at 0, with how far each output's window lies from the first's. A batch of 64 outputs reads its rows and windows from those tables, instead of stepping the clock through each output, whose carries no branch predictor learns. |
| Whole cycles | Where the input holds 16 cycles of the fraction (32 and 64 for denominators of 2 and 1), the outputs at the same place of each cycle share a row and have windows a period apart. They go to one loop that loads each tap once for four outputs, which takes 5/8 of the loads of the dot products. Its sums are the batches', so where a run ends changes no bit. |
| Single taps | When a quarter of the rows or more have one tap that is not 0, their outputs are products. When upsampling by 2, the outputs at the input samples are those, as the sinc is exactly 0 at the other whole numbers. |
| Windows | Padded to whole vectors: 4 samples of `f32`, 2 of `f64` |
| Steep filter | In `f64` a transform 8 times the filter's length, two buffers in turn so that the history is copied once a block, and the products with the filter's spectrum in loops without a branch per bin |
| Stereo | Interleaved stereo is split into its two channels, and joined again, in one loop each |
| QQ | The weights of each numerator worked out once, when their table fits in `coef_size_kbytes` |

### Against libsoxr

Measured on 2026-11-08 the way libsoxr's speed was observed, every case reached at least 0.9 of
it, and the lowest 0.94.

- **How:** on an Apple M5, one thread, interleaved stereo, the same type in and out. Each run makes
  the resampler, gives it 4096 frames at a time, drains it at the end and drops it. The best of
  three runs is divided by libsoxr's figure for the same case.
- **Input:** 30 seconds of 3 songs from Hiphile's cache.
- **Warm-up:** 1 second before measuring, so that the CPU's clock has risen. Without it, the first
  case measured, QQ at 44.1 → 48 kHz in `f32`, came out at 0.99.

Speed as a multiple of libsoxr's:

| Rates (kHz) | Type | QQ | LQ | MQ | HQ | VHQ | 32-bit | HQ, steep | VHQ, no rolloff |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | `f32` | 1.68 | 1.67 | 1.34 | 1.71 | 1.19 | 1.12 | 1.57 | 1.27 |
| 44.1 → 48 | `f64` | 1.52 | 1.67 | 1.30 | 1.63 | 1.19 | 1.13 | 1.57 | 1.25 |
| 44.1 → 88.2 | `f32` | 2.29 | 1.74 | 1.55 | 1.66 | 1.28 | 1.21 | 1.69 | 1.28 |
| 44.1 → 88.2 | `f64` | 2.15 | 1.70 | 1.50 | 1.64 | 1.25 | 1.17 | 1.67 | 1.25 |
| 44.1 → 96 | `f32` | 1.59 | 2.49 | 1.34 | 1.79 | 1.23 | 1.20 | 1.66 | 1.39 |
| 44.1 → 96 | `f64` | 1.56 | 2.46 | 1.29 | 1.79 | 1.23 | 1.23 | 1.65 | 1.40 |
| 44.1 → 176.4 | `f32` | 1.83 | 1.52 | 1.51 | 1.57 | 1.14 | 1.02 | 1.69 | 1.14 |
| 44.1 → 176.4 | `f64` | 1.75 | 1.49 | 1.49 | 1.54 | 1.11 | 1.01 | 1.65 | 1.12 |
| 44.1 → 192 | `f32` | 1.55 | 1.82 | 1.86 | 1.93 | 1.32 | 1.24 | 1.91 | 1.37 |
| 44.1 → 192 | `f64` | 1.51 | 1.80 | 1.85 | 1.93 | 1.29 | 1.25 | 1.91 | 1.36 |
| 48 → 44.1 | `f32` | 1.56 | 1.39 | 1.34 | 1.46 | 1.07 | 1.07 | 1.46 | 1.09 |
| 48 → 44.1 | `f64` | 1.46 | 1.34 | 1.25 | 1.40 | 1.03 | 1.02 | 1.40 | 1.07 |
| 48 → 88.2 | `f32` | 1.56 | 2.02 | 1.23 | 1.85 | 1.19 | 1.19 | 1.52 | 1.28 |
| 48 → 88.2 | `f64` | 1.41 | 1.32 | 1.19 | 1.74 | 1.16 | 1.16 | 1.58 | 1.33 |
| 48 → 96 | `f32` | 2.14 | 1.69 | 1.44 | 1.57 | 1.20 | 1.13 | 1.56 | 1.21 |
| 48 → 96 | `f64` | 2.03 | 1.54 | 1.38 | 1.57 | 1.19 | 1.13 | 1.58 | 1.20 |
| 48 → 176.4 | `f32` | 1.64 | 2.90 | 1.23 | 1.87 | 1.23 | 1.26 | 1.70 | 1.44 |
| 48 → 176.4 | `f64` | 1.65 | 3.03 | 1.26 | 1.88 | 1.25 | 1.25 | 1.69 | 1.38 |
| 48 → 192 | `f32` | 1.69 | 1.42 | 1.43 | 1.50 | 1.10 | 1.01 | 1.69 | 1.13 |
| 48 → 192 | `f64` | 1.67 | 1.42 | 1.44 | 1.51 | 1.10 | 0.94 | 1.64 | 1.07 |
| 96 → 44.1 | `f32` | 1.46 | 1.40 | 1.38 | 1.52 | 1.01 | 0.99 | 1.63 | 1.08 |
| 96 → 44.1 | `f64` | 1.39 | 1.47 | 1.44 | 1.52 | 1.00 | 0.99 | 1.62 | 1.06 |
| 96 → 48 | `f32` | 2.74 | 1.59 | 1.39 | 1.49 | 1.10 | 1.01 | 1.52 | 1.09 |
| 96 → 48 | `f64` | 2.28 | 1.49 | 1.36 | 1.45 | 1.07 | 1.02 | 1.51 | 1.06 |

- The machine carried other load, at a load average of about 3.5, so a full run moves by a few
  percent even at the best of three. One other full run gave 0.86 for LQ in `f64` at 96 → 44.1 kHz,
  which measured 1.44 to 1.46 alone. In the same way, LQ in `f64` at 48 → 88.2 kHz measured 2.14 to
  2.16 alone, and 32-bit in `f64` at 48 → 192 kHz 0.98 to 0.99.
- The least margin is at fourfold upsampling with the `f64` engine and at downsampling by ratios
  that are not whole numbers, 0.94 to 1.02 at 32-bit.

### Vector instructions

`simd.rs` has the inner loops: a dot product, a batch of them over rows and windows given apart,
and a filter whose windows lie a stride apart. On AArch64 with NEON and on x86-64 with SSE2, which
those targets have from their start, they use vector instructions. On any other target the scalar
loops run. The choice is made when the crate compiles (`cfg(target_feature)`), so no CPU is asked at
run time.

- **`unsafe` stays in `simd.rs`**, which allows it while the crate denies it elsewhere. Each vector
  function has `#[target_feature]` and is called from a safe function under a `cfg` that
  guarantees the feature at compile time. Each load reads one vector from a slice, at an offset
  that the public functions check beforehand: the slices as long as each other, every window within
  its slice, and every length whole vectors. The loads take no alignment.
- **One order of sums.** Every loop of a build sums an output in four lanes, lane `i` taking the
  products of the indices `i` mod 4, and ends as `(lane 0 + lane 2) + (lane 1 + lane 3)`. So an
  output has the same bits whichever loop computes it and wherever a batch or a run ends, so
  within a build neither the split into calls nor the threads change the output.
  `src/simd/tests.rs` checks this.
- **Between builds the bits differ.** NEON adds each product by fused multiply-add, with one
  rounding, where SSE2 and the scalar loops round twice, which the user allowed. Either is within
  the precision.
- **No dependency**: `std::arch` has the instructions.

## Samples

### Types and gain

Full scale is 1 for `f32` and `f64`, 2^31 for `i32` and 2^15 for `i16`. On the way in each sample
is multiplied in 64-bit floating point by `scale` over its type's full scale, and on the way out by
the output type's full scale, so the engine works where full scale is 1 and the gain overall is
`scale × FS(output) / FS(input)`. Floating-point output is not held to ±1.

Up to a precision of 20 without `double_precision`, the engine computes in `f32`, as libsoxr's
does, so `i32`
output carries about 24 bits. From 44.1 to 48 kHz at HQ it differs from the `f64` engine's by up to
some 500 LSBs of 32 bits, 2^-22 of full scale.

### Rounding and clipping

| Case | Integer output |
| --- | --- |
| Rounding | To the nearest, ties away from zero (`f64::round`) on every platform, as libsoxr on arm64, rather than to even (see [Following libsoxr](#following-libsoxr)) |
| Out of range | Held to the type's end and counted, when the rounded value lies outside. So +32767.5 LSB rounds to 32768 and −32768.5 LSB to −32769, and both clip. |
| NaN | 0, not counted |
| The count | Of every channel, in `u64`, until `reset_clips` or `clear` |

### Dither

TPDF dither goes on `i16` output only, before rounding, with a ratio of 1 too. Its sequence is not
libsoxr's.

- **The noise** is the difference of two uniform values in [0, 1) from SplitMix64 (Steele, Lea and
  Flood 2014), triangular on ±1 LSB. The generator is a few lines, so it takes no dependency.
- **Each channel has its own sequence**, from the seed and the channel's index, so a channel comes
  out as it would alone, and the split into calls changes nothing.
- **The seed** is `Dither::Tpdf { seed: Some(…) }`, or by default one drawn for each resampler from
  `RandomState`, which differs between resamplers and between processes, as libsoxr's dither does.
- **After `clear`**, a resampler not given a seed starts from a seed that every such resampler
  shares, so all of them give the same dither after `clear`, in any process, as in libsoxr. A
  resampler given a seed starts from it again. The given seed is this crate's addition, as libsoxr's
  dither cannot be repeated.

## The API

The public API is Rust's rather than a copy of the C API.

| libsoxr | This crate |
| --- | --- |
| `soxr_create` | `Resampler::new`, or `Resampler::builder(…).build()` |
| `soxr_process(in, out)` | `Resampler::resample(input, output)` |
| `soxr_process(NULL, out)` | `Resampler::finish(output)` |
| `soxr_set_input_fn` and `soxr_output` | `Resampler::output(source, output)`, with a `Source` passed to each call |
| `soxr_oneshot` | `Builder::oneshot(input)`, which returns the whole output, at LQ unless a quality is given, as in libsoxr |
| `soxr_set_io_ratio` | `Resampler::set_io_ratio`, and `io_ratio` for the ratio of the next step |
| `soxr_delay`, `soxr_num_clips`, `soxr_engine`, `soxr_error` | `delay`, `clips` and `reset_clips`, `engine`, `error` |
| `soxr_clear` | `Resampler::clear` |
| `soxr_delete` | `Drop` |
| Data types and layouts | `Resampler<I, O>` over `f32`, `f64`, `i32` and `i16`, with `Input` and `Output` interleaved or split, chosen per call |
| `soxr_io_spec`, its `scale`, `SOXR_TPDF` and `SOXR_NO_DITHER` | `IoSpec`, with `with_scale`, `Dither::Tpdf { seed }` and `Dither::None` |

- **Input is taken only as far as the output has room**, `ceil(room × io_ratio)` frames, the rest
  being for a later call. This is libsoxr's behaviour when the caller asks how much input was taken,
  and keeps what a resampler holds bounded. There is no call that takes input and ends it together:
  `finish` ends the input.
- **Everything is set when the resampler is made.** There is no unconfigured resampler, so
  libsoxr's deferred configuration and its quirks do not arise, nor do `soxr_set_num_channels` and
  its errors. The channels are at least 1.
- **After `clear`, a quality without `keep_ratio_on_clear`**, which the LSR presets are, takes a
  new ratio from `set_io_ratio`, once and before the next call of `resample`, `output` or `finish`,
  as libsoxr configures an LSR handle again. The ratio is checked against the quality spec, and an
  error stops the resampler until `clear`, as in libsoxr. With no new ratio, the resampler keeps the
  one it had before the `clear`, where libsoxr crashed. Other qualities keep their ratio, and a
  variable one goes back to its ratio of creation.
- **What C needs and Rust does not** is left out: null handles and buffers cannot occur, and
  `soxr_set_error` has no counterpart, as errors are returned.
- **A source lends its input**, like `BufRead::fill_buf`: `Source::pull(frames)` returns
  an `Input` of its own, interleaved or split, with no more than `frames` frames, and none for the
  end of the input. The resampler copies nothing it does not need and holds no buffer of the
  requested length, which at a ratio of 10⁶ with no most would run to gigabytes.
- **The most frames is the source's**, `Source::most_frames`, so `clear` cannot lose it and the
  requests after it are those before it, where libsoxr's asked for 0 frames. Every pull of a call
  asks for the same `min(most, ceil(room × io_ratio))`.
- **A failing source** stops the resampler with `Error::InputFailure` until `clear`.
  An `Err`, more frames than asked for, and buffers that do not match the channels are failures
  alike. When the call has written output before the failure, it returns that output, and the error
  comes with the next call, as with `std::io::Read`. No room asks for nothing and returns 0, as in
  libsoxr.
- **Pushing and pulling mix**: the source is passed to each call rather than kept, so a resampler
  takes input from `resample` and `output` alike, as one stream.
- **Input after the end is an error**, `Error::InputAfterEnd`, returned without stopping the
  resampler, where libsoxr crashed.
- **A ratio of 1 with a gain of 1 holds nothing back**, so its delay is 0 at once. With another
  gain, from `scale` or the types, it holds the last 2 frames until the end of the input, and its
  delay is 2, as in libsoxr. Types of the same full scale at a `scale` of 1, such as `i16` to
  `i16`, hold nothing.
- **`set_io_ratio` on a constant ratio** accepts the same ratio within 5e-16 of it, relative to it,
  and otherwise returns `Error::VaryingRatio` without keeping it. libsoxr accepted differences up to
  1e-15 at a ratio of 2 and refused 1.5e-15.
- **Latency** is not libsoxr's, and is not made so (see [Following libsoxr](#following-libsoxr)).
  From 44.1 to 48 kHz, the first output takes 832 input frames at HQ and 1777 at VHQ, against
  libsoxr's 922 and 886.

## In Hiphile

Hiphile can resample PCM with `soxr`, beside [Rubato](rubato.md) and [r8brain](r8brain.md). Resampler
under Resampling on the Audio page chooses among them. A converter exists only for a song whose rate
differs from the device's: with a fixed `sample_rate`, when the device lacks the song's rate, or for
a multiple such as `x2`. DSD output oversamples with its own Rubato stages, so the soxr groups are
hidden while DSD is on.

| Crate | Responsibility |
| --- | --- |
| `audio` | `SoxrConverter` in `src/convert/soxr_converter.rs`, which runs one `Resampler<f32, f32>` over the interleaved channels. `Resampling::Soxr` and `SoxrSettings` in `src/output.rs`, and `OutputSettings::from_config` in `src/from_config.rs`. |
| `config` | The `soxr_*` keys, their ranges and the presets |
| `settings` | The quality, and the soxr filter and soxr performance groups of the Audio page, in `soxr_settings.rs` and `soxr_section.rs`. The dropdowns of numbers are in `audio_options.rs`, shared with the DSD groups. |

### The converter

`SoxrConverter` gives `RateConverter` what `R8brainConverter` does: `push` takes a decoded packet
and appends the output, `undelivered_frames` counts what the input still owes, and `finish` lets it
out at the end of a song.

- **The output's room** is `ceil(frames / io_ratio)` frames of the packet and 4096 more, for what
  an FFT block lets out at once. `resample` takes only the input that room completes, so
  `push` calls it again until the packet is taken and the output stops short of the room.
- **The length is soxr's**, `floor(input frames / io_ratio + 0.5)`, from the input's first frame
  (see [Timing and length](#timing-and-length)). `finish` calls `Resampler::finish` until it gives
  nothing more, so a song ends where
  its rate says.
- **A sample that is not a finite number is taken as 0**, as one would spread through a whole FFT
  block.
- **One thread.** The engine resamples a packet at a time, a few thousand frames, under the 2^19
  multiply-adds a call takes before `soxr` shares the work (see [Threads](#threads)).
- **An error** of `soxr`, such as a quality it refuses, is `Error::Resample` with soxr's message.

### Settings

Resampler under Resampling on the Audio page chooses Rubato, r8brain or soxr, and Quality under it
sets the quality of the one chosen. With soxr chosen and DSD off, the soxr filter and soxr
performance groups follow. As with r8brain, changing a value of the filter switches Quality to
Custom, which keeps the current values, and choosing Custom alone changes nothing. The performance
values do not change Quality.

| Preset | Precision | Pass-band end | Input before the first output, 44.1 to 48 kHz |
| --- | --- | --- | --- |
| LQ | 16 bits | 0.676 | 471 frames, 10.7 ms |
| MQ | 16 bits | 0.916 | 355 frames, 8.0 ms |
| HQ | 20 bits | 0.914 | 832 frames, 18.9 ms |
| VHQ (default) | 28 bits | 0.911 | 1777 frames, 40.3 ms |

The presets are libsoxr's, under its names, at linear phase, computed in `f32` up to 20 bits and in
`f64` above. A stereo song takes under 0.1 % of one core of an Apple M5 at
any of them, from 44.1 to 48 kHz. LQ's pass-band ends at 0.676 of the Nyquist frequency, but its
transition band runs to the Nyquist frequency: from 44.1 kHz, a sine is 3.5 dB lower at 18 kHz and
22 dB lower at 20 kHz, and from 48 kHz, 3.2 dB lower at 19.5 kHz.

| Decision | Choice |
| --- | --- |
| Default | VHQ. HQ, libsoxr's default, keeps 20 bits, short of the 24 that the decoder's 32-bit floating point holds. |
| Quick | Not offered. It filters nothing, so aliases and images stay about 6 dB down. |
| Steep | Not a choice of its own: it only moves the pass-band's end, which Custom sets. |
| Stop-band | Its beginning stays at 1, the Nyquist frequency, as in every preset, so that nothing aliases into the band. Pass-band end alone chooses how much treble to keep. |
| Rolloff | Not offered, as it changes nothing in `soxr` (see [Rolloff](#rolloff)). Every quality, custom ones too, takes `Rolloff::Small`, so a custom quality with a preset's values is that preset to the bit. |
| Phase | Linear, Intermediate or Minimum: libsoxr's named phases. Maximum, which libsoxr leaves unnamed, puts the ringing before each sound and is left out. |
| Precision | 15 to 33 bits, the limits of a quality spec (see [Checks](#checks)). The dropdown offers 16, 20, 24, 28 and 32. |
| Pass-band end | 0.5 to 0.998 of the lower Nyquist frequency, saved as that fraction: the lowest end a quality spec takes, and the narrowest transition band, 0.002, below a stop-band at 1. The dropdown offers 0.90, 0.95, 0.98 and 0.99. |
| Double precision | `QualitySpec::double_precision`, which matters only up to 20 bits. |
| Clock and variable rate | Left at libsoxr's defaults: Hiphile's rates are whole numbers at a constant ratio. |
| Coefficient table | `coef_size_kbytes`, 1 to 65536 kB, 400 by default as in libsoxr. The dropdown offers 100 to 3200 kB, doubling. |
| Coefficient interpolation | Auto, Linear or Cubic: `CoefInterpolation::Auto`, `Low` and `High`. |
| Smallest and largest DFT | Powers of two within the ranges of [The runtime spec](#the-runtime-spec), 256 to 32768 and 256 to 1048576, 1024 and 131072 by default. Saved as the size, not its logarithm. |
| Threads | Fixed at 1, as above. |

Precision and Pass-band end are dropdowns rather than number fields: the presets lie on few
values, and a value from the file is added to the list where it falls, as each preset's pass-band
end is. They have no reset of their own, as choosing a preset brings them back. Phase and Double
precision reset to VHQ's, and the performance values to their defaults.

### Keys

The keys are listed under [Configuration](../../README.md#configuration) in the README.
`resampling = "soxr"` chooses soxr, and `soxr_quality` holds its quality. The keys of the filter,
`soxr_precision`, `soxr_passband_end`, `soxr_phase` and `soxr_double_precision`, are read only with
`soxr_quality = "custom"`, any of them missing taking VHQ's, and choosing a preset removes
them. The performance keys, `soxr_coef_table_kb`, `soxr_coef_interpolation`, `soxr_min_dft_size`
and `soxr_large_dft_size`, are read with any quality. Saving writes only the values that changed,
and of those only what differs from the defaults, and keeps the soxr values while another
resampler is chosen, as with r8brain. `OutputSettings::from_config` turns them into a
`QualitySpec` and a `RuntimeSpec`.

### Tests in Hiphile

- `crates/config/src/profile.rs`: the keys round-trip, presets save by name, only changes are
  saved, the environment is set aside, and every value outside a range is an error naming its key.
- `crates/audio/src/convert.rs`: soxr keeps the time and the channels of the input at HQ and
  VHQ, every option keeps the length and the level, a value soxr refuses is an error, and a reset
  starts over at any split of the input.
- `crates/audio/src/from_config.rs`: the presets are libsoxr's, every choice reaches the
  resampler, and the ends of the ranges are what it takes.
- `crates/settings/src/soxr_settings/tests.rs`: the quality, the filter and the performance values,
  and how they save.
- `crates/audio/src/engine.rs`: `resampled_tracks_join_without_a_gap_on_soxr`,
  `…_on_the_soxr_presets` and `…_on_soxr_with_other_options` play on a device and are
  `#[ignore]`d.

## Measurements

What was established about `crates/soxr` against what was observed of libsoxr, and how it compares
with r8brain and Rubato, as of 2026-11-08, with the fourfold measurement of 2026-11-09 under
[Fourfold upsampling](#fourfold-upsampling). For the measurements of the other two resamplers on
their own, see [Rubato](rubato.md#measurements) and [r8brain](r8brain.md#measurements).

Each claim is tagged by how far it was checked.

| Tag | Meaning |
| --- | --- |
| [Verified] | Checked by running or measuring it |
| [Read] | Checked by reading code or documents, without running it |
| [Inferred] | Reasoned from observations, without checking directly |
| [Unknown] | The cause or behavior is not understood |

### Environment and scope

| Item | Value |
| --- | --- |
| Mac | Apple M5, 24 GB of memory, macOS 26.2 |
| Rust | 1.99.1, release builds |
| soxr | This crate |
| r8brain | Hiphile's port, `crates/r8brain`, at its four presets, through a copy of Hiphile's `R8brainConverter` |
| Rubato | 5.0.1, through a copy of Hiphile's `FftConverter`, with the Blackman-Harris² window and blocks sized by the lower rate |
| Songs | 31 songs from Hiphile's cache, decoded with ffmpeg |

libsoxr itself was neither read nor run, so everything said of it comes from observations of its
behaviour made before this crate was written. All three resamplers were measured here by the same programs, so the
figures for r8brain and Rubato are those programs' own. They agree with the measurements of
2026-11-07 in [r8brain](r8brain.md#measurements) and [Rubato](rubato.md#measurements) to a few dB
where the methods are the same, and differ more where the methods differ (see
[Testing notes](#testing-notes)).

Nobody has listened to compare the settings. Every claim about what can be heard is [Inferred].
Nothing was run on any platform other than macOS.

### Summary

- **The tests against libsoxr's observed behaviour pass** (see
  [Agreement with libsoxr](#agreement-with-libsoxr)). Lengths, the delay's formula, presets and
  messages agree with libsoxr's to the bit; output values agree to the precision asked for, not to
  the bit. The C compatibility layer is out of scope. [Verified]
- **Up to 20 kHz**, soxr's VHQ and 32-bit stay below the rounding of a 32-bit output at every pair
  measured. HQ, which computes in 32-bit floating point as libsoxr's does, lies at −143 to −150 dB:
  2 to 19 dB above r8brain's Low and Medium, within 3 dB of Rubato's Medium, and under Rubato's
  larger sizes. [Verified]
- **LQ is not a hi-fi setting.** libsoxr's preset ends its pass-band at 0.676 of the Nyquist
  frequency, 14.9 kHz from 44.1 kHz, which this crate keeps, and its transition band runs to the
  Nyquist frequency: from 44.1 kHz it is 3.5 dB down at 18 kHz and 22 dB at 20 kHz. Against an
  ideal resampler its error up to 20 kHz is −46 to −69 dB. [Verified]
- **Above 20 kHz**, soxr's MQ to 32-bit take away about as much as r8brain's Low, and more than
  r8brain's other presets and Rubato's larger sizes. [Verified]
- **Its filters ring briefly**: 2.2 ms each side at HQ down to −144 dB, about as r8brain's Low and
  Rubato's Medium, against 6.9 ms for r8brain's High and 9.5 ms for Rubato's High. [Verified]
- **Speed**, measured again after the speed work (see [Making it fast](#making-it-fast)): soxr's
  LQ to HQ ran at 930 to 3780 times real time in stereo, ahead of every setting of r8brain and
  Rubato at every pair. VHQ and 32-bit ran at 480 to 2040 times, behind r8brain from 192 kHz and
  behind Rubato's fastest setting at most ratios other than 2. The variable rate is 18 to 70 times
  slower than HQ, at 19 to 112 times real time. [Verified]

### Agreement with libsoxr

The tests compare the crate with the observations of libsoxr. [Verified]

| Test | What is compared | Result |
| --- | --- | --- |
| Precision | Chirps at 44.1 ↔ 48, 44.1 ↔ 96 and 44.1 → 50.000123 kHz | HQ 20.95 to 21.24 bits and VHQ 29.84 to 30.99, against 20.7 to 21.0 and 29.6 to 30.5 for libsoxr |
| Delay and clear | Random splits of the input and output | The delay equals libsoxr's formula exactly at every call; a second run after `clear` is the first to the bit |
| Splits | 11 qualities × 7 ratios × 200 000 samples, as libsoxr was checked | Every split gives one call's output to the bit, at any phase response too |
| Length | 5 qualities × 17 ratios × 10 lengths, and the boundaries where libsoxr's count was observed | `floor(N / ρ + 0.5)` in 64-bit floating point, also through the pull API |
| Response | 12 presets at 1 → 2, and 48 ↔ 44.1 kHz | The stop-band, the −3 dB point and the pass-band within the bounds set from libsoxr's |
| Impulses | 4 qualities × 5 phase responses × 6 ratios | Peaks within ±1.5 of where expected, DC gain 1 ± 1e-5 |
| Integers | libsoxr's observed integer output | libsoxr's values and clip counts, with ties rounded away from zero |
| Errors | libsoxr's observed errors | The same order of checks and the same messages, word for word |
| Slews | A ramp, ratio 1 to 2 over 1000 frames | The step grows by 10 frames per 100, linearly, to within 1e-3 frames |
| Channels | 3 channels at 44.1 → 48 kHz | 5442 frames; interleaved and split buffers, and each channel alone, agree to the bit |
| Ratio 1 | libsoxr's observed output at a ratio of 1 | The input times the gain, with the last 2 frames held until the end when the gain is not 1 |
| QQ | libsoxr's observed impulse responses | libsoxr's values exactly, such as 0.5625, −0.0625 and 0.8203125 |

#### Where it differs on purpose

| | This crate | libsoxr |
| --- | --- | --- |
| Input before the first output, HQ at 44.1 → 48 kHz | 832 frames | 922 |
| The same, minimum phase | 837 frames | 824 |
| The same, intermediate phase 25 | 756 frames | 1866 |
| Spread of HQ at 1 → 2, minimum phase and phase 25, from the peak to where the response falls under 1e-7 | −7/+347 and −190/+267 | −8/+348 and −194/+272 |
| Pass-band ripple of HQ at minimum phase | 4e-6 dB | 8.5e-4 dB |
| Timing of a variable rate | On time | 1.4 to 2.9 output frames late |
| `delay` at a variable rate | The input held | Always 100 |
| The maximum request of a pull source after `clear` | Kept, as the source's | Lost |
| Rolloff | The pass-band flat with any | Takes effect at some ratios, by amounts that vary |

The rows are libsoxr's bugs, the variable rate's timing and `delay` and the pull source after
`clear`, which the crate fixes, or behaviours that change irregularly with the ratio and the
quality, which the user chose not to follow: the latency, the timing and spread of minimum and
intermediate phase, and the rolloff. The rest
follows libsoxr (see [Following libsoxr](#following-libsoxr)). [Read]

### Presets measured

#### Response, aliases and images

Sine waves of half full scale in 64-bit floating point, with each tone's level fitted. The alias is
the worst output of tones from 23.1 to 24 kHz at 48 → 44.1 kHz, in 30 Hz steps, and the image the
worst tone at 44.1 kHz minus each tone from 20 to 22 kHz at 44.1 → 48 kHz. [Verified]

| Setting | 20 kHz | 20.5 kHz | 21 kHz | 21.5 kHz | 21.8 kHz | 22 kHz | Worst alias | Worst image |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| soxr LQ | −21.8 dB | −30.8 dB | −42.9 dB | −60.4 dB | −76.8 dB | −98.3 dB | −102.6 dB | −102.7 dB |
| soxr MQ | 0.000 dB | −0.095 dB | −3.56 dB | −20.7 dB | −45.1 dB | −79.9 dB | −108.5 dB | −103.0 dB |
| soxr HQ | 0.000 dB | −0.074 dB | −3.80 dB | −24.2 dB | −54.0 dB | −96.4 dB | −130.1 dB | −126.9 dB |
| soxr VHQ | 0.000 dB | −0.030 dB | −4.05 dB | −32.3 dB | −76.0 dB | −147.4 dB | −178.1 dB | −175.1 dB |
| soxr 32-bit | 0.000 dB | −0.016 dB | −4.14 dB | −37.0 dB | −89.8 dB | −206.4 dB | −202.2 dB | −198.0 dB |
| soxr VR | −0.002 dB | −0.628 dB | −7.28 dB | −29.8 dB | −58.7 dB | −97.8 dB | −131.0 dB | −126.8 dB |
| r8brain Low | 0.000 dB | −0.068 dB | −3.99 dB | −26.3 dB | −58.8 dB | −104.3 dB | −151.9 dB | −140.5 dB |
| r8brain Medium | 0.000 dB | 0.000 dB | 0.000 dB | −2.36 dB | −28.6 dB | −87.4 dB | −158.1 dB | −142.9 dB |
| r8brain High | 0.000 dB | 0.000 dB | 0.000 dB | −0.320 dB | −25.6 dB | −106.0 dB | −203.6 dB | −189.7 dB |
| r8brain Max | 0.000 dB | 0.000 dB | 0.000 dB | 0.000 dB | 0.000 dB | −43.8 dB | −222.7 dB | −234.9 dB |
| Rubato Medium | 0.000 dB | −0.079 dB | −5.11 dB | −34.2 dB | −74.3 dB | −124.2 dB | −137.4 dB | −153.5 dB |
| Rubato High | 0.000 dB | 0.000 dB | 0.000 dB | 0.000 dB | −11.8 dB | −90.9 dB | −138.4 dB | −162.8 dB |
| Rubato Max | 0.000 dB | 0.000 dB | 0.000 dB | 0.000 dB | 0.000 dB | −22.5 dB | −137.5 dB | −169.2 dB |
| Rubato 16384 | 0.000 dB | 0.000 dB | 0.000 dB | 0.000 dB | 0.000 dB | 0.000 dB | −137.4 dB | −173.3 dB |

- **Every soxr preset reaches its stop-band** to within 1 dB, `(precision + 1) × 6.02` dB: 102.3 dB
  for LQ and MQ, 126.4 dB for HQ, 174.6 dB for VHQ and 198.7 dB for 32-bit, whose worst image came
  out at −198.0 dB. [Verified]
- **soxr's presets end their pass-bands where libsoxr's do**, at 0.913 of the Nyquist frequency for
  HQ, so they start to fall near 20 kHz and cross −3 dB near 21 kHz, close to r8brain's Low. The
  higher the precision, the steeper the fall, as the transition band is the same. [Verified]
- **The variable rate's pass-band ends earlier**, at 0.9 of the Nyquist frequency, and its
  transition is centred at 0.95. [Verified]
- **Minimum phase changes no level**: HQ at phase 0 measured the same levels to 0.003 dB and the
  same aliases and images to 1 dB. [Verified]
- Rubato's aliases stay near −137 dB at every size, as on 2026-11-07, which points to its 32-bit
  arithmetic rather than its filter. [Inferred]

#### Ringing

How long the response to an impulse at 44.1 → 48 kHz stays above each level under its peak, before
and after it. Linear phase rings as long on each side. [Verified]

| Setting | −60 dB | −100 dB | −120 dB | −144 dB |
| --- | --- | --- | --- | --- |
| soxr LQ | 0.3 ms | 0.4 ms | 0.5 ms | 0.5 ms |
| soxr MQ | 1.0 ms | 1.6 ms | 1.7 ms | 1.8 ms |
| soxr HQ | 1.1 ms | 1.8 ms | 2.0 ms | 2.2 ms |
| soxr VHQ | 1.3 ms | 2.2 ms | 2.5 ms | 2.8 ms |
| soxr 32-bit | 1.3 ms | 2.5 ms | 2.8 ms | 3.1 ms |
| soxr HQ, minimum phase | 0.1 / 2.5 ms | 0.1 / 3.3 ms | 0.1 / 3.7 ms | 0.1 / 4.0 ms |
| soxr HQ, phase 25 | 0.5 / 1.8 ms | 1.3 / 2.6 ms | 1.8 / 2.9 ms | 2.2 / 3.0 ms |
| soxr VHQ, minimum phase | 0.1 / 3.2 ms | 0.1 / 4.3 ms | 0.1 / 4.7 ms | 0.1 / 5.1 ms |
| soxr VR | 0.9 ms | 1.6 ms | 1.8 ms | 1.9 ms |
| r8brain Low | 1.1 ms | 2.0 ms | 2.2 ms | 2.4 ms |
| r8brain High | 2.5 ms | 5.2 ms | 6.1 ms | 6.9 ms |
| r8brain Max | 5.5 ms | 20.1 ms | 25.0 ms | 29.2 ms |
| Rubato Medium | 1.3 ms | 2.1 ms | 2.5 ms | 2.8 ms |
| Rubato High | 3.4 ms | 7.1 ms | 8.4 ms | 9.5 / 23.3 ms |
| Rubato Max | 5.8 ms | 24.4 ms | 30.3 ms | 35.7 ms |
| Rubato 16384 | 7.0 ms | 79.9 ms | 108 ms | 133 ms |

- soxr's presets ring less than r8brain's and Rubato's settings of as deep a stop-band: VHQ, at
  174.6 dB, rings 2.8 ms to r8brain's High 6.9 ms at 180 dB. [Verified] They give the transition
  band more width. [Inferred]
- Minimum phase moves nearly all the ringing after the peak, and makes it longer there. [Verified]
- Rubato's High rang 23.3 ms after the peak at −144 dB but 9.5 ms before. Why was not found.
  [Unknown]

#### Delay in playback

Input each converter takes before its first output, given one frame at a time, in ms. libsoxr's
figures were observed the same way. [Verified] for soxr, [Read] for libsoxr, and for
r8brain and Rubato from the measurements of 2026-11-07.

| Setting | 44.1 → 48 | 48 → 44.1 | 96 → 44.1 | libsoxr at the same |
| --- | --- | --- | --- | --- |
| soxr LQ | 10.7 | 10.8 | 10.7 | 0.5, 10.2, 10.2 |
| soxr MQ | 8.0 | 8.1 | 8.1 | 21.3, 19.4, 19.4 |
| soxr HQ | 18.9 | 19.0 | 19.0 | 20.9, 19.0, 19.0 |
| soxr VHQ | 40.3 | 40.4 | 40.4 | 20.1, 39.5, 39.5 |
| soxr 32-bit | 39.3 | 39.5 | 39.4 | 42.9, 39.1, 39.1 |
| soxr HQ, minimum phase | 19.0 | 19.1 | 19.1 | 18.7, 16.8, 16.8 |
| soxr HQ, phase 25 | 17.1 | 17.2 | 17.2 | 42.3, 38.5, 38.5 |
| soxr VR | 1.9 | 1.9 | 1.9 | — |
| r8brain Low / High | 9.3 / 38.5 | 8.3 / 34.8 | 8.3 / 34.8 | |
| Rubato Medium / High | 6.7 / 23.3 | 6.7 / 23.3 | 6.7 / 23.3 | |

- soxr's delay is mostly the block of its steep filter's FFT, which goes by powers of two, so the
  presets come out close together and minimum phase saves almost none of it. [Inferred] from the
  design (see [Steep filter](#steep-filter)). VHQ and 32-bit take about twice as long as HQ since
  the speed work, as their transforms in `f64` are 8 times the filter's length, against
  4 in `f32`. [Read] in [Steep filter](#steep-filter).
- As with the other converters, this is not heard as a later start: the delay is removed and output
  starts at the song's first frame. [Inferred]

#### Creation

Making a stereo resampler at 44.1 → 48 kHz, the first time in a process and the third. soxr keeps no
filters between resamplers, only the FFT plans. [Verified]

| Setting | First | Third |
| --- | --- | --- |
| soxr LQ, MQ, HQ | 0.04 to 0.10 ms | 0.03 to 0.05 ms |
| soxr VHQ, 32-bit | 0.12 to 0.13 ms | 0.08 to 0.09 ms |
| soxr LQ, minimum phase | 0.08 ms | 0.07 ms |
| soxr HQ, minimum phase | 0.76 ms | 0.55 ms |
| soxr VHQ, minimum phase | 1.83 ms | 1.62 ms |
| soxr 32-bit, minimum phase | 2.53 ms | 2.51 ms |
| soxr VR | 1.51 ms | under 0.01 ms |

A variable rate's first resampler builds the table that every one shares. [Read]

### Real songs

#### Method

The same as on 2026-11-07 for r8brain and Rubato, repeated on this crate:

- **Songs:** 31 of the 296 songs in Hiphile's cache: 10 at 44.1 kHz and 16 bits, 3 at 44.1 kHz and
  24 bits, 8 at 48 kHz, 8 at 96 kHz and 2 at 192 kHz, all FLAC and at least 2 minutes long, spread
  evenly over each group sorted by file name. The cache held no more at 192 kHz. The song at 48 kHz
  and 16 bits and the ALAC songs were left out.
- **Excerpt:** 60 seconds from 30 % into each song, decoded by ffmpeg to 32-bit floating point, as
  Hiphile's decoder gives it, with 1-second raised-cosine fades at both ends.
- **Reference:** an ideal brickwall at the lower Nyquist frequency, made by one FFT of the whole
  excerpt.
- **Converters:** each in blocks of 1024 frames. soxr and r8brain took the samples in 64-bit
  floating point, and Rubato in 32-bit. Rubato's reference was moved by its offset in time.
- **Error:** the output minus the reference over the middle 56 seconds, under a Blackman-Harris
  window, split into 0 to 20 kHz and 20 kHz to the lower Nyquist frequency.
- **Levels:** dB against a full-scale square wave, so a full-scale sine wave is −3 dB. Music up to
  20 kHz measured −9 to −12 dB.
- **As delivered:** the output rounded to 32-bit floating point, as Hiphile hands it on.

The programs used were temporary and are not in the repository.

#### Up to 20 kHz

Error against the ideal, as the median RMS over songs and channels, with the worst in brackets.

Before rounding, in 64-bit floating point:

| Rates (kHz) | Songs | soxr LQ | soxr MQ | soxr HQ | soxr VHQ | soxr 32-bit | r8brain Low | r8brain High | r8brain Max | f32 rounding |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | 13 | −50 (−40) | −123 (−119) | −144 (−140) | −198 (−186) | −208 (−186) | −149 (−145) | −193 (−188) | −221 (−200) | −162 (−159) |
| 44.1 → 88.2 | 13 | −50 (−40) | −139 (−135) | −147 (−144) | −209 (−189) | −210 (−189) | −187 (−182) | −221 (−193) | −226 (−202) | −166 (−162) |
| 44.1 → 96 | 13 | −50 (−40) | −123 (−119) | −144 (−141) | −198 (−188) | −210 (−189) | −149 (−145) | −193 (−188) | −222 (−201) | −165 (−162) |
| 48 → 44.1 | 8 | −57 (−55) | −126 (−121) | −146 (−141) | −197 (−194) | −210 (−195) | −156 (−147) | −198 (−192) | −225 (−204) | −165 (−160) |
| 48 → 96 | 8 | −69 (−65) | −142 (−138) | −150 (−144) | −218 (−208) | −246 (−209) | −191 (−186) | −244 (−211) | −250 (−218) | −171 (−165) |
| 96 → 48 | 8 | −67 (−56) | −142 (−138) | −150 (−146) | −214 (−195) | −218 (−195) | −191 (−187) | −227 (−201) | −232 (−208) | −165 (−161) |
| 96 → 44.1 | 8 | −57 (−46) | −127 (−123) | −146 (−142) | −196 (−191) | −210 (−191) | −153 (−148) | −194 (−192) | −218 (−201) | −165 (−161) |
| 192 → 48 | 2 | −56 (−53) | −125 (−125) | −145 (−145) | −208 (−207) | −221 (−221) | −147 (−147) | −195 (−194) | −238 (−234) | −162 (−162) |
| 192 → 44.1 | 2 | −46 (−43) | −120 (−120) | −143 (−142) | −190 (−182) | −193 (−182) | −145 (−145) | −190 (−189) | −225 (−224) | −162 (−161) |

As delivered, in 32-bit floating point. Where a setting lies under the rounding, the rounding is
what it delivers: soxr's VHQ and 32-bit and r8brain's High and Max did so at every pair, and are
left out. soxr's LQ, MQ and HQ deliver the figures above. [Verified]

| Rates (kHz) | soxr HQ | r8brain Low | r8brain Medium | Rubato Medium | Rubato High | Rubato Max | Rubato 16384 | f32 rounding |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | −144 (−140) | −149 (−145) | −149 (−145) | −135 (−131) | −133 (−130) | −122 (−119) | −128 (−125) | −162 (−159) |
| 44.1 → 88.2 | −147 (−144) | −165 (−161) | −165 (−162) | −139 (−130) | −131 (−127) | −125 (−121) | −116 (−112) | −166 (−162) |
| 44.1 → 96 | −144 (−141) | −149 (−145) | −149 (−145) | −135 (−132) | −133 (−130) | −122 (−119) | −128 (−125) | −165 (−162) |
| 48 → 44.1 | −146 (−141) | −155 (−147) | −155 (−147) | −147 (−142) | −137 (−130) | −133 (−128) | −130 (−125) | −165 (−160) |
| 48 → 96 | −150 (−144) | −168 (−163) | −169 (−163) | −147 (−141) | −133 (−127) | −127 (−122) | −119 (−113) | −171 (−165) |
| 96 → 48 | −150 (−146) | −165 (−161) | −165 (−161) | −136 (−132) | −139 (−136) | −120 (−117) | −129 (−126) | −165 (−161) |
| 96 → 44.1 | −146 (−142) | −153 (−148) | −153 (−148) | −149 (−145) | −139 (−135) | −135 (−130) | −119 (−115) | −165 (−161) |
| 192 → 48 | −145 (−145) | −147 (−147) | −147 (−147) | −138 (−138) | −119 (−119) | −132 (−132) | −127 (−126) | −162 (−162) |
| 192 → 44.1 | −143 (−142) | −145 (−145) | −145 (−145) | −144 (−144) | −137 (−136) | −121 (−120) | −118 (−117) | −162 (−161) |

- **soxr's VHQ and 32-bit** differ from the ideal by less than the rounding of the 32-bit output at
  every pair, as r8brain's High and Max do. [Verified]
- **soxr's HQ** lies 15 to 21 dB above that rounding. Its precision of 20 makes it compute in
  32-bit floating point, as libsoxr's HQ does. [Read] in [Types and gain](#types-and-gain). It is 2 to
  19 dB above r8brain's Low and Medium, which are 64-bit with a stop-band 10 dB deeper. Against
  Rubato it lies from 3 dB above its Medium, at 96 → 44.1 kHz, to 31 dB under its 16384. That is
  some 40 dB under the quantization noise of 16 bits (−101 dB). [Verified]
- **soxr's MQ** lies at −120 to −142 dB, within the range of Rubato's sizes, with a stop-band of
  102 dB. [Verified]
- **soxr's LQ** lowers the treble from about 17 kHz from 44.1 kHz, as its pass-band ends at
  libsoxr's 0.676, so its error is the treble it takes away. [Verified] It could be heard on music
  with treble. [Inferred]
- **The errors follow the steps.** Where one doubling or halving does the work (44.1 → 88.2,
  48 → 96 and 96 → 48), soxr's MQ, HQ and VHQ come out cleaner than at 44.1 → 48, as r8brain's do.
  [Verified]

#### Above 20 kHz

The source's level between 20 kHz and the lower Nyquist frequency, and the level of what each
setting takes away there against it, as medians. 0 dB takes away all of it. [Verified]

| Rates (kHz) | Source, median (loudest) | soxr LQ | soxr MQ | soxr HQ | soxr VHQ | soxr 32-bit | r8brain Low | r8brain High | r8brain Max | Rubato Medium | Rubato High | Rubato Max | Rubato 16384 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 or 96 | −53 (−39) dB | −0.3 dB | −8.7 dB | −8.4 dB | −8.0 dB | −7.9 dB | −8.2 dB | −14.4 dB | −21.6 dB | −7.7 dB | −15.7 dB | −23.7 dB | −30.5 dB |
| 44.1 → 88.2 | −53 (−39) dB | −0.3 dB | −8.7 dB | −8.4 dB | −8.0 dB | −7.9 dB | −8.2 dB | −14.4 dB | −21.6 dB | −6.7 dB | −15.7 dB | −23.6 dB | −30.5 dB |
| 48 → 44.1 | −61 (−55) dB | −0.2 dB | −7.8 dB | −7.5 dB | −7.2 dB | −7.0 dB | −7.4 dB | −13.0 dB | −20.2 dB | −6.8 dB | −14.7 dB | −21.9 dB | −27.9 dB |
| 48 → 96 | −60 (−53) dB | −2.9 dB | −16.7 dB | −16.4 dB | −16.1 dB | −16.0 dB | −16.3 dB | −22.6 dB | −28.2 dB | −14.6 dB | −23.8 dB | −29.5 dB | −35.6 dB |
| 96 → 48 | −55 (−45) dB | −1.7 dB | −11.0 dB | −10.8 dB | −10.5 dB | −10.4 dB | −10.7 dB | −14.9 dB | −22.4 dB | −9.2 dB | −16.0 dB | −24.4 dB | −30.4 dB |
| 96 → 44.1 | −56 (−47) dB | −0.2 dB | −5.6 dB | −5.4 dB | −5.3 dB | −5.2 dB | −5.3 dB | −9.1 dB | −14.9 dB | −5.1 dB | −10.6 dB | −17.0 dB | −24.0 dB |
| 192 → 48 | −48 (−45) dB | −3.8 dB | −38.0 dB | −37.8 dB | −37.5 dB | −37.4 dB | −37.7 dB | −41.7 dB | −47.7 dB | −36.7 dB | −43.0 dB | −49.1 dB | −55.5 dB |
| 192 → 44.1 | −48 (−45) dB | −0.3 dB | −14.5 dB | −14.0 dB | −13.5 dB | −13.3 dB | −13.7 dB | −40.8 dB | −47.9 dB | −12.5 dB | −43.2 dB | −49.2 dB | −55.7 dB |

- soxr's MQ to 32-bit take away nearly the same, about as much as r8brain's Low and Rubato's
  Medium, as their pass-bands end in the same place. [Verified]
- Sound above 20 kHz at these levels is unlikely to be heard by adults. [Inferred]

#### Samples over full scale

soxr's MQ to 32-bit stayed within 0.08 dB of the ideal's highest peak at every pair, as r8brain's
Low did. LQ came out 0.37 dB under it to 0.39 dB over it, as it changes the waveform's treble.
These are peaks between samples in the recordings, not overshoot of the filters, as on
2026-11-07. [Verified]

### Speed

Stereo, 30 seconds of white noise in 32-bit floating point, on one thread, in blocks of 1024
frames. Best of three, as times real time, measured again after the speed work, with
176.4 and 192 kHz added. [Verified]

| Rates (kHz) | soxr LQ | soxr MQ | soxr HQ | soxr VHQ | soxr 32-bit | soxr HQ, minimum phase | soxr VR | r8brain Low | r8brain High | r8brain Max | Rubato Medium | Rubato High | Rubato Max | Rubato 16384 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | 2374 | 2017 | 2028 | 1198 | 1149 | 1956 | 112 | 826 | 816 | 673 | 1045 | 1231 | 1186 | 992 |
| 44.1 → 88.2 | 3784 | 3038 | 3139 | 2036 | 1992 | 3003 | 62 | 1100 | 919 | 813 | 1786 | 1584 | 1335 | 1159 |
| 44.1 → 96 | 1677 | 1500 | 1421 | 817 | 769 | 1380 | 54 | 574 | 595 | 481 | 1045 | 857 | 817 | 657 |
| 44.1 → 176.4 | 1662 | 1528 | 1538 | 950 | 912 | 1477 | 31 | 764 | 730 | 585 | 1029 | 916 | 805 | 689 |
| 48 → 44.1 | 1957 | 1679 | 1718 | 1088 | 1017 | 1688 | 88 | 839 | 815 | 626 | 1573 | 1220 | 1165 | 994 |
| 48 → 96 | 3403 | 2711 | 2782 | 1750 | 1713 | 2598 | 56 | 989 | 908 | 757 | 1694 | 1506 | 1363 | 1155 |
| 48 → 192 | 1636 | 1440 | 1462 | 928 | 886 | 1434 | 29 | 629 | 564 | 509 | 921 | 842 | 725 | 626 |
| 96 → 48 | 3397 | 2666 | 2758 | 1753 | 1706 | 2619 | 41 | 1088 | 1037 | 856 | 1651 | 1442 | 1317 | 1154 |
| 96 → 44.1 | 1722 | 1518 | 1458 | 840 | 788 | 1478 | 42 | 689 | 680 | 534 | 1085 | 860 | 813 | 659 |
| 192 → 48 | 1487 | 1350 | 1335 | 768 | 708 | 1353 | 19 | 842 | 777 | 659 | 1014 | 881 | 713 | 625 |
| 192 → 44.1 | 1108 | 1035 | 933 | 526 | 484 | 956 | 21 | 545 | 549 | 438 | 684 | 555 | 490 | 384 |

- The slowest constant-ratio case here, soxr's 32-bit at 192 → 44.1, takes about 0.2 % of one
  core. [Verified]
- soxr is fastest where one doubling or halving does the work. From 192 kHz its VHQ and 32-bit lose
  to r8brain. [Verified] There it first halves the rate by short filters, before its steep one.
  [Read] in [How it converts at a constant ratio](#how-it-converts-at-a-constant-ratio).
- r8brain's and Rubato's code did not change, but they came out up to 15 % and 36 % slower than in
  the first measurement here, and the variable rate up to 10 %. The machine carried
  other load this time, at a load average of about 3.5. [Inferred] In the first measurement,
  r8brain's figures were 15 to 38 % under those of 2026-11-07, and Rubato's up to 21 % over.
  The programs and the inputs differ, and which accounts for it was not found. [Unknown]
- **The variable rate** costs about the same per input frame at any ratio when it lowers the rate,
  and per output frame when it raises it: 0.9 % of one core from 44.1 to 48 kHz, and 5 % from
  192 kHz, where each output frame takes 4 times the taps. [Verified]
- **Threads** help a call only when it holds enough work: with 8 channels at HQ from 44.1 to
  48 kHz, 10 seconds took 42 ms on one thread, 25 ms on four in calls of up to 8192 frames and
  13 ms in one call. The figures are under [Threads](#threads). [Verified]

### Fourfold upsampling

Measured on 2026-11-09 together with Rubato and r8brain, with the rate raised to four times the
song's, as `sample_rate = "x4"` asks for. Up by four, soxr's steep filter doubles the rate and the
interpolation doubles it again.

- **Songs:** 8 FLAC songs from Hiphile's cache: 4 at 44.1 kHz and 16 bits, 2 at 48 kHz and 24 bits
  and 2 at 96 kHz and 24 bits, decoded by Hiphile's decoder.
- **Excerpt:** the middle 30 seconds, faded over 20 ms at each end, with 1 second of silence before
  and after.
- **Converter:** a copy of each of Hiphile's three converters, with the settings that
  `OutputSettings::from_config` makes for each preset, given 4096 frames at a time.
- **Reference:** the ideal band-limited interpolation, the FFT of the whole excerpt widened with
  zeros to four times its length and transformed back, in 64-bit floating point.
- **Error:** the power of the output minus the reference from 0 to 20 kHz, and the images, the
  power of the output above the song's Nyquist frequency, each against the power of the whole
  reference. So the dB here are against the music, not against full scale.
- **Speed:** from making the converter to its last output, on one thread, best of three. **Delay:**
  the input taken before the first output, given 8 frames of silence at a time.

The songs of each rate are taken together. [Verified]

| Preset | 44.1 → 176.4 kHz: error / images | 48 → 192 kHz | 96 → 384 kHz |
| --- | --- | --- | --- |
| LQ | −36.0 / −112.8 dB | −56.9 / −112.9 dB | −117.7 / −112.4 dB |
| MQ | −116.9 / −113.0 dB | −119.1 / −113.0 dB | −123.3 / −112.5 dB |
| HQ | −136.6 / −136.6 dB | −136.9 / −136.5 dB | −138.8 / −136.2 dB |
| VHQ | −158.6 / −153.4 dB | −159.2 / −153.6 dB | −162.5 / −153.3 dB |
| f32 rounding | −159.9 / −154.7 dB | −160.4 / −154.8 dB | −163.1 / −154.5 dB |

| Preset | Speed, times real time: 44.1 / 48 / 96 kHz | Delay: 44.1 / 48 / 96 kHz |
| --- | --- | --- |
| LQ | 1,714 / 1,577 / 784 | 10.7 / 9.8 / 4.9 ms |
| MQ | 1,525 / 1,410 / 700 | 8.2 / 7.5 / 3.8 ms |
| HQ | 1,501 / 1,392 / 699 | 18.9 / 17.3 / 8.7 ms |
| VHQ | 1,013 / 946 / 472 | 40.5 / 37.2 / 18.6 ms |

- **VHQ reaches the f32 rounding** in the band and above it, within 1.3 dB, as r8brain's High and
  Max do, at about the speed of r8brain's High. [Verified]
- **HQ** lies about 137 dB under the music in the band and above it, as r8brain's Low and Medium
  do. Its precision of 20 bits makes it compute in 32-bit floating point. [Verified]
- **LQ and MQ leave images about 113 dB under the music**, from their stop-band of 102 dB.
  [Verified]
- **LQ's error is the treble it lowers** from 44.1 and 48 kHz: from 44.1 kHz it takes 2.2 dB from
  16 to 20 kHz and 29.0 dB from 20 kHz to the Nyquist frequency, and from 48 kHz 9.3 dB from 20 kHz
  up. A sine from 44.1 kHz is flat to 16 kHz, 3.5 dB down at 18 kHz and 22 dB at 20 kHz; from
  48 kHz, 3.2 dB down at 19.5 kHz. From 96 kHz its fall lies above 20 kHz. [Verified]
- MQ, HQ and VHQ take 2.3 dB from 20 kHz to the Nyquist frequency from 44.1 kHz, as much as
  r8brain's Low, less than Rubato's Medium (3.1 dB) and more than the other presets of both.
  [Verified]
- **LQ to HQ are the fastest settings of any resampler** here, at 1.3 to 2.2 times the speed of
  r8brain's and Rubato's presets. [Verified]

### What it comes to

- Up to 20 kHz, soxr's VHQ and 32-bit differ from an ideal resampler by less than the rounding of
  the 32-bit output, as r8brain's High and Max do. HQ differs by −143 to −150 dB, a little more
  than r8brain's Low and Medium and about as much as Rubato's Medium. [Verified] None of that is
  likely to be heard. [Inferred]
- LQ lowers the treble from about 17 kHz from 44.1 kHz, by libsoxr's pass-band end. [Verified] It
  is the one soxr preset whose difference is likely to be heard. [Inferred]
- Above 20 kHz, soxr's presets keep less than r8brain's High and Max and Rubato's larger sizes, and
  ring much more briefly. [Verified] Neither is likely to be heard. [Inferred]
- Delay, creation time and CPU load matter little in playback, at a constant ratio or a variable
  one. [Inferred]

### Not checked

- libsoxr's own output, which was not to be run. Every comparison with libsoxr is with
  observations of its behaviour.
- Listening comparisons of any kind.
- The variable rate on real songs, and against Rubato's variable-ratio resamplers.
- Integer output, dither and minimum phase on real songs.
- The presets Bits16, Bits24 and the LSR presets on real songs.
- Threads on real songs, and with other numbers of channels than in the tests.
- Songs at 48 kHz and 16 bits, ALAC songs, more than 2 songs at 192 kHz, and anything outside the
  31 songs.

### Testing notes

- **Split the error under a window.** A plain FFT of the middle 56 seconds lets the large error
  just above 20 kHz, the treble that each filter removes, leak into the bins below it. Every
  setting then read about −130 dB up to 20 kHz, whatever its stop-band. A Blackman-Harris window
  keeps each band's error in its band.
- **Move Rubato's reference by its offset**, ⌊N / 2⌋ · M / N − ⌊M / 2⌋ output frames for blocks of
  N and M frames, or its High reads −24 to −30 dB at the pairs where the offset is not zero.
- **Give tones in 64-bit floating point.** Tones made in 32-bit floating point leave a floor at
  −153 dB under a half-scale tone, which hides the stop-bands of VHQ, 32-bit and r8brain's High.
- A zsh loop that reads into a variable named `path` replaces the shell's `PATH`, and every later
  command in it is not found.
- `cargo test -p soxr` runs the crate's 99 tests in a few seconds. The tests of
  `tests/channels.rs` need calls large enough for threads to start, or they compare one thread with
  itself.
