# Native Shell evaluation

Researched on 2026-11-09; no decision yet. The question is whether Hiphile's UI should move from GPUI and
GPUI Kit to [Native Shell](https://nsh.dev) (NSH): the core in Rust, the screens in TSX, drawn with AppKit
on macOS and Win32 on Windows. The research covered NSH 0.4.0 (CLI `e626a8d`) and its source at `36b91689`.

| | |
| --- | --- |
| Feasible | Yes. A spike linked Hiphile's core crates into an NSH app and drove them from TSX, with headless tests |
| Prerequisite | [Extracting the core](../core/extraction.md), needed whatever the decision |
| Biggest constraints | Rust and TSX share no signals; one call carries at most 2^20 entries; the generated runner has no place for media controls' window handle or for restoring the device on quit |
| Biggest risks | NSH is three weeks old and changes daily; its own site now points desktop-only apps to GPUI Kit; macOS 14 becomes the minimum |
| Recommended path if adopted | Build the NSH app next to the GPUI one in the same workspace and release GPUI until the NSH app catches up |

## How it was researched

Four investigations, run in parallel:

1. An inventory of the UI: every window, screen, overlay, primitive, binding and test.
2. The boundary between the core and the UI: where state lives, how it flows, what binds it to GPUI.
   Its plan is [Extracting the core](../core/extraction.md).
3. An audit of NSH's docs and source against what a music player needs.
4. A spike: an NSH app with Rust and TSX that depends on Hiphile's crates, run with `nsh check`,
   `nsh test` and `cargo test` only. No window was opened, so nothing was played.

## The spike

| Result | Detail |
| --- | --- |
| Hiphile's crates link | `subsonic`, `client`, `catalog`, `config`, `fuzzy`, `net`, `playback` and `audio` build as dependencies of an NSH crate. Crates with `workspace = true` dependencies work by path from outside the workspace too |
| No conflicts | No `links` clash; tokio unifies on 1.54; only semver-incompatible duplicates (sha2, md-5, base64, rand). Adding NSH to Hiphile's `Cargo.lock` changes no locked version (195 crates added), and the GPUI app still checks in the same workspace |
| Stock cargo | huacnlee/cargo is recommended by NSH, not required |
| Build times | Cold `nsh test`: 3 min 7 s (9.2 GB of `target`). After touching `lib.rs`: `nsh check` 3.0 s, `nsh test` about 7 s. A TSX change only rebundles (1 to 5 ms) |
| Exports | A `#[class]` over a temporary catalog with 500 fake albums (created in 6 ms), a `#[module]` with sync and async fuzzy search, a Rust `#[component]` |
| Events | A position sent every 50 ms from a std thread and from a tokio runtime reached a TSX signal through a channel, a task on the engine thread and `Callback::post`, and stopped with the returned `Stop` |
| 20,000 rows (debug build) | An array handed to TSX: 15 to 19 ms to convert, 35.8 ms to the first frame. A Rust `#[component]` with `virtual_list`: 9.7 ms to the first frame |
| Tests | 13 of 13 passed under `nsh test`, which runs the crate's `cargo test` and the `*.test.tsx` files |

Pitfalls the spike hit:

- `nsh-testing`'s `bundle` feature pulls `rolldown_*` crates that break semver upstream. Pin
  `rolldown_devtools`, `rolldown_devtools_action`, `rolldown_std_utils`, `rolldown_fs` and `rolldown_tracing`
  to 1.2.13 with `cargo update -p … --precise 1.2.13`, in that order.
- `spawn` inside a `#[module]` function panics. Take `cx: CallCx` and start the task with `cx.start`.
- A heavy synchronous call blocks the engine thread (building a 20,000-song index took about 300 ms
  without optimization). Heavy work belongs in an `async fn` that runs it on another thread.
- The generated runner (`.nsh/rust`, "do not edit") ignores the workspace's `[profile.dev.package]`, so
  `soxr`, `r8brain` and `dsd` lose their `opt-level = 3` in `nsh run`. Audio may not keep up in
  development; this was not tried.
- The runner resolves a lock of its own, separate from Hiphile's `Cargo.lock`.
- `nsh.json` has to sit in the app crate's own directory (`"rust": "."`). From outside, `nsh test` skips
  the crate's Rust tests.
- The docs on NSH's `main` are ahead of 0.4.0 (`layout_nodes()` is missing from the release). Read the
  docs at the release's tag.

## NSH against Hiphile's needs

| Need | NSH | Gap and workaround |
| --- | --- | --- |
| Rust state in TSX | `on(event, listener) -> Stop` and `Callback::post`; `async fn` gives a `Promise` | Signals are not shared between Rust and JS, and class getters are not reactive. Rust code reached from a synchronous member cannot touch signals (it panics) |
| Other threads to the UI | Any thread may wake the engine's tasks; a channel awaited by a task on the engine thread works | No `Send` handle that posts to the engine; the bridge owns its listening tasks |
| Large data | Rust `#[component]` with `virtual_list`; `DataTable` takes columns from Rust | One call carries at most 2^20 array entries plus fields: 50,000 songs fit, 100,000 do not. QuickJS has no JIT, so sorting and filtering stay in Rust |
| Lists | Virtualized `List`: rows of varying height, sticky headers, columns, `scrollToIndex` | `List` has no keyboard navigation or selection (`DataTable`, `Tree` and `Command` do) |
| Covers | `Image` loads `http(s)`, `file://` and absolute paths, decodes at the view's size, caches | No custom loader, scheme or request headers, and the disk cache is keyed by URL alone: Subsonic's salted URLs would always miss. Give it the file path from Hiphile's own `CoverStore` instead, which also keeps credentials out of the UI |
| Windows | Several windows on one engine; `level: "floating"` for the mini player; `restoreBounds`; hidden title bar with drag regions | Not verified: opening the mini player without taking focus, as GPUI's `WindowKind::PopUp` with `focus: false` does |
| Menus and keys | Application menu, context menus, `keymap.bind` with `when` | About 120 bindings and 20 key contexts to map; menus that rebuild with state go in an `effect` |
| Controls | `nsh/base`: `Slider` with `onChangeEnd`, `Input`, `Switch`, `Select`, `RadioGroup`, `NumberInput`, `Dialog`, `AlertDialog`, `Toast`, `Tooltip`, `Command`, `ResizablePanelGroup` | Unstyled: the app draws its own look, as with gpui-kit's base layer |
| Reordering by drag | No general drag and drop | Build it on `PanResponder` and animated values; the preview is a view drawn by the app |
| Lyrics | Animated values run on the UI thread with no app code per frame; a Rust component can follow the position and fade words | Animated values cannot drive a scroll offset: move the lines with `translateY`, or use `scrollToOffset` |
| Media keys, Now Playing | None | Keep souvlaki (`media-controls`). On Windows it needs the HWND, which the generated runner does not hand to app code |
| Quitting | `Application.quit` and `restart`. By default `run_app` returns when its window closes or the app quits | No hook before quitting was found. Restoring the device's sample rate needs a runner of Hiphile's own that calls `shutdown()` after `run_app` returns |
| Updates | `nsh/updater` with GitHub releases | Its manifest and archive names differ from `self_update`'s. Keep `self_update` first; switching needs one bridging release |
| Packaging | `nsh package` makes an ad hoc signed `.app` (macOS 14+, host architecture) and a zip on Windows | The release already builds with `cargo build` and packages on its own (`script/package-macos`, a zip on Windows), so a runner of Hiphile's own fits it; the TSX bundle from `nsh build` ships inside |
| Testing | `nsh test` with `nsh/test`, `nsh_testing::Headless` in `cargo test`, real text engines | About 420 GPUI view tests to rewrite as Rust or TSX tests of the new views |

## How Hiphile would be built on NSH

- **Crates:** the extracted core, one bridge crate (`hiphile-native`, the only one that depends on NSH, with
  `#[native_module("hiphile/native")]`), and `crates/app` with `nsh.json`, `theme.json` and the TSX.
- **Rust components** for what is large or changes often: the song, album, artist and queue lists (with
  their keyboard navigation, selection and reordering), the lyrics lines, the position bar and volume,
  cover art.
- **TSX** for the rest: the shell and navigation, the title bar and tabs, the player bar's text and buttons,
  dialogs, notices, the settings window, sign-in, menus and shortcuts.
- **State:** the core publishes on `watch`; the bridge's task on the engine thread posts each change to TSX
  listeners (`Player.on("now", …)`) or Rust components. Services that exist once are `#[module]`s; things
  that live as long as a screen are `#[class]`es.
- **A runner of Hiphile's own** (`main.rs` with `AppView` or `run_app`): it connects media controls with the
  window handle on the main thread, and calls `shutdown()` on the way out.

## The UI to port

Measured on the GPUI app as of 2026-11-09.

- **Windows:** the main window (quits the app when closed), Settings (one at a time), the mini player
  (above all apps, does not take focus, remembers its place and display).
- **Main window:** a custom title bar with Back and Forward, a segmented tab bar for the destinations and
  "Restart to update"; the screen and a resizable queue; the player bar below; the lyrics over both, with
  the cover flying from the bar.
- **Screens:** Home (shelves of cards with a 2D keyboard grid), Albums, Artists and Playlists (list and
  detail), Songs (the whole library in one list with sortable columns), Favorites (three tabs), Search (a
  command palette), Sign in, Settings (Audio with three resamplers and DSD, Queue, Lyrics, Storage,
  Discord, Appearance, Updates).
- **Behaviour:** context menus with an "Add to Playlist" submenu on every row, a playlist chooser, dialogs
  that stay open on failure, notices with progress and with Undo, mouse side buttons for Back and Forward,
  shortcuts enabled only where their action makes sense.

The hardest parts, most risky first:

1. Separating `player.rs` from its view (covered by the core extraction).
2. The whole library in one virtualized list, with keyboard navigation and a selection that follows the
   song through sorts.
3. The artist detail: a list of varying heights that keeps its place while its parts change.
4. Lyrics: words fading in and out in time with playback, and following the current line until the user
   scrolls.
5. Opening the lyrics: the cover flying from the bar, parts fading on their own schedules, reversible
   midway.
6. The mini player's window.
7. Reordering the queue and playlists by drag.
8. The bindings, key contexts and menus.

## Risks

- **Maturity.** 0.1.0 shipped on 2026-10-17 and 0.4.0 on 2026-11-04, and 303 commits followed in five
  days. NSH promises compatibility for released APIs, but it has weeks of use behind it.
- **Positioning.** On 2026-11-08 (`8fe220be`), NSH's home page began pointing desktop-only apps to GPUI Kit.
  NSH's focus is mobile, the web, and TSX delivered at run time.
- **macOS 14.** NSH's AppKit backend needs macOS 14; releases now target macOS 12. The release also builds
  for Intel Macs, which was not tried.
- **Audio in development.** See the runner's profile above.
- **Windows media controls.** Getting the HWND is unproven.

## Plan if adopted

The NSH app would grow in the same workspace while GPUI releases continue. Embedding NSH screens in the
GPUI app (`nsh-platform-gpui`'s `AppView`, as in NSH's `examples/embed-gpui`) would allow shipping screen
by screen, but its glue (GPUI's drawing differences, focus and keys across the two) would be thrown away
at the end.

| Phase | Content |
| --- | --- |
| 0 | [Extract the core](../core/extraction.md) |
| 1 | Proofs of concept, before committing: the own runner with souvlaki on macOS, the HWND on Windows, `shutdown()` on quit, a mini player that takes no focus, playback in a development build, an Intel build |
| 2 | The bridge crate, `crates/app`, the theme, sign-in, the player bar |
| 3 | The Rust components |
| 4 | The TSX screens, menus, shortcuts and the settings window |
| 5 | Packaging and CI, the bridging release for updates, removing GPUI |

Decisions this needs:

1. Whether lists, lyrics and the position bar as Rust components fit "screens in TSX".
2. Whether to raise the minimum to macOS 14, and whether to keep Windows (which needs the HWND proof).
3. Building next to the GPUI app, or embedding NSH screens in it.
