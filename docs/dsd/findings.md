# Findings on DSD (DoP) output

What was established while building Hiphile's DoP output, as of 2026-11-07. For how the feature
works, see [implementation.md](implementation.md).

Each claim is tagged by how far it was checked.

| Tag | Meaning |
| --- | --- |
| [Verified] | Checked by running or measuring it |
| [Read] | Checked by reading code or headers, without running it |
| [Inferred] | Reasoned from observations, without checking directly |
| [Unknown] | The cause or behavior is not understood |

## Environment and scope

| Item | Value |
| --- | --- |
| Mac | Apple M5, 24 GB of memory, macOS 26.2 |
| DAC | FiiO K9 (USB) |
| Loopback | BlackHole 2ch |
| Audio API | cpal 0.18.2 (CoreAudio) |

On the K9, the only evidence is what the user heard and reported, and properties read from the
device. All automated tests and all checks of recordings ran on BlackHole. BlackHole's recording
stream pins its rate, so it cannot reproduce a real device's clock or rate switching.

Nothing was built on any platform other than macOS.

## What the K9 showed

| Finding | Status |
| --- | --- |
| DoP at 705.6 kHz is recognized as DSD256 | [Verified] by user report |
| DoP at 768 kHz is recognized as 768 kHz PCM | [Verified] by user report |
| Locking onto DoP takes time. 250 ms of silence lost a few hundred milliseconds of the start, and 1 s solved it. | [Verified] by user report |
| The physical formats at 705.6 and 768 kHz are the same set (32-bit int, 16-bit int, float32) | [Verified] by reading the properties |
| 32-bit int comes as both mixable (`0xc`) and non-mixable (`0x4c`) | [Verified] by reading the properties |

These led to the following defaults.

- **Base rate:** the 44.1 kHz family. 48 kHz songs are also converted to 705.6 kHz. The 48 kHz
  family's rates are chosen through a confirmation dialog.
- **Lead-in:** 1 s.
- **Fade in:** 3 s, which can be turned off in Settings.

## Measurements

### CPU load

DSD256, encoding 10 seconds of a sine wave on a single thread. Load is processing time over audio
time, as a fraction of one core. [Verified]

| Order | Sinc length | 44.1 kHz | 48 kHz | 96 kHz | 192 kHz |
| --- | --- | --- | --- | --- | --- |
| 5 | 256 | 0.283 | 0.290 | 0.287 | 0.290 |
| 7 | 512 | 0.357 | 0.356 | 0.345 | 0.349 |
| 7 | 1024 | 0.366 | 0.378 | 0.383 | 0.382 |
| 7 | 2048 | 0.449 | 0.449 | 0.451 | 0.452 |
| 9 | 2048 | 0.387 | 0.380 | 0.379 | 0.384 |

Order 9 coming out lighter than order 7 may be measurement noise, such as competition from other
work. [Unknown]

### Real songs

All were encoded to the end at DSD256. [Verified]

| Settings | Songs | Result |
| --- | --- | --- |
| Order 7, Sinc 1024 | 14 | Completed. Load 0.41, peak memory 72 MB. |
| Same, 48 kHz family to 705.6 kHz | 9 | Completed. Load about 0.41, 66 MB. Two songs came out one word longer, with no effect. |
| Order 7, Sinc 2048 | 9 (44.1, 48 and 96 kHz) | Completed. Load 0.45 to 0.55, at low priority alongside other work. |
| FFT 16384 | | Load 0.42, memory 645 MB. |

### Noise

Noise from 20 to 48 kHz with a −6 dBFS, 1 kHz tone as input, against 0 dBFS. [Verified]

| Order | DSD64 | DSD128 | DSD256 |
| --- | --- | --- | --- |
| 5 | −56.8 dB | −90.1 dB | −123.3 dB |
| 7 | −50.7 dB | −95.7 dB | −140.2 dB |
| 9 | −40.0 dB | −96.2 dB | −126.3 dB |

- At a noise gain of 1.4, order 9 came out no quieter than order 7, which is why Max uses order 7.
- This measurement cannot compare noise from 20 Hz to 20 kHz. Leakage from the analysis window sets
  the floor, and every setting reads the same −104 dB against the −6 dBFS tone.
- Measurements with silence as input were not used for comparison. Idle patterns dominate them, and
  they do not represent music.

### Stability

Noise gain 1.4. Full-level sine waves (1 kHz and 19 kHz), a square wave, white noise and a sine wave
at +6 dB, for 5 seconds each.

| Condition | Result |
| --- | --- |
| Limiter at +1 dBTP, all preset candidates below, DSD64, DSD128 and DSD256 | All stable [Verified] |
| No limiter, order 7, DSD256 | Sine waves stable. Square wave, noise and +6 dB unstable. [Verified] |
| No limiter, order 5, DSD256 | Sine waves and square wave stable. Noise and +6 dB unstable. [Verified] |

The candidates were order 5 with Sinc 256, order 7 with Sinc 512, 1024 and 2048, and order 9 with
Sinc 2048.

### Delay of the sinc stage

Rubato's `Async::output_delay()` reports more than the real delay, by 2 output frames at a whole
ratio and by about 1 at any other. The amount trimmed is corrected by this much. [Verified] by
measuring phase.

## Open problem: pitch drop after Previous

### Symptom

As reported by the user:

- It happens now and then when a 48 kHz song plays at base rate 44.1 kHz and Previous is pressed
  partway through.
- The K9 keeps showing DSD256.
- Pitch drops by a little over a semitone, and noise appears.
- Moving to another song, for example, clears it.

Whether it also happens with 44.1 kHz songs is not known.

### What is known

44.1 / 48 = 0.91875 is about −1.47 semitones, which fits the symptom. Pitch is the device's clock
over the rate the encoder assumed, so the cause is probably one of these. [Inferred]

| Candidate | Description | Does it explain the noise? |
| --- | --- | --- |
| A | The K9 plays data made for 768 kHz at 705.6 kHz | Yes, if the K9 stays on DSD256 and overflows on the incoming data |
| B | 48 kHz data is processed as 44.1 kHz | Not easily |

A different K9 clock alone would not lower the pitch. Data made for 705.6 kHz played at 768 kHz would
raise it. [Inferred]

For A to happen, 705.6 kHz would have to vanish for a moment from what `dop_device::frame_rates`
returns during a reopen, so that `plan_dop` chooses 768 kHz. Whether the K9 stays in DSD while cpal
asks for 768 kHz has not been checked. [Unknown]

### What Previous does

Even more than 3 seconds into a song, Previous closes the stream and opens it again rather than
seeking to the start. [Read]

1. `previous()` calls `play_current` without moving back in the queue.
2. `engine.stop()` in `open_current` discards the playback, which also releases the DoP stream and
   hog mode.
3. `open_track` reopens the song, and `engine.load` creates a new stream and encoder.

Each opening reads the supported rates and chooses between 705.6 and 768 kHz again.

### Conditions that did not reproduce it

All on BlackHole. [Verified]

| Test | Result |
| --- | --- |
| A 48 kHz tone reopened 21 times by the engine, at intervals of 0 to 500 ms | All at 997.000 Hz, level 0.25, with valid markers |
| A real 48 kHz FLAC with a 44.1 kHz next track, 21 times, after 4.5 to 5.8 s of playback | The 3 s after the lead-in were bit-identical every time, with 0 invalid markers |
| Previous in the real app, 14 times | All 15 opens went from 48000 to 705600 |

If only software were involved, B did not occur on BlackHole. A cannot be checked on BlackHole.

### Logs needed to settle it

Reproducing it on the K9 with these logged would tell the candidates apart.

- `Playback::open`: the source spec, the rates the device returned, the chosen output rate, and the
  physical format and nominal rate (before and after reading the rates, after taking hog mode and
  after creating the stream)
- While playing, every 2 seconds: input frames per second, output frames per second,
  `frames_played` per second, the encoder's settings and the device's rate
- Engine commands: `Load`, `Stop`, `StreamInvalidated` and joins

An encoder at 768000 means A, and input of about 44100 frames per second means B. If neither, suspect
the K9's own behavior.

### Later changes that may matter

No reproduction has been tried since these changes.

- Non-mixable formats are no longer used, so reopening on the K9 should no longer switch formats
  twice. [Inferred]
- Rate and format are confirmed after opening a stream, and a mismatch is an error.

## Code review findings and fixes

All are fixed. "Checked by" says how the problem was established before the fix.

| # | Problem | Checked by | Fix |
| --- | --- | --- | --- |
| 1 | Changing DSD settings while paused or fading lost the fade | Running it | Every reopen fades in |
| 2 | Sound came at full level when DSD turned on during loading, or on resuming from `Stopped` | Partly running it | Right after DSD turns on, playback starts paused, then fades in on Play |
| 3 | A slow encoder kept the engine from reading commands | Running it | Commands are read about every 5 ms |
| 4 | NaN or Inf panicked the engine thread | Running it | They count as silence |
| 5 | `Exclusive::take` chose non-mixable formats | Reading the code and the properties | Non-mixable formats are excluded |
| 6 | Format and rate were not checked after opening | Reading the code | They are confirmed after opening, and a mismatch is an error |
| 7 | Moving to the next track cut a fade short | Running it | The fade carries over |
| 8 | Delay remained counted after `finish`, so a very short next track never got `Ended` | Running it | `finish` brings it to 0 |
| 9 | The end of a short input was lost | Running it | The limiter is flushed after modulating |
| 10 | Changing one DSD value also saved values from environment variables | Reading the code | Only changed keys are saved |
| 11 | A disabled Noise band stayed in effect at 48 kHz | Reading the code | Fixed at 20 kHz below DSD256 |
| 12 | Changing Sample rate during DoP interrupted playback | Running it | No reopen, and the item is disabled |
| 13 | Cutoff options appeared twice | Number conversion | The setting's type is now `f64` |
| 14 | The description of an omitted `dsd_rate` did not match behavior | Reading the code | The description was corrected |
| 15 | An error message did not match the reason hog mode failed | Reading the code | Split out as `DeviceHeld` |
| 16 | The SAFETY claim of `read_list` could fail to hold | Reading the code | The size is aligned to a multiple of the element |
| 17 | Violations of copy and comment rules | Reading the code | Corrected |

Before and after, on BlackHole. [Verified]

| Scenario | Before | After |
| --- | --- | --- |
| Settings change while paused (level over time, full level is 0.25) | 0.233 → 0.250 | 0.002 → 0.003 → 0.005 … |
| DSD turned on during loading | 0.007 → 0.250 | 0.002 → 0.004 → 0.006 … |
| A next track of 10 frames | Never ends | Ends normally |
| A song with NaN and Inf | The thread panics | Plays to the end |
| Pause while running at low priority | No effect after 25 s | Under 1 ms in the logs |
| Shutdown under the same conditions | 46.5 s | About 0.5 s |

The same reasoning as #12 was later applied to Resampling, which also has no effect on DoP. Its
group is disabled while DSD is on, and changing it no longer reopens the stream while a next track
is joined. [Verified] The test for this fails without the fix.

## Testing notes

- Run device tests with `HIPHILE_TEST_DEVICE="BlackHole 2ch"`, and narrow `--ignored` by name. The
  default device can be a real DAC, so do not run them all.
- BlackHole may play stale data right after finishing. Tests that read a recording should cut at
  the first zero.
- If BlackHole stays at a high rate, other tests fail. Set it back to 48 kHz after testing.
- `resampled_tracks_join_without_a_gap_at_the_largest_fft` fails now and then. It failed once in a
  batch after the fixes and passed 6 of 6 times alone. Its relation to the changes has not been
  checked. [Unknown]
- A player test that leaves DoP running makes later tests fail, so stop at the end of the test.
  [Inferred] All tests passed once the stop was added.
- `Player::new` uses the real settings. Changing the volume in a test changes the real config file.
- A test that must catch a brief state should be seen to fail without the fix. The first test for
  the Resampling fix passed without it, as its one change missed the short time a next track is
  joined.
- The programs used for these measurements were temporary and are not in the repository.
