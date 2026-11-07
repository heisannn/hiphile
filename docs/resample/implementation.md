# Resampling in Hiphile

Hiphile resamples PCM when the output device runs at another rate than the song. It can use
[Rubato](https://crates.io/crates/rubato)'s FFT resampler, or `r8brain`, a Rust port of
[r8brain-free-src](https://github.com/avaneev/r8brain-free-src) by Aleksey Vaneev of Voxengo. This
document explains how both work in Hiphile, most of all the port, and why it was built this way.
Measurements and the comparison of the two are in [findings.md](findings.md).

| | |
| --- | --- |
| Original | r8brain-free-src 7.7, C++, MIT license |
| Port | The `r8brain` crate. No I/O and no GPUI. |
| Arithmetic | 64-bit floating point, one channel per `Resampler` |
| In Hiphile | 32-bit floating point in and out, for PCM only |
| Default preset | High: a 2 % transition band, 180.15 dB and linear phase |
| Agreement with the original | Linear phase within about 2e-15, and minimum phase within about 3e-4 |

## Where the code lives

| Crate | Responsibility |
| --- | --- |
| `r8brain` | The resampler: filter design, convolution, interpolation and the filter caches |
| `audio` | `R8brainConverter`, which runs a `Resampler` per channel over interleaved samples. `Resampling::R8brain` and `OutputSettings::from_config`. |
| `config` | The `r8brain_*` keys, their ranges and the presets |
| `settings` | The r8brain filter and r8brain performance groups of the Audio page |

## When it runs

Resampler under Resampling chooses Rubato's FFT resampler or this port. A converter exists only for a song whose rate differs from
the device's: with a fixed `sample_rate`, when the device lacks the song's rate, or for a multiple
such as `x2`. DSD output oversamples with its own Rubato stages, so the r8brain groups are hidden
while DSD is on.

## Rubato's FFT resampler

Rubato under Resampler runs Rubato 5.0.1's `Fft` resampler through `FftConverter` in
`crates/audio/src/convert.rs`, at one of these qualities.

| Quality | FFT size | Window |
| --- | --- | --- |
| Medium (default) | 256 | Blackman-Harris² |
| High | 1024 | Blackman-Harris² |
| Max | 4096 | Blackman-Harris² |
| Custom | 256 to 16384 | Any of six |

- **Blocks.** Rubato cuts the input into blocks of N frames and the output into blocks of
  M = N × target ÷ source. `FftConverter` asks for blocks whose side at the lower rate has at least
  the FFT size in frames, rounded up to a multiple of the smallest block that the two rates allow,
  such as 147 frames at 44.1 kHz for 44.1 to 48 kHz and back.
- **Filter.** Each block is padded to 2N, transformed, and multiplied by the spectrum of an N-tap
  windowed sinc. The spectrum is then cut or padded to 2M bins, transformed back and overlap-added.
- **Cutoff.** A fit for each window sets it at 1 / (1 + k₁/L + k₂/L² + k₃/L³) of the lower Nyquist
  frequency, where L is the shorter block. Rubato itself sizes N from the input side alone, which
  leaves a large fall in rate a short M and a low cutoff: at Medium, 192 to 48 kHz would get M = 64
  and lose 2 dB at 18 kHz. Sizing by the lower rate gives a fall in rate the blocks of the same
  rise.
- **Precision.** Hiphile runs it in 32-bit floating point.
- **Delay.** The converter trims the ⌊M/2⌋ output frames of Rubato's `output_delay()` from the
  start, and flushes the end with silence. The filter's centre lies ⌊N/2⌋ · M/N output frames in,
  so when N or M is odd, the output stays off by half a frame of that block's rate. High's block of
  1029 frames at 44.1 kHz leaves it about 11 µs off.

How much treble Rubato's own sizing takes from real songs is in
[findings.md](findings.md#rubato-when-lowering-the-rate), and the offset in
[findings.md](findings.md#timing).

## Signal path

```text
decoder (f32, interleaved)
  │
  ▼  R8brainConverter (audio)
split into channels, f32 to f64, anything not finite to 0
  │
  ▼  a Resampler per channel (r8brain), in blocks of max_input_len frames
the steps of its chain
  │
  ▼
f64 to f32, interleaved again ──▶ ring buffer ──▶ device
```

A `Resampler` removes the latency of its own filters, so output frame 0 is input frame 0, and the
converter trims nothing. A sample that is not finite becomes silence, as it would otherwise spread
through a whole FFT block.

At the end of a song, `finish` feeds silence until the output has `round(frames in × ratio)` frames
and drops any surplus. `undelivered_frames` tells the engine how much of the song is still inside
the filters, which gapless joins and crossfades count on.

## How the original converts

The original splits the work between one steep filter and short ones. For 44.1 to 48 kHz:

```text
44.1 kHz ──▶ steep low-pass, ×2 by FFT ──▶ 88.2 kHz ──▶ sinc bank, 147 in : 80 out ──▶ 48 kHz
             (BlockConvolver)                            (Interpolator)
```

The steep filter does everything that needs a sharp edge, at twice the lower of the two rates. The
interpolator then only fills in between samples of a signal with nothing near its Nyquist
frequency, which filters of 8 to 30 taps can do.

`Chain::build` in `resampler.rs` picks the steps by the ratio:

| Ratio | Steps |
| --- | --- |
| Equal rates | None. `process` hands the input back. |
| 1:2, 1:3, 2:3, 3:2 or 3:4 | One block convolver that does both factors |
| Up by 2ⁿ or 3·2ⁿ | Block convolver ×2 or ×3, then half-band doublers |
| Any other rise, or a fall to more than half the rate | Block convolver ×2, then the interpolator. For large rises, the interpolator stops a power of two short, and another ×2 convolver and half-band doublers follow. |
| A fall to half the rate or less | Half-band halvers until under four times the target, the block convolver (÷2 or ÷3 when that is exact), then the interpolator unless it was exact |

### Steep filter

`fir_filter.rs` designs a Kaiser-windowed sinc. The window's β of 125 is raised to a power, and the
power and the cutoff's distance below the band edge come from the original's closed-form fit to the
transition band and attenuation (`design`). The filter grows with the attenuation and as the
transition band narrows.

- **Linear phase:** the kernel is centred on sample 0, so its spectrum is real and cheaper to
  apply.
- **Minimum phase:** a cepstral transform with 16 times oversampling in frequency. It rarely comes to
  a whole number of samples, so the output can stay delayed by `latency_frac`.

### Block convolver

`block_convolver.rs` runs overlap-save convolution in one FFT block, the next power of two above the
kernel, doubled. `extended_fft` doubles it again. A power-of-two factor happens in the frequency
domain: the input's spectrum is mirrored to double the rate, or the output's spectrum is cut short
to halve it. Other factors stuff zeros or drop samples.

Output comes from the block before the one being filled, which costs one block of latency. So the
latency goes up in powers of two rather than with the filter's length, and within one block size a
shorter filter saves nothing. This is why the presets' delays come in steps of about double.

### Interpolator

`interpolator.rs` and `filter_bank.rs` reach the target rate with a bank of short sinc filters at
fractional delays.

- **Whole stepping:** if the rates are at a ratio of whole numbers with an output step of 1500 or
  less, the bank holds one row of taps per output position, which keeps time exactly. Every pair of
  standard rates steps wholly, such as 147:80 for 88.2 to 48 kHz and 320:147 for 96 to 44.1 kHz.
- **Fractional:** any other ratio interpolates between rows, each tap as a quadratic through eight
  rows. The number of rows is `⌈6.4^(a / 50)⌉`, where `a` is the attenuation that the bank's window
  reaches, unless `filter_fractions` sets it.

The bank's filters have 8 to 30 taps, or 6 to 24 where only a third of the band has to pass. Their
Kaiser windows come from the original's tables, as the shortest that reaches the attenuation.

### Half-band steps

`half_band/` doubles and halves the rate with sparse symmetric filters, where half of the taps are
zero. Their kernels are the original's tables, chosen by attenuation and by how steep that step has
to be. The halvers are relaxed. They only guarantee a stop-band from 0.75 of the Nyquist frequency
at the last one, as what aliases there lands where the next filter removes it.

### Caches

Filters are costly to design, so `cache.rs` keeps them for later resamplers in one process, shared
between threads. While one thread builds an item, other threads wait for it. Items in use always
stay, and past the limit the cache drops the least recently used item that is not in use.

| Cache | Limit |
| --- | --- |
| Steep filters | 96 (`CacheLimits::filters`) |
| Whole-step banks | 12 (`CacheLimits::filter_banks`) |
| Fractional banks of the default size | Never dropped. There are few of them. |
| Banks with a set `filter_fractions` | Kept only while a resampler uses them. One of 65536 rows takes about 45 MB. |

`CacheLimits::apply` changes the limits for every resampler, and the converter applies the
configured limits each time it is created.

### Map to the original

| Original | Port |
| --- | --- |
| `CDSPResampler.h` | `resampler.rs` (`Resampler`, `Chain`) |
| `CDSPProcessor.h` | `processor.rs` |
| `CDSPBlockConvolver.h` | `block_convolver.rs` |
| `CDSPFIRFilter.h` | `fir_filter.rs` |
| `CDSPSincFilterGen.h` | `sinc.rs` |
| `CDSPFracInterpolator.h` | `interpolator.rs`, `filter_bank.rs` |
| `CDSPHBUpsampler.h`, `CDSPHBDownsampler.h` and their `.inc` files | `half_band/` |
| `CDSPRealFFT.h` | `fft.rs` |
| `r8bbase.h` | `cache.rs`, `math.rs` |

## Differences from the original

- **FFT.** The port uses `realfft` instead of the Ooura code that the original bundles. Results agree
  up to rounding, not bit for bit.
- **Build options become runtime options.**

  | Original | Port |
  | --- | --- |
  | `R8B_EXTFFT` | `Options::extended_fft` |
  | `R8B_FLTTEST` and `InterpFilterFracs` | `Options::whole_stepping = false` and `Options::filter_fractions` |
  | `R8B_FILTER_CACHE_MAX` | `CacheLimits::filters` |
  | `R8B_FRACBANK_CACHE_MAX` | `CacheLimits::filter_banks` |

- **Banks with a set number of rows are kept only while in use.** The original has one number for
  the whole build. In the port each resampler can set its own, and large ones could otherwise pile
  up tens of megabytes each.
- **Not ported.** The FFT back-ends (`R8B_IPP`, `R8B_PFFFT`, `R8B_PFFFT_DOUBLE`), the allocator
  classes (`R8B_BASECLASS`, `R8B_MEMALLOCCLASS`, `R8B_DSPBASECLASS`), the DLL and the benchmarks.
- **Values the original cannot take are errors.** Rates must be finite and positive, the transition
  band from 0.5 to 45 %, the attenuation from 49 to 218 dB, `max_input_len` at least 1 and cache
  limits at least 2. `filter_fractions` must be at least 5: with fewer, some rows of the bank delay
  by a whole number of samples other than 0 or 1, where the sinc divides by zero.
- **The original's classes are constants.** `CDSPResampler16`, `CDSPResampler16IR` and
  `CDSPResampler24` are `Options::BITS_16`, `BITS_16_IMPULSE_RESPONSE` and `BITS_24`.
  `Options::default()` is `BITS_27`, the original's default of 206.91 dB.

## Settings

Resampler under Resampling on the Audio page chooses Rubato or r8brain, and Quality under it sets
the quality of the one chosen. Each comes back at the quality it had. With r8brain chosen and DSD
off, the r8brain filter and r8brain performance groups follow.

### Quality

r8brain's Quality sets the values that decide the sound. Changing one of them switches Quality to
Custom, which keeps the current values, and choosing Custom alone changes nothing. Low, Medium,
High and Max are its presets. Attenuation and Transition band have no reset of their own, as
choosing a preset brings them back.

| Preset | Transition band | Attenuation | Delay, 44.1 to 48 kHz |
| --- | --- | --- | --- |
| Low | 5 % | 136.45 dB | 9.3 ms |
| Medium | 2.4 % | 136.45 dB | 18.2 ms |
| High (default) | 2 % | 180.15 dB | 38.5 ms |
| Max | 0.5 % | 218 dB | 147.2 ms |

All presets use linear phase, whole stepping and the automatic number of filter positions.

- **Low** is in the shortest step of block size that keeps 20 kHz at full level, a little wider
  than the narrowest band that stays in that step at every pair of rates measured.
- **Medium** is the narrowest tenth of a percent in the next step of block size, from every common
  song rate to every rate a device can be fixed at.
- **High** is the original's setting for 24-bit and 32-bit floating point, which the decoder gives.
- **Max** is the end of the original's ranges.

The delay is how much input a resampler takes before its first output. The reasons for these values
and the measurements behind them are in [findings.md](findings.md#why-these-values).

### The groups

| Group | Item | Value |
| --- | --- | --- |
| Resampling | Resampler | Rubato or r8brain |
| | Quality | Low, Medium, High, Max or Custom |
| r8brain filter | Bit depth | Sets the attenuation: 16-bit 136.45 dB, 16-bit impulse response 109.56 dB, 24-bit 180.15 dB or 27-bit 206.91 dB. Other is shown for any other value. |
| | Attenuation | 49 to 218 dB |
| | Transition band | 0.5 to 45 % |
| | Phase | Linear or Minimum |
| | Whole-step filters | On or off |
| | Filter positions | 5 to 65536, or empty for automatic |
| r8brain performance | Extended FFT | On or off |
| | Input block | 1 to 1048576 frames, 1024 by default |
| | Filter cache | 2 to 4096, 96 by default |
| | Filter bank cache | 2 to 4096, 12 by default |

The performance values leave the sound alone and do not change Quality. A number field takes
effect 500 ms after typing stops, since each change restarts the resampling of what is playing. A
number beyond the range waits for Enter or for leaving the field, which hold it to the range.

### Keys

The keys are listed under [Configuration](../../README.md#configuration) in the README.
`resampling` holds Rubato's quality or `r8brain`, and `r8brain_quality` holds r8brain's. The keys
of the filter values are read only with `r8brain_quality = "custom"`, and choosing a preset removes
them. The performance keys are read with any quality. Saving writes only the values that changed,
as the rest may have come from the environment, and of those only what differs from the defaults.
It keeps the r8brain values while Rubato is chosen. `OutputSettings::from_config` in
`crates/audio/src/from_config.rs` turns them into `R8brainSettings`.

## Performance

At the High preset, the port converts about 100 million samples a second of one channel from 44.1 to
48 kHz on an Apple M5, within 15 % of the original either way. A stereo song takes at most about
0.3 % of one core at any preset and standard rate, the most from 44.1 to 768 kHz. At a preset, a
resampler takes under 0.5 ms to create the first time, and reuses its filters after that. Other
values can take longer the first time: about 8 ms for minimum phase at Max, and about 30 ms for
65536 filter positions. Full tables are in [findings.md](findings.md#speed).

The `r8brain`, `rubato` and `realfft` crates are built with `opt-level = 3` in debug builds too.

## Limits of the port

- Hiphile runs the channels one after another on the engine thread.
- Minimum phase and ratios that do not step wholly were checked only with synthetic signals.
- It is not used for DSD.
- It was built and tested only on macOS, although the crate has no platform code.

## Testing

`cargo test -p r8brain` needs no device.

- `tests/resampling.rs` checks properties on synthetic signals: timing and level in the pass-band,
  silence above the target's band, independence from how the input is split, the stated latency and
  the ends of every range.
- `tests/upstream_reference.rs` compares 71 routes with what the original makes of the same noise:
  31 by default, the same 31 with `extended_fft`, and 9 that interpolate fractionally. Linear phase
  must agree within 1e-13, and minimum phase within 1e-3.

`tests/upstream_reference/generate.cpp` writes the expected files from a checkout of the original.
Its first lines give the commands, including the `R8B_EXTFFT` and `R8B_FLTTEST` builds.

The other crates are tested with `cargo test -p audio -p settings -p config@0.3.0`. The version
tells Hiphile's `config` crate from a dependency of the same name.

Gapless joins on a loopback device are tested by name:

```sh
HIPHILE_TEST_DEVICE="BlackHole 2ch" cargo test -p audio --lib -- \
  --ignored --test-threads=1 resampled_tracks_join_without_a_gap_on
```

That filter runs `…_on_r8brain`, `…_on_the_r8brain_presets` and `…_on_r8brain_with_other_options`.

> [!WARNING]
> Do not run the whole `--ignored` set, which can reach the default device.
