# Extracting the core

Planned on 2026-11-09. The aim is to move every piece of state and logic that does not draw into crates
without GPUI, so that a mobile app made later can reuse the core through [UniFFI](https://mozilla.github.io/uniffi-rs/),
from Swift and Kotlin. The same core serves the desktop app, the CLI and a move to Native Shell, should
it happen (see the [NSH evaluation](../nsh/evaluation.md)), and the logic can be tested without a window.

| | |
| --- | --- |
| Goal | Only views depend on GPUI, and they hold only view state: focus, selection, scroll, slider drags, animation |
| Size | The 15 crates that depend on gpui-kit hold about 41,000 lines, 10,000 of them tests: about 420 `#[gpui_kit::test]` and 170 `#[test]` |
| Order | Pure files first, then one service at a time, the player last (see [Steps](#steps)) |
| Check | `script/check-gpui-free` lists every crate that must stay free of GPUI; `script/check-mobile` builds `hiphile-core` for iOS and fails on a desktop-only dependency |

## Progress

| Step | State |
| --- | --- |
| 0 | Done. `executor` (the trait, `Task`, the tokio implementation, `Saver`), `gpui_host::executor` (GPUI's, at first in `ui`), an empty `hiphile-core`; the check covers the new crates and the five that were free but unchecked |
| 1 | Done. The [pure files](#free-of-gpui-but-inside-gpui-crates) are in `hiphile-core` and `playback`, with three items they used from GPUI crates; the views use them there |
| 2 | Done. `updater` is free of GPUI and the rules held; see [The updater pilot](#the-updater-pilot) |
| Before 3 | `gpui-host` split from `ui`, and the [decisions](#decided-before-steps-3-to-6) for steps 3 to 6 |
| 3 | Done. `session` is free of GPUI and checked: `Readings` is a handle, the sign-in procedure is `session::Accounts`, and GPUI's HTTP client is in `ui` (`ui::set_http_client`; the globals for covers and readings it had then went in step 8) |
| 4 | Done. The settings models are handles in `hiphile_core::settings`; the `settings` crate keeps the window, its fields and applying the appearance. The seven `*Changed` events are gone |
| 5 | Done. `FavoritesState` and `PlaylistsState` in `hiphile-core` load, keep and write the favorites and the playlists; `is_instrumental` and what is skipped are in `playback`. The `actions` globals hold copies the shell feeds from the handles. See [Found in steps 5 and 6](#found-in-steps-5-and-6) |
| 6 | Done. `LibrarySync`, `LibraryActivity`, `SearchIndex` and `DailyRecommendations` in `hiphile-core`; `LibraryEvent` keeps only `LeftAlbum`, the rest are `LibraryNotice`s |
| 7 | Done. `playback::Player` holds the queue, the states, the engine, scrobbling and the volume; `hiphile_core::desktop` (`DiscordStatus`, `SystemMediaControls`) follows it; the current song's lyrics and their timing are `lyrics::CurrentLyrics` and `lyrics::timing`. See [Found in step 7](#found-in-step-7) |
| 8 | Done. `hiphile_core::Hiphile` is the app's core: the settings, `Readings`, the cover cache, signing in and the signed-in `hiphile_core::Workspace`, which makes the handles and routes between them. `hiphile-cli` signs in, syncs and lists through it on tokio. See [Found in step 8](#found-in-step-8) |
| 9 | Done. The workspace's handles load what the views loaded: `artists::ArtistList` and `ArtistDetail`, `songs::SongList`, `home::HomeShelves`, all into `hiphile_core::Shelf`; an album's detail is `library::AlbumDetail` and its songs for an action `library::album_songs`; `FavoritesState::toggle` picks the direction. A host gives the cache folder (`Caches::At`), the settings window counts kept songs through the core, and `hiphile-cli play` plays through `Hiphile::player`. See [Found in step 9](#found-in-step-9) |
| 10 | Done. What the survey after step 9 found: the settings' setters check what they are given, the albums', artists' and playlists' lists are indexed and arranged in the core on one shape (`Shown<T>`), the workspace's notices are `alerts` worded in the core, showing a screen, Refresh and resuming a sign-in are the core's, and the player's and the lyrics' rules left in views moved. See [Found in step 10](#found-in-step-10) |

## Where things stand

As planned on 2026-11-09; [Progress](#progress) says what has moved since.

### Already free of GPUI

`subsonic`, `client`, `net`, `audio`, `playback`, `presence`, `catalog`, `config`, `fuzzy` and
`hiphile-cli` were checked by `script/check-gpui-free`. `lyrics`, `media-controls`, `dsd`, `r8brain` and `soxr`
were free of GPUI too but not checked; step 0 added them.

They already work with any executor, which is what lets `hiphile-cli` run them with
`futures::executor::block_on`:

| Crate | Threads | How it reports |
| --- | --- | --- |
| `net` | Its own tokio runtime (2 workers) for reqwest | Futures that complete through `futures::channel::oneshot`. The last `Client` must not be dropped inside a tokio runtime |
| `audio` | An engine thread, a decoder thread per track, the device callback; samples cross on `rtrb` | Commands on `mpsc`, events on `async_channel`; the position is an atomic frame count that callers poll |
| `playback` | None | `open_track` is async but blocks on the song cache; `Queue` is plain data |
| `catalog` | None | rusqlite behind a `Mutex`: every method blocks on the disk. Sync reports through a `Fn(SyncProgress)` |
| `config` | None | TOML and the keychain, both blocking; the keychain can show a system prompt |
| `lyrics` | None | `Finder::find_lyrics` is async over `net` |
| `presence` | A `discord-presence` thread | Fire and forget over `std::sync::mpsc` |
| `media-controls` | souvlaki | Commands on `async_channel`; needs the window handle (the HWND on Windows) |

### Free of GPUI but inside GPUI crates

These files reference no GPUI type and moved as they were in step 1, their tests with them:

| File | What it holds | Moved to |
| --- | --- | --- |
| `hiphile/src/navigation.rs` | `History`, `Location` and the rule for what is worth going back to | `hiphile_core::navigation` |
| `library/src/order.rs`, `library/src/filter.rs` | Album sort orders and the album filter | `hiphile_core::library` |
| `songs/src/arrange.rs` | Song columns, sorts and the filter index | `hiphile_core::songs` |
| `artists/src/list.rs` | Artist sorts and dates, albums by artist | `hiphile_core::artists` |
| `favorites/src/lists.rs` | `FavoriteLists` | `hiphile_core::favorites` |
| `playlists/src/entries.rs`, `playlists/src/writes.rs` | Playlist entries and the serial write queue | `hiphile_core::playlists` |
| `search/src/results.rs` | `SearchLibrary`, hits and results | `hiphile_core::search` |
| `home/src/recommend.rs`, `home/src/recent.rs`, `home/src/layout.rs` | Daily recommendations, recently played, the shelf layout | `hiphile_core::home` |
| `player/src/format.rs` | The playing format's text | `playback` (`describe_format`) |
| `updater/src/install.rs` | Releases and installing with `self_update` | Stays: step 2 frees `updater` itself |

Three of them used an item of a GPUI crate, which moved along: `Destination` from `app_shell.rs` (into
`hiphile_core::navigation`), `actions::Favorite` (into `hiphile_core::favorites`) and
`ui::format_sample_rate` (into `playback`, which the settings window now uses for its sample rates). The
test in `layout.rs` steps through `ui::card_grid`, so it stays in home's view tests. Each domain of
`hiphile-core` is a module named after its view crate that re-exports what the views use. Items that
became public follow the coding guide: public structs with public fields are `#[non_exhaustive]`, which
gave `Recommendations` a `new`, and `History` a `Default`.

### Bound to GPUI

| Crate | Logic to move | How GPUI is involved |
| --- | --- | --- |
| `player` | Most of `player.rs`: the states (`State`, `Resume`, `DopStart`), play / next / previous / seek / stop, opening and preparing the next song, engine events, failures and their text, queue operations, skipping instrumentals, scrobbling, Discord, OS media controls, the volume and its save. `lyrics_view/current.rs` (fetching, the cache of 64) and the line timing in `lyrics_view/lines.rs` | `Player` is an `Entity` that also holds view state: three `Entity<SliderState>` (the volume level lives in its slider), `is_cover_hidden`, `set_panels(Fn(&Window, &App))`. `MediaControls::connect(window)` takes the GPUI `Window`; seeking takes `&mut Window`; covers come through `cx.http_client()`; the position refresh is a `background_executor().timer` loop; `cx.on_app_quit` calls `engine.shutdown()`; `actions::is_skipped` reads a `Global` |
| `hiphile` | In `app_shell.rs`: sending now-playing to the screens, library events to the activity and the screens, settings to the player and the session; the session's lifetime (sign in, open the workspace, sign out). `library_activity.rs`: `Status` and when the notice shows | Wiring of `Entity`s through 15 `Subscription`s, `push_notification`, `set_menus` |
| `library` | `run_sync` and `watched_sync`: the stages of a sync, its progress and the events that follow | `run_sync(&WeakEntity<Library>, &mut AsyncApp)`, progress emitted with `cx.emit` |
| `session` | `Session` (client, catalog, scrobbles, cover URLs kept per session), `describe`, `Readings` (the dictionary download) | `init` replaces GPUI's HTTP client with `NetHttpClient`, which serves covers from `catalog::CoverStore`; `GlobalCovers` and `GlobalReadings`; `reading_book(cx)`; URLs as `SharedString` |
| `account` | The sign-in procedure: resume, validate, ping, open the catalog and scrobbles, remember the profile and the password | Reads the form's `Entity<InputState>`s; `SignInEvent` |
| `settings` | The models in `*_settings.rs`: values, what is unsaved, choices, errors, the serial save | `Entity` and `EventEmitter` (`OutputChanged`, ...), `GlobalSettings`, choices as `SharedString` pairs, device listing with `background_spawn` |
| `favorites` | The known favorites and their optimistic writes | `KnownFavorites` is a `Global` in `actions` |
| `playlists` | Running the write queue, reading back on failure, the playlist choices | `PlaylistChoices` is a `Global` in `actions` |
| `home` | The daily cache of recommendations | In the view |
| `search` | Building the index and running searches | In the view |
| `updater` | The 6-hour check, installing automatically, the saved choice | `Entity`, `GlobalUpdater`, `UpdateState(SharedString)`, `cx.restart()` |
| `actions` | `is_instrumental` and the skip setting | `SkipInstrumentals` is a `Global` |

## Design rules

1. **No UI framework in a core crate.** Each one is added to `script/check-gpui-free`.
2. **Handles are `Clone + Send + Sync + 'static`**, with an `Arc` inside, so any host can keep one in a
   global, a task or another thread.
3. **State changes synchronously and is published as a snapshot.** A handle changes its state under its own
   lock and then sends the new snapshot on a `tokio::sync::watch` (the core's `cx.notify()`). One-off notices
   go on a channel (`async_channel`, as `audio` does, or `tokio::sync::broadcast`). `tokio::sync` needs no
   tokio runtime. No lock is held across an `.await`. Changing state synchronously keeps what views and
   tests rely on today: a call's effect is visible when it returns. `executor::Published<S>` is that lock
   and `watch`: `update` publishes `S`'s snapshot before it lets go of the lock.
   A result applies only to the work it was started for. On GPUI's foreground a replaced task never ran
   again; on background threads one may finish after it was replaced, so a handle numbers its work and
   drops the result of work that was replaced (`Readings` keeps a `work_id`).
4. **Tasks and time come from an injected executor.** The core defines a small trait (spawn, spawn blocking
   work, a timer); the task handle cancels on drop, as GPUI's `Task` does. GPUI hosts pass GPUI's
   `BackgroundExecutor`, so the 30 test files that use `run_until_parked` and the 5 that use `advance_clock`
   still drive the core deterministically. Hosts on tokio (the CLI, NSH) pass a tokio one. The core never
   starts a runtime of its own. In `executor`: a host implements `Spawner` (`spawn`, `spawn_blocking`,
   `timer`), which returns a `Spawned` handle per piece of work that cancels it on drop and can be
   detached. The core keeps an `Executor` (an `Arc<dyn Spawner>`), whose `spawn` and `spawn_blocking`
   give a typed `Task<R>` that cancels on drop as well. A host must not run the work before `spawn`
   returns, as the core spawns under its own locks. GPUI's is `gpui_host::executor(cx)`; tokio's is
   `Executor::tokio(handle)`, whose timers keep to that runtime's clock whatever polls them, so a test on
   a current-thread runtime with paused time moves them with `tokio::time::advance`. The core reads no
   clock of its own for waiting (`Instant::now()`): a wait is the executor's timer, which tests move.
5. **Blocking work only through the executor's blocking spawn:** rusqlite, `CoverStore`, `SongCache`, config
   writes, the keychain, `self_update`, the fuzzy reader, `audio::output_devices`. Reading the saved
   settings in a constructor is the exception (see [Decided before steps 3 to 6](#decided-before-steps-3-to-6)).
   The catalog's sync is async but blocks on the disk between its requests, so `LibrarySync` runs it
   with `block_on` on a blocking thread and tells it to stop at its next await once it is dropped. A
   sync now holds one of GPUI's background threads while it waits on the network.
6. **Text is `String` or `Arc<str>`, not `SharedString`.** Error text is made in the core (the `describe`
   functions), so every UI says the same thing.
7. **View state stays in views.** Slider drags, focus, selection, scroll, which panels are open and
   animation are the view's. The core takes values (`seek_to(Duration)`, `set_volume(level)`).
8. **The host gives the core what only a window has.** The window handle that media controls need comes in
   once the workspace is open (`Workspace::connect_media_controls`). Raising the window becomes a notice that
   the host acts on. The host calls `Hiphile::shutdown()` when the app quits, which writes what waits,
   clears the system's media controls and restores the device's sample rate and hog mode.
9. **Saves are serial and coalesced:** one write at a time, the last value wins. One helper replaces the
   four copies in `Player::save_volume`, `AudioSettings`, `Updater::save` and `Readings::save_choice`.
   It is `executor::Saver<T>`: `save(value)` replaces the value still waiting, and `save_with(|waiting| …)`
   builds on it, for `AudioSettings`, which writes only the fields that changed. The write runs on a
   blocking thread and reports through the owner's closure, and what was given is written even if the
   saver is dropped first. `take_waiting` hands the host the value still waiting, for a write before
   quitting: `Hiphile::shutdown()` calls `save_before_quitting()` on the handles that hold a typed value not
   yet committed, and stops the player.
10. **Constructors take their stores.** Before step 7, `Player::new` used the user's real config store, so a
    test that changed the volume changed the user's saved volume. Core constructors take the config store
    (and the catalog, the caches), and tests give them temporary ones.
11. **One `net::Client` for the life of the process**, owned by the app, with clones elsewhere, so the last
    one is never dropped inside a tokio runtime.

## Target layout

Confirmed in step 0: with every edge below added to the manifests, `cargo metadata` finds no cycle, and
`playback`, `session`, `updater`, `lyrics`, `media-controls` and `hiphile-core` reach no GPUI once
`session` and `updater` drop gpui-kit. Existing crates whose purpose already covers the logic take it.
Everything else goes into one new crate, apart from the executor.

| Logic | From | To |
| --- | --- | --- |
| The executor trait, `Task`, the tokio implementation, the serial saver | New | New `executor`, under every core crate. Not `hiphile-core`: `updater`, `session` and `playback` need it and `hiphile-core` depends on them, a cycle cargo rejects |
| The player's state machine, queue operations, preparing the next song, scrobbling, the volume | `player/src/player.rs` | `playback`, whose description is already "Play queue and playback orchestration. No GPUI." |
| Showing what plays on Discord and in the system's media controls, and taking their commands | `player/src/player.rs` | `hiphile_core::desktop`, behind `hiphile-core`'s `desktop` feature, which only the desktop app turns on. They follow the player as any other host would (see [Decided before steps 7 and 8](#decided-before-steps-7-and-8)) |
| The format text, `is_instrumental` and the skip setting | `player/src/format.rs`, `actions/src/instrumentals.rs` | `playback` |
| Fetching the current song's lyrics, their cache, the line and word timing | `player/src/lyrics_view/current.rs`, `lyrics_view/lines.rs` | `lyrics` |
| `Session`, cover URLs, `describe`, `Readings` | `session` | `session`, made free of GPUI (done in step 3). `NetHttpClient` and the `Global`s are in `ui` |
| The sign-in procedure | `account/src/sign_in.rs` | `session::Accounts` (done in step 3). `account` keeps the form |
| The update check, automatic installs, the saved choice | `updater` | `updater`, made free of GPUI. The app makes it and hands it to the settings window and the shell, so `GlobalUpdater` is gone; the views call `cx.restart()` |
| The app and the signed-in workspace, routing between them, the settings models, the library sync and its activity, favorites, playlists, the search index, home recommendations, the sort and filter modules, navigation history | `hiphile`, `settings`, `library`, `favorites`, `playlists`, `search`, `home`, `songs`, `artists` | New `hiphile-core` |

`hiphile-core` depends on `playback`, `session`, `updater`, `lyrics` and the crates under them, `executor`
among them; the view crates depend on `hiphile-core`, and so does `actions` (and through it `ui`) for
`Favorite` and `PlaylistChoice`, and on `playback` for what is skipped. No core crate may depend on
`actions` or `ui`. `playback` gains `session` but not `presence` or `media-controls`; `session`
does not depend on `playback`, so no cycle appears. `updater` stays with the desktop app: `hiphile-core`
does not depend on it.

## Steps

Each step leaves the app working and releasable. The GPUI views switch to the moved code in the same
step, and the existing tests pass at its end. Logic tests move with their code as plain tests; view tests
stay with the views.

| Step | Content | Parallel |
| --- | --- | --- |
| 0 | Create `hiphile-core`; the executor trait, its GPUI implementation (in `ui`) and a tokio one for tests and the CLI; the coalescing saver; add the core crates to `script/check-gpui-free` | — |
| 1 | Move the [pure files](#free-of-gpui-but-inside-gpui-crates) | By file |
| 2 | Pilot the rules on `updater`: small, with a timer, a saved choice and an `advance_clock` test | — |
| 3 | `session`, the sign-in procedure, `Readings` | 3 and 4 in parallel after 2 |
| 4 | The settings models | |
| 5 | Favorites, playlists, skipping instrumentals | 5 and 6 in parallel after 3: their logic needs `Session` and `Readings`, which only step 3 frees of GPUI. All four also meet in `app_shell.rs` and in the views that follow `Readings` |
| 6 | The library sync and its activity, the search index, home recommendations | |
| 7 | The player into `playback`, in parts: states and queue operations; engine events and preparing the next song; scrobbling; Discord and media controls as followers in `hiphile_core::desktop`; the volume, the window handle and `shutdown()`; the current song's lyrics into `lyrics` | One part at a time |
| 8 | The routing and the session's lifetime from `app_shell.rs` into `hiphile-core` | Last |
| 9 | What steps 1 to 8 left in the views (see [Step 9](#step-9)) | In four parts |
| 10 | What the survey after step 9 found (see [Step 10](#step-10)) | In five parts |

## The updater pilot

Step 2 tried the design rules on `updater`. They held, with these details settled:

- **The handle.** `Updater` is an `Arc` around a `Mutex` with its status and tasks. `status()` reads the
  current `Status` (the state, whether updates are automatic, the errors), `subscribe()` gives a
  `watch::Receiver<Status>`, and `check`, `install` and `set_automatic` change the status before they
  return. `Updater::of_this_app(executor, config::Store::user())` takes its store (rule 10).
- **Publish under the lock.** Every change goes through one function that publishes the status before
  it lets go of the lock; publishing after would let an older status overwrite a newer one. The order is
  the state's lock, then the `watch`'s, so a borrow of the receiver must not be held across a call into
  the handle.
- **One change, one status.** A look that finds a version while updates are automatic starts the install
  under the same lock, so no view sees `Available` in between, as before with one `cx.notify()`.
- **Tasks hold a weak handle**, as GPUI tasks hold a `WeakEntity`: dropping the last `Updater` drops its
  tasks and so cancels the schedule and any look.
- **Saving.** `set_automatic` hands the choice to `Saver` under the lock, so the choices reach the disk in
  the order they were made; the save reports its error back into the status.
- **Views.** `gpui_host::notify_on_change(updater.subscribe(), cx)` replaces `cx.observe`. It notifies on the
  next turn of the foreground executor rather than at once, so a GPUI test runs `run_until_parked` before
  it looks at what is drawn.
- **Tests.** The eight tests run on `gpui_host::executor`, and the `advance_clock` one is unchanged. The others
  read `updater.status()` where they read the entity, and compare `String`s (rule 6). Two are new: every
  change is published at once (on GPUI's executor), and the 6-hour schedule on tokio with a paused clock.

## Decided before steps 3 to 6

- **The GPUI host is its own crate.** `gpui-host` holds `executor(cx)` and the bridges that views follow
  the core with: `observe` and `observe_in` for a `watch`, `subscribe` and `subscribe_in` for a
  `broadcast`, in the shape of `cx.observe` and `cx.subscribe`, and `notify_on_change`. It
  depends only on gpui-kit, `executor` and tokio's `sync`. A core crate takes it as a dev-dependency to
  test on GPUI's executor, which keeps the existing tests as they are while their code moves. With
  `ui` it could not: `ui` depends on `actions`, which depends on `hiphile-core`, so a core crate under
  `hiphile-core` would reach itself through its dev-dependencies, which cargo builds as a second copy
  whose types do not cross. Should GPUI go, a deterministic test executor in `executor` takes its place.
- **Notices go on `tokio::sync::broadcast`.** A change of state is no notice: views follow the
  snapshot's `watch` (rule 3), so events such as `OutputChanged`, `TransitionsChanged`,
  `SkipInstrumentalsChanged`, `CacheLimitChanged` and `ReadingsChanged` become `watch` subscriptions.
  What is left, a one-off such as `ReadingsFailed`, `SignedIn`, the end of a sync or a request to raise
  the window, goes on a `broadcast`: like `cx.subscribe`, it gives every receiver each notice sent after
  it subscribed, and several listen today (`ReadingsChanged` has six, `PlayerEvent` five), where
  `async_channel` would give each notice to one receiver only. The capacity leaves room for a burst; a
  receiver that lags goes on from the oldest notice it still has. `audio` keeps `async_channel`, which
  has one receiver, the player.
- **Every constructor takes `Result<config::Store, config::Error>`**, as `Updater::of_this_app` does,
  and makes its own text for a failure ("… won't be saved: …"). The app passes `config::Store::user()`
  to each, as `config::Error` is not `Clone`; tests pass `Ok(config::Store::at(dir))` of a temporary
  directory.
- **Saved settings are read in the constructor.** They are one small local file, and the state starts
  from them: read later, a view would show the defaults first and the user's first change could race
  the read. Everything else that can block still goes through blocking work (rule 5), the keychain
  above all, as it can show a system prompt.
- **What runs at startup.** Besides reading the saved settings, opening the cover cache
  (`session::open_covers`) stays on the UI thread at startup, as it was before step 3.
- **Following a snapshot.** A snapshot is published when any part of it changes, so whoever followed one
  event before compares the part it used and passes it on only when that part changed, as `app_shell.rs`
  does for the output, the transitions and the cache limits.
- **Counting changes in tests.** A `watch` keeps only the last value, so a test that counted events
  cannot count publishes. `AudioSettings` keeps a `#[cfg(test)]` counter where it emitted its two events;
  later steps should assert on the state reached rather than add such counters.
- **Not yet injected** until step 8, which gave `Readings` its folder (`Readings::at`): `Readings` found
  the dictionaries' folder itself (the user's cache) and removed them when the choice was off.
- **Release.** `executor`, `gpui-host` and `hiphile-core` are kept out of release-plz (`git_only =
  false`) until a `v*` tag contains them. Remove those entries after the next release.

## Found in steps 5 and 6

- **Handles that follow handles.** `LibraryActivity` follows `LibrarySync` and `Readings`, and
  `SearchIndex` follows `Readings`, inside the core: `hiphile_core::follow` runs `observe` and `subscribe`
  on the executor, as `gpui-host` does on GPUI's foreground.
- **Views keep a copy and compare it with the core's.** A view that copies part of a snapshot compares
  the new snapshot with the last one it took from the core, not with its copy, so a later publish of the
  same value does nothing. A view that has just called the core takes the snapshot at once (the
  playlists view's `sync`), so a command right after it does not act on the old one. On a notice, a view
  first catches up with the current snapshot.
- **A request a dialog waits for returns a `Task`**: `PlaylistsState::create` gives
  `Task<Result<PlaylistId, String>>`.
- **The `actions` globals hold copies.** The known favorites, the playlist choices and the skip setting
  are fed by the shell from the handles' `watch`es, so a change reaches the stars and menus one turn
  later. A global could hold the handle instead, but about ten view tests set these copies directly.
  Since step 9 the known favorites only label and disable the stars and menus: the core decides which
  way a toggle goes.
- **Names.** Handles are named after what they do (`LibrarySync`, `SearchIndex`, `DailyRecommendations`).
  The favorites' and playlists' handles are `FavoritesState` and `PlaylistsState`, as their views have
  the plain names.
- **Test support.** `hiphile-core`'s `test-support` feature gives the fake favorites and playlists
  servers and a signed-in session (`test_support::session`, `session_saving_into`); the view crates take
  it as a dev-dependency without a cycle. What needs GPUI's test context stays in the core's own
  `testing`. View tests wait for the core to settle before they put data in.
- **Behaviour that changed.** When a reload drops the selected favorite, the row at its place is
  selected rather than the first. The core judges whether today's recommendations are empty by its own
  list, while the view leaves out albums on other shelves (`keep_shelves_off_recommended`): if every
  recommended album is on another shelf, a sync no longer starts a new pick. Step 9 ended that by moving
  the other shelves into the core.
- **Still in GPUI crates.** The album detail and its catalog cache (`select_album`) and the `act_on_album`
  helpers; home's recently played, most played and new albums shelves (these three moved in step 9);
  the filter indexes of the albums
  and the playlists, which only the view's filter uses; the confirmations ("Added to …"), with
  `AddToPlaylist` routed through the playlists view for them (a notice since step 8);
  `actions::toggle_favorite` deciding the direction from its copy (until step 9). The routing in
  `app_shell.rs` is step 8.

## Decided before steps 7 and 8

- **The purpose is a mobile app on UniFFI.** A later binding crate (say `hiphile-ffi`) wraps the core's
  handles: a handle becomes a UniFFI object (it is already an `Arc` that is `Send + Sync`), a snapshot or
  a notice a record or an enum of plain data, following a `watch` or a `broadcast` a callback interface
  the app implements, and the executor a tokio one the binding crate owns (`Executor::tokio`). The core
  does not depend on `uniffi`. So a handle's methods take and return plain data, and what only one host
  has comes in through a value, a trait or a feature.
- **`script/check-mobile`** builds `hiphile-core` for `aarch64-apple-ios` with Xcode's compiler (a
  nix-wrapped clang adds macOS flags to every target, so SQLite and aws-lc fail to build otherwise) and
  fails if it reaches GPUI, `presence`, `media-controls`, `updater` or their libraries. It passed at
  f460b76. Android is not checked: this machine has no NDK.
- **Desktop integrations follow the player.** Discord and the system's media controls (souvlaki) exist
  only on the desktop; a mobile app has its own (`MPNowPlayingInfoCenter`, `MediaSession`). So `playback`
  does not depend on `presence` or `media-controls`. The player publishes what they need (the song, its
  cover URL, playing or paused, the duration) and takes their commands (play, pause, next, previous,
  seek), and `hiphile_core::desktop`, behind the `desktop` feature, connects them, as a mobile app will
  in Swift or Kotlin. The window handle the media controls need comes from the app when it starts them
  (rule 8); uploading covers for Discord (ImgBB) goes with the Discord follower.
- **The engine stays `audio`.** It builds for iOS (cpal), and its desktop parts (hog mode, switching the
  device's rate) are already per operating system. Whether a mobile app plays through it or through the
  platform's player is decided with the mobile app; the player is not put behind an engine trait now.
- **The position is read, not published.** A `watch` that changed ten times a second would wake every
  follower; the player's snapshot changes with the song and the state, and `position()` reads the
  engine's frame count, as views and bindings poll it while they show it.
- **Still to do for mobile, outside these steps.** `config` picks the Linux secret service as the
  keychain on iOS (any Unix but macOS); a mobile app keeps the password in its own keychain or the core
  takes the store from the host. Since step 9 a host gives the folder for the covers, the songs, the
  dictionaries and Musixmatch's token (`Caches::At`). Still from `dirs`: the library's database
  (`catalog::Catalog::open_server`, under the user's caches) and the scrobbles waiting to be sent
  (under the user's data), both opened by `session::Accounts`, and Discord's cover links, on the
  desktop only.

## Found in step 7

- **A lock, not a task.** The player keeps its state in `Published` like the other handles: a command's
  effect is visible when it returns, as the views and their tests expect. One task owning the state was
  not needed.
- **Parts 1 and 2 are one commit.** Moving the states and commands without preparing the next song left a
  gap between songs, so they went in together to keep every commit releasable.
- **Dropping the player.** The core drops the task that reads the engine's events before the engine: the
  engine's thread closes the channel as it ends, which would wake the task while GPUI's test scheduler
  tears down ("Detected activity on thread hiphile-audio").
- **Stores.** `playback::Player::new` takes `Result<config::Store, config::Error>`; the volume is saved
  through `Saver`, and `shutdown()` writes a volume still waiting. Tests no longer write the user's
  config.
- **Covers for the followers** come through the cover cache: the rule for a kept cover is
  `session::answer_cover` (GPUI's image loads use it too), and `session::cover_image` gives a follower the
  image; the workspace gets the cache from `Hiphile` (`Parts::covers`).
- **Dependencies.** `playback` takes `config`; `lyrics` takes `config` (for `LyricsHighlight`),
  `executor`, `subsonic` and tokio's `sync`; the `desktop` feature takes `presence`, `media-controls`,
  `net`, `url` and `async-channel`. `media_controls::MediaControls` is `Send`, so a follower can hold it.
- **Behaviour that changed.** souvlaki is called from a background thread rather than the main one; on
  macOS it only writes `MPNowPlayingInfoCenter`, but the real app has yet to show it. A seek from the
  system's controls moves the slider at the view's next refresh (250 ms). `set_volume` does nothing while
  the volume is locked (the slider was already disabled). The lyrics' lines start over on the next turn
  after the lookup changes, and also when the song goes.
- **Still in the `player` crate.** What is drawn: the scrub, the position as a fraction, the lines'
  position and colours, the opening and closing animation, the highlight's refresh, the cover URLs, and
  closing the mini player at the end of the queue; also `player::song_cache(cx)`, which read the cache
  limit from the user's store until step 8 gave the workspace the song cache.

## Found in step 8

- **One core object.** `Hiphile::new(executor, net, store, keychain, caches)` is all a host makes. It
  holds the seven settings handles (`hiphile_core::settings::Settings`), `Readings`, the cover cache and
  `session::Accounts`; `subscribe()` publishes who is signed in, `sign_in` opens the workspace and
  remembers the profile and the password, `sign_out` closes it and forgets the password, and `shutdown()`
  writes what waits and stops the player. `connect` signs in without remembering anything or opening a
  workspace, for the CLI. The desktop app adds the updater and, with the `desktop` feature,
  `Workspace::show_on_discord` and `Workspace::connect_media_controls`.
- **The store is read once.** `config::Error::Shared(Arc<Error>)` lets each handle take its own
  `Result<config::Store, config::Error>` from one `Store::user()`, and each still words its own failure.
- **Caches.** `Caches::User` keeps covers, songs and the dictionaries in the user's caches, `Caches::None`
  keeps none, for tests and the CLI. Step 9 added `Caches::At`, a folder the host gives.
- **The workspace starts before its host follows it.** Signing in makes the workspace and starts its
  first sync on the executor, and the shell makes the screens on a later turn, when it sees the new
  snapshot. A notice sent in between reaches no one, so a host reads the state when it starts following:
  the shell puts up the activity's notice when the library is busy already, and the screens read what a
  sync already brought when they load. A binding layer has to do the same. Since step 10 the core collects
  the alerts from the start (`Workspace::alerts`, `Hiphile::alerts`), so no host catches up on those.
  Inside the core, a follower subscribes before it
  reads the value it starts from: the workspace makes the player from the audio settings it subscribed
  to, and `LibraryActivity` starts from the sync and the download under way. The routing between the
  handles runs on executor tasks, and the workspace and the player are made on a background thread.
- **Navigation stays in the shell.** Where the user is comes from the views' selection, so in the core it
  would take the views' state at every step.
- **The CLI does not open a workspace**, which would start syncing, loading the favorites, the playlists
  and the recommendations, scrobbling and the audio engine for a ping. It connects and uses the handles
  it needs on `Executor::tokio`: `LibrarySync` (its `StageSynced` notices report each stage) for `sync`
  and `albums`. `search` still asks the server, which `SearchIndex` would limit to the saved library, and
  `play` still used `playback`'s pieces directly until step 9 gave it the core's player.
- **Behaviour that changed.** "Added to …" is a notice (`PlaylistsNotice::SongsAdded`) and shows a turn
  later. The CLI words its failures as the core does, goes on to the songs when the artists fail, and
  opens the catalog and the scrobbles for every command.
- **Still in GPUI crates.** The settings window opens the song cache itself to count what it keeps
  (`SongCacheUsage`), and `musixmatch::Client::user` keeps its token in the `dirs` cache (both moved in
  step 9); the rest is listed under steps 5 to 7.

## Decided after step 8

On 2026-11-09, after the user tried the app:

- The CLI keeps to the handles it needs and does not open a workspace.
- Navigation stays in the shell.
- `lyrics` may depend on `config` for `LyricsHighlight`.
- The UniFFI pilot waits.

## Step 9

What the steps left in the views that a mobile app would need too. Building the filter indexes was to
stay in the views; the songs' moved into the core after step 9 and the others in step 10.

| Part | Content |
| --- | --- |
| A | The artists' list (the catalog, then the server), an artist's detail (albums, saved songs), top and similar songs, playing an artist; the songs' list (loading and arranging) |
| B | An album's detail and its catalog cache (`select_album`); one core way to get an album's songs for every `act_on_album`, with its error text; toggling a favorite by the core's set rather than the `actions` copy |
| C | Home's recently played, most played and new albums shelves with `RecentlyPlayed`, and keeping them off the recommendations in the core, which ends the behaviour change noted in step 6 |
| D | A cache folder the host gives (`Caches`), where `musixmatch` keeps its token too; the settings window counting kept songs through the core's song cache; `hiphile-cli play` through `playback::Player` |

## Found in step 9

- **One shelf.** `hiphile_core::Shelf<T>` (`Loading`, `Loaded`, `Failed`) is what every list that loads
  publishes: the home screen's shelves and recommendations, the artists, an artist's albums, top songs
  and similar artists, and the songs. A reload keeps what is on show, even when it fails. `ui::Shelf` is
  gone.
- **Home's shelves.** `HomeShelves` loads recently played, most played and new albums when the
  workspace opens, and the workspace reloads them after a play or a sync. It takes the
  `DailyRecommendations` and, under its own lock, tells it which albums it shows (`keep_off`), so a pick
  and the check for an empty list leave them out as before step 6. It is the first handle that calls
  another under its lock; the order is the shelves, then the recommendations, never the other way.
- **Artists and songs.** The workspace makes `ArtistList` (the catalog, then the server), `ArtistDetail`
  (an artist's page, which it reads again after a sync) and `SongList` (reading and arranging the songs).
  `ArtistPage` is a record with public fields, as a UniFFI record would be. `ArtistDetail::songs` gives
  the songs for playing an artist, with the core's error text. A list read again with the same content
  keeps its `Arc`, so a view's index still matches it.
- **The songs' index made a round trip.** As first decided, the filter index stayed in the view, which
  built it with the dictionaries and handed it back (`SongList::set_index`) for the core to arrange by.
  After step 9 the core builds it itself (see [Decided after step 9](#decided-after-step-9)).
- **Views take the snapshot at once, and keep the last one taken.** A view that has just called a handle
  takes its snapshot then and compares later ones with it (`taken_page`), not with its copy, so a test
  can change the copy.
- **An album's detail** is `library::AlbumDetail`, which each screen that shows an album makes for itself
  rather than the workspace, as screens show different albums. It shows the saved songs first, then the
  server's, which it saves. `album_songs` gives an action an album's songs, saved or from the server, with
  the core's error text; `actions::AlbumAction::start` runs it, so `actions` depends on `gpui-host` and
  `session`.
- **Favorites.** `FavoritesState::toggle` decides the direction from the core's set, including changes
  not yet sent; `actions::ToggleFavoriteOf` replaces `SetFavorite`. `set_favorite` stays for a host that
  knows the direction.
- **Caches.** `Caches::At(folder)` takes a host's folder; the crates take the folder (`CoverStore::in_cache`,
  `SongStore::in_cache`, `fuzzy::bundle::root_in`, `musixmatch::Client::with_cache`) instead of finding it
  with `dirs`. The core keeps one `SongStore` for as long as it lives: the player's cache and
  `Hiphile::song_cache_usage` go through it, and a new limit reaches it through `set_capacity`.
- **The readings only when needed.** `Hiphile` makes `Readings` on the first `readings()` or when a
  workspace opens, which gives the workspace's session the readings; `session::Accounts` no longer does.
  The CLI shares the app's cache folder for the songs, and making the readings at start would fetch or
  remove the app's dictionaries while it plays. The app's shell asks for them when it opens.
- **The core's player without a window.** `Hiphile::player(session)` is a player for a host without a
  workspace, with the audio settings of the moment, the saved volume and the core's song cache.
  `hiphile-cli play` follows it on `Executor::tokio` (`play::play_through`). Its test plays two songs that
  fail to open, so no device opens; playing a song to its end is left to the real CLI.
- **Merging the parts.** Parts A and C each made a shelf type, so the one at the crate's root stayed; the
  shell's library routing went entirely to the workspace, and the shell only tells sync failures
  (`follow_sync_failures`).
- **Behaviour that changed.** Home's shelves start loading when the workspace opens and reach the view a
  turn later. Songs showed once the view's index had reached the core, a turn later than before (since
  "Decided after step 9", once the core has indexed and arranged them), and the header's count and "Updating" change with the new arrangement. A reload that brings back the same
  artists no longer rebuilds the filter index or scrolls. Two quick clicks on a star both count. Leaving
  an album in the library stops its load. The settings window's Clear also resets what the player
  counted as kept. `hiphile-cli play` plays at the saved volume rather than full scale, sends now playing
  and scrobbles (and those waiting from before), prepares the next song so that none waits between songs,
  words its failures as the core does, fetches the songs by ID before it plays, shows the format only when
  it changes, and waits for the device's rate to be put back before it exits.
- **Still in GPUI crates** at the end of step 9, all moved in step 10 but for the selection itself: the
  filter indexes of the albums, the playlists and the artists; the selection rules of the lists (the first item, an item that went, revealing
  one); which parts an artist's page shows and how many top songs; the play queue made from the songs on
  show; the confirmations. A workspace test of the routing between the library and the screens' handles
  is missing: a `LibraryNotice` cannot be sent from outside `library::sync`.

## Decided after step 9

On 2026-11-09:

- `hiphile-cli play` keeps sending now playing and scrobbles, and plays at the saved volume.
- `SongList` builds the songs' filter index itself, with the dictionaries when they are there, and builds
  it again when they come or go, as `SearchIndex` does. The view keeps the filter text and the selection.
  The round trip and its turn are gone: songs show once the core has read, indexed and arranged them.

## Reviewed after step 9

On 2026-11-09, five reviewers went over `653d7e4..3a48bf5` against the code before the extraction.
Fixed then:

- `Player::set_repeat` prepares the following song again, as `cycle_repeat` does.
- A library without songs shows as loaded, not as loading for ever.
- An audio setting changed while the workspace is made reaches the player.
- The shell puts up the activity's notice and tells a failed load of the favorites that came before it
  followed them (see [Found in step 8](#found-in-step-8)).

Left, all minor:

Fixed in step 10: the first sync's failure (now always told), taking a favorite off twice quickly,
deleting the open playlist while the list failed, the song cache's count after Clear, `library::Albums`
(now `Shelf<Album>`), the playlist shown twice and the rename or deletion dropped after a failed save, and
the shell's part of following what came first (now the alerts' core tests). Still left:

- A replaced open of a song runs on to the stream's headers: it blocks a thread, where the old one was
  dropped at its next `.await`.
- A value written on quitting can be overwritten by an older one the saver was writing.
- GPUI's `spawn_blocking` shares the background pool with the core's tasks. That pool is fixed on Linux,
  which is not released.
- Untested: `Hiphile::sign_in` up to an open workspace (`Accounts::connect` opens the catalog in the
  user's folders), and the lists indexed again when the dictionaries come or go (a `Readings` with
  dictionaries needs a real bundle).

## Surveyed after step 9

On 2026-11-09, four researchers looked for what the GPUI crates still decide that a second UI would have
to write again. Besides what "Still in GPUI crates" lists, they found:

- **Setters that trust their callers.** `CacheLimit::set_gib` takes a negative or NaN size, which the next
  start cannot read, and then keeps no songs; `change_r8brain` takes values out of range, which the next
  start replaces with the defaults; `PlaylistsState::create` and `rename` take a blank name, and `rename`
  and `delete` do not check that the playlist is the user's; `turn_dsd_on` ignores `dsd_obstacle`, and the
  warning before it is only in the view. The views check all of these today.
- **Blocking work off the executor.** The albums', artists' and playlists' views read the dictionaries
  (`ReadingBook::read`, which reads and writes the catalog) on `cx.background_spawn`, against rule 5.
- **What the shell decides.** The text of the workspace's notices, when one is not shown (the screen that
  shows the failure itself), what each one that came before the shell followed needs; what showing a
  screen and Refresh ask of the core; resuming a sign-in, only once and not after signing out, and one
  attempt at a time (`Hiphile::sign_in` takes a second while the first runs).
- **Smaller rules in views.** Which queue commands apply to an entry; when the current song's lyrics are
  looked up; which parts of a line light and how much (the view adds `SEEK_ROUNDING` itself); whether the
  position can be sought; the queue made from the songs on show and the top songs shown; an unopened
  playlist's songs; an artist page's parts and figures; an album's duration; the home screen's sections;
  durations and counts as text (the CLI writes durations of an hour or more differently); the r8brain
  field's settling, which the cache limit and the ImgBB key already do in the core; fetching a cover for
  a host, done three times.
- **Bugs from before the extraction.** At volume 0 and not muted, the button says Unmute and mutes; the
  artists' list shows "0 albums" where the artist's page shows none; a playlist that diverged offers Move
  and Remove, which do nothing; the home screen's shelves ask for as many albums as the widest window
  shows, while the view's columns have no limit.

## Decided before step 10

On 2026-11-09:

- A first sync's failure is told whenever it comes, as a requested one's is.
- The core words the notices in full, as `playback` does ("in Settings, then press Play").
- The albums', artists' and playlists' filter indexes move into the core, as the songs' did after step 9.
  The rule in [Step 9](#step-9) that they stay in the views is withdrawn.
- The bugs above are fixed in step 10.

## Step 10

| Part | Content |
| --- | --- |
| A | Settings: the setters check and clamp what they are given; the r8brain field's settling, parsing and reset into the core; `save_before_quitting` covers the audio settings; DSD checks its obstacle and words its warnings; the notes on what DSD turns off; refreshing the device list while a host shows it |
| B | Albums and artists: `AlbumList` and a sorted, filtered `ArtistList` in the core on one shared shape with `SongList`; the queue from the songs on show and the top songs shown; an artist page's parts and figures; an album's duration; the home screen's sections and how many albums it asks for; `library::Albums` becomes `Shelf<Album>` |
| C | Playlists and favorites: the playlists' filter index; names and who may edit checked in the core; diverged playlists offer no song edits; an unopened playlist's songs; the notices and confirmations worded in the core; what is selected after a removal, and the favorites' screen toggling through the core |
| D | The shell: the workspace's and the app's notices (`alerts`) with their text and the catch-up; what showing a screen and Refresh do; resuming a sign-in and one attempt at a time; clearing the media controls on shutdown; a cover for a host; one `Placement` |
| E | The player and text: the queue entries' commands; when the lyrics are looked up; lit parts of the lines; seekable and silent; volume 0 unmutes; durations and counts as text, shared with the CLI |

The parts run in parallel, each in its own worktree; they meet in `workspace.rs`, `hiphile-core/src/lib.rs`
and the shell.

## Found in step 10

- **Settings (A).** The setters clamp what they are given (`CacheLimit::set_gib`, the r8brain numbers), and
  the r8brain field's parsing, settling, ranges, resets and presets are the core's: a host shows
  `AudioSnapshot::r8brain_text` and calls `type_r8brain` and `commit_r8brain`. `NumberRange` gives a
  stepper its bounds. The closures (`change_r8brain`, `change_soxr`) are private. `save_before_quitting`
  writes the audio settings too, a value still settling included. `turn_dsd_on` does nothing while
  `dsd_obstacle` says why not; its warnings are `DsdWarning` data and its choices `Choice`s;
  `dsd_note(PcmSetting)` says what DSD turns off. `Freshness` is the guard a host holds while it shows
  the devices or the song count, in place of the window's 2 s loop. A device for tests comes from
  `audio::OutputDevice::listed` (`test-support`).
- **Lists (B).** `Shown<T>` (`items`, `visible`, `queue_from`) is what the songs, the albums
  (`library::AlbumList`, `Workspace::albums`) and the artists (`ArtistList`) show, through one
  `pub(crate)` arranger that indexes twice (at once without the dictionaries, then with them) and drops
  stale work by id. `AlbumList` follows `LibrarySync` and compares its shelf by pointer. `ArtistPage` says
  its sections, picture, counts and the top songs on show; `album_count_to_show` hides a count of 0
  everywhere. `LoadedAlbum` gives its duration. Home's `sections` and `MAX_COLUMNS` (12) decide what shows
  and how many albums a shelf asks for. Giving up revealing an album keys on `SyncFinished`, as a `watch`
  joins the states between.
- **Playlists and favorites (C).** `PlaylistsState::set_query` and `PlaylistsSnapshot::visible` index the
  playlists as the lists do, though not yet through `Shown<T>`. Names are trimmed and a blank one refused,
  and `rename` and `delete` refuse what the user cannot edit (`is_editable`, `are_songs_editable`; named
  `is_`/`are_` as the coding guide asks). Notices and results carry their text (`SongsAdded`, `Removal`,
  `CreatedPlaylist`, `delete_confirmation`, `new_playlist_scope`). `songs_of` gives an unopened playlist's
  songs. What is selected after a change is `selection::after_removal`, `kept_selection` and
  `OpenPlaylist::follow`; the selection itself stays in the views.
- **The shell (D).** `hiphile_core::alerts` words the sync's, the favorites' and the dictionaries' failures
  and the skipped instrumentals (`PlayerNotice::Skipped`), and keeps up to 16 from the start; `told_on`
  names the screen that shows the failure itself. `Workspace::showing`, `refresh` and `app_activated`
  hold what a screen and Refresh ask of the core, which takes only a `Destination`; navigation stays in
  the shell, and `Refresh` is the app crate's action. `Hiphile::resume` resumes once per start and never
  after signing out, and `sign_in` refuses a second attempt (`SignInRefused::SigningIn`).
  `Hiphile::cover_image` fetches a cover for a host and for the desktop followers.
- **The player and text (E).** `Queue::commands_for`, `is_clearable` and `entry_state`;
  `PlayerSnapshot::is_seekable` and `is_silent`; `lyrics::timing::lit_lines`, which returns only the lines
  lit and adds the seek rounding itself (its ranges are UTF-8 bytes: a Swift or Kotlin binding converts
  them to UTF-16); `CurrentLyrics::set_song`, fed by the workspace, `want()` (a counted guard a view holds
  while it shows the lyrics) and `retry()`. `hiphile_core::format` writes durations and counts for the
  views and the CLI.
- **Behaviour that changed.** A first sync's failure is told whenever it comes. Albums, artists and their
  filters show a turn after the core has them, and the selection scrolls into view after a filter or a
  sort. Unmuting at volume 0 goes back to the last level saved above 0, or half. Closing and opening the
  lyrics no longer retries a failed lookup; Retry does. A command on a playlist whose songs are loading or
  failed reads them from the server. The rename dialog closes once the core accepts the name. A resumed
  sign-in no longer fills the password field, and editing any field stops it. Typing in an r8brain field
  publishes a snapshot per key, and a value typed out of range is clamped when the app quits. The CLI
  writes durations of an hour or more as `h:mm:ss`. Home shows at most 12 cards across.
- **Tests.** A test that ends with a request on its way can drop the last `net::Client` on the network's
  thread and abort the test process (rule 11); wait for the requests before the test ends. Some
  workspace tests against `NOWHERE` still carry that risk. One run of `cargo test --workspace` failed
  once at `df52f9e` and passed four times after; the test was not found.
- **Still open.** The soxr setters do not check ranges. `PlaylistsSnapshot::visible` could become a
  `Shown<Playlist>`. The workspace's own notices (favorites' and playlists' write failures, home's
  reload failures) are still told by the views.

## Done when

- Only view crates depend on gpui-kit, and they hold only view state.
- `script/check-gpui-free` covers `playback`, `session`, `updater`, `lyrics`, `media-controls`,
  `hiphile-core` and the crates that are free of GPUI today.
- The core alone can sign in, sync, browse and play, without a window. `hiphile-cli` on the core is the
  smoke test.
- `script/check-mobile` passes: `hiphile-core` builds for iOS without a desktop-only dependency.

## Open questions

- The crate split above, in particular whether `hiphile-core` should be split per domain later for build
  times.
- A UniFFI pilot, later (see [Decided after step 8](#decided-after-step-8)): a binding crate that wraps
  signing in, the library and playing, with Swift bindings generated in a test, to find what the
  handles' shapes still lack.
