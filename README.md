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
| GUI features | `account`, `home`, `library`, `player`, `playlists`, `search`, `settings` |
| GUI shared | `session`, `actions`, `ui` |
| GPUI-independent | `subsonic`, `net`, `client`, `playback`, `presence`, `audio`, `catalog`, `config` |

Feature crates never depend on each other; they communicate through `actions`.
`script/check-gpui-free` fails if a GPUI-independent crate gains a GPUI dependency.

## Settings

Settings live in `~/.config/hiphile/config.toml` on every platform:

```toml
server = "https://music.example.com"
username = "alice"
```

The app writes it on sign-in and keeps any other keys and comments in the file.
`HIPHILE_SERVER` and `HIPHILE_USERNAME` override it. Signing in saves the
password to the system keychain (Keychain on macOS, Credential Manager on
Windows, the Secret Service elsewhere), the next launch signs in with it, and
signing out deletes it.

To skip the keychain, for example while testing, add `password = "..."` to the
file by hand. It is stored as plain text and the app never writes it. While it
is there, it is used instead of the keychain for the file's `server` and
`username`, so the next launch signs in with it even after signing out. Signing
in to a different server or user removes it.

hiphile > Settings… chooses the output device and the sample rate, which take
effect at once and are kept as `output_device` and `sample_rate` (in Hz).
Without them, playback follows the system's default device and each song's own
rate. `HIPHILE_OUTPUT_DEVICE` and `HIPHILE_SAMPLE_RATE` override them.
Settings… also sets the song cache's limit, `song_cache_gib`, which
`HIPHILE_SONG_CACHE_GIB` overrides, shows how much the cache takes, and clears
it. It likewise sets the cover cache's limit, `cover_cache_gib`, which
`HIPHILE_COVER_CACHE_GIB` overrides.

Settings… also switches between light and dark, kept as `appearance` (`"light"`
or `"dark"`). Without it, the app follows the system. `HIPHILE_APPEARANCE`
overrides it, and takes `system` as well.

Settings… also chooses whether lyrics light up by word or by line, kept as
`lyrics_highlight` (`"word"` or `"line"`). Without it, they light up by word.
`HIPHILE_LYRICS_HIGHLIGHT` overrides it.

## Home

The app opens on albums to try, the songs played last, the albums played most and
the albums added last, all asked of the server: Subsonic has no method for the
songs, so they come from the albums played last, whose songs say when each was
played. Plays from other clients show the next time the list loads, which is on
signing in and after each play.

The albums to try are picked once a day and stay until the next. They come from
songs similar to the albums played last (`getSimilarSongs2`), one album per
artist, leaving out albums played in the last two weeks and those on the shelves
of the most played and of new ones. Similar songs need the server to be set up with Last.fm or the like;
the most played albums past that shelf and random ones fill what they leave open.

## Playlists

The Playlists tab lists the server's playlists and the songs of the one selected.
Playlists of other users that they don't let you edit can be played but not
changed. A song can be on a playlist more than once.

Edits show at once and are saved in the order they were made, one at a time. If a
save fails, the app says so, drops the edits waiting on that playlist and reads it
again, taking no more changes to its songs until it has; if that fails too, the
error takes the songs' place. Reordering or removing songs replaces the playlist's
whole song list (`createPlaylist`), so of two clients editing one playlist the last
one wins. Removing the last song takes it out by position (`updatePlaylist`)
instead, as a list of no songs may be read as no change. A request whose URL would
be too long goes by POST (the `formPost` extension).

## Scrobbling

Once half of a song has been listened to, or four minutes of a long one, the
app tells the server with `scrobble`, which updates the song's play count and
last played time, and forwards the play to Last.fm if the server is set up to.
Parts skipped by seeking do not count. Plays wait in `scrobbles.sqlite` in the
system data directory (`~/Library/Application Support/hiphile/` on macOS) until
the server has them, so those made without the network are sent with the next
play or launch. The app also tells the server when a song starts or resumes,
which shows it as now playing there without counting a play. The CLI does
neither.

## Discord

While a song plays and Discord is running on the same computer, the user's
status on Discord says they are listening, with the song's title, artist, album
and progress. Pausing or stopping takes it away. The CLI does not set a
status.

The cover shows only with an API key from [ImgBB](https://api.imgbb.com/), set
in hiphile > Settings… and kept in `config.toml` as `imgbb_api_key`, as plain
text. `HIPHILE_IMGBB_API_KEY` overrides it. Discord needs a public URL for the
cover, and the one on the server holds the credentials for it, so the app uploads
the image to ImgBB when a song with a new cover starts. **Anyone with the link can
see the image for 180 days**, after which ImgBB deletes it. Nothing but the image
goes up. `cover_links.sqlite` in the system cache directory
(`~/Library/Caches/hiphile/` on macOS) records the links, so an image goes up again
only once its link has run out. Without a key, nothing is uploaded.

## Library cache

Each server's albums and songs are kept in a SQLite database in the system
cache directory (`~/Library/Caches/hiphile/servers/` on macOS), shared by the
app and the CLI and kept after signing out. The app shows it at once and
updates it in the background when the server reports a change or the copy is
a day old; Library > Refresh Library fetches everything again. A sync removes
what the server no longer has only after it completes, and not while the
server is scanning. Cover art is kept beside each library once downloaded
and fetched again once it is 30 days old, up to 8 GiB in all unless `config.toml` sets `cover_cache_gib` (`0` means no
limit, and a value that cannot be read keeps no more); past that,
the images used longest ago are removed.
A song that arrives whole is kept there too, and plays from the disk next
time, as long as the server still reports the same file size; a kept file
that turns out to be damaged is removed and the song fetched again. Songs
share 8 GiB unless `config.toml` sets `song_cache_gib` (`0` means no limit, and
a value that cannot be read keeps none); past that, the songs played longest
ago are removed the next time a song is kept. A song larger than three quarters of that is not
kept.
The cache holds no credentials, and a version of it the app cannot read is
emptied and filled again.

## CLI

The CLI uses the app's last sign-in by default: its server, its user, and the
password in `config.toml` or else the keychain. Options or environment variables
override each.

```sh
cargo run -p hiphile-cli -- ping
cargo run -p hiphile-cli -- ping --server https://music.example.com --username alice
# The password is read from --password, HIPHILE_PASSWORD, config.toml, or the keychain;
# --api-key or HIPHILE_API_KEY also works.
cargo run -p hiphile-cli -- devices
cargo run -p hiphile-cli -- sync                 # fetch every album and song into the cache
cargo run -p hiphile-cli -- albums               # list cached albums, updating them when stale
cargo run -p hiphile-cli -- albums --offline     # list them without contacting the server
cargo run -p hiphile-cli -- search "blue in green"   # prints song IDs
cargo run -p hiphile-cli -- play <SONG_ID>...
cargo run -p hiphile-cli -- play --search "kind of blue"
```

`play` uses the output device and sample rate from the app's settings. Without
a fixed rate, it switches the device to each file's rate when the device
supports it, and resamples only when it does not.

The workspace has a `config` crate, and so does crates.io (config-rs, used inside
it as `config_rs`). Name ours by version where Cargo asks: `cargo test -p config@0.1.0`.
