# Crossfeed and balance

Two DSP stages that act on the stereo image: Crossfeed, which mixes part of each channel into the
other for headphones, and Balance, which sets the level of the left channel against the right.

| | |
| --- | --- |
| Crossfeed | The bs2b design: a first-order lowpass from each channel into the other, and a matching high shelf on the direct path |
| Balance | Lowers one channel by up to 12 dB, in 0.5 dB steps. It never raises either one |
| Where | Before the resampler on PCM and before the encoder on DoP, at the song's own rate |
| Output | PCM in every mode, shared and exclusive, and DoP |
| Level | Crossfeed lowers its output by 3 dB, so that it never goes over full scale |
| When off | Samples pass bit for bit |
| A change | Heard once the 500 ms buffer reaches it, without reopening the stream |
| Devices | One setting for all of them |
| Platforms | All |

## Where the code lives

| Crate | Responsibility |
| --- | --- |
| `dsp` | Settings, filter design, and the processor that runs on interleaved frames. No I/O, no GPUI and no dependencies, so a mobile host can run it too |
| `audio` | `Engine::set_dsp`, running the processor in `Converter`, and `from_config` |
| `config` | The saved keys, presets and ranges |
| `playback` | `Player::set_dsp`, passed on to the engine |
| `hiphile-core` | `DspSettings`, a handle of its own beside `AudioSettings`, which `Hiphile::player` and `Workspace` pass to the player |
| `settings` | The DSP page |
| `hiphile-cli` | Plays with the settings through `Hiphile::player`, and shows their errors |

## Signal path

```text
decoder ─▶ Converter: channel map ─▶ crossfeed ─▶ balance ─┬─▶ resampler ──▶ crossfade mix ─▶ ring buffer ─▶ callback: volume, fade ─▶ device
                      (song's rate)                         └─▶ DSD encoder ───────────────────▶ ring buffer ─▶ DoP callback ─▶ device
```

The processor runs in `Converter`, after the channel map and before the resampler or the DSD
encoder:

- PCM and DoP share one place, one code path and one set of tests, and sound the same.
- Rate conversion stays the last stage before the device, as the oversampler already is for DSD.
  The processor runs at the song's rate, which costs the least: with `sample_rate = "x16"` it
  would otherwise run at up to 768 kHz. A later stage such as an equalizer or convolution costs
  more the higher the rate, so it needs this place too.
- The engine thread is not real-time, so later stages may allocate. The callback stays a copy with
  the volume and the fade.
- The processor has no delay, so positions, `undelivered_frames` and the joints are unchanged.
- The resamplers work in floating point and clip nothing, so they pass on whatever crossfeed
  gives them (see [Level](#level)).

What this costs: a change is heard only once the ring buffer reaches it, about 0.5 s later, as on
DoP. Balance is set once rather than ridden like the volume, so the delay is accepted. If dragging
it turns out to need an immediate response, Balance on PCM can move into the callback, next to the
volume, and Crossfeed stays here.

Crossfeed comes before balance, because balance corrects for the ears or the headphones, which
come after the mix that crossfeed imitates.

## Crossfeed

The algorithm is bs2b, by Boris Mikhaylov ([bs2b.sourceforge.net](https://bs2b.sourceforge.net/)).
Hiphile implements it from the published formulas and takes no code from libbs2b, whose library
is MIT and whose plugins are GPL.

Each channel runs two first-order filters:

```text
lo[n]  = a0_lo · x[n] + b1_lo · lo[n-1]                     lowpass, fed to the other channel
hi[n]  = a0_hi · x[n] + a1_hi · x[n-1] + b1_hi · hi[n-1]    high shelf, kept on this channel
out_L  = gain · (hi_L + lo_R)
out_R  = gain · (hi_R + lo_L)
```

They are designed from a cutoff `fc` and a separation `S` in dB, at the song's rate:

```text
GB_lo = -5/6 · S - 3               G_lo  = 10^(GB_lo / 20)
GB_hi =  1/6 · S - 3               G_hi  = 1 - 10^(GB_hi / 20)
fc_hi = fc · 2^((GB_lo - 20 · log10(G_hi)) / 12)
x     = exp(-2π · fc / rate)       a0_lo = G_lo · (1 - x)          b1_lo = x
x_hi  = exp(-2π · fc_hi / rate)    a0_hi = 1 - G_hi · (1 - x_hi)   a1_hi = -x_hi   b1_hi = x_hi
gain  = 10^(-3 / 20) / (1 - G_hi + G_lo)
```

`S` is how much lower the bass a channel feeds to the other is than its own. The higher it is, the
weaker the crossfeed, which is why Settings calls it Separation rather than bs2b's "feed level".

The `-3` in `gain` is the headroom described in [Level](#level); bs2b's own `gain` is
`1 / (1 - G_hi + G_lo)`. Computed for the three presets, with the headroom:

| | bs2b | Chu Moy | Jan Meier |
| --- | --- | --- | --- |
| Cutoff, separation | 700 Hz, 4.5 dB | 700 Hz, 6.0 dB | 650 Hz, 9.5 dB |
| Bass fed to the other channel, against its own | −4.5 dB | −6.0 dB | −9.5 dB |
| Center content, bass | −3 dB | −3 dB | −3 dB |
| Center content, treble | −4.8 dB | −4.5 dB | −4.1 dB |
| Fed to the other channel at 10 kHz | −34 dB | −35 dB | −38 dB |
| Highest possible peak, at 44.1 kHz and at 768 kHz | −0.6 dB, −0.3 dB | −0.7 dB, −0.4 dB | −1.2 dB, −1.0 dB |

- The ranges are bs2b's: cutoff from 300 to 2000 Hz and separation from 1 to 15 dB. Settings offers
  the cutoff in 50 Hz steps and the separation in 0.5 dB steps, which hold all three presets.
- Songs at other rates sound the same. Between 44.1 and 768 kHz, the direct path differs by at most
  0.13 dB up to 20 kHz, and the crossfed path by at most 0.18 dB up to 5 kHz.
- Coefficients and state are `f64`, and samples stay `f32`. A state below 1e-20 is set to zero
  after each block, so that a filter decaying into silence never runs on subnormal numbers, which
  are slow on x86.
- Center content, mono sources among it, comes out with its treble lowered, as in the table. This
  is bs2b's character, not something to correct.

### Level

bs2b's own correction keeps center bass at 0 dB and lowers everything else, but content that
differs between the channels can still go over full scale, by up to 2.4 dB at 44.1 kHz and 2.7 dB
at 768 kHz. Real masters do: bs2b alone took 211 of 352 cached songs over full scale, the highest
to +1.16 dBFS.

So crossfeed lowers its output by a further 3 dB (`HEADROOM_DB` in `crossfeed.rs`), which keeps
the highest possible peak below full scale at every rate, as in the table. The same songs then
peaked at −1.84 dBFS, none of them over.

- Turning crossfeed on makes every song 3 to 4.8 dB quieter. The Settings description and the
  README say so, as a comparison by ear at unmatched levels would favor crossfeed off.
- On PCM the volume no longer has to leave room for crossfeed, and on DoP the limiter after the
  oversampler is left for what else goes over.

## Balance

- The value runs from −12 to +12 dB. Settings moves it in 0.5 dB steps, and a value from the file
  or the environment may be any tenth of a dB, which is what `dsp` holds. A negative value lowers
  the right channel by that much and a positive one lowers the left. The other channel stays at 0 dB, so balance never
  raises a level and cannot clip. Neither side goes silent at the ends.
- The lowered channel's gain is `10^(-|value| / 20)`.
- It applies to the first two channels after the channel map, the left and right that
  `map_channels` keeps. On an output with one channel, neither balance nor crossfeed does anything.

## Changes while playing

| Change | Effect |
| --- | --- |
| Balance | The gains move linearly to the new value over 20 ms and end on it exactly. A change part-way through starts from where they are, so dragging the slider moves smoothly |
| Crossfeed on or off, its preset, cutoff or separation | The output crossfades from the old filter to the new over 20 ms, along a smoothstep. A new filter starts from the old one's state when there was one. A change that comes during a crossfade waits for it to end, and only the latest one waits |
| Any of them | Never reopens the stream or breaks the sound |

When both settings are neutral and nothing is moving, `process` returns without touching the
samples, so the output is bit for bit what it would be without this feature. That is why the
gains have to end exactly on 1.0.

## Settings in the engine

- `dsp::Settings` is `Copy` and packs into a `u64` (`to_bits`, `from_bits`): crossfeed on or off,
  the cutoff in Hz, the separation in tenths of a dB, and the balance in tenths of a dB.
- `Engine::set_dsp` stores it in `Shared::dsp`, an `Arc<AtomicU64>`, as `set_volume` stores the
  volume. It sends no command, and a new stream is never needed.
- Each `Converter` holds a clone of that `Arc` and a `dsp::Processor` for its source rate and output
  channels. It loads the atomic for each packet and, when it has changed, designs the new filter
  and starts the transition.
- A converter made for a new stream starts its processor at the current settings, with no
  transition.
- `Hiphile::player` sets the settings on each player it makes, before anything plays, and
  `Workspace` passes on each change.

How the processor keeps going between songs:

| Case | What happens |
| --- | --- |
| Gapless joint through the same converter | Nothing: one processor runs on |
| Gapless joint through `Converter::following` | The processor moves to the new converter, redesigned for the new rate with its state kept, as the DSD encoder keeps its modulators. The joint stays continuous |
| Crossfade between songs | The incoming converter gets a copy of the outgoing one's processor, so both follow the settings. Crossfeed is linear, so processing each song before the blend sounds as processing the mix would |
| Seek, take-back | The processor stays as it is. The signal jumps anyway, and the filters forget it in under a millisecond |

## The `dsp` crate

```rust
/// Cutoff in Hz and separation in tenths of a dB, within bs2b's ranges.
pub struct Crossfeed { .. }

impl Crossfeed {
    pub const CUTOFFS_HZ: RangeInclusive<u16> = 300..=2000;
    pub const SEPARATIONS_DB: RangeInclusive<f32> = 1.0..=15.0;
    /// `None` outside the ranges.
    pub fn new(cutoff_hz: u16, separation_db: f32) -> Option<Self>;
    pub fn cutoff_hz(self) -> u16;
    pub fn separation_db(self) -> f32;
}

/// Tenths of a dB. Negative lowers the right channel, positive the left.
pub struct Balance(i16);

impl Balance {
    pub const CENTER: Self;
    pub const MAX_DB: f32 = 12.0;
    /// Rounded to a tenth and clamped. NaN is the center.
    pub fn from_db(db: f32) -> Self;
    pub fn db(self) -> f32;
    pub fn is_centered(self) -> bool;
}

pub struct Settings { .. }

impl Settings {
    pub fn new(crossfeed: Option<Crossfeed>, balance: Balance) -> Self;
    pub fn crossfeed(&self) -> Option<Crossfeed>;
    pub fn balance(&self) -> Balance;
    pub fn is_neutral(&self) -> bool;
    pub fn to_bits(self) -> u64;
    /// Clamps what is out of range.
    pub fn from_bits(bits: u64) -> Self;
}

pub struct Processor { .. }

impl Processor {
    pub fn new(rate: u32, channels: usize, settings: Settings) -> Self;
    /// What it is moving to.
    pub fn settings(&self) -> Settings;
    /// Starts a transition when `settings` differ from those it is moving to.
    pub fn follow(&mut self, settings: Settings);
    /// Whole interleaved frames.
    pub fn process(&mut self, samples: &mut [f32]);
    /// For the next song's rate, with the state kept.
    pub fn at_rate(self, rate: u32) -> Self;
}
```

The presets live in `config`, which turns them into numbers as it does for the DSD Quality presets,
and `audio::from_config` makes the `dsp::Settings` from those. `dsp` knows nothing of presets.

## Saved settings

| Key | Values | Default |
| --- | --- | --- |
| `crossfeed` | `true` or `false` | `false` |
| `crossfeed_preset` | `bs2b`, `chu-moy`, `jan-meier` or `custom` | `bs2b` |
| `crossfeed_cutoff_hz` | With `custom`: from 300 to 2000 | `700` |
| `crossfeed_separation_db` | With `custom`: from 1 to 15, read in tenths so that what plays is what Settings shows | `4.5` |
| `balance_db` | From −12 to 12. Negative lowers the right channel | `0` |

These follow the rules the other audio keys have:

- A preset writes no custom keys, and saving writes only the keys that changed.
- `HIPHILE_CROSSFEED`, `HIPHILE_CROSSFEED_PRESET` and the rest override the file.
- A value out of range is named in the error, and the others still apply.
- The preset and values stay while crossfeed is off, in the file too, so turning it on again
  comes back to them.
- Custom starts from the preset in use, so choosing it changes nothing until a value changes.
  Changing Separation or Cutoff switches to Custom and keeps the current values.

## Settings window

A page of its own, `DSP`, after Audio, with the `SlidersHorizontal` icon:

```text
DSP
  Crossfeed     [on]
                Makes headphones sound more like speakers. Lowers the level by 3 dB.
  Preset        ● bs2b   ○ Chu Moy   ○ Jan Meier   ○ Custom       only while on
  Separation    [ 4.5 dB ▾ ]                                       only while on
  Cutoff        [ 700 Hz ▾ ]                                       only while on
  errors

  Balance       ────────●────────   Center
```

- Crossfeed and Balance are separate groups, as neither belongs to the other. The errors from
  reading or saving either one go below the first, as on the Audio page.
- **Balance** is the `BalanceSlider` view, from −12 to +12 with a step of 0.5, given by
  `DspSettings::balance_range`. It is built from gpui-kit's base slider parts, as the player's
  `HoverSlider` is, because the component `Slider` fills its line from the minimum, and a balance's
  line has to fill from the center towards the thumb. Its value reads `Center`, `Left -1.5 dB` or
  `Right -1.5 dB`, naming the channel that is lowered. Its accessibility label is `Balance`.
- Tab or a press on it, the thumb's included, gives it focus, and its thumb then shows the focus
  ring. Left and Right move it by a step, and Home and End to its ends; gpui-kit's slider takes no
  keys, so these are its own actions in the `BalanceSlider` key context. Assistive technology's
  Increment and Decrement move it by a step too. The page's reset button, shown while it is off
  center, centers it.
- Each move goes to `DspSettings::set_balance`. The saver writes one value at a time and the last
  one wins, so a drag does not queue writes. Only the slider changes the balance, so it does not
  follow the settings, as `CacheLimitField` does not.
- **Crossfeed** is a `Switch`, as it takes effect without a further step.
- **Preset** is a radio group, like the resampler's Quality. The first preset is called `bs2b`
  rather than bs2b's own `Default`, which would read as the default setting next to a switch that
  is off by default.
- **Separation** and **Cutoff** are dropdowns, like the DSD values.
- Only Crossfeed has a description, as nothing on the screen says what it does or that it lowers
  the level. Separation's name already says that a higher value is weaker.
- Search keywords: `headphones` and `bs2b` on Crossfeed, `crossfeed` and `headphones` on its other
  rows, and `left`, `right` and `channel` on Balance. The page's title finds `dsp`.
- Both work on DSD, so neither gets a `dsd_note`.

## Core and hosts

- `DspSettings` is a handle of its own in `Settings`, beside `AudioSettings`, with its own
  `Published` state and `Saver`. The DSP page has nothing to share with the Audio page's
  settings, which are large enough already.
- `DspSnapshot::settings()` is what plays, its crossfeed `None` while crossfeed is off. It also
  gives what the page shows: `is_crossfeed_on`, `crossfeed_preset_ix`, the separation and cutoff
  options and values, `balance_db`, `balance_text` and `errors`. The crossfeed values stay while
  crossfeed is off.
- `DspSettings` has `set_balance(db)`, which rounds to 0.5 dB and clamps, `set_crossfeed(is_on)`,
  `select_crossfeed_preset(ix)`, `set_crossfeed_separation(value)`, `set_crossfeed_cutoff(value)`
  and `save_before_quitting`. What it gives the saver carries the crossfeed as written and as it is
  now, so that only the changed keys are written.
- `Hiphile::player` sets the snapshot's `settings()` on the player it makes, and
  `Workspace::follow_dsp_settings` passes each change on. `Player::set_dsp` calls
  `Engine::set_dsp`.
- `hiphile-cli` gets them through `Hiphile::player` too, and shows the snapshot's errors with the
  others.
- Everything is plain data (numbers, flags, an index), so a UniFFI layer maps it to records, and a
  mobile app can run `dsp::Processor` wherever its own pipeline needs it.

## Testing

`dsp`, with no device:

- Neutral settings leave samples bit for bit, also after moving back to neutral.
- The separation measured on a steady 25 Hz sine is within 0.05 dB of the setting, and center
  bass is at −3 dB. Every preset is checked at 44.1, 48, 88.2, 96, 192 and 384 kHz, and the treble
  values in the table are checked too.
- The highest possible peak, the sum of the absolute impulse responses, stays below full scale at
  44.1 and 768 kHz.
- Balance gains, rounding and clamping, and that no gain is ever above 1.
- Switching crossfeed and moving balance on a full-scale 1 kHz sine change no sample-to-sample step
  by more than the sine's own steepest one plus a small margin, and each transition ends after
  exactly 20 ms.
- `to_bits` and `from_bits` round-trip.
- After a signal and then 10 s of silence, every state is exactly zero.
- Channels after the first two are untouched, and so is a one-channel output.

`audio`, with no device, on `Converter`:

- With crossfeed, a left-only signal reaches the right channel, with and without a resampler, and
  on DoP's encoder too. Balance lowers its side.
- Neutral settings, also after crossfeed has been on, leave the samples bit for bit. This is
  checked where no resampler follows; with one, the processor returns at the same point, before
  the samples reach it.
- A `set_dsp` applies from the next packet.
- `Converter::following` keeps the processor's state at the same rate, so the joint has no step,
  and the converter it makes follows the settings as the one before it does. The gapless joint
  and a crossfade's incoming song both get their converter this way. Going on at another rate is
  checked in `dsp`, on `Processor::at_rate`.

On a device, marked `#[ignore]` and run by exact name on BlackHole through `HIPHILE_TEST_DEVICE`:
one PCM test and one DoP test in which a left-only tone with crossfeed reaches the right channel
at the expected level. Do not run the whole `--ignored` set, which can reach the default device.

`config`: round trips, presets with no custom keys, only the changes saved, the environment
overriding the file, values out of range named, and a custom separation read in tenths.

`hiphile-core`: Custom taking over the current values, the values staying while crossfeed is off,
only the changed keys written, what waits written before quitting, each value that cannot be read
named, the workspace passing the settings to the player from the start and on each change, and
`Hiphile::player` playing with the saved settings.

`settings`: the balance slider moving the balance by a 0.5 dB step and to its ends with the keys,
to where the pointer puts and drags it, and back to the center on reset. That the crossfeed rows
show only while it is on is left to the real app, as the page cannot be read back in a test.

## Checking in the real app

Drag Balance while a song plays, switch crossfeed and change its preset during a gapless joint and
during a crossfade, and play on DSD.

## Limits

- Only the first two channels are processed. An output with one channel gets neither.
- While either is on, the output is not bit-perfect, and the format line does not say so.
- Crossfeed lowers the level by 3 dB or more.
- A change is heard about 0.5 s later, once the ring buffer reaches it.
- One setting applies to every device, speakers included.
