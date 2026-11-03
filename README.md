# hiphile

A lightweight Navidrome client built in Rust with GPUI.

## Roadmap

- [ ] pure Rust audio backend
- [ ] automatic device sample rate switching
- [ ] local music library caching
- [ ] fuzzy search

## Workspace

Crates are split by capability. The protocol, transport, audio, playback, cache,
and configuration crates do not depend on GPUI, so the GUI and the CLI share them.

| Layer | Crates |
| --- | --- |
| Binaries | `hiphile` (GUI shell), `hiphile-cli` |
| GUI features | `account`, `library`, `player`, `search` |
| GUI shared | `session`, `actions`, `ui` |
| GPUI-independent | `subsonic`, `net`, `client`, `playback`, `audio`, `catalog`, `config` |

Feature crates never depend on each other; they communicate through `actions`.
`script/check-gpui-free` fails if a GPUI-independent crate gains a GPUI dependency.

## CLI

```sh
cargo run -p hiphile-cli -- ping --server https://music.example.com --username alice
# The password is read from HIPHILE_PASSWORD; --api-key or HIPHILE_API_KEY also works.
cargo run -p hiphile-cli -- devices
cargo run -p hiphile-cli -- search "blue in green"   # prints song IDs
cargo run -p hiphile-cli -- play <SONG_ID>...
cargo run -p hiphile-cli -- play --search "kind of blue"
```

`play` switches the output device to each file's sample rate when the device
supports it, and resamples only when it does not.
