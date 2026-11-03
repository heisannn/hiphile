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

## Settings

Settings live in `~/.config/hiphile/config.toml` on every platform:

```toml
server = "https://music.example.com"
username = "alice"
```

The app writes it on sign-in and keeps any other keys and comments in the file.
`HIPHILE_SERVER` and `HIPHILE_USERNAME` override it. Passwords never go in the
file: signing in saves the password to the system keychain (Keychain on macOS,
Credential Manager on Windows, the Secret Service elsewhere), the next launch
signs in with it, and signing out deletes it.

## Library cache

Each server's albums and songs are kept in a SQLite database in the system
cache directory (`~/Library/Caches/hiphile/servers/` on macOS), shared by the
app and the CLI and kept after signing out. The app shows it at once and
updates it in the background when the server reports a change or the copy is
a day old; Library > Refresh Library fetches everything again. A sync removes
what the server no longer has only after it completes, and not while the
server is scanning. Cover art is kept beside each library once downloaded
and fetched again once it is 30 days old, up to 512 MiB in all; past that,
the images used longest ago are removed.
A song that arrives whole is kept there too, and plays from the disk next
time, as long as the server still reports the same file size; a kept file
that turns out to be damaged is removed and the song fetched again. Songs
share 8 GiB unless `config.toml` sets `song_cache_gib` (`0` keeps none, and
a value that cannot be read keeps none either); past that, the songs played
longest ago are removed. A song larger than three quarters of that is not
kept.
The cache holds no credentials, and a version of it the app cannot read is
emptied and filled again.

## CLI

The CLI uses the app's last sign-in by default: its server, its user, and the
password saved in the keychain. Options or environment variables override each.

```sh
cargo run -p hiphile-cli -- ping
cargo run -p hiphile-cli -- ping --server https://music.example.com --username alice
# The password is read from --password, HIPHILE_PASSWORD, or the keychain;
# --api-key or HIPHILE_API_KEY also works.
cargo run -p hiphile-cli -- devices
cargo run -p hiphile-cli -- sync                 # fetch every album and song into the cache
cargo run -p hiphile-cli -- albums               # list cached albums, updating them when stale
cargo run -p hiphile-cli -- albums --offline     # list them without contacting the server
cargo run -p hiphile-cli -- search "blue in green"   # prints song IDs
cargo run -p hiphile-cli -- play <SONG_ID>...
cargo run -p hiphile-cli -- play --search "kind of blue"
```

`play` switches the output device to each file's sample rate when the device
supports it, and resamples only when it does not.

The workspace has a `config` crate, and so does crates.io (config-rs, used inside
it as `config_rs`). Name ours by version where Cargo asks: `cargo test -p config@0.1.0`.
