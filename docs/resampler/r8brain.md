# r8brain

`r8brain` is a Rust port of [r8brain-free-src](https://github.com/avaneev/r8brain-free-src) by
Aleksey Vaneev of Voxengo. This document explains how it converts, how Hiphile runs it, what its
settings do and how it measured, against the original and on real songs. The other two resamplers
are [Rubato](rubato.md) and [soxr](soxr.md).

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
| `audio` | `R8brainConverter` in `src/convert/r8brain_converter.rs`, which runs a `Resampler` per channel over interleaved samples. `Resampling::R8brain` and `R8brainSettings` in `src/output.rs`, and `OutputSettings::from_config` in `src/from_config.rs`. |
| `config` | The `r8brain_*` keys, their ranges and the presets |
| `settings` | The r8brain quality, and the r8brain filter and r8brain performance groups of the Audio page |

## When it runs

Resampler under Resampling on the Audio page chooses Rubato, r8brain or soxr. A converter exists only
for a song whose rate differs from the device's: with a fixed `sample_rate`, when the device lacks
the song's rate, or for a multiple such as `x2`. DSD output oversamples with its own Rubato stages,
so the r8brain groups are hidden while DSD is on.

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
shorter filter saves nothing. This is why the presets' delays come in steps of about double (see
[Why these values](#why-these-values)).

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

Resampler under Resampling on the Audio page chooses Rubato, r8brain or soxr, and Quality under it
sets the quality of the one chosen. With r8brain chosen and DSD off, the r8brain filter and r8brain
performance groups follow.

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

The delay is how much input a resampler takes before its first output. The measurements behind
these values are under [Why these values](#why-these-values).

### The groups

| Group | Item | Value |
| --- | --- | --- |
| Resampling | Resampler | Rubato, r8brain or soxr |
| | Quality | With r8brain, Low, Medium, High, Max or Custom |
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
`resampling = "r8brain"` chooses r8brain, and `r8brain_quality` holds its quality. The keys of the
filter values are read only with `r8brain_quality = "custom"`, and choosing a preset removes them.
The performance keys are read with any quality. Saving writes only the values that changed, as the
rest may have come from the environment, and of those only what differs from the defaults. It keeps
the r8brain values while another resampler is chosen. `OutputSettings::from_config` turns them into
`R8brainSettings`.

## Measurements

Three sets of measurements cover r8brain, each by its own temporary programs, which are not in the
repository:

| Date | With | Songs | Where |
| --- | --- | --- | --- |
| 2026-11-07 | The original, and Rubato | 33 songs, decoded by ffmpeg | This document, and [Rubato](rubato.md#measurements) |
| 2026-11-08 | soxr and Rubato | 31 songs, decoded by ffmpeg | [soxr](soxr.md#measurements), which compares all three |
| 2026-11-09 | Rubato and soxr | 8 songs, decoded by Hiphile's decoder, raised fourfold | [Fourfold upsampling](#fourfold-upsampling) |

Unless a section says otherwise, the figures below are those of 2026-11-07.

Each claim is tagged by how far it was checked.

| Tag | Meaning |
| --- | --- |
| [Verified] | Checked by running or measuring it |
| [Read] | Checked by reading code, without running it |
| [Inferred] | Reasoned from observations, without checking directly |
| [Unknown] | The cause or behavior is not understood |

| Item | Value |
| --- | --- |
| Mac | Apple M5, 24 GB of memory, macOS 26.2 |
| Rust | 1.99.1, release builds |
| Original | r8brain-free-src 7.7 (commit `9ca64da`), built with clang 21.1.8 and its default Ooura FFT |

Nobody has listened to compare the settings. Every claim about what can be heard is [Inferred].
Nothing was run on any platform other than macOS.

### Summary

- **The port agrees with the original** within 2.2e-15 at linear phase and 2.7e-4 at minimum phase,
  with the same latency and lengths, at about the original's speed. [Verified]
- **Up to 20 kHz:** High and Max differ from an ideal resampler by less than the rounding of the
  32-bit output on every route. Low and Medium lie 9 to 17 dB above that rounding where more than
  one step runs, far under the quantization noise of 16 bits. [Verified] None of it is likely to be
  heard. [Inferred]
- **Above 20 kHz** the presets differ by design, in how much of 20 to 22 kHz they keep and in how
  long they ring there. [Verified]
- **Speed:** every preset runs at least 527 times faster than real time in stereo at the songs'
  pairs of rates. [Verified]

### Agreement with the original

Both checks feed the same white noise to the port and to the original. [Verified]

| Check | Routes | Linear phase | Minimum phase |
| --- | --- | --- | --- |
| Whole outputs of 120 000 samples, given in chunks of 1 to 5000 | 80, in each of three builds: default, `R8B_EXTFFT` and `R8B_FLTTEST` | Within 2.2e-15 | Within 2.7e-4 |
| `tests/upstream_reference.rs`, 32 samples per route | 31 default, 31 `R8B_EXTFFT`, 9 `R8B_FLTTEST` | Within 1.0e-15 | Within 1.6e-4 |

- On every route of the second check, the input before the first output and the number of output
  samples are exactly the original's. [Verified]
- The fractional latency of minimum phase is within 1e-4 of the original's. [Verified]
- Minimum phase agrees less closely because its transform takes logarithms of values near the
  FFT's rounding noise, where two FFTs differ. [Inferred]

#### Speed against the original

Million input samples a second of one channel, at `CDSPResampler24` (the High preset), in blocks
of 1024 samples. Best of three runs, with the original at `-O3`. [Verified]

| Rates (kHz) | Port | Original | Port / original |
| --- | --- | --- | --- |
| 44.1 → 48 | 100.4 | 100.5 | 1.00 |
| 44.1 → 96 | 74.2 | 77.5 | 0.96 |
| 44.1 → 88.2 | 160.5 | 149.9 | 1.07 |
| 48 → 44.1 | 99.4 | 101.7 | 0.98 |
| 96 → 44.1 | 167.0 | 166.7 | 1.00 |
| 192 → 44.1 | 275.2 | 255.4 | 1.08 |
| 44.1 → 705.6 | 62.0 | 55.5 | 1.12 |
| 44.1 → 768 | 37.0 | 34.5 | 1.07 |
| 768 → 48 | 882.5 | 764.6 | 1.15 |
| 44.1 → 47.9995 | 51.5 | 55.1 | 0.93 |

The port's first run after building measured 53 rather than 100 at 44.1 to 48 kHz. [Unknown]

### Presets

#### Values and response

Measured with sine waves on the port, at 64-bit floating point, with each tone's level fitted.
Delay is the input a resampler takes before its first output. The alias is the worst output of
tones from 23.1 to 24 kHz at 48 → 44.1 kHz, in 30 Hz steps, and the image the worst tone at 44.1 kHz
minus each tone from 20 to 22 kHz at 44.1 → 48 kHz. [Verified]

| Preset | Delay 44.1 → 48 / 48 → 44.1 kHz | Level at 44.1 → 48 kHz | Worst alias | Worst image |
| --- | --- | --- | --- | --- |
| Low | 9.3 / 8.3 ms | 20 kHz 0.000 dB, 20.5 kHz −0.068 dB, 21 kHz −3.99 dB | −151.2 dB | −147.5 dB |
| Medium | 18.2 / 16.3 ms | 21 kHz 0.000 dB, 21.5 kHz −2.36 dB | −156.0 dB | −140.6 dB |
| High | 38.5 / 34.8 ms | 21.5 kHz −0.32 dB, 21.8 kHz −25.6 dB | −198.9 dB | −186.1 dB |
| Max | 147.2 / 132.1 ms | 21.8 kHz 0.000 dB, 22 kHz −43.8 dB | −200.0 dB | −243.2 dB |

Lowering from 96 and 192 kHz, tones from just above the target's Nyquist frequency to 1.4 times it
came out at −140 to −206 dB. [Verified]

Delay at every pair of rates measured, in ms. [Verified]

| Rates (kHz) | Low | Medium | High | Max |
| --- | --- | --- | --- | --- |
| 44.1 → 48, 44.1 → 96 | 9.3 | 18.2 | 38.5 | 147.2 |
| 44.1 → 88.2 | 9.2 | 18.1 | 38.4 | 147.0 |
| 48 → 44.1, 96 → 44.1 | 8.3 | 16.3 | 34.8 | 132.1 |
| 48 → 96, 96 → 48 | 8.4 | 16.6 | 35.3 | 135.1 |
| 192 → 48 | 8.5 | 16.7 | 35.4 | 135.2 |
| 192 → 44.1 | 8.4 | 16.4 | 34.9 | 132.2 |

#### Why these values

The delay comes from the FFT block, which goes by powers of two, so it moves in steps. Within one
step, a wider band or a lower attenuation saves almost no delay. A wider band can even take a little
more, as with 4 % against 3 % at 136.45 dB. [Verified]

| Transition band, attenuation | 44.1 → 48 | 48 → 44.1 | Level at 21 kHz |
| --- | --- | --- | --- |
| 10 %, 136.45 dB | 4.7 ms | 4.2 ms | −28.3 dB (−4.5 dB at 20 kHz) |
| 8 %, 136.45 dB | 10.2 ms | 9.2 ms | −19.1 dB (−0.96 dB at 20 kHz) |
| 5 %, 136.45 dB | 9.3 ms | 8.3 ms | −3.99 dB |
| 4.5 %, 136.45 dB | 9.0 ms | 18.7 ms | −2.08 dB |
| 4 %, 136.45 dB | 20.2 ms | 18.4 ms | −0.75 dB |
| 3 %, 136.45 dB | 19.2 ms | 17.3 ms | −0.002 dB |
| 2.4 %, 136.45 dB | 18.2 ms | 16.3 ms | 0.000 dB |
| 2.3 %, 136.45 dB | 18.0 ms | 37.4 ms | 0.000 dB |
| 2 %, 136.45 dB | 40.4 ms | 36.6 ms | 0.000 dB |
| 2 %, 180.15 dB | 38.5 ms | 34.8 ms | 0.000 dB |
| 1 %, 218 dB | 73.7 ms | 66.1 ms | 0.000 dB |
| 0.5 %, 218 dB | 147.2 ms | 132.1 ms | 0.000 dB |

The edges of the steps were found in steps of 0.01 % over 160 pairs: each song rate of 8, 11.025,
16, 22.05, 24, 32, 44.1, 48, 64, 88.2, 96, 176.4, 192, 352.8, 384, 705.6 and 768 kHz to each of the
10 rates a device can be fixed at. [Verified]

- **Low:** the step under Low's needs a band wider than 8 %, and 8 % already takes 20 kHz down by
  about 1 dB. In Low's step, 4.65 % is the narrowest band that keeps all 160 pairs. At 4.64 %, 15
  of them fall into the next step, such as 11.025 to 48 kHz, and 30 at 4.60 %. [Verified] Low's
  5 % leaves room above that edge.
- **Medium:** in the next step, 2.31 % is the narrowest band that keeps all 160 pairs, and 2.4 % the
  narrowest tenth of a percent. At 2.30 %, 15 of them fall into the next step, such as 48 to
  44.1 kHz. [Verified] Against 3 %, 2.4 % takes about 1 ms less and stays flat to 21 kHz.
- **Medium at 2 %** would take slightly longer than High and stop less. [Verified]

### Real songs

- **Songs:** 33 of the 323 songs in Hiphile's cache: 10 at 44.1 kHz and 16 bits, 3 at 44.1 kHz and
  24 bits, 8 at 48 kHz, 8 at 96 kHz and 4 at 192 kHz, all FLAC and at least 2 minutes long. They
  were spread evenly over each group, sorted by file name. The 2 songs at 48 kHz and 16 bits and
  the 2 ALAC songs were left out.
- **Excerpt:** 60 seconds from 30 % into each song, with 1-second fades at both ends, decoded to
  64-bit floating point by ffmpeg.
- **Reference:** an ideal brickwall at the lower Nyquist frequency, made by one FFT of the whole
  excerpt.
- **Converter:** `Resampler::oneshot` in blocks of 1024 samples.
- **Error:** the output minus the reference, split into 0 to 20 kHz and 20 kHz to the lower Nyquist
  frequency, and measured over the middle 56 seconds.
- **Levels:** dB against a full-scale square wave, so a full-scale sine wave is −3 dB. Music up to
  20 kHz measured about −10 dB.
- **f32 rounding:** the reference rounded to 32-bit floating point, as the converter rounds its
  output.

#### Up to 20 kHz

Error against the ideal, as the median RMS over songs and channels, with the worst in brackets.
[Verified]

At 64-bit floating point, before the converter rounds it:

| Rates (kHz) | Songs | Low | Medium | High | Max | f32 rounding |
| --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | 13 | −151 (−144) | −151 (−144) | −195 (−188) | −229 (−222) | −163 |
| 44.1 → 88.2 | 13 | −188 (−184) | −192 (−190) | −246 (−242) | −293 (−280) | −167 |
| 44.1 → 96 | 13 | −151 (−144) | −151 (−144) | −195 (−188) | −229 (−222) | −166 |
| 48 → 44.1 | 8 | −153 (−147) | −153 (−147) | −196 (−191) | −230 (−224) | −162 |
| 48 → 96 | 8 | −189 (−187) | −193 (−191) | −247 (−244) | −296 (−293) | −168 |
| 96 → 48 | 8 | −189 (−188) | −193 (−192) | −247 (−245) | −296 (−294) | −163 |
| 96 → 44.1 | 8 | −152 (−149) | −152 (−149) | −195 (−193) | −229 (−226) | −163 |
| 192 → 48 | 4 | −148 (−147) | −148 (−147) | −196 (−194) | −243 (−242) | −163 |
| 192 → 44.1 | 4 | −146 (−145) | −146 (−145) | −191 (−190) | −226 (−225) | −163 |

As delivered, in 32-bit floating point:

| Rates (kHz) | Low | Medium | High | Max |
| --- | --- | --- | --- | --- |
| 44.1 → 48 | −151 (−144) | −151 (−144) | −163 (−160) | −163 (−160) |
| 44.1 → 88.2 | −165 (−163) | −165 (−163) | −165 (−163) | −165 (−163) |
| 44.1 → 96 | −151 (−144) | −151 (−144) | −166 (−163) | −166 (−163) |
| 48 → 44.1 | −153 (−147) | −153 (−147) | −162 (−160) | −162 (−160) |
| 48 → 96 | −166 (−164) | −166 (−164) | −166 (−164) | −167 (−166) |
| 96 → 48 | −163 (−162) | −163 (−162) | −163 (−162) | −163 (−162) |
| 96 → 44.1 | −152 (−149) | −152 (−149) | −163 (−162) | −163 (−162) |
| 192 → 48 | −148 (−147) | −148 (−147) | −163 (−162) | −163 (−162) |
| 192 → 44.1 | −146 (−145) | −146 (−145) | −163 (−162) | −163 (−162) |

- The largest single sample of error was −125 dB for Low and Medium, −172 dB for High and −206 dB
  for Max. [Verified]
- High and Max stay below the rounding of the 32-bit output on every route, so what they deliver is
  the ideal plus that rounding. [Verified]
- Low and Medium lie 9 to 17 dB above that rounding where more than one step runs. That is 43 dB or
  more under the quantization noise of 16 bits (−101 dB), and near that of 24 bits (−149 dB).
  [Verified] They give the same error there, as the error follows the attenuation they share.
  [Verified]
- Where one block convolver does all the work (44.1 → 88.2, 48 → 96 and 96 → 48), every preset
  comes out 37 to 67 dB cleaner than at 44.1 → 48. [Verified]
- Rubato's error is 25 to 43 dB above r8brain's at High, 30 to 44 dB at Max, and 1 to 29 dB at
  Medium (see [Rubato](rubato.md#up-to-20-khz)). [Verified]

#### Above 20 kHz

Each preset takes away part of what lies between 20 kHz and the lower Nyquist frequency. The table
gives the source's level in that band, and the level of what each preset takes away against the
source's level there, as medians. [Verified]

| Rates (kHz) | Source, median (loudest) | Low | Medium | High | Max |
| --- | --- | --- | --- | --- | --- |
| 44.1 → 48 or 96 | −54 (−46) dB | −8.9 dB | −13.7 dB | −14.6 dB | −25.5 dB |
| 44.1 → 88.2 | −54 (−46) dB | −8.9 dB | −13.7 dB | −14.6 dB | −25.5 dB |
| 48 → 44.1 | −61 (−45) dB | −7.6 dB | −13.7 dB | −14.8 dB | −22.6 dB |
| 48 → 96 | −61 (−45) dB | −33.7 dB | −39.0 dB | −41.8 dB | −55.3 dB |
| 96 → 48 | −58 (−54) dB | −9.1 dB | −13.0 dB | −13.9 dB | −21.6 dB |
| 96 → 44.1 | −60 (−55) dB | −5.5 dB | −8.9 dB | −9.5 dB | −16.2 dB |
| 192 → 48 | −48 (−43) dB | −23.9 dB | −27.2 dB | −27.9 dB | −34.1 dB |
| 192 → 44.1 | −48 (−44) dB | −10.0 dB | −22.5 dB | −25.4 dB | −31.7 dB |

- Music above 20 kHz lay about 40 to 50 dB under the music as a whole in these songs. [Verified]
- Low takes away the most of the presets, as its response starts to fall at about 20.5 kHz. Max
  keeps nearly all of it up to 21.8 kHz. [Verified]
- Sound above 20 kHz at these levels is unlikely to be heard by adults. [Inferred]

#### Samples over full scale

Every group had songs that peaked at 0 dBFS. Resampled, many of them went over full scale with
every preset. The ideal brickwall did the same. These are peaks between samples in the recordings,
not overshoot of the filters. [Verified]

| Rates (kHz) | Highest peak, ideal | Presets |
| --- | --- | --- |
| 44.1 → 48 | +1.32 dB | +1.33 to +1.36 dB |
| 44.1 → 88.2 | +2.00 dB | +1.88 to +2.02 dB |
| 44.1 → 96 | +2.01 dB | +1.89 to +2.02 dB |
| 48 → 44.1 | +0.89 dB | +0.88 to +0.90 dB |
| 48 → 96 | +1.02 dB | +1.02 dB |
| 96 → 48 | +0.56 dB | +0.56 to +0.59 dB |
| 96 → 44.1 | +0.46 dB | +0.47 to +0.51 dB |
| 192 → 48 | +0.36 dB | +0.31 to +0.33 dB |
| 192 → 44.1 | +0.13 dB | +0.14 to +0.24 dB |

- The presets stayed within 0.15 dB of the ideal. [Verified]
- At a volume of 1.0, Hiphile hands these 32-bit floating point samples to the device unchanged.
  [Read] Whether the device or CoreAudio clips them was not checked. [Unknown] Lowering the volume
  a little would leave them room. [Inferred]

### Ringing

How long the response to an impulse at 44.1 → 48 kHz stays above each level under its peak, on
each side. Every preset is linear-phase, so the ringing before the peak equals the ringing after.
[Verified]

| Preset | −60 dB | −100 dB | −120 dB | −144 dB |
| --- | --- | --- | --- | --- |
| Low | 1.2 ms | 2.0 ms | 2.2 ms | 2.4 ms |
| Medium | 2.0 ms | 3.9 ms | 4.3 ms | 4.9 ms |
| High | 2.5 ms | 5.3 ms | 6.1 ms | 6.9 ms |
| Max | 5.5 ms | 20.2 ms | 25.0 ms | 29.2 ms |

Low and High at the other pairs of rates measured (48 → 44.1, 44.1 → 88.2 and 96 → 44.1) came out
within 0.4 ms of these, and Max within 1.6 ms. The ringing is at the top of the pass-band.
[Inferred] Whether the longer ringing before transients can be heard was not checked.

### Delay in playback

The delay of each preset is under [Values and response](#values-and-response). It is not heard as
a later start. The converter removes it, so output starts at the song's first frame, and the
decoder runs far faster than real time. [Inferred] from reading the converter and the engine.

### Speed

At the High preset, the port converts about 100 million samples a second of one channel from 44.1 to
48 kHz on an Apple M5, within 15 % of the original either way (see
[Speed against the original](#speed-against-the-original)). The `r8brain` and `realfft` crates are
built with `opt-level = 3` in debug builds too.

Stereo, from real songs, on one thread, in blocks of 1024 frames. Best of three, as times real
time. [Verified]

| Rates (kHz) | Low | Medium | High | Max |
| --- | --- | --- | --- | --- |
| 44.1 → 48 | 1132 | 1049 | 1074 | 842 |
| 44.1 → 88.2 | 1809 | 1602 | 1675 | 1235 |
| 44.1 → 96 | 853 | 801 | 803 | 646 |
| 48 → 44.1 | 1062 | 975 | 996 | 762 |
| 48 → 96 | 1652 | 1481 | 1542 | 1136 |
| 96 → 48 | 1755 | 1558 | 1642 | 1189 |
| 96 → 44.1 | 859 | 798 | 822 | 629 |
| 192 → 48 | 1194 | 1104 | 1140 | 868 |
| 192 → 44.1 | 730 | 663 | 681 | 527 |

- Larger changes of rate take longer. On a 1 kHz sine wave, 768 → 44.1 kHz ran at 327 (Max) to 424
  (Low) times real time, and 44.1 → 768 kHz at 308 (Max) to 368 (High), which is about 0.3 % of one
  core. [Verified]
- Creating a resampler at a preset took at most 0.31 ms the first time in a process, and 0.002 to
  0.012 ms once its filters were cached. Other values took longer the first time: 4.5 to 8 ms for
  minimum phase at Max, and 28 ms for 65536 filter positions. [Verified]

### Fourfold upsampling

Measured on 2026-11-09, with the rate raised to four times the song's, as `sample_rate = "x4"`
asks for. Up by four, the chain is the block convolver doubling the rate and one half-band doubler.

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
| Low | −137.7 / −137.6 dB | −137.7 / −137.5 dB | −137.2 / −137.0 dB |
| Medium | −137.7 / −137.6 dB | −137.7 / −137.5 dB | −137.1 / −137.0 dB |
| High | −158.6 / −153.4 dB | −159.2 / −153.6 dB | −162.5 / −153.3 dB |
| Max | −158.6 / −153.4 dB | −159.4 / −153.6 dB | −162.4 / −153.4 dB |
| f32 rounding | −159.9 / −154.7 dB | −160.4 / −154.8 dB | −163.1 / −154.5 dB |

| Preset | Speed, times real time: 44.1 / 48 / 96 kHz | Delay: 44.1 / 48 / 96 kHz |
| --- | --- | --- |
| Low | 1,148 / 1,053 / 525 | 9.4 / 8.7 / 4.3 ms |
| Medium | 1,058 / 960 / 486 | 18.3 / 16.8 / 8.4 ms |
| High | 1,051 / 961 / 484 | 38.6 / 35.5 / 17.8 ms |
| Max | 776 / 712 / 377 | 147.3 / 135.3 / 67.7 ms |

- **High and Max reach the f32 rounding** in the band and above it, within 1.3 dB. [Verified]
- **Low and Medium give the same error and images**, about 137 dB under the music, which follows
  the attenuation of 136.45 dB they share. [Verified] [Inferred] for the cause.
- **High costs no more than Medium**, and Max takes 1.3 times High's time and about four times its
  delay for no difference that this measurement can see. [Verified]
- From 44.1 kHz, Low takes 2.3 dB from 20 kHz to the Nyquist frequency, Medium 0.9 dB, High 0.7 dB
  and Max 0.2 dB. [Verified]
- Against the others in the same measurement, soxr's VHQ comes out the same as High, and Rubato
  lies 27 to 48 dB above it. [Verified]

## What it comes to

- Up to 20 kHz, High and Max differ from an ideal resampler by less than the rounding of the
  32-bit output, and Low and Medium by far less than the quantization noise of a 16-bit
  recording. [Verified] None of that is likely to be heard. [Inferred]
- High, the default, reaches the limit of the 32-bit output within 10 % of Low's speed, and Max
  adds only delay and time to it. [Verified]
- Above 20 kHz, the presets differ in how much of 20 to 22 kHz stays and in how long the ringing
  there lasts. [Verified] Neither is likely to be heard. [Inferred]
- Delay and CPU load matter little in playback. [Inferred]

## Not checked

- Listening comparisons of any kind.
- Minimum phase, and ratios that do not step wholly, on real songs.
- Songs at 48 kHz and 16 bits, ALAC songs, and anything outside the songs measured.
- Whether samples over full scale clip on a real DAC.

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

That filter runs `…_on_r8brain`, `…_on_the_r8brain_presets` and `…_on_r8brain_with_other_options`,
and the same three for soxr.

> [!WARNING]
> Do not run the whole `--ignored` set, which can reach the default device.

- The expected files under `crates/r8brain/tests/upstream_reference/` come from the original. When
  porting a change from it, write them again with `generate.cpp` and check that the test still
  passes.
- A NaN can hide in a check that folds with `f64::max`, which ignores NaN. The test for the fewest
  filter positions looks at each sample instead.
