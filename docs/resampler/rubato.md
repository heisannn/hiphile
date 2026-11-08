# Rubato

Hiphile's default resampler is the FFT resampler of [Rubato](https://crates.io/crates/rubato), a
Rust crate. This document explains how Hiphile runs it, what its settings do and how it measured.
The other two resamplers are [r8brain](r8brain.md) and [soxr](soxr.md).

| | |
| --- | --- |
| Library | Rubato 5.0.1, its synchronous FFT resampler `Fft` |
| Arithmetic | 32-bit floating point |
| In Hiphile | 32-bit floating point in and out, for PCM. DSD output runs Rubato stages of its own (see [DSD output](../dsd/implementation.md)). |
| Default preset | Medium: FFT size 256 and the Blackman-Harris² window |

## Where the code lives

| Crate | Responsibility |
| --- | --- |
| `audio` | `FftConverter` in `src/convert.rs`, which runs one `Fft` over the interleaved channels. `Resampling::Fft` in `src/output.rs`, and `OutputSettings::from_config` in `src/from_config.rs`. |
| `config` | The `resampling`, `resampling_fft_size` and `resampling_window` keys and their ranges |
| `settings` | Quality, FFT size and Window under Resampling on the Audio page |

## When it runs

Resampler under Resampling on the Audio page chooses Rubato, r8brain or soxr. A converter exists only
for a song whose rate differs from the device's: with a fixed `sample_rate`, when the device lacks
the song's rate, or for a multiple such as `x2`. DSD output resamples with its own stages, so the
choice applies to PCM only.

## How it converts

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
  frequency, where L is the shorter block. So a larger FFT size moves the cutoff closer to the
  Nyquist frequency.
- **Precision.** Hiphile runs it in 32-bit floating point.
- **Delay.** The converter trims the ⌊M/2⌋ output frames of Rubato's `output_delay()` from the
  start. The filter's centre lies ⌊N/2⌋ · M/N output frames in, so when N or M is odd, the output
  stays off by half a frame of that block's rate (see [Timing](#timing)).
- **The end of a song.** `finish` converts what is left of the input as a partial block, then
  feeds silence until the output has `round(frames in × ratio)` frames, and drops any surplus.
  `undelivered_frames` tells the engine how much of the song is still inside the filter, which
  gapless joins and crossfades count on.
- **A sample that is not finite** becomes silence, as it would make the FFT panic.

### Blocks sized by the lower rate

Rubato sets its cutoff by the shorter of its two blocks, which is the output block when it lowers
the rate, but sizes the blocks by the input alone. [Read] in Rubato's source. Lowering by four from
an input block of 256 frames leaves an output block of 64. `FftConverter` therefore asks for blocks
whose side at the lower rate has at least the FFT size in frames, which gives a fall in rate the
blocks of the same rise.

Level of a sine wave at FFT size 256, fitted so that a delay does not change it. [Verified]

| Rates (kHz) | By the input: blocks in / out | 18 kHz | 19 kHz | 20 kHz | 21 kHz | By the lower rate: blocks in / out | 20 kHz | 21 kHz |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 96 → 48 | 256 / 128 | 0.00 dB | 0.00 dB | −0.04 dB | −2.01 dB | 512 / 256 | 0.00 dB | 0.00 dB |
| 96 → 44.1 | 320 / 147 | 0.00 dB | −0.10 dB | −5.63 dB | −36.0 dB | 640 / 294 | 0.00 dB | −5.11 dB |
| 192 → 48 | 256 / 64 | −2.03 dB | −6.39 dB | −14.9 dB | −29.1 dB | 1024 / 256 | 0.00 dB | 0.00 dB |
| 192 → 44.1 | 640 / 147 | 0.00 dB | −0.10 dB | −5.63 dB | −36.0 dB | 1280 / 294 | 0.00 dB | −5.11 dB |
| 44.1 → 48, for comparison | 294 / 320 | 0.00 dB | 0.00 dB | 0.00 dB | −5.11 dB | 294 / 320 | 0.00 dB | −5.11 dB |

- By the input, the songs' error up to 20 kHz was −48 dB at 192 → 48 kHz, −60 dB at 192 → 44.1 kHz
  and −75 dB at 96 → 44.1 kHz, as medians. By the lower rate, it is −139, −145 and −147 dB.
  [Verified]
- Every Rubato size, sized by the lower rate, stayed within 0.01 dB from 10 to 20 kHz at these
  pairs. [Verified]

### Timing

`FftConverter` trims `output_delay()`, which is ⌊M / 2⌋ output frames for an output block of M
frames. The filter's centre lies ⌊N / 2⌋ · M / N output frames in, for an input block of N. When N
or M is odd, the two differ, and the whole output stays that far from the input. [Read] in Rubato's
source, and [Verified] by sine waves at 1 kHz, whose error fell from −26 dB to between −126 and
−129 dB once the reference moved by it.

Output frames by which Rubato's output lags, where it is not zero. A negative lag is early. Each
comes from High's block of 1029 frames at 44.1 kHz.

| Rates (kHz) | Rubato High |
| --- | --- |
| 44.1 → 48 | −0.544 |
| 44.1 → 96 | −1.088 |
| 48 → 44.1 | +0.5 |
| 96 → 44.1 | +0.5 |
| 192 → 44.1 | +0.5 |

Every other pair and size measured was 0. Each lag comes to about 11 µs, and a constant offset
that small changes nothing that can be heard. [Inferred] It does move peaks between samples (see
[Samples over full scale](#samples-over-full-scale)), and measurements against the input have to
allow for it.

## Settings

Resampler under Resampling on the Audio page chooses Rubato, r8brain or soxr, and Quality under it
sets the quality of the one chosen. With Rubato chosen, Quality offers Medium, High, Max and Custom,
and FFT size and Window follow it. Changing either of them switches Quality to Custom, which keeps
the current values, and choosing Custom alone changes nothing. Window offers Hann, Hann², Blackman,
Blackman², Blackman-Harris and Blackman-Harris², and FFT size the powers of two from 256 to 16384.

The keys are listed under [Configuration](../../README.md#configuration) in the README.
`resampling` holds Rubato's quality, `medium`, `high`, `max` or `custom`, or the name of another
resampler. `resampling_fft_size` and `resampling_window` are read only with `resampling = "custom"`.
As the one key holds either, Rubato's quality is not kept in the file while another resampler is
chosen, and it comes back as Medium. `OutputSettings::from_config` turns the keys into
`Resampling::Fft`.

## Measurements

Three sets of measurements cover Rubato, each by its own temporary programs, which are not in the
repository:

| Date | With | Songs | Where |
| --- | --- | --- | --- |
| 2026-11-07 | r8brain | 33 songs, decoded by ffmpeg | This document, and [r8brain](r8brain.md#measurements) |
| 2026-11-08 | soxr and r8brain | 31 songs, decoded by ffmpeg | [soxr](soxr.md#measurements), which compares all three |
| 2026-11-09 | r8brain and soxr | 8 songs, decoded by Hiphile's decoder, raised fourfold | [Fourfold upsampling](#fourfold-upsampling) |

Unless a section says otherwise, the figures below are those of 2026-11-07. Rubato was measured at
Hiphile's three Quality presets and at 16384, the largest size that Custom offers, all with the
Blackman-Harris² window and with blocks sized by the lower rate.

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
| Rubato | 5.0.1, through a copy of Hiphile's `FftConverter` |

Nobody has listened to compare the settings. Every claim about what can be heard is [Inferred].
Nothing was run on any platform other than macOS.

### Summary

- **Up to 20 kHz**, Rubato's error against an ideal resampler measured −116 to −149 dB against
  full scale on real songs. Its High and Max lie 25 to 44 dB above r8brain's High and Max, which
  reach the rounding of the 32-bit output. [Verified] It is not likely to be heard. [Inferred]
- **A larger FFT is not cleaner up to 20 kHz.** Medium was the cleanest of the four sizes at eight
  of nine pairs of rates, and the fourfold measurement found the error growing from Medium to Max
  at every song rate, mostly under 500 Hz. [Verified] Why was not found. [Unknown]
- **Above 20 kHz**, a larger FFT keeps more of the band up to the Nyquist frequency, and rings
  longer. [Verified]
- **Rubato sizes its blocks by the input**, which leaves a large fall in rate a short output block
  and a low cutoff. Hiphile sizes them by the lower rate instead (see
  [Blocks sized by the lower rate](#blocks-sized-by-the-lower-rate)). [Verified]
- **The output is off in time** by about 11 µs at 5 of the 36 pairs of size and rates measured,
  all at High. [Verified]
- **Speed:** every size ran at least 340 times faster than real time in stereo at the songs' pairs
  of rates. [Verified]

### Response, aliases and images

Sine waves, with each tone's level fitted. The alias is the worst output of tones from 23.1 to
24 kHz at 48 → 44.1 kHz, and the image the worst tone at 44.1 kHz minus each tone from 20 to 22 kHz
at 44.1 → 48 kHz. [Verified]

| Setting | Level at 44.1 → 48 kHz | Worst alias | Worst image |
| --- | --- | --- | --- |
| Medium | 20.5 kHz −0.079 dB, 21 kHz −5.11 dB | −130.0 dB | −146.3 dB |
| High | 21.5 kHz 0.000 dB, 21.8 kHz −11.8 dB | −128.6 dB | −171.8 dB |
| Max | 21.8 kHz 0.000 dB, 22 kHz −22.5 dB | −127.3 dB | −170.5 dB |
| 16384 | 22 kHz 0.000 dB | −126.3 dB | −172.9 dB |

- Lowering from 96 and 192 kHz, tones from just above the target's Nyquist frequency to 1.4 times
  it came out at −129 to −135 dB, against −140 to −206 dB for r8brain. [Verified]
- The aliases stay near −130 dB at every size, which points to Rubato's 32-bit arithmetic rather
  than its filter. [Inferred]

### Real songs

- **Songs:** 33 of the 323 songs in Hiphile's cache: 10 at 44.1 kHz and 16 bits, 3 at 44.1 kHz and
  24 bits, 8 at 48 kHz, 8 at 96 kHz and 4 at 192 kHz, all FLAC and at least 2 minutes long. They
  were spread evenly over each group, sorted by file name.
- **Excerpt:** 60 seconds from 30 % into each song, with 1-second fades at both ends, decoded by
  ffmpeg.
- **Reference:** an ideal brickwall at the lower Nyquist frequency, made by one FFT of the whole
  excerpt, and moved by Rubato's offset in time, so that the error shows the filter and not the
  offset.
- **Converter:** the copy of `FftConverter`, in blocks of 1024 frames, in 32-bit floating point.
- **Error:** the output minus the reference, split into 0 to 20 kHz and 20 kHz to the lower Nyquist
  frequency, and measured over the middle 56 seconds.
- **Levels:** dB against a full-scale square wave, so a full-scale sine wave is −3 dB. Music up to
  20 kHz measured about −10 dB.
- **f32 rounding:** the reference rounded to 32-bit floating point, as the converter rounds its
  output.

#### Up to 20 kHz

Error against the ideal, as the median RMS over songs and channels, with the worst in brackets.
[Verified]

| Rates (kHz) | Songs | Medium | High | Max | 16384 | f32 rounding |
| --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | 13 | −135 (−132) | −134 (−130) | −122 (−119) | −130 (−126) | −163 |
| 44.1 → 88.2 | 13 | −139 (−136) | −130 (−128) | −124 (−122) | −116 (−113) | −167 |
| 44.1 → 96 | 13 | −135 (−132) | −134 (−130) | −122 (−119) | −130 (−126) | −166 |
| 48 → 44.1 | 8 | −145 (−142) | −133 (−131) | −131 (−129) | −128 (−126) | −162 |
| 48 → 96 | 8 | −144 (−141) | −131 (−128) | −125 (−122) | −116 (−114) | −168 |
| 96 → 48 | 8 | −134 (−133) | −137 (−136) | −119 (−117) | −128 (−126) | −163 |
| 96 → 44.1 | 8 | −147 (−146) | −137 (−136) | −133 (−131) | −117 (−116) | −163 |
| 192 → 48 | 4 | −139 (−138) | −120 (−119) | −132 (−131) | −128 (−126) | −163 |
| 192 → 44.1 | 4 | −145 (−144) | −138 (−136) | −121 (−120) | −119 (−117) | −163 |

- The largest single sample of error was −102 to −127 dB. [Verified]
- Rubato's error is 25 to 43 dB above r8brain's at High, 30 to 44 dB at Max, and 1 to 29 dB at
  Medium. [Verified] r8brain's High and Max lie under the f32 rounding.
- Of the four sizes, Medium is the cleanest at eight of the nine pairs of rates, and 16384 the least
  clean at five. The error does not follow the size steadily either: at 192 → 48 kHz, High is the
  least clean. [Verified] Why was not found. [Unknown]

#### Above 20 kHz

Each setting takes away part of what lies between 20 kHz and the lower Nyquist frequency. The table
gives the source's level in that band, and the level of what each setting takes away against the
source's level there, as medians. [Verified]

| Rates (kHz) | Source, median (loudest) | Medium | High | Max | 16384 |
| --- | --- | --- | --- | --- | --- |
| 44.1 → 48 or 96 | −54 (−46) dB | −8.4 dB | −16.8 dB | −27.5 dB | −39.2 dB |
| 44.1 → 88.2 | −54 (−46) dB | −7.3 dB | −16.8 dB | −27.4 dB | −39.2 dB |
| 48 → 44.1 | −61 (−45) dB | −7.1 dB | −16.7 dB | −24.2 dB | −31.9 dB |
| 48 → 96 | −61 (−45) dB | −32.2 dB | −45.6 dB | −56.6 dB | −62.6 dB |
| 96 → 48 | −58 (−54) dB | −8.0 dB | −15.8 dB | −23.4 dB | −29.9 dB |
| 96 → 44.1 | −60 (−55) dB | −5.1 dB | −11.0 dB | −17.7 dB | −23.8 dB |
| 192 → 48 | −48 (−43) dB | −22.9 dB | −29.2 dB | −35.5 dB | −41.2 dB |
| 192 → 44.1 | −48 (−44) dB | −9.3 dB | −26.9 dB | −33.0 dB | −38.6 dB |

- Music above 20 kHz lay about 40 to 50 dB under the music as a whole in these songs. [Verified]
- 16384 keeps the most of any setting of any resampler, flat to 22 kHz at 44.1 → 48. Medium takes
  away about as much as r8brain's Low. [Verified]
- Sound above 20 kHz at these levels is unlikely to be heard by adults. [Inferred]

#### Samples over full scale

Every group had songs that peaked at 0 dBFS. Resampled, many of them went over full scale with
every setting. The ideal brickwall did the same. These are peaks between samples in the
recordings, not overshoot of the filter. [Verified]

| Rates (kHz) | Highest peak, ideal | Medium | High | Max | 16384 |
| --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | +1.32 dB | +1.36 dB | **+1.98 dB** | +1.33 dB | +1.32 dB |
| 44.1 → 88.2 | +2.00 dB | +1.86 dB | +2.02 dB | +2.00 dB | +2.00 dB |
| 44.1 → 96 | +2.01 dB | +1.89 dB | +1.98 dB | +2.01 dB | +2.01 dB |
| 48 → 44.1 | +0.89 dB | +0.90 dB | **+0.80 dB** | +0.88 dB | +0.89 dB |
| 48 → 96 | +1.02 dB | +1.02 dB | +1.02 dB | +1.02 dB | +1.02 dB |
| 96 → 48 | +0.56 dB | +0.60 dB | +0.56 dB | +0.56 dB | +0.56 dB |
| 96 → 44.1 | +0.46 dB | +0.51 dB | **+0.63 dB** | +0.46 dB | +0.46 dB |
| 192 → 48 | +0.36 dB | +0.35 dB | +0.33 dB | +0.34 dB | +0.35 dB |
| 192 → 44.1 | +0.13 dB | +0.25 dB | **+0.96 dB** | +0.19 dB | +0.14 dB |

- The figures in bold are where the output is off in time and the peak differs from the ideal's by
  more than 0.05 dB. Shifting the samples by half a frame catches the waveform between other
  samples, which peak higher or lower. [Inferred]
- At a volume of 1.0, Hiphile hands these 32-bit floating point samples to the device unchanged.
  [Read] Whether the device or CoreAudio clips them was not checked. [Unknown]

### Ringing

How long the response to an impulse at 44.1 → 48 kHz stays above each level under its peak, on
each side. Every size is linear-phase, so the ringing before the peak equals the ringing after.
[Verified]

| Setting | −60 dB | −100 dB | −120 dB | −144 dB |
| --- | --- | --- | --- | --- |
| Medium | 1.3 ms | 2.2 ms | 2.5 ms | 2.8 ms |
| High | 3.4 ms | 7.2 ms | 8.4 ms | 9.6 ms |
| Max | 5.8 ms | 24.4 ms | 30.3 ms | 35.7 ms |
| 16384 | 7.0 ms | 79.9 ms | 108 ms | 133 ms |

The ringing is at the top of the pass-band. [Inferred] The measurement of 2026-11-08 found High
ringing 23.3 ms after the peak at −144 dB but 9.5 ms before. Why was not found. [Unknown]

### Delay in playback

Input the converter takes before its first output, in ms. [Verified]

| Setting | 44.1 → 48 | 48 → 44.1 | 96 → 44.1 |
| --- | --- | --- | --- |
| Medium | 6.7 | 6.7 | 6.7 |
| High | 23.3 | 23.3 | 23.3 |
| Max | 93.3 | 93.3 | 93.3 |
| 16384 | 373.3 | 373.3 | 373.3 |

This is not heard as a later start. The converter removes its delay, so output starts at the song's
first frame, and the decoder runs far faster than real time. At 16384 the decoder has to read about
0.37 s more of the song before the first output. [Inferred] from reading the converter and the
engine.

Rubato takes one block, which is about the FFT size in frames of the lower rate. So it grows as that
rate falls, to about 1 s at 16384 from 44.1 kHz to a device at 16 kHz. [Read]

### Speed

Stereo, from real songs, on one thread, in blocks of 1024 frames. Best of three, as times real
time. [Verified]

| Rates (kHz) | Medium | High | Max | 16384 |
| --- | --- | --- | --- | --- |
| 44.1 → 48 | 1409 | 1175 | 1150 | 942 |
| 44.1 → 88.2 | 1541 | 1391 | 1302 | 1108 |
| 44.1 → 96 | 982 | 806 | 757 | 633 |
| 48 → 44.1 | 1392 | 1151 | 1133 | 942 |
| 48 → 96 | 1407 | 1284 | 1162 | 1039 |
| 96 → 48 | 1377 | 1241 | 1157 | 1002 |
| 96 → 44.1 | 975 | 790 | 745 | 625 |
| 192 → 48 | 791 | 706 | 641 | 551 |
| 192 → 44.1 | 575 | 475 | 428 | 347 |

- The slowest case here, 16384 at 192 → 44.1, still takes under 0.3 % of one core. [Verified]
- Larger changes of rate take longer. On a 1 kHz sine wave, 768 → 44.1 kHz ran at 99 (16384) to 160
  (Medium) times real time, as the blocks grow with a fall in rate, and 44.1 → 768 kHz at 99 to
  167. [Verified]
- Rubato keeps no cache. The converter took 0.02 to 0.11 ms to create each time at Medium and High,
  0.13 to 0.51 ms at Max and 0.44 to 1.8 ms at 16384 at the pairs above, and up to 6.9 ms at
  768 → 44.1 kHz. [Verified]

### Fourfold upsampling

Measured on 2026-11-09, with the rate raised to four times the song's, as `sample_rate = "x4"`
asks for.

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

| Setting | 44.1 → 176.4 kHz: error / images | 48 → 192 kHz | 96 → 384 kHz |
| --- | --- | --- | --- |
| Medium | −127.1 / −140.9 dB | −133.8 / −141.0 dB | −135.9 / −141.3 dB |
| High | −120.8 / −139.8 dB | −121.0 / −139.9 dB | −121.0 / −140.0 dB |
| Max | −114.5 / −139.1 dB | −114.3 / −139.2 dB | −114.1 / −139.2 dB |
| f32 rounding | −159.9 / −154.7 dB | −160.4 / −154.8 dB | −163.1 / −154.5 dB |

| Setting | Speed, times real time: 44.1 / 48 / 96 kHz | Delay: 44.1 / 48 / 96 kHz |
| --- | --- | --- |
| Medium | 1,113 / 990 / 508 | 5.8 / 5.3 / 2.7 ms |
| High | 962 / 914 / 456 | 23.2 / 21.3 / 10.7 ms |
| Max | 865 / 783 / 392 | 92.9 / 85.3 / 42.7 ms |

- **The error grows with the FFT size** at every rate: High's lies 6 to 15 dB above Medium's, and
  Max's about 7 dB above High's. [Verified]
- **High's and Max's error lies mostly under 500 Hz.** From 0 to 500 Hz alone it was −122 dB for
  High and −114 dB for Max at every rate, and no more than −130 dB in any other 500 Hz band up to
  20 kHz. From 44.1 kHz, Medium's lies mostly at 18 to 20 kHz, where its cutoff begins. [Verified]
  Why the error gathers at the lowest frequencies was not found. [Unknown]
- **The images** stay near −140 dB at every size, 13 to 16 dB above the f32 rounding. [Verified]
- From 44.1 kHz, Medium takes 3.1 dB from 20 kHz to the Nyquist frequency, High 0.6 dB and Max
  0.1 dB. [Verified]
- Against the others in the same measurement, r8brain's High and Max and soxr's VHQ lie at the f32
  rounding, 27 to 48 dB under Rubato. Medium is about as fast as r8brain's Low to High, and soxr's
  LQ to HQ are faster. [Verified]

## What it comes to

- Up to 20 kHz, Rubato's error measured −116 to −149 dB against full scale on real songs, and −114
  to −136 dB against the music when raising the rate fourfold. [Verified] None of it is likely to
  be heard. [Inferred]
- Medium is the cleanest size up to 20 kHz at nearly every pair measured, the fastest and the
  shortest in delay. Larger sizes keep more of 20 to 22 kHz, which is unlikely to be heard, and ring
  longer. [Verified]
- With its blocks sized by the input, Medium lowered the treble near 20 kHz when lowering the rate
  from 96 or 192 kHz, the one difference found within the audible band. Sizing them by the lower
  rate removes it. [Verified]
- Delay and CPU load matter little in playback. [Inferred]

## Not checked

- Listening comparisons of any kind.
- Windows other than Blackman-Harris², and sizes between those measured.
- Songs at 48 kHz and 16 bits, ALAC songs, and anything outside the songs measured.
- Whether samples over full scale clip on a real DAC.

## Testing

`cargo test -p audio` checks `FftConverter` with synthetic signals in `src/convert.rs`: the length
and level at every ratio, the FFT size and window reaching Rubato, the treble kept at a large fall in
rate, a reset, samples that are not numbers and a short input. Gapless joins on a loopback device are
tested by name:

```sh
HIPHILE_TEST_DEVICE="BlackHole 2ch" cargo test -p audio --lib -- \
  --ignored --test-threads=1 resampled_tracks_join_without_a_gap_at_the_largest_fft
```

> [!WARNING]
> Do not run the whole `--ignored` set, which can reach the default device.

- Comparing Rubato's output with its input needs its offset in time, or a pure delay reads as an
  error of −26 dB at 1 kHz. Measuring a level by the highest sample is thrown off by it too; fit
  the tone instead.
