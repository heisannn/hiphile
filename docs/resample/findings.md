# Findings on resampling

What was established while porting r8brain-free-src, choosing its presets and comparing it with
Rubato, as of 2026-11-07. For how both resamplers work, see [implementation.md](implementation.md).

Each claim is tagged by how far it was checked.

| Tag | Meaning |
| --- | --- |
| [Verified] | Checked by running or measuring it |
| [Read] | Checked by reading code, without running it |
| [Inferred] | Reasoned from observations, without checking directly |
| [Unknown] | The cause or behavior is not understood |

## Environment and scope

| Item | Value |
| --- | --- |
| Mac | Apple M5, 24 GB of memory, macOS 26.2 |
| Rust | 1.99.1, release builds |
| Original | r8brain-free-src 7.7 (commit `9ca64da`), built with clang 21.1.8 and its default Ooura FFT |
| Rubato | 5.0.1, through a copy of Hiphile's `FftConverter` |
| Songs | 33 songs from Hiphile's local cache, decoded with ffmpeg |

Rubato was measured at Hiphile's three Quality presets, Medium (FFT size 256), High (1024) and Max
(4096), and at 16384, the largest that Custom offers. All use the Blackman-Harris² window and
Hiphile's sizing of the blocks by the lower rate, unless a figure says otherwise (see
[Rubato when lowering the rate](#rubato-when-lowering-the-rate)).

Nobody has listened to compare the settings. Every claim about what can be heard is [Inferred].
Nothing was run on any platform other than macOS.

## Summary

- **Up to 20 kHz:** every setting stays more than 100 dB under the music. As delivered in 32-bit
  floating point, r8brain's error is 25 to 44 dB lower than Rubato's at High and Max. [Verified]
  Neither is likely to be heard. [Inferred]
- **Rubato sizes its blocks by the input**, which leaves a large fall in rate a short output block
  and a low cutoff. At FFT size 256, 192 to 48 kHz lost 2 dB at 18 kHz and 15 dB at 20 kHz. Hiphile
  sizes the blocks by the lower rate instead, and every setting then stays within 0.01 dB up to
  20 kHz at every pair measured with sine waves. [Verified]
- **Rubato's output is off in time** by about 11 µs at 5 of the 36 pairs of size and rates
  measured, all at High. [Verified]
- **Above 20 kHz** the settings differ by design, in how much of 20 to 22 kHz they keep and in how
  long they ring there. [Verified]
- **Speed** is alike: every setting runs at least 340 times faster than real time in stereo at the
  songs' pairs of rates, and about 100 times at the worst measured, Rubato's 16384 between 44.1 and
  768 kHz. [Verified]

## Agreement with the original

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

### Speed against the original

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

## r8brain presets

### Values and response

Measured with sine waves on the port, at 64-bit floating point. Delay is the input a resampler takes
before its first output. [Verified]

| Preset | Transition band, attenuation | Delay 44.1 → 48 / 48 → 44.1 kHz | Level at 44.1 → 48 kHz | Worst alias at 48 → 44.1 kHz |
| --- | --- | --- | --- | --- |
| Low | 5 %, 136.45 dB | 9.3 / 8.3 ms | 20 kHz 0.000 dB, 20.5 kHz −0.068 dB, 21 kHz −3.99 dB | −151.2 dB |
| Medium | 2.4 %, 136.45 dB | 18.2 / 16.3 ms | 21 kHz 0.000 dB, 21.5 kHz −2.36 dB | −156.0 dB |
| High | 2 %, 180.15 dB | 38.5 / 34.8 ms | 21.5 kHz −0.32 dB, 21.8 kHz −25.6 dB | −198.9 dB |
| Max | 0.5 %, 218 dB | 147.2 / 132.1 ms | 21.8 kHz 0.000 dB, 22 kHz −43.8 dB | −200.0 dB |

The alias is the worst output of tones from 23.1 to 24 kHz, in 30 Hz steps.

Delay at every pair of rates measured, in ms. [Verified]

| Rates (kHz) | Low | Medium | High | Max |
| --- | --- | --- | --- | --- |
| 44.1 → 48, 44.1 → 96 | 9.3 | 18.2 | 38.5 | 147.2 |
| 44.1 → 88.2 | 9.2 | 18.1 | 38.4 | 147.0 |
| 48 → 44.1, 96 → 44.1 | 8.3 | 16.3 | 34.8 | 132.1 |
| 48 → 96, 96 → 48 | 8.4 | 16.6 | 35.3 | 135.1 |
| 192 → 48 | 8.5 | 16.7 | 35.4 | 135.2 |
| 192 → 44.1 | 8.4 | 16.4 | 34.9 | 132.2 |

### Why these values

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

## Real songs

### Method

- **Songs:** 33 of the 323 songs in Hiphile's cache: 10 at 44.1 kHz and 16 bits, 3 at 44.1 kHz and
  24 bits, 8 at 48 kHz, 8 at 96 kHz and 4 at 192 kHz, all FLAC and at least 2 minutes long. They
  were spread evenly over each group, sorted by file name. The 2 songs at 48 kHz and 16 bits and
  the 2 ALAC songs were left out.
- **Excerpt:** 60 seconds from 30 % into each song, with 1-second fades at both ends, decoded to
  64-bit floating point by ffmpeg.
- **Reference:** an ideal brickwall at the lower Nyquist frequency, made by one FFT of the whole
  excerpt.
- **r8brain:** `Resampler::oneshot` in blocks of 1024 samples.
- **Rubato:** the copy of `FftConverter`, in blocks of 1024 frames, in 32-bit floating point. Its
  reference was moved by Rubato's offset in time (see [Timing](#timing)), so that the error shows
  the filter and not the offset.
- **Error:** the output minus the reference, split into 0 to 20 kHz and 20 kHz to the lower Nyquist
  frequency, and measured over the middle 56 seconds.
- **Levels:** dB against a full-scale square wave, so a full-scale sine wave is −3 dB.
- **f32 rounding:** the reference rounded to 32-bit floating point, as both converters round their
  output.

The programs used for these measurements were temporary and are not in the repository.

### Up to 20 kHz

Error against the ideal, as the median RMS over songs and channels, with the worst in brackets.
Music in this band measured about −10 dB. [Verified]

r8brain at 64-bit floating point, before the converter rounds it:

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

| Rates (kHz) | r8brain Low | r8brain Medium | r8brain High | r8brain Max | Rubato Medium | Rubato High | Rubato Max | Rubato 16384 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | −151 (−144) | −151 (−144) | −163 (−160) | −163 (−160) | −135 (−132) | −134 (−130) | −122 (−119) | −130 (−126) |
| 44.1 → 88.2 | −165 (−163) | −165 (−163) | −165 (−163) | −165 (−163) | −139 (−136) | −130 (−128) | −124 (−122) | −116 (−113) |
| 44.1 → 96 | −151 (−144) | −151 (−144) | −166 (−163) | −166 (−163) | −135 (−132) | −134 (−130) | −122 (−119) | −130 (−126) |
| 48 → 44.1 | −153 (−147) | −153 (−147) | −162 (−160) | −162 (−160) | −145 (−142) | −133 (−131) | −131 (−129) | −128 (−126) |
| 48 → 96 | −166 (−164) | −166 (−164) | −166 (−164) | −167 (−166) | −144 (−141) | −131 (−128) | −125 (−122) | −116 (−114) |
| 96 → 48 | −163 (−162) | −163 (−162) | −163 (−162) | −163 (−162) | −134 (−133) | −137 (−136) | −119 (−117) | −128 (−126) |
| 96 → 44.1 | −152 (−149) | −152 (−149) | −163 (−162) | −163 (−162) | −147 (−146) | −137 (−136) | −133 (−131) | −117 (−116) |
| 192 → 48 | −148 (−147) | −148 (−147) | −163 (−162) | −163 (−162) | −139 (−138) | −120 (−119) | −132 (−131) | −128 (−126) |
| 192 → 44.1 | −146 (−145) | −146 (−145) | −163 (−162) | −163 (−162) | −145 (−144) | −138 (−136) | −121 (−120) | −119 (−117) |

The largest single sample of error was −125 dB for r8brain's Low and Medium, −172 dB for High and
−206 dB for Max. For Rubato it was −102 to −127 dB. [Verified]

- r8brain's High and Max stay below the rounding of the 32-bit output on every route, so what they
  deliver is the ideal plus that rounding. [Verified]
- r8brain's Low and Medium lie 9 to 17 dB above that rounding where more than one step runs. That is
  43 dB or more under the quantization noise of 16 bits (−101 dB), and near that of 24 bits
  (−149 dB). [Verified] They give the same error there, as the error follows the attenuation they
  share. [Verified]
- Where one block convolver does all the work (44.1 → 88.2, 48 → 96 and 96 → 48), every r8brain
  preset comes out 37 to 67 dB cleaner than at 44.1 → 48. [Verified]
- Rubato's error is 25 to 43 dB above r8brain's at High, 30 to 44 dB at Max, and 1 to 29 dB at
  Medium. [Verified]
- A larger FFT is not cleaner up to 20 kHz: of Rubato's four sizes, Medium is the cleanest at eight
  of the nine pairs of rates, and 16384 the least clean at five. [Verified] The error does not
  follow the size steadily either: at 192 → 48 kHz, High is the least clean. Why was not found.
  [Unknown]

### Rubato when lowering the rate

Rubato sets its cutoff by the shorter of its two blocks, which is the output block when it lowers
the rate, but sizes the blocks by the input alone. [Read] in Rubato's source. Lowering by four from
an input block of 256 frames leaves an output block of 64. Hiphile's `FftConverter` therefore asks
for blocks whose side at the lower rate has at least the FFT size in frames, which gives a fall in
rate the blocks of the same rise.

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
- Every r8brain preset, and every Rubato size by the lower rate, stayed within 0.01 dB from 10 to
  20 kHz at these pairs. [Verified]

### Above 20 kHz

Here the settings differ by design: each takes away part of what lies between 20 kHz and the lower
Nyquist frequency. The table gives the source's level in that band, and the level of what each
setting takes away against the source's level there, as medians. [Verified]

| Rates (kHz) | Source, median (loudest) | r8brain Low | r8brain Medium | r8brain High | r8brain Max | Rubato Medium | Rubato High | Rubato Max | Rubato 16384 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 or 96 | −54 (−46) dB | −8.9 dB | −13.7 dB | −14.6 dB | −25.5 dB | −8.4 dB | −16.8 dB | −27.5 dB | −39.2 dB |
| 44.1 → 88.2 | −54 (−46) dB | −8.9 dB | −13.7 dB | −14.6 dB | −25.5 dB | −7.3 dB | −16.8 dB | −27.4 dB | −39.2 dB |
| 48 → 44.1 | −61 (−45) dB | −7.6 dB | −13.7 dB | −14.8 dB | −22.6 dB | −7.1 dB | −16.7 dB | −24.2 dB | −31.9 dB |
| 48 → 96 | −61 (−45) dB | −33.7 dB | −39.0 dB | −41.8 dB | −55.3 dB | −32.2 dB | −45.6 dB | −56.6 dB | −62.6 dB |
| 96 → 48 | −58 (−54) dB | −9.1 dB | −13.0 dB | −13.9 dB | −21.6 dB | −8.0 dB | −15.8 dB | −23.4 dB | −29.9 dB |
| 96 → 44.1 | −60 (−55) dB | −5.5 dB | −8.9 dB | −9.5 dB | −16.2 dB | −5.1 dB | −11.0 dB | −17.7 dB | −23.8 dB |
| 192 → 48 | −48 (−43) dB | −23.9 dB | −27.2 dB | −27.9 dB | −34.1 dB | −22.9 dB | −29.2 dB | −35.5 dB | −41.2 dB |
| 192 → 44.1 | −48 (−44) dB | −10.0 dB | −22.5 dB | −25.4 dB | −31.7 dB | −9.3 dB | −26.9 dB | −33.0 dB | −38.6 dB |

- Music above 20 kHz lay about 40 to 50 dB under the music as a whole in these songs. [Verified]
- r8brain's Low takes away the most of its presets, as its response starts to fall at about
  20.5 kHz. Max keeps nearly all of it up to 21.8 kHz. [Verified]
- Rubato's 16384 keeps the most of any setting, flat to 22 kHz at 44.1 → 48. [Verified]
- Sound above 20 kHz at these levels is unlikely to be heard by adults. [Inferred]

### Response, aliases and images

Sine waves, with each tone's level fitted. [Verified]

| Setting | Level at 44.1 → 48 kHz | Worst alias, 48 → 44.1 kHz | Worst image, 44.1 → 48 kHz |
| --- | --- | --- | --- |
| r8brain Low | 20.5 kHz −0.068 dB, 21 kHz −3.99 dB | −151.2 dB | −147.5 dB |
| r8brain Medium | 21 kHz 0.000 dB, 21.5 kHz −2.36 dB | −156.0 dB | −140.6 dB |
| r8brain High | 21.5 kHz −0.32 dB, 21.8 kHz −25.6 dB | −198.9 dB | −186.1 dB |
| r8brain Max | 21.8 kHz 0.000 dB, 22 kHz −43.8 dB | −200.0 dB | −243.2 dB |
| Rubato Medium | 20.5 kHz −0.079 dB, 21 kHz −5.11 dB | −130.0 dB | −146.3 dB |
| Rubato High | 21.5 kHz 0.000 dB, 21.8 kHz −11.8 dB | −128.6 dB | −171.8 dB |
| Rubato Max | 21.8 kHz 0.000 dB, 22 kHz −22.5 dB | −127.3 dB | −170.5 dB |
| Rubato 16384 | 22 kHz 0.000 dB | −126.3 dB | −172.9 dB |

- The alias is the worst output of tones from 23.1 to 24 kHz, and the image the worst tone at 44.1
  kHz minus each tone from 20 to 22 kHz.
- Lowering from 96 and 192 kHz, tones from just above the target's Nyquist frequency to 1.4 times
  it came out at −140 to −206 dB for r8brain and −129 to −135 dB for Rubato. [Verified]
- Rubato's aliases stay near −130 dB at every size, which points to its 32-bit arithmetic rather
  than its filter. [Inferred]

### Samples over full scale

Every group had songs that peaked at 0 dBFS. Resampled, many of them went over full scale with
every setting. The ideal brickwall did the same. These are peaks between samples in the
recordings, not overshoot of the filters. [Verified]

| Rates (kHz) | Highest peak, ideal | r8brain presets | Rubato Medium | Rubato High | Rubato Max | Rubato 16384 |
| --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | +1.32 dB | +1.33 to +1.36 dB | +1.36 dB | **+1.98 dB** | +1.33 dB | +1.32 dB |
| 44.1 → 88.2 | +2.00 dB | +1.88 to +2.02 dB | +1.86 dB | +2.02 dB | +2.00 dB | +2.00 dB |
| 44.1 → 96 | +2.01 dB | +1.89 to +2.02 dB | +1.89 dB | +1.98 dB | +2.01 dB | +2.01 dB |
| 48 → 44.1 | +0.89 dB | +0.88 to +0.90 dB | +0.90 dB | **+0.80 dB** | +0.88 dB | +0.89 dB |
| 48 → 96 | +1.02 dB | +1.02 dB | +1.02 dB | +1.02 dB | +1.02 dB | +1.02 dB |
| 96 → 48 | +0.56 dB | +0.56 to +0.59 dB | +0.60 dB | +0.56 dB | +0.56 dB | +0.56 dB |
| 96 → 44.1 | +0.46 dB | +0.47 to +0.51 dB | +0.51 dB | **+0.63 dB** | +0.46 dB | +0.46 dB |
| 192 → 48 | +0.36 dB | +0.31 to +0.33 dB | +0.35 dB | +0.33 dB | +0.34 dB | +0.35 dB |
| 192 → 44.1 | +0.13 dB | +0.14 to +0.24 dB | +0.25 dB | **+0.96 dB** | +0.19 dB | +0.14 dB |

- r8brain's presets stayed within 0.15 dB of the ideal. [Verified]
- The Rubato figures in bold are where its output is off in time and the peak differs from the
  ideal's by more than 0.05 dB. Shifting the samples by half a frame catches the waveform between
  other samples, which peak higher or lower. [Inferred]
- At a volume of 1.0, Hiphile hands these 32-bit floating point samples to the device unchanged.
  [Read] Whether the device or CoreAudio clips them was not checked. [Unknown] Lowering the volume
  a little would leave them room. [Inferred]

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
that small changes nothing that can be heard. [Inferred] It does move peaks between samples, as
above, and measurements against the input have to allow for it.

### Ringing

How long the response to an impulse at 44.1 → 48 kHz stays above each level under its peak, on
each side. Every setting is linear-phase, so the ringing before the peak equals the ringing after.
[Verified]

| Setting | −60 dB | −100 dB | −120 dB | −144 dB |
| --- | --- | --- | --- | --- |
| r8brain Low | 1.2 ms | 2.0 ms | 2.2 ms | 2.4 ms |
| r8brain Medium | 2.0 ms | 3.9 ms | 4.3 ms | 4.9 ms |
| r8brain High | 2.5 ms | 5.3 ms | 6.1 ms | 6.9 ms |
| r8brain Max | 5.5 ms | 20.2 ms | 25.0 ms | 29.2 ms |
| Rubato Medium | 1.3 ms | 2.2 ms | 2.5 ms | 2.8 ms |
| Rubato High | 3.4 ms | 7.2 ms | 8.4 ms | 9.6 ms |
| Rubato Max | 5.8 ms | 24.4 ms | 30.3 ms | 35.7 ms |
| Rubato 16384 | 7.0 ms | 79.9 ms | 108 ms | 133 ms |

r8brain's Low and High at the other pairs of rates measured (48 → 44.1, 44.1 → 88.2 and 96 → 44.1)
came out within 0.4 ms of these, and Max within 1.6 ms. The ringing is at the top of the
pass-band. [Inferred] Whether the longer ringing before transients can be heard was not checked.

### Delay in playback

Input each converter takes before its first output, in ms. [Verified]

| Setting | 44.1 → 48 | 48 → 44.1 | 96 → 44.1 |
| --- | --- | --- | --- |
| r8brain Low | 9.3 | 8.3 | 8.3 |
| r8brain Medium | 18.2 | 16.3 | 16.3 |
| r8brain High | 38.5 | 34.8 | 34.8 |
| r8brain Max | 147.2 | 132.1 | 132.1 |
| Rubato Medium | 6.7 | 6.7 | 6.7 |
| Rubato High | 23.3 | 23.3 | 23.3 |
| Rubato Max | 93.3 | 93.3 | 93.3 |
| Rubato 16384 | 373.3 | 373.3 | 373.3 |

This is not heard as a later start. Both converters remove their delay, so output starts at the
song's first frame, and the decoder runs far faster than real time. At these rates, Rubato's 16384
takes the most, about 0.37 s more of the song to decode before the first output. [Inferred] from
reading the converters and the engine.

Rubato takes one block, which is about the FFT size in frames of the lower rate. So it grows as that
rate falls, to about 1 s at 16384 from 44.1 kHz to a device at 16 kHz. [Read]

### Speed

Stereo, from real songs, on one thread, in blocks of 1024 frames. Best of three, as times real
time. [Verified]

| Rates (kHz) | r8brain Low | r8brain Medium | r8brain High | r8brain Max | Rubato Medium | Rubato High | Rubato Max | Rubato 16384 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 44.1 → 48 | 1132 | 1049 | 1074 | 842 | 1409 | 1175 | 1150 | 942 |
| 44.1 → 88.2 | 1809 | 1602 | 1675 | 1235 | 1541 | 1391 | 1302 | 1108 |
| 44.1 → 96 | 853 | 801 | 803 | 646 | 982 | 806 | 757 | 633 |
| 48 → 44.1 | 1062 | 975 | 996 | 762 | 1392 | 1151 | 1133 | 942 |
| 48 → 96 | 1652 | 1481 | 1542 | 1136 | 1407 | 1284 | 1162 | 1039 |
| 96 → 48 | 1755 | 1558 | 1642 | 1189 | 1377 | 1241 | 1157 | 1002 |
| 96 → 44.1 | 859 | 798 | 822 | 629 | 975 | 790 | 745 | 625 |
| 192 → 48 | 1194 | 1104 | 1140 | 868 | 791 | 706 | 641 | 551 |
| 192 → 44.1 | 730 | 663 | 681 | 527 | 575 | 475 | 428 | 347 |

- The slowest case here, Rubato 16384 at 192 → 44.1, still takes under 0.3 % of one core.
  [Verified]
- Larger changes of rate take longer. On a 1 kHz sine wave, 768 → 44.1 kHz ran at 99 (16384) to
  160 (Medium) times real time on Rubato, whose blocks grow with a fall in rate, and 327 (Max) to
  424 (Low) on r8brain. 44.1 → 768 kHz ran at 99 to 167 on Rubato and 308 (Max) to 368 (High) on
  r8brain, which is about 0.3 % of one core. [Verified]
- Creating an r8brain resampler at a preset took at most 0.31 ms the first time in a process, and
  0.002 to 0.012 ms once its filters were cached. Other values took longer the first time: 4.5 to
  8 ms for minimum phase at Max, and 28 ms for 65536 filter positions. [Verified]
- Rubato keeps no cache. Its converter took 0.02 to 0.11 ms each time at Medium and High, 0.13 to
  0.51 ms at Max and 0.44 to 1.8 ms at 16384 at the pairs above, and up to 6.9 ms at
  768 → 44.1 kHz. [Verified]

## What it comes to

- Up to 20 kHz, r8brain's High and Max differ from an ideal resampler by less than the rounding of
  the 32-bit output, and Low and Medium by far less than the quantization noise of a 16-bit
  recording. Rubato's error measured −116 to −147 dB. [Verified] None of that is likely to be
  heard. [Inferred]
- With its blocks sized by the input, Rubato's Medium lowered the treble near 20 kHz when lowering
  the rate from 96 or 192 kHz, the one difference found within the audible band. Sizing them by
  the lower rate removes it. [Verified]
- Above 20 kHz, the settings differ in how much of 20 to 22 kHz stays and in how long the ringing
  there lasts. [Verified] Neither is likely to be heard. [Inferred]
- Delay and CPU load matter little in playback. [Inferred]

## Not checked

- Listening comparisons of any kind.
- Rubato's windows other than Blackman-Harris², and its sizes between those measured.
- Minimum phase, and ratios that do not step wholly, on real songs.
- Songs at 48 kHz and 16 bits, ALAC songs, and anything outside the 33 songs.
- Whether samples over full scale clip on a real DAC.

## Testing notes

- Run device tests with `HIPHILE_TEST_DEVICE="BlackHole 2ch"`, and narrow `--ignored` by name. The
  default device can be a real DAC.
- The expected files under `crates/r8brain/tests/upstream_reference/` come from the original. When
  porting a change from it, write them again with `generate.cpp` and check that the test still
  passes.
- A NaN can hide in a check that folds with `f64::max`, which ignores NaN. The test for the fewest
  filter positions looks at each sample instead.
- Comparing Rubato's output with its input needs its offset in time, or a pure delay reads as an
  error of −26 dB at 1 kHz. Measuring a level by the highest sample is thrown off by it too; fit
  the tone instead.
