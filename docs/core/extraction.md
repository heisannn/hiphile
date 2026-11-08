# Extracting the core

Planned on 2026-11-09. The aim is to move every piece of state and logic that does not draw into crates
without GPUI, whatever UI the app ends up with. It is a prerequisite of a move to Native Shell (see the
[NSH evaluation](../nsh/evaluation.md)), but it is worth doing on its own: the logic can then be tested
without a window, and the CLI can share it.

| | |
| --- | --- |
| Goal | Only views depend on GPUI, and they hold only view state: focus, selection, scroll, slider drags, animation |
| Size | The 15 crates that depend on gpui-kit hold about 41,000 lines, 10,000 of them tests: about 420 `#[gpui_kit::test]` and 170 `#[test]` |
| Order | Pure files first, then one service at a time, the player last (see [Steps](#steps)) |
| Check | `script/check-gpui-free` lists every crate that must stay free of GPUI |

## Progress

| Step | State |
| --- | --- |
| 0 | Done. `executor` (the trait, `Task`, the tokio implementation, `Saver`), `gpui_host::executor` (GPUI's, at first in `ui`), an empty `hiphile-core`; the check covers the new crates and the five that were free but unchecked |
| 1 | Done. The [pure files](#free-of-gpui-but-inside-gpui-crates) are in `hiphile-core` and `playback`, with three items they used from GPUI crates; the views use them there |
| 2 | Done. `updater` is free of GPUI and the rules held; see [The updater pilot](#the-updater-pilot) |
| Before 3 | `gpui-host` split from `ui`, and the [decisions](#decided-before-steps-3-to-6) for steps 3 to 6 |
| 3 | Done. `session` is free of GPUI and checked: `Readings` is a handle, the sign-in procedure is `session::Accounts`, and GPUI's HTTP client and the globals for covers and readings are in `ui` (`ui::init_network`, `ui::readings`, `ui::set_cover_capacity`, `ui::on_readings_changed`) |
| 4 | Done. The settings models are handles in `hiphile_core::settings`; the `settings` crate keeps the window, its fields and applying the appearance. The seven `*Changed` events are gone |
| 5 | Done. `FavoritesState` and `PlaylistsState` in `hiphile-core` load, keep and write the favorites and the playlists; `is_instrumental` and what is skipped are in `playback`. The `actions` globals hold copies the shell feeds from the handles. See [Found in steps 5 and 6](#found-in-steps-5-and-6) |
| 6 | Done. `LibrarySync`, `LibraryActivity`, `SearchIndex` and `DailyRecommendations` in `hiphile-core`; `LibraryEvent` keeps only `LeftAlbum`, the rest are `LibraryNotice`s |

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
   when the player starts. Raising the window becomes a notice that the host acts on. The host calls
   `shutdown()` when the app quits, which restores the device's sample rate and hog mode (today
   `cx.on_app_quit`).
9. **Saves are serial and coalesced:** one write at a time, the last value wins. One helper replaces the
   four copies in `Player::save_volume`, `AudioSettings`, `Updater::save` and `Readings::save_choice`.
   It is `executor::Saver<T>`: `save(value)` replaces the value still waiting, and `save_with(|waiting| …)`
   builds on it, for `AudioSettings`, which writes only the fields that changed. The write runs on a
   blocking thread and reports through the owner's closure, and what was given is written even if the
   saver is dropped first. `Readings::save_choice` is not serial today. `take_waiting` hands the host the
   value still waiting, for a write before quitting: the settings window's `cx.on_app_quit` calls
   `save_before_quitting()` on the handles that hold a typed value not yet committed, as it calls
   `shutdown()` for the player.
10. **Constructors take their stores.** `Player::new` uses the user's real config store today, so a test that
    changes the volume changes the user's saved volume. Core constructors take the config store (and the
    catalog, the caches), and tests give them temporary ones.
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
| The player's state machine, queue operations, preparing the next song, scrobbling, Discord and media controls, the volume | `player/src/player.rs` | `playback`, whose description is already "Play queue and playback orchestration. No GPUI." |
| The format text, `is_instrumental` and the skip setting | `player/src/format.rs`, `actions/src/instrumentals.rs` | `playback` |
| Fetching the current song's lyrics, their cache, the line and word timing | `player/src/lyrics_view/current.rs`, `lyrics_view/lines.rs` | `lyrics` |
| `Session`, cover URLs, `describe`, `Readings` | `session` | `session`, made free of GPUI (done in step 3). `NetHttpClient` and the `Global`s are in `ui` |
| The sign-in procedure | `account/src/sign_in.rs` | `session::Accounts` (done in step 3). `account` keeps the form |
| The update check, automatic installs, the saved choice | `updater` | `updater`, made free of GPUI. The app makes it and hands it to the settings window and the shell, so `GlobalUpdater` is gone; the views call `cx.restart()` |
| The app and the signed-in workspace, routing between them, the settings models, the library sync and its activity, favorites, playlists, the search index, home recommendations, the sort and filter modules, navigation history | `hiphile`, `settings`, `library`, `favorites`, `playlists`, `search`, `home`, `songs`, `artists` | New `hiphile-core` |

`hiphile-core` depends on `playback`, `session`, `updater`, `lyrics` and the crates under them, `executor`
among them; the view crates depend on `hiphile-core`, and so does `actions` (and through it `ui`) for
`Favorite` and `PlaylistChoice`, and on `playback` for what is skipped. No core crate may depend on
`actions` or `ui`. `playback` gains `session`, `presence` and `media-controls`; `session`
does not depend on `playback`, so no cycle appears.

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
| 7 | The player into `playback`, in four parts: states and queue operations; engine events and preparing the next song; scrobbling, Discord and media controls; the volume, the window handle and `shutdown()` | One part at a time |
| 8 | The routing and the session's lifetime from `app_shell.rs` into `hiphile-core` | Last |

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
- **Not yet injected.** `Readings` finds the dictionaries' folder itself (the user's cache) and removes
  them when the choice is off, so no test makes one. It should take the folder as it takes its store.
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
  recommended album is on another shelf, a sync no longer starts a new pick. Moving the other shelves
  into the core ends that.
- **Still in GPUI crates.** The album detail and its catalog cache (`select_album`) and the `act_on_album`
  helpers; home's recently played, most played and new albums shelves; the filter indexes of the albums
  and the playlists, which only the view's filter uses; the confirmations ("Added to …"), with
  `AddToPlaylist` routed through the playlists view for them; `actions::toggle_favorite` deciding the
  direction from its copy. The routing in `app_shell.rs` is step 8.

## Done when

- Only view crates depend on gpui-kit, and they hold only view state.
- `script/check-gpui-free` covers `playback`, `session`, `updater`, `lyrics`, `media-controls`,
  `hiphile-core` and the crates that are free of GPUI today.
- The core alone can sign in, sync, browse and play, without a window. `hiphile-cli` on the core is the
  smoke test.

## Open questions

- The crate split above, in particular whether `hiphile-core` should be split per domain later for build
  times.
- Whether a service with many writers (the player) is better as one task that owns the state and takes
  commands than as a lock. The lock is the default because it keeps today's synchronous semantics.
