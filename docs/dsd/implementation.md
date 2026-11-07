# DSD output in Hiphile

Hiphile can play any song as DSD. It converts PCM to a 1-bit stream in real time and sends that
stream to the DAC over DSD over PCM (DoP). This document explains how that works and why it was
built this way. Measurements and open problems are in [findings.md](findings.md).

| | |
| --- | --- |
| Input | Whatever the decoder produces, as PCM. DSF and DFF files are not read. |
| Output | DoP at DSD64, DSD128 or DSD256 |
| Platform | macOS only |
| Volume | Fixed. It is set on the DAC. |
| Modulator | Hiphile's own, with a parametric noise transfer function |
| Oversampling | [Rubato](https://crates.io/crates/rubato) |

## Where the code lives

| Crate | Responsibility |
| --- | --- |
| `dsd` | PCM to DSD words: oversampling, level stages and the modulator. No I/O and no GPUI. |
| `audio` | Device control, the audio callback that adds DoP markers, and stream lifecycle |
| `config` | The saved settings and the Quality presets |
| `settings` | The DSD part of the Settings window |
| `player` | How sound starts after DSD turns on, the volume lock and error messages |
| `hiphile-cli` | Plays through the same `OutputSettings`, so it follows the saved DSD settings |

## Signal path

```text
decoder (f32 PCM, any rate)
  │
  ▼  Converter (audio)
channel map to stereo
  │
  ▼  Encoder (dsd); everything after the oversampler runs at the DSD bit rate
oversampler ──▶ input level ──▶ fade ──▶ limiter ──▶ modulator L/R
  │
  ▼  16-bit words, older bits higher, stored as f32 in the ring buffer
ring buffer (500 ms)
  │
  ▼  audio callback (audio)
marker + word, as i32 ──▶ cpal ──▶ CoreAudio ──▶ DAC
```

Encoding runs on the engine thread. The audio callback only copies words and adds markers, so a
busy encoder shows up as silence words rather than as a blocked callback.

Mono is copied to both channels. A source with more than two channels keeps its first two, as
there is no surround downmix.

## DoP framing

DoP disguises DSD as PCM. Each channel of each frame carries 16 DSD bits and one marker byte. The
markers tell the DAC that the frames are DSD rather than PCM.

```text
          i32 sample handed to cpal
 ┌──────────┬───────────┬───────────┬──────┐
 │  marker  │ DSD high  │ DSD low   │  0   │
 └──────────┴───────────┴───────────┴──────┘
```

- The marker alternates between `0x05` and `0xFA` frame by frame, and is the same on both channels
  of a frame.
- The oldest bit of a word is its highest bit.
- The silence word is `0x6969`, an even pattern that a DAC decodes to its mid level.
- The frame rate is the DSD bit rate divided by 16.

| DSD rate | Bit rate, 44.1 kHz family | DoP frames | Bit rate, 48 kHz family | DoP frames |
| --- | --- | --- | --- | --- |
| DSD64 | 2.8224 MHz | 176.4 kHz | 3.072 MHz | 192 kHz |
| DSD128 | 5.6448 MHz | 352.8 kHz | 6.144 MHz | 384 kHz |
| DSD256 | 11.2896 MHz | 705.6 kHz | 12.288 MHz | 768 kHz |

Markers are added in the callback (`dop_callback` in `crates/audio/src/engine.rs`) rather than in
the encoder, because their alternation has to survive anything that drops words on the way to the
device, such as a seek. A broken alternation makes a DAC treat the frames as PCM at full level.
Lead-in silence and underruns get the same markers for the same reason.

## Choosing the rate

`Settings::frame_rates_for` returns two frame rates, best first, and `plan_dop` takes the first one
the device has.

- **Base rate 44.1 kHz (default):** every song plays at the 44.1 kHz family's rate, so 48 kHz songs
  are converted to it. These are the standard DSD rates.
- **Match each song:** a source rate divisible by 11025 uses the 44.1 kHz family, and any other uses
  the 48 kHz family. Some DACs take 48 kHz family DoP for PCM, which is loud noise, so this option
  asks for confirmation.

The second rate is the other family's. A device that lacks the first rate gets it, rather than a
failure or a different DSD rate. Settings offers only the DSD rates the device has, and turning DSD
on picks the highest of them.

## Oversampler

The oversampler takes the source to the DSD bit rate. Its output has the same number of frames per
second of input whatever the filter's delay: the delay is trimmed from the start, and silence
flushes the tail at the end of a track.

There are two filter types.

| Filter | Stages | Character |
| --- | --- | --- |
| Sinc (default) | Sinc to 352.8 or 384 kHz, then FFT 256 up to the bit rate | Low memory. Length, window and cutoff are adjustable. |
| FFT | One FFT stage from the source to the bit rate | Memory grows quickly with size, up to about 645 MB at 16384. |

Sinc has two stages because its cost grows with the output rate. Going straight to 11.2896 MHz
would need one full filter calculation per output frame. The sinc stage does the part that needs a
steep filter, and a short FFT stage does the rest. A source already at or above the middle rate
skips the sinc stage.

Details that matter when changing this code:

- The sinc stage uses Rubato's `Async` with an exact table when the rate ratio allows one of up to
  4096 rows, and with cubic interpolation over a 4096-row table otherwise. An exact table for a rate
  with no small ratio would take gigabytes.
- `Async::output_delay()` overstates the delay, by 2 output frames at a whole ratio and by about 1
  otherwise. `Stage::sinc` corrects for this, and a test would catch a Rubato release that changes it.
- With `cutoff` left at `None`, Rubato picks the highest cutoff that the length and window leave
  free of aliasing.

## Level stages

These run in this order, at the bit rate, between the oversampler and the modulator.

**Input level** is a fixed gain.

**Fade** rises from silence as `(t / T)²` over 3 seconds. It survives seeks and moves to the next
track.

**Limiter** keeps peaks from overloading the modulator.

- Linked stereo: both channels share one gain, so the stereo image holds.
- 1 ms look-ahead, so the gain is down before the peak arrives.
- 50 ms release, with the gain smoothed by a moving average over the look-ahead.
- It runs after oversampling, so it sees the peaks between source samples. That is why Settings
  labels its ceiling in dBTP.
- The ceiling defaults to +1 dB against full scale PCM.

The encoder counts the limiter's delay, discarding the first frames and pushing out what is left at
the end of a track, so timing and length are unaffected.

Samples that are not finite numbers become silence on the way in, as they would otherwise panic
inside the FFT.

## Modulator

Each channel has its own 1-bit error-feedback modulator
(`crates/dsd/src/modulator.rs`). Its signal transfer function is 1, and its noise transfer function
(NTF) is `N(z) / D(z)`. Full scale PCM maps to 50 % modulation, the level DSD players expect.

```text
feedback[n] = Σ (n_k - d_k) · error[n-k]  -  Σ d_k · feedback[n-k]
input[n]    = 0.5 · sample[n] + feedback[n]
bit[n]      = input[n] + dither >= 0
error[n]    = (bit[n] ? +1 : -1) - input[n]
```

The output is therefore `0.5 · sample + (N/D) · error`.

### NTF design

`NoiseTransfer::design` in `crates/dsd/src/noise_transfer.rs` builds the NTF from settings, at the
start of each stream.

- **Zeros** set where noise is lowest. With Optimized zeros on, they are spread over the noise band
  at the roots of Legendre polynomials, which lowers total noise in the band. With it off, they all
  sit at 0 Hz.
- **Poles** set how much gain the noise gets above the band. They follow a Butterworth pattern. A
  bisection searches for the corner frequency at which the gain at half the sample rate equals
  Noise gain.

| Setting | Effect |
| --- | --- |
| Order (3, 5, 7, 9) | Number of poles and zeros. More allows steeper shaping, but does not always measure quieter. |
| Noise gain | Out-of-band gain. Higher lowers in-band noise and brings instability closer. |
| Noise band (20 or 48 kHz) | The band kept quiet. 48 kHz applies only at DSD256, as at lower rates it raises the noise below 20 kHz too much. |
| Optimized zeros | Spreads the zeros as above. |
| Dither | Triangular noise at the quantizer's decision, shaped along with the error. Off by default. |

### Stability

A high-order 1-bit loop can lose control on loud input. On a stable design, music takes the
quantizer's input to about 5. If its absolute value passes 30, the modulator resets its state and
marks itself lost. The encoder then returns `Error::Unstable`, and playback stops with a message
saying what to lower. It does not try to continue, because what drove the loop there tends to go
on, and it rarely sounds right again.

With the default Noise gain of 1.4, the limiter is what keeps hard material stable. At order 7,
square waves, white noise and overdriven sine waves went unstable without it and stayed stable with
it. See [findings.md](findings.md#stability).

## Device control on macOS

cpal alone is not enough for DoP, which needs bit-exact samples at the DoP rate with nothing mixed
in. `crates/audio/src/exclusive_device/core_audio.rs` adds what is missing through `objc2-core-audio`.
This is the only unsafe code in this feature.

`Playback::open` does the following while holding a process-wide lock on device setup:

1. **List rates.** `frame_rates` reports which of the six DoP rates the device has in a usable
   physical format. Settings uses the same list.
2. **Plan.** `plan_dop` picks the frame rate. The device's prior rate is noted, to be restored the
   same way as for PCM playback.
3. **Take the device.** `Exclusive::for_dop` picks the best physical format, enters hog mode so no
   other app can play or change the format, then sets the format and waits up to 5 seconds for the
   background switch to finish.
4. **Open the stream.** The stream is opened as `i32`, because that gets cpal to set an integer-only
   device to more than 16 bits.
5. **Confirm.** `Exclusive::confirm` reads the nominal rate and physical format back. Any other
   rate would resample DoP into loud noise, so a mismatch is an error.

A format counts as usable if it is linear PCM, not non-mixable, has at least two channels, and is
either integer of at least 24 bits or 32-bit float. Float qualifies because 24-bit integers survive
in it exactly. Non-mixable formats are excluded because they turn the device's virtual format into
integers too, and cpal cannot read that configuration, which every later stream starts from. Among
usable formats, exactly two channels come first, then integers, then more bits.

Hog mode is released when `Exclusive` is dropped. The physical format it replaced comes back with
the device's rate, once nothing has played for a few seconds.

Other platforms get a stub in `crates/audio/src/exclusive_device.rs` that reports no rates, and
`CAN_PLAY_DOP` hides the option from Settings. Their shared audio modes resample and mix, which would
turn DoP into noise.

## Playback behavior

### Starting a stream

A DAC needs time to lock onto DoP, loses whatever comes before that, and reports nothing about it.
Every DoP stream therefore starts with 1 second of silence words. A FiiO K9 at DSD256 still lost a
few hundred milliseconds after 250 ms.

The ring buffer is filled before sound starts. After that, the encoder works in slices of about
5 ms, so the engine keeps reading commands such as Pause even when encoding is behind.

### Pausing

The stream keeps running and sends silence words, so the DAC stays locked and resuming takes no
lead-in. Stopping the stream, as PCM does, would lose the lock.

### Seeking

`Encoder::reset` replaces the oversampler and limiter, and keeps the modulators and any fade. The
stream stays open, so a seek takes no lead-in.

### Gapless playback

If the next track plans to the same output rate, it joins on the same stream.

- With the same source format, the same converter goes on, so there is no joint at all.
- With another source format, `Encoder::for_next_track` builds new filters around the existing
  modulators, so noise shaping and any fade continue.

`undelivered_frames` says how much audio is still inside the encoder, which is how the engine knows
when the device reaches the new track.

With the default base rate, every track plans to the same rate, so a track joins as long as enough
of it has arrived. With Match each song, a track from the other family needs a new stream, and so
gets a new lead-in.

### Changing settings

| Change | Effect |
| --- | --- |
| DSD rate, base rate, Quality or any filter or modulator value | Reopens at the same position, with a lead-in and, if Fade in is on, a fade |
| Fade in, Sample rate, Resampling | Applies without interrupting the sound |
| Stream invalidated by the system | Reopens the same way as a settings change |

Every reopen onto DoP fades in, because it may be taking over from PCM, whose volume may have been
far below full level. Sample rate and Resampling have no effect on DoP, as DoP sets the rate and
`Converter::for_settings` builds only the encoder. Settings shows both as disabled while DSD is on.

## Protection against loud output

DoP cannot be attenuated, because scaling the samples would destroy the words. A device that does
not decode DoP plays it as noise at full level, and no device reports beforehand whether it can.
These measures follow from that.

| Measure | Where |
| --- | --- |
| Turning DSD on asks for confirmation and says to turn the device down | `confirm_dsd` |
| DSD applies only to the device it was turned on for, and never to the system default, which can change | `OutputSettings::from_config`, `with_dop` |
| The volume control is disabled, and its tooltip points to the DAC | `Player::is_volume_locked`, `VolumeControl` |
| Turning DSD on pauses a playing song, and what opens next waits for Play | `DopStart` in `crates/player/src/player.rs` |
| Sound rises over 3 seconds after DSD turns on or its settings change. `dsd_fade_in` turns this off. | `DOP_FADE_IN` |
| The stream's rate and format are confirmed after opening | `Exclusive::confirm` |
| Markers keep alternating through silence, underruns and seeks | `dop_callback` |

A song opened normally, with DSD already settled, starts at full level after the lead-in.

## Settings

Quality chooses a preset. Changing any value below it switches Quality to Custom, which keeps the
current values.

| Quality | Filter | Order | Noise gain | Limiter |
| --- | --- | --- | --- | --- |
| Low | Sinc 256 | 5 | 1.4 | +1 dB |
| Medium | Sinc 512 | 7 | 1.4 | +1 dB |
| High (default) | Sinc 1024 | 7 | 1.4 | +1 dB |
| Max | Sinc 2048 | 7 | 1.4 | +1 dB |

All presets use the Blackman-Harris² window, the automatic cutoff, a 20 kHz noise band, optimized
zeros, no dither and an input level of 0 dB. Max stays at order 7 because order 9 measured no
quieter at these noise gains.

The keys are listed under [Configuration](../../README.md#configuration) in the README.
`OutputSettings::from_config` in `crates/audio/src/from_config.rs` turns them into `dsd::Settings`.
Saving writes only the keys that changed, so a value set through an environment variable does not
end up in the file.

## Errors

| Error | Cause | What the user sees |
| --- | --- | --- |
| `UnsupportedDopRate` | No usable format at the rate, or the confirmation failed | Asks to choose another DSD rate |
| `DeviceHeld` | Another process holds the device in hog mode | Asks to close that app and press Play |
| `ExclusiveOutput` | Hog mode could not be entered | Asks to press Play to try again |
| `DsdUnstable` | The modulator lost control | Asks to lower Noise gain or Input level, then press Play |

## Performance

The encoder takes about 37 % of one core at DSD256 with the High preset on an Apple M5, and about
45 % with Max. Load depends on the settings far more than on the source rate. Sinc runs at about
70 MB of memory, and FFT at size 16384 at about 645 MB. Full tables are in
[findings.md](findings.md#measurements).

The `dsd` and `rubato` crates and Rubato's FFT dependencies are built with `opt-level = 3` in debug
builds too.

## Limits

- DSF and DFF files are not read natively.
- Only DoP is implemented, not native DSD.
- DSD256 is the highest rate offered. DoP at DSD512 would need frame rates of 1.4112 or 1.536 MHz.
- macOS is the only platform.
- Volume cannot be changed in the app while DSD is on.

## Testing

`cargo test -p dsd` covers the encoder, limiter, modulator, NTF design and oversampler, and needs
no device.

Tests that need a device are marked `#[ignore]`. Run them by exact name on a loopback device:

```sh
HIPHILE_TEST_DEVICE="BlackHole 2ch" cargo test -p audio --lib -- \
  --ignored --test-threads=1 --exact engine::tests::dop_without_its_fade_starts_at_full_level
```

> [!WARNING]
> Do not run the whole `--ignored` set, which can reach the default device. If that is a DAC, DoP
> can be dangerously loud.

More notes on testing are in [findings.md](findings.md#testing-notes).
