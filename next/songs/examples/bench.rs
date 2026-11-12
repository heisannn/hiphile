//! Times the Songs screen on 20,000 songs without a window: selecting, moving with the keys,
//! scrolling, sorting from the headings and filtering, a frame each, on the UI thread and on the
//! GPU. The songs come from a fake server on this machine, and the core sorts and filters them on
//! its own threads, so sorting and filtering are also timed from the input until the rows show
//! the new order. `cargo run -p next-songs --release --example bench`. It needs a GPU, not a
//! display.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anti_webui::core::Ui;
use anti_webui::core::access::AccessNode;
use anti_webui::core::damage;
use anti_webui::core::event::{
    Key, KeyEvent, Modifiers, NamedKey, PointerButton, ScrollPhase, ScrollSource, WindowEvent,
};
use anti_webui::core::geometry::{PointF, Scale, SizeI};
use anti_webui::core::scene::Scene;
use anti_webui::prelude::*;
use anti_webui::wgpu::{Gpu, Offscreen, Renderer, wgpu};
use anti_webui_md3e::{color_scheme, set_color_scheme};
use hiphile_core::navigation::Destination;
use hiphile_core::songs::SongListStatus;
use hiphile_core::test_support::SongsServer;
use next_host::follow;
use next_songs::songs;
use next_ui::pane::GAP;
use next_ui::testing::{nodes, recording_commands, screen_context, signed_in_core};
use next_ui::theme::{DEFAULT_SEED, scheme};
use next_ui::type_scale::body;
use next_ui::{Screen, app_bar};
use serde_json::{Value, json};

const SONGS: usize = 20_000;
/// The main window on a Retina display, less the open rail.
const WINDOW: (i32, i32) = (1200 - 220, 760);
const SCALE: f64 = 2.0;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// How long the core may take to sort or filter before the bench gives up.
const PATIENCE: Duration = Duration::from_secs(30);

fn library() -> Vec<Value> {
    (0..SONGS)
        .map(|n| {
            // Every third title is too long for its lane, and fades
            let title = if n % 3 == 0 {
                format!(
                    "Song {n}: {}",
                    "a title far too long for its lane ".repeat(3)
                )
            } else {
                format!("Song {n}")
            };
            let (artist, album) = (n % 300, n % 1700);
            json!({
                "id": format!("s{n}"),
                "title": title,
                "duration": 120 + n % 300,
                "artist": format!("Artist {artist}"),
                "artistId": format!("artist-{artist}"),
                "album": format!("Album {album}"),
                "albumId": format!("album-{album}"),
            })
        })
        .collect()
}

struct Frames {
    ui: Ui,
    gpu: Gpu,
    renderer: Renderer,
    target: Offscreen,
    last: Scene,
    clock: Instant,
}

/// Milliseconds of one frame.
#[derive(Clone, Copy)]
struct Frame {
    ui: f64,
    gpu: f64,
}

impl Frames {
    fn frame(&mut self) -> Frame {
        self.clock += Duration::from_micros(8333);
        let begin = Instant::now();
        self.ui.animate(self.clock);
        self.ui.update();
        let scene = self.ui.paint(self.clock);
        let damage = damage::diff(&self.last, &scene);
        let painted = Instant::now();
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.renderer.render_damaged(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &self.target.view,
            &scene,
            &damage,
        );
        self.gpu.queue.submit([encoder.finish()]);
        let _ = self.gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let done = Instant::now();
        self.last = scene;
        Frame {
            ui: (painted - begin).as_secs_f64() * 1000.0,
            gpu: (done - painted).as_secs_f64() * 1000.0,
        }
    }

    /// The frame after each of `count` inputs, with the input counted on the UI thread.
    fn after_each(&mut self, count: usize, mut input: impl FnMut(&mut Ui, usize)) -> Vec<Frame> {
        (0..count)
            .map(|i| {
                let begin = Instant::now();
                input(&mut self.ui, i);
                let took = begin.elapsed().as_secs_f64() * 1000.0;
                let frame = self.frame();
                Frame {
                    ui: frame.ui + took,
                    gpu: frame.gpu,
                }
            })
            .collect()
    }

    /// After each of `count` inputs: the frame after it, the frame that shows what the core
    /// arranged for it, and the milliseconds from the input to the end of that frame.
    fn until_arranged(
        &mut self,
        count: usize,
        seen: Signal<SongListStatus>,
        mut input: impl FnMut(&mut Ui, usize),
    ) -> Arranging {
        let mut arranging = Arranging::default();
        for i in 0..count {
            let before = arranged(seen);
            let begin = Instant::now();
            let mut last = self.after_each(1, |ui, _| input(ui, i))[0];
            arranging.input.push(last);
            let checking = self.until(|| arranged(seen) != before && rows_show(seen), &mut last);
            arranging.shown.push(last);
            arranging
                .waits
                .push((begin.elapsed() - checking).as_secs_f64() * 1000.0);
        }
        arranging
    }

    /// Draws frames until `done`, with `last` the last of them, and returns how long asking took.
    fn until(&mut self, mut done: impl FnMut() -> bool, last: &mut Frame) -> Duration {
        let begin = Instant::now();
        let mut checking = Duration::ZERO;
        loop {
            let check = Instant::now();
            let is_done = done();
            checking += check.elapsed();
            if is_done {
                return checking;
            }
            assert!(begin.elapsed() < PATIENCE, "the core arranged nothing");
            *last = self.frame();
        }
    }
}

#[derive(Default)]
struct Arranging {
    input: Vec<Frame>,
    shown: Vec<Frame>,
    waits: Vec<f64>,
}

/// Which arrangement of the songs the core last published.
fn arranged(seen: Signal<SongListStatus>) -> Option<*const [usize]> {
    seen.with_untracked(|status| status.shown().map(|shown| Arc::as_ptr(shown.visible())))
}

/// Whether the rows on show are in the order the core last published.
fn rows_show(seen: Signal<SongListStatus>) -> bool {
    let titles: Vec<String> = rows().into_iter().filter_map(|row| row.label).collect();
    seen.with_untracked(|status| {
        let Some(shown) = status.shown() else {
            return false;
        };
        let Some(first) = titles.first() else {
            return shown.visible().is_empty();
        };
        let Some(top) = shown.position(|song| song.title() == first) else {
            return false;
        };
        titles.iter().enumerate().all(|(ix, title)| {
            shown
                .get(top + ix)
                .is_some_and(|song| song.title() == title)
        })
    })
}

fn key(ui: &mut Ui, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        ui.handle_event(WindowEvent::Key(KeyEvent {
            key: key.clone(),
            text: None,
            modifiers,
            pressed,
            repeat: false,
        }));
    }
}

fn named(ui: &mut Ui, named: NamedKey, modifiers: Modifiers) {
    key(ui, Key::Named(named), modifiers);
}

fn typed(ui: &mut Ui, c: char) {
    ui.handle_event(WindowEvent::Key(KeyEvent {
        key: Key::Character(c.to_string()),
        text: Some(c.to_string()),
        modifiers: Modifiers::default(),
        pressed: true,
        repeat: false,
    }));
}

/// A click at `(x, y)` in logical pixels.
fn click(ui: &mut Ui, (x, y): (f32, f32), modifiers: Modifiers, time: u32) {
    let scale = SCALE as f32;
    let position = PointF::new(x * scale, y * scale);
    ui.handle_event(WindowEvent::PointerMoved {
        position,
        modifiers,
    });
    for pressed in [true, false] {
        ui.handle_event(WindowEvent::PointerButton {
            position,
            button: PointerButton::Primary,
            pressed,
            modifiers,
            time,
        });
    }
}

fn scroll(ui: &mut Ui, at: (f32, f32), dy: f32) {
    let scale = SCALE as f32;
    ui.handle_event(WindowEvent::Scroll {
        position: PointF::new(at.0 * scale, at.1 * scale),
        delta_x: 0.0,
        delta_y: dy * scale,
        modifiers: Modifiers::default(),
        source: ScrollSource::Unknown,
        phase: ScrollPhase::Single,
        time: 0,
    });
}

fn middle(node: &AccessNode) -> (f32, f32) {
    let b = node.bounds;
    (b.x + b.width / 2.0, b.y + b.height / 2.0)
}

/// The rows on show, from the top.
fn rows() -> Vec<AccessNode> {
    let mut rows = nodes(Role::ListOption);
    rows.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y));
    rows
}

fn selected() -> usize {
    rows()
        .iter()
        .filter(|row| row.selected == Some(true))
        .count()
}

/// The middle of the heading that sorts by `label`.
fn heading(label: &str) -> (f32, f32) {
    let heading = nodes(Role::Button)
        .into_iter()
        .find(|button| {
            button
                .label
                .as_deref()
                .is_some_and(|name| name == label || name.starts_with(&format!("{label}, ")))
        })
        .expect("the heading");
    middle(&heading)
}

fn percentiles(times: &[f64]) -> (f64, f64) {
    let mut times = times.to_vec();
    times.sort_by(f64::total_cmp);
    (times[times.len() / 2], times[times.len() - 1])
}

fn report(name: &str, frames: &[Frame]) {
    let (ui_p50, ui_max) = percentiles(&frames.iter().map(|f| f.ui).collect::<Vec<_>>());
    let (gpu_p50, gpu_max) = percentiles(&frames.iter().map(|f| f.gpu).collect::<Vec<_>>());
    let (all_p50, all_max) = percentiles(&frames.iter().map(|f| f.ui + f.gpu).collect::<Vec<_>>());
    println!(
        "{name:<30} {:>5} {ui_p50:>7.2} {ui_max:>7.2} {gpu_p50:>7.2} {gpu_max:>7.2} {all_p50:>7.2} {all_max:>7.2}",
        frames.len()
    );
}

fn report_arranging(name: &str, arranging: &Arranging) {
    report(name, &arranging.input);
    report("  the frame that shows it", &arranging.shown);
    let (p50, max) = percentiles(&arranging.waits);
    println!("  input to on show: p50 {p50:.2} ms, max {max:.2} ms");
}

/// The bar, and the screen's panes below it.
fn window(screen: Screen) -> Div {
    let Screen {
        title,
        actions,
        body: panes,
        ..
    } = screen;
    body(div())
        .size_full()
        .flex()
        .flex_col()
        .dyn_style(|s| {
            let c = color_scheme();
            s.bg(c.surface).text_color(c.on_surface)
        })
        .child(app_bar(Vec::new(), title, actions))
        .child(
            div()
                .flex_1()
                .min_h(0)
                .flex()
                .flex_col()
                .ml(GAP)
                .mr(GAP)
                .mb(GAP)
                .child(panes.flex_1().min_h(0)),
        )
}

fn main() {
    let gpu = Gpu::headless().expect("a GPU to draw with");
    println!("{}", gpu.adapter.get_info().name);
    let (width, height) = (
        (f64::from(WINDOW.0) * SCALE) as u32,
        (f64::from(WINDOW.1) * SCALE) as u32,
    );
    let server = SongsServer::new(library());
    let (_core, hiphile) = signed_in_core(server.url());
    let workspace = hiphile.workspace().expect("signed in");

    let begin = Instant::now();
    workspace.showing(Destination::Songs);
    let kept = Rc::new(Cell::new(None));
    let mut ui = Ui::new({
        let (kept, hiphile) = (kept.clone(), hiphile.clone());
        move || {
            set_color_scheme(scheme(DEFAULT_SEED, false));
            kept.set(Some(follow(workspace.songs().subscribe())));
            let (commands, _) = recording_commands();
            window(songs(&screen_context(&hiphile, commands)))
        }
    });
    ui.handle_event(WindowEvent::Resized {
        size: SizeI::new(width as i32, height as i32),
        scale: Scale::from_f64(SCALE),
    });
    ui.handle_event(WindowEvent::Focused(true));
    let built = begin.elapsed().as_secs_f64() * 1000.0;
    let seen: Signal<SongListStatus> = kept.get().expect("made with the view");
    let renderer = Renderer::new(&gpu.device, FORMAT);
    let target = Offscreen::new(&gpu, FORMAT, width, height);
    let mut frames = Frames {
        ui,
        gpu,
        renderer,
        target,
        last: Scene::default(),
        clock: Instant::now(),
    };
    let first = frames.frame();
    let mut shown = first;
    let checking = frames.until(|| arranged(seen).is_some() && rows_show(seen), &mut shown);
    let loaded = (begin.elapsed() - checking).as_secs_f64() * 1000.0;
    println!(
        "{SONGS} songs at {}x{} logical, scale {SCALE}",
        WINDOW.0, WINDOW.1
    );
    println!(
        "building {built:.2} ms, first frame: UI {:.2} ms, GPU {:.2} ms",
        first.ui, first.gpu
    );
    println!(
        "read from the server, indexed, sorted and on show: {loaded:.0} ms; the frame that shows them: UI {:.2} ms, GPU {:.2} ms",
        shown.ui, shown.gpu
    );
    // Shaders compile on the first frames that use them
    for _ in 0..30 {
        frames.frame();
    }
    println!(
        "{:<30} {:>5} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
        "ms a frame", "n", "UI p50", "max", "GPU p50", "max", "all p50", "max"
    );

    let shown_rows: Vec<(f32, f32)> = rows()
        .iter()
        .take(12)
        .map(|row| (row.bounds.x + 40.0, middle(row).1))
        .collect();
    assert_eq!(shown_rows.len(), 12, "a dozen rows on show");
    let mut time = 0u32;
    let mut at = move || {
        time += 1000;
        time
    };
    let none = Modifiers::default();
    let clicks = frames.after_each(60, |ui, i| click(ui, shown_rows[i % 12], none, at()));
    report("click a row", &clicks);
    let shift_clicks = frames.after_each(30, |ui, i| {
        click(ui, shown_rows[i % 12], none.with_shift(), at());
    });
    report("shift-click a range", &shift_clicks);
    assert!(selected() > 1);
    let down = frames.after_each(240, |ui, _| named(ui, NamedKey::ArrowDown, none));
    report("down", &down);
    assert_eq!(selected(), 1);
    let shift_down = frames.after_each(120, |ui, _| {
        named(ui, NamedKey::ArrowDown, none.with_shift());
    });
    report("shift-down", &shift_down);
    let pages = frames.after_each(120, |ui, i| {
        let page = if i < 60 {
            NamedKey::PageDown
        } else {
            NamedKey::PageUp
        };
        named(ui, page, none);
    });
    report("page down, page up", &pages);
    let ends = frames.after_each(40, |ui, i| {
        let end = if i % 2 == 0 {
            NamedKey::End
        } else {
            NamedKey::Home
        };
        named(ui, end, none);
    });
    report("end, home", &ends);
    let over_rows = shown_rows[5];
    let scrolls = frames.after_each(480, |ui, i| {
        scroll(ui, over_rows, if i < 240 { 24.0 } else { -24.0 });
    });
    report("scroll 24 px", &scrolls);
    let flings = frames.after_each(240, |ui, i| {
        scroll(ui, over_rows, if i < 120 { 400.0 } else { -400.0 });
    });
    report("scroll 400 px", &flings);
    let all_selected = frames.after_each(20, |ui, i| {
        if i % 2 == 0 {
            key(ui, Key::Character("a".into()), Modifiers::PRIMARY);
        } else {
            click(ui, shown_rows[2], none, at());
        }
    });
    report("select all, click", &all_selected);
    assert_eq!(selected(), 1);

    let (title, artist) = (heading("Title"), heading("Artist"));
    let sorts = frames.until_arranged(20, seen, |ui, i| {
        click(ui, if i % 2 == 0 { artist } else { title }, none, at());
    });
    report_arranging("sort from a heading", &sorts);
    let turns = frames.until_arranged(20, seen, |ui, _| click(ui, title, none, at()));
    report_arranging("turn the order around", &turns);

    let filter = middle(
        &nodes(Role::TextInput)
            .into_iter()
            .find(|input| input.label.as_deref() == Some("Filter Songs"))
            .expect("the filter"),
    );
    click(&mut frames.ui, filter, none, at());
    frames.frame();
    let query = ['1', '2', '3', '4'];
    let typing = frames.until_arranged(8, seen, |ui, i| {
        if i < query.len() {
            typed(ui, query[i]);
        } else {
            named(ui, NamedKey::Backspace, none);
        }
    });
    report_arranging("type 1234, erase it", &typing);
    assert!(seen.with_untracked(|status| status.query().is_empty()));
}
