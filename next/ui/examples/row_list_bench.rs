//! Times a row list of 20,000 songs without a window: selecting, moving with the keys, scrolling,
//! sorting and filtering, a frame each, on the UI thread and on the GPU.
//! `cargo run -p next-ui --release --example row_list_bench [-- <shot.png>]`. It needs a GPU, not a
//! display, and writes what the list looks like at the end to `<shot.png>` if given.

use std::rc::Rc;
use std::time::{Duration, Instant};

use anti_webui::app::headless;
use anti_webui::core::Ui;
use anti_webui::core::damage;
use anti_webui::core::event::{
    Key, KeyEvent, Modifiers, NamedKey, PointerButton, ScrollPhase, ScrollSource, WindowEvent,
};
use anti_webui::core::geometry::{PointF, Scale, SizeI};
use anti_webui::core::scene::Scene;
use anti_webui::prelude::*;
use anti_webui::wgpu::{Gpu, Offscreen, Renderer, wgpu};
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::typography::TextStyle;
use next_ui::row_list::{ROW_HEIGHT, RowList, Selection, row_list};
use next_ui::row_text::row_text;
use next_ui::theme::{DEFAULT_SEED, cover_scheme};

const SONGS: u32 = 20_000;
/// The main window on a Retina display.
const WINDOW: (i32, i32) = (1200, 760);
const SCALE: f64 = 2.0;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
struct Song {
    id: u32,
    title: String,
    details: String,
    time: String,
}

fn library() -> Rc<[Song]> {
    (0..SONGS)
        .map(|id| Song {
            id,
            // Every third title is too long for its row, and fades
            title: if id % 3 == 0 {
                format!(
                    "Song {id}: {}",
                    "a title far too long to be shown whole in a row. ".repeat(4)
                )
            } else {
                format!("Song {id}")
            },
            details: format!("Artist {} — Album {}", id % 300, id % 1700),
            time: format!("{}:{:02}", 2 + id % 5, id % 60),
        })
        .collect()
}

fn song_row(song: &Song) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(12.0_f32)
        .px(8.0_f32)
        .child(
            div()
                .flex_none()
                .w(40)
                .h(40)
                .rounded(4.0_f32)
                .dyn_style(|s| s.bg(color_scheme().surface_container_highest)),
        )
        .child(row_text(song.title.clone(), Some(song.details.clone())).flex_1())
        .child(
            TextStyle::BODY_MEDIUM
                .apply(div())
                .flex_none()
                .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
                .child(text(song.time.clone())),
        )
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

fn click(ui: &mut Ui, y: f32, modifiers: Modifiers, time: u32) {
    let position = PointF::new(400.0 * SCALE as f32, y * SCALE as f32);
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

fn scroll(ui: &mut Ui, dy: f32) {
    ui.handle_event(WindowEvent::Scroll {
        position: PointF::new(400.0 * SCALE as f32, 300.0 * SCALE as f32),
        delta_x: 0.0,
        delta_y: dy * SCALE as f32,
        modifiers: Modifiers::default(),
        source: ScrollSource::Unknown,
        phase: ScrollPhase::Single,
        time: 0,
    });
}

fn report(name: &str, frames: &[Frame]) {
    let stats = |pick: fn(&Frame) -> f64| {
        let mut times: Vec<f64> = frames.iter().map(pick).collect();
        times.sort_by(f64::total_cmp);
        (times[times.len() / 2], times[times.len() - 1])
    };
    let (ui_p50, ui_max) = stats(|f| f.ui);
    let (gpu_p50, gpu_max) = stats(|f| f.gpu);
    let (all_p50, all_max) = stats(|f| f.ui + f.gpu);
    println!(
        "{name:<26} {:>5} {ui_p50:>7.2} {ui_max:>7.2} {gpu_p50:>7.2} {gpu_max:>7.2} {all_p50:>7.2} {all_max:>7.2}",
        frames.len()
    );
}

fn window(rows: Signal<Rc<[Song]>>, selection: Selection<u32>, list: NodeRef) -> Ui {
    Ui::new(move || {
        anti_webui_md3e::set_color_scheme(cover_scheme(DEFAULT_SEED, false));
        let list = RowList::new(
            "Songs",
            move || rows.get(),
            |song: &Song| song.id,
            |song, _| song_row(song),
        )
        .selection(selection)
        .node_ref(list)
        .on_open(|_| {})
        .on_reorder(|_, _| {});
        next_ui::type_scale::body(div())
            .size_full()
            .flex()
            .flex_col()
            .p(16.0_f32)
            .dyn_style(|s| s.bg(color_scheme().surface))
            .child(
                div()
                    .flex_1()
                    .min_h(0)
                    .flex()
                    .flex_col()
                    .rounded(28.0_f32)
                    .overflow_hidden()
                    .dyn_style(|s| s.bg(color_scheme().surface_container))
                    .child(row_list(list)),
            )
    })
}

fn main() {
    let shot = std::env::args().nth(1);
    let gpu = Gpu::headless().expect("a GPU to draw with");
    println!("{}", gpu.adapter.get_info().name);
    let (width, height) = (
        (WINDOW.0 as f64 * SCALE) as u32,
        (WINDOW.1 as f64 * SCALE) as u32,
    );
    let all = library();
    let rows = signal(all.clone());
    let selection = Selection::<u32>::new();
    let list = node_ref();

    let begin = Instant::now();
    let mut ui = window(rows, selection, list);
    ui.handle_event(WindowEvent::Resized {
        size: SizeI::new(width as i32, height as i32),
        scale: Scale::from_f64(SCALE),
    });
    ui.handle_event(WindowEvent::Focused(true));
    let built = begin.elapsed().as_secs_f64() * 1000.0;
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
    println!(
        "{SONGS} rows of {ROW_HEIGHT} at {}x{} logical, scale {SCALE}",
        WINDOW.0, WINDOW.1
    );
    println!(
        "building {built:.2} ms, first frame: UI {:.2} ms, GPU {:.2} ms",
        first.ui, first.gpu
    );
    // Shaders compile on the first frames that use them
    for _ in 0..30 {
        frames.frame();
    }
    println!(
        "{:<26} {:>5} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
        "ms a frame", "n", "UI p50", "max", "GPU p50", "max", "all p50", "max"
    );

    let mut time = 0u32;
    let mut at = move || {
        time += 1000;
        time
    };
    let none = Modifiers::default();
    let rows_y = |row: usize| 16.0 + ROW_HEIGHT * (row as f32 + 0.5);
    let clicks = frames.after_each(60, |ui, i| click(ui, rows_y(i % 12), none, at()));
    report("click a row", &clicks);
    assert_eq!(selection.cursor(), Some(11));
    let shift_clicks = frames.after_each(30, |ui, i| {
        click(ui, rows_y(i % 12), none.with_shift(), at())
    });
    report("shift-click a range", &shift_clicks);
    let selected =
        || selection.with(|s| s.count_in(rows.get_untracked().iter().map(|song| song.id)));
    assert!(selected() > 1);
    let down = frames.after_each(240, |ui, _| named(ui, NamedKey::ArrowDown, none));
    report("down", &down);
    assert_eq!(selection.cursor(), Some(5 + 240));
    assert!(list.scroll_offset().y > 0.0);
    let shift_down = frames.after_each(120, |ui, _| {
        named(ui, NamedKey::ArrowDown, none.with_shift())
    });
    report("shift-down", &shift_down);
    let pages = frames.after_each(120, |ui, i| {
        let page = if i < 60 {
            NamedKey::PageDown
        } else {
            NamedKey::PageUp
        };
        named(ui, page, none)
    });
    report("page down, page up", &pages);
    let ends = frames.after_each(40, |ui, i| {
        let end = if i % 2 == 0 {
            NamedKey::End
        } else {
            NamedKey::Home
        };
        named(ui, end, none)
    });
    report("end, home", &ends);
    assert_eq!(selection.cursor(), Some(0));
    let top = list.scroll_offset().y;
    let scrolls = frames.after_each(480, |ui, i| scroll(ui, if i < 240 { 24.0 } else { -24.0 }));
    report("scroll 24 px", &scrolls);
    assert_eq!(list.scroll_offset().y, top);
    let flings = frames.after_each(240, |ui, i| {
        scroll(ui, if i < 120 { 400.0 } else { -400.0 })
    });
    report("scroll 400 px", &flings);
    let all_selected = frames.after_each(20, |ui, i| {
        if i % 2 == 0 {
            key(ui, Key::Character("a".into()), Modifiers::PRIMARY);
        } else {
            click(ui, rows_y(2), none, at());
        }
    });
    report("select all, click", &all_selected);
    assert_eq!(selected(), 1);
    let reversed: Rc<[Song]> = all.iter().rev().cloned().collect();
    let sorts = frames.after_each(20, |_, i| {
        rows.set(if i % 2 == 0 {
            reversed.clone()
        } else {
            all.clone()
        })
    });
    report("sort, selection follows", &sorts);
    assert_eq!(selection.cursor(), Some(2));
    let filtered: Rc<[Song]> = all
        .iter()
        .filter(|song| song.id % 7 == 0)
        .cloned()
        .collect();
    let filters = frames.after_each(20, |_, i| {
        rows.set(if i % 2 == 0 {
            filtered.clone()
        } else {
            all.clone()
        })
    });
    report("filter to a seventh, back", &filters);

    if let Some(shot) = shot {
        let Frames {
            mut ui,
            gpu,
            mut renderer,
            ..
        } = frames;
        click(&mut ui, rows_y(1), none, at());
        click(&mut ui, rows_y(3), none.with_shift(), at());
        ui.update();
        let scene = ui.paint(Instant::now());
        let pixels = gpu.render_to_rgba(&mut renderer, &scene);
        headless::write_png(&shot, width, height, &pixels).expect("the shot written");
        println!("{shot}");
    }
}
