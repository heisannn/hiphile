# Hiphile

English | [日本語](README.ja.md)

A native Navidrome client built with Rust and GPUI, with a focus on performance.

Hiphile is developed on macOS. Windows and Linux are not regularly tested.

![Home](docs/screenshots/home.png)

| Albums | Playlists | Lyrics |
| --- | --- | --- |
| ![Albums](docs/screenshots/albums.png) | ![Playlists](docs/screenshots/playlists.png) | ![Lyrics](docs/screenshots/lyrics.png) |

## Features

- **Library:** Browse albums, artists and songs, get daily recommendations on the home screen, edit playlists, and keep favorites.
- **Playback:** Gapless playback, on PCM trimming the silence around songs, crossfade and fading on play and pause, repeat, shuffle and an editable queue.
- **Audio output:** Choose an output device, and switch its sample rate automatically to match each file or a multiple of it. On macOS and Windows, keep other apps off the device with exclusive mode. On macOS, play as DSD up to DSD256 through DoP, with the filter and the modulator adjustable. See [how DSD output works](docs/dsd/implementation.md). Resample with Rubato, with a built-in port of r8brain-free-src, or with a built-in resampler that has the functions of libsoxr. See how [Rubato](docs/resampler/rubato.md), [r8brain](docs/resampler/r8brain.md) and [soxr](docs/resampler/soxr.md) work.
- **DSP:** Crossfeed for headphones, with the presets of Boris Mikhaylov's [bs2b](https://bs2b.sourceforge.net/) or values of your own, and balance between the channels, on PCM and DSD alike. See [how they work](docs/dsp/design.md).
- **Lyrics:** Synced lyrics, highlighted word by word or line by line.
- **Mini player:** A small window above all others, with the controls and the lyrics.
- **Local cache:** Library data, cover art and songs are cached on disk, and library data syncs in the background.
- **Scrobbling:** Report plays and the current song to your server. Plays made offline are sent later.
- **Media controls:** Control playback with the media keys and see the current song in your system's media controls.
- **Discord:** Show the current song as your Discord status.
- **CLI:** Sync, search and play from the terminal.

## Install

Download a build for macOS or Windows from [Releases](https://github.com/heisannn/hiphile/releases).

The builds are not signed with a developer certificate, so the system warns about them:

- **macOS:** Move `hiphile.app` to Applications, then run
  `xattr -dr com.apple.quarantine /Applications/hiphile.app` before the first launch.
- **Windows:** If SmartScreen stops the app, choose **More info**, then **Run anyway**.

From then on, the app installs new versions by itself, and they start the next time it opens.
Turn this off in **Settings > Updates**.

## Getting started

```sh
cargo run                            # Run the app
cargo run -p hiphile-cli -- --help   # Show CLI usage
```

By default, the CLI uses the app's most recent sign-in. Command-line options and
environment variables override it.

## Configuration

Change settings in the app under **hiphile > Settings…**. They are saved to
`~/.config/hiphile/config.toml`, and your password is saved to the system keychain.

| Key | Environment variable | Default |
| --- | --- | --- |
| `server` | `HIPHILE_SERVER` | None |
| `username` | `HIPHILE_USERNAME` | None |
| `output_device` | `HIPHILE_OUTPUT_DEVICE` | System default |
| `sample_rate` (Hz, or `x2`, `x4`, `x8` or `x16` for that multiple of each file's rate, or the next lower one the device supports; on Windows, multiples need `exclusive_mode`) | `HIPHILE_SAMPLE_RATE` | Each file's rate |
| `exclusive_mode` (`true` keeps other apps off the output device while playing or paused; macOS uses hog mode and Windows WASAPI exclusive mode) | `HIPHILE_EXCLUSIVE_MODE` | `false` |
| `trim_silence` (`true` leaves out the silence around songs and any longer than 10 seconds inside them; PCM only) | `HIPHILE_TRIM_SILENCE` | `false` |
| `play_pause_fade` (`true` fades the sound in and out when playing, pausing and stopping; PCM only) | `HIPHILE_PLAY_PAUSE_FADE` | `false` |
| `crossfade_seconds` (from `0` to `12`; PCM only; songs that need the device at another rate play apart, so set a fixed `sample_rate` to crossfade them all) | `HIPHILE_CROSSFADE_SECONDS` | `0` |
| `resampling` (`medium`, `high`, `max`, `custom`, `r8brain`, a built-in port of [r8brain-free-src](https://github.com/avaneev/r8brain-free-src), or `soxr`, a built-in resampler with the functions of [libsoxr](https://sourceforge.net/projects/soxr/), neither of which takes an FFT size or window) | `HIPHILE_RESAMPLING` | `medium` |
| `r8brain_quality` (with `r8brain`: `low`, `medium`, `high`, `max` or `custom`) | `HIPHILE_R8BRAIN_QUALITY` | `high` |
| `r8brain_transition_band` (with `r8brain_quality = "custom"`: percent, from 0.5 to 45) | `HIPHILE_R8BRAIN_TRANSITION_BAND` | `2` |
| `r8brain_attenuation` (with `r8brain_quality = "custom"`: decibels, from 49 to 218) | `HIPHILE_R8BRAIN_ATTENUATION` | `180.15` |
| `r8brain_phase` (with `r8brain_quality = "custom"`: `linear` or `minimum`) | `HIPHILE_R8BRAIN_PHASE` | `linear` |
| `r8brain_max_input_frames` (with `r8brain`: how many go through at a time, from 1 to 1048576) | `HIPHILE_R8BRAIN_MAX_INPUT_FRAMES` | `1024` |
| `r8brain_extended_fft` (with `r8brain`: doubles the filter's FFT block, usually faster with more delay) | `HIPHILE_R8BRAIN_EXTENDED_FFT` | `false` |
| `r8brain_whole_stepping` (with `r8brain_quality = "custom"`: whether rates at a ratio of small whole numbers use a filter per position) | `HIPHILE_R8BRAIN_WHOLE_STEPPING` | `true` |
| `r8brain_filter_fractions` (with `r8brain_quality = "custom"`: positions of the interpolating filters, from 5 to 65536) | `HIPHILE_R8BRAIN_FILTER_FRACTIONS` | chosen from the attenuation |
| `r8brain_filter_cache_size` (with `r8brain`: filters kept for later songs, from 2 to 4096) | `HIPHILE_R8BRAIN_FILTER_CACHE_SIZE` | `96` |
| `r8brain_filter_bank_cache_size` (with `r8brain`: filter banks kept for later songs, from 2 to 4096) | `HIPHILE_R8BRAIN_FILTER_BANK_CACHE_SIZE` | `12` |
| `soxr_quality` (with `soxr`: `low`, `medium`, `high`, `very-high` or `custom`) | `HIPHILE_SOXR_QUALITY` | `very-high` |
| `soxr_precision` (with `soxr_quality = "custom"`: bits, from 15 to 33) | `HIPHILE_SOXR_PRECISION` | `28` |
| `soxr_passband_end` (with `soxr_quality = "custom"`: where the filter starts removing highs, a fraction of the lower rate's Nyquist frequency, from 0.5 to 0.998) | `HIPHILE_SOXR_PASSBAND_END` | `very-high`'s, about `0.911` |
| `soxr_phase` (with `soxr_quality = "custom"`: `linear`, `intermediate` or `minimum`) | `HIPHILE_SOXR_PHASE` | `linear` |
| `soxr_double_precision` (with `soxr_quality = "custom"`: computes in 64-bit floating point at 20 bits or less too) | `HIPHILE_SOXR_DOUBLE_PRECISION` | `false` |
| `soxr_coef_table_kb` (with `soxr`: the most memory for the filter's steps between samples, in kB, from 1 to 65536) | `HIPHILE_SOXR_COEF_TABLE_KB` | `400` |
| `soxr_coef_interpolation` (with `soxr`: `auto`, `linear` or `cubic`, how the steps it doesn't keep are worked out) | `HIPHILE_SOXR_COEF_INTERPOLATION` | `auto` |
| `soxr_min_dft_size` (with `soxr`: a power of two from 256 to 32768) | `HIPHILE_SOXR_MIN_DFT_SIZE` | `1024` |
| `soxr_large_dft_size` (with `soxr`: a power of two from 256 to 1048576) | `HIPHILE_SOXR_LARGE_DFT_SIZE` | `131072` |
| `resampling_fft_size` (with `custom`: a power of two from 256 to 16384) | `HIPHILE_RESAMPLING_FFT_SIZE` | `256` |
| `resampling_window` (with `custom`: `hann`, `hann-2`, `blackman`, `blackman-2`, `blackman-harris` or `blackman-harris-2`) | `HIPHILE_RESAMPLING_WINDOW` | `blackman-harris-2` |
| `output_format` (`pcm` or `dsd`; macOS, for the device in `dop_device`) | `HIPHILE_OUTPUT_FORMAT` | `pcm` |
| `dop_device` (the output device DSD is on for) | `HIPHILE_DOP_DEVICE` | None |
| `dsd_rate` (`64`, `128` or `256`) | `HIPHILE_DSD_RATE` | `256`; turning DSD on in Settings picks the highest the device has |
| `dsd_base_rate` (`44100`, or `match` to play songs of the 48 kHz family at DSD rates based on 48 kHz, which some DACs don't decode) | `HIPHILE_DSD_BASE_RATE` | `44100` |
| `dsd_fade_in` (whether sound rises over 3 seconds when DSD turns on or its settings change) | `HIPHILE_DSD_FADE_IN` | `true` |
| `dsd_quality` (`low`, `medium`, `high`, `max` or `custom`) | `HIPHILE_DSD_QUALITY` | `high` |
| `dsd_filter` (with `custom`: `sinc` or `fft`) | `HIPHILE_DSD_FILTER` | `sinc` |
| `dsd_sinc_length` (with `sinc`: `256`, `512`, `1024` or `2048`) | `HIPHILE_DSD_SINC_LENGTH` | `1024` |
| `dsd_fft_size` (with `fft`: a power of two from 256 to 16384) | `HIPHILE_DSD_FFT_SIZE` | `4096` |
| `dsd_window` (with `custom`: as `resampling_window`) | `HIPHILE_DSD_WINDOW` | `blackman-harris-2` |
| `dsd_cutoff` (with `sinc`: a fraction of the source's Nyquist frequency, from `0.5` to `1`) | `HIPHILE_DSD_CUTOFF` | Chosen by the resampler |
| `dsd_order` (with `custom`: `3`, `5`, `7` or `9`) | `HIPHILE_DSD_ORDER` | `7` |
| `dsd_noise_gain` (with `custom`: `1.2` to `1.5`) | `HIPHILE_DSD_NOISE_GAIN` | `1.4` |
| `dsd_noise_band` (with `custom`: `20000` or `48000`) | `HIPHILE_DSD_NOISE_BAND` | `20000` |
| `dsd_optimized_zeros` (with `custom`) | `HIPHILE_DSD_OPTIMIZED_ZEROS` | `true` |
| `dsd_dither_db` (with `custom`: `-160` to `-20`) | `HIPHILE_DSD_DITHER_DB` | None |
| `dsd_input_level_db` (with `custom`: `-24` to `6`) | `HIPHILE_DSD_INPUT_LEVEL_DB` | `0` |
| `dsd_limiter_ceiling_db` (with `custom`: `off`, or `-6` to `3`) | `HIPHILE_DSD_LIMITER_CEILING_DB` | `1` |
| `crossfeed` (`true` feeds part of each channel into the other, as speakers do, and lowers the level by 3 dB so that loud songs don't clip) | `HIPHILE_CROSSFEED` | `false` |
| `crossfeed_preset` (`bs2b`, `chu-moy`, `jan-meier` or `custom`; the first three cut off at 700, 700 and 650 Hz and separate by 4.5, 6 and 9.5 dB) | `HIPHILE_CROSSFEED_PRESET` | `bs2b` |
| `crossfeed_cutoff_hz` (with `custom`: where what each channel feeds into the other starts to fall away, from 300 to 2000) | `HIPHILE_CROSSFEED_CUTOFF_HZ` | `700` |
| `crossfeed_separation_db` (with `custom`: how much quieter each channel's low end is in the other, from 1 to 15, read in tenths) | `HIPHILE_CROSSFEED_SEPARATION_DB` | `4.5` |
| `balance_db` (from `-12` to `12`; a negative value lowers the right channel and a positive one the left) | `HIPHILE_BALANCE_DB` | `0` |
| `skip_instrumentals` (grey out songs whose titles mark them as instrumental, and keep them out of the queue; the app only, not `hiphile-cli`) | `HIPHILE_SKIP_INSTRUMENTALS` | `false` |
| `song_cache_gib` (`0` for no limit) | `HIPHILE_SONG_CACHE_GIB` | `8` |
| `cover_cache_gib` (`0` for no limit) | `HIPHILE_COVER_CACHE_GIB` | `8` |
| `appearance` (`"light"` or `"dark"`) | `HIPHILE_APPEARANCE` (also accepts `system`) | System setting |
| `lyrics_highlight` (`"word"` or `"line"`) | `HIPHILE_LYRICS_HIGHLIGHT` | `"word"` |
| `romaji_search` (find kanji by romaji; downloads a dictionary) | `HIPHILE_ROMAJI_SEARCH` | `true` |
| `discord_presence` (show the current song as your Discord status) | `HIPHILE_DISCORD_PRESENCE` | `true` |
| `auto_update` (install new versions by itself) | `HIPHILE_AUTO_UPDATE` | `true` |
| `imgbb_api_key` | `HIPHILE_IMGBB_API_KEY` | None |

## Roadmap

- [x] Pure Rust audio backend
- [x] Automatic device sample rate switching
- [x] Local music library caching
- [x] Fuzzy search
- [x] DSD output over DoP

## Disclaimer

This project is AI-generated and is provided "as is", without warranty of any kind.
The author accepts no responsibility for any damage or loss resulting from its use,
including harm to your data, accounts or audio equipment. Use it at your own risk.

hiphile is not affiliated with or endorsed by Navidrome, Musixmatch, PetitLyrics,
ImgBB or Discord. It uses third-party services as follows:

- **Lyrics:** Song details are sent to Musixmatch and PetitLyrics through their
  unpublished APIs, which may stop working at any time. Their terms may prohibit
  this kind of use, so please review them yourself.
- **Cover art on Discord:** If you set an ImgBB API key, cover images are uploaded
  to ImgBB, where **anyone with the link can view them for 180 days**. Without a
  key, nothing is uploaded.

## License

[MIT](LICENSE). The dictionaries that `romaji_search` downloads stay under their own terms, listed
in [script/readings-notices/NOTICE.txt](script/readings-notices/NOTICE.txt).

The `r8brain` resampling is a port of r8brain-free-src, under [its MIT license](crates/r8brain/LICENSE).
Sample rate converter designed by Aleksey Vaneev of Voxengo.
