use std::rc::Rc;

use anti_webui::prelude::*;
use anti_webui_md3e::color_scheme;
use anti_webui_md3e::typography::TextStyle;
use next_ui::type_scale::error_text;

pub(crate) type When = Rc<dyn Fn() -> bool>;
type Words = Rc<dyn Fn() -> Option<String>>;
type Build = Rc<dyn Fn() -> Div>;

/// The corners of the segmented list of Material 3 Expressive, as `anti_webui_md3e` gives its
/// items: round at the ends of a group and nearly square between its rows.
const OUTER: f32 = 16.0;
const INNER: f32 = 4.0;
/// What the headline keeps before the control goes below it.
const WORDS: f32 = 200.0;

/// A setting: what it is, what more there is to know about it, and what changes it.
#[derive(Clone)]
pub(crate) struct Row {
    headline: Option<&'static str>,
    supporting: Option<Words>,
    control: Option<Build>,
    visible: Option<When>,
    disabled: Option<When>,
    stacked: bool,
}

impl Row {
    pub(crate) fn new(headline: &'static str, control: impl Fn() -> Div + 'static) -> Self {
        Self {
            control: Some(Rc::new(control)),
            ..Self::bare(headline)
        }
    }

    /// A control that says itself what it is, as a text field does with its label, alone in a row.
    pub(crate) fn unnamed(control: impl Fn() -> Div + 'static) -> Self {
        Self {
            headline: None,
            stacked: true,
            ..Self::new("", control)
        }
    }

    fn bare(headline: &'static str) -> Self {
        Self {
            headline: Some(headline),
            supporting: None,
            control: None,
            visible: None,
            disabled: None,
            stacked: false,
        }
    }

    pub(crate) fn supporting_text(self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.supporting(move || Some(text.clone()))
    }

    /// A line below the headline while `f` has one.
    pub(crate) fn supporting(mut self, f: impl Fn() -> Option<String> + 'static) -> Self {
        self.supporting = Some(Rc::new(f));
        self
    }

    pub(crate) fn visible_when(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.visible = Some(Rc::new(f));
        self
    }

    /// Faded while `f` returns true. The control is disabled by its own style.
    pub(crate) fn disabled_when(mut self, f: impl Fn() -> bool + 'static) -> Self {
        self.disabled = Some(Rc::new(f));
        self
    }

    /// The control below the words rather than after them, for one too wide to share the line.
    pub(crate) fn stacked(mut self) -> Self {
        self.stacked = true;
        self
    }

    fn is_visible(&self) -> bool {
        self.visible.as_ref().is_none_or(|f| f())
    }

    fn view(&self, corners: impl Fn() -> [f32; 4] + 'static) -> Div {
        let off = self.disabled.clone();
        let is_off = move || off.as_ref().is_some_and(|f| f());
        let faded = is_off.clone();
        let headline = self.headline.map(|headline| {
            TextStyle::BODY_LARGE
                .apply(div().child(text(headline)))
                .dyn_style(move |s| {
                    let c = color_scheme();
                    s.text_color(if faded() {
                        c.on_surface.with_alpha(0.38)
                    } else {
                        c.on_surface
                    })
                })
        });
        let supporting = self.supporting.clone().map(|line| {
            let shows = line.clone();
            show(
                move || shows().is_some(),
                move || {
                    let (line, faded) = (line.clone(), is_off.clone());
                    TextStyle::BODY_MEDIUM
                        .apply(div().child(text(move || line().unwrap_or_default())))
                        .dyn_style(move |s| {
                            let c = color_scheme();
                            s.text_color(if faded() {
                                c.on_surface.with_alpha(0.38)
                            } else {
                                c.on_surface_variant
                            })
                        })
                },
            )
        });
        let words = (headline.is_some() || supporting.is_some()).then(|| {
            div()
                .flex()
                .flex_col()
                .justify_center()
                .min_w(0)
                .children(headline)
                .children(supporting)
        });
        let control = self.control.as_ref().map(|build| build());
        let row = div()
            .flex()
            .min_h(if self.supporting.is_some() { 72 } else { 56 })
            .px(16.0_f32)
            .py(10.0_f32)
            .gap(12.0_f32)
            .dyn_style(move |s| {
                let [top_left, top_right, bottom_right, bottom_left] = corners();
                let mut s = s.bg(color_scheme().surface_bright);
                s.radius_top_left = Some(top_left);
                s.radius_top_right = Some(top_right);
                s.radius_bottom_right = Some(bottom_right);
                s.radius_bottom_left = Some(bottom_left);
                s
            });
        if self.stacked {
            row.flex_col()
                .items_start()
                .children(words)
                .children(control)
        } else {
            row.flex_row()
                .flex_wrap()
                .items_center()
                .children(words.map(|words| {
                    words
                        .flex_grow(1.0_f32)
                        .flex_shrink(1.0_f32)
                        .flex_basis(WORDS)
                }))
                .children(control.map(|control| control.flex_none()))
        }
    }
}

/// Settings that go together, in a segmented list under a title.
#[derive(Clone, Default)]
pub(crate) struct Group {
    title: Option<&'static str>,
    note: Option<Words>,
    rows: Vec<Row>,
    errors: Option<Rc<dyn Fn() -> Vec<String>>>,
}

impl Group {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn titled(title: &'static str) -> Self {
        Self {
            title: Some(title),
            ..Self::default()
        }
    }

    /// A line under the title, while `f` has one, about the group as a whole.
    pub(crate) fn note(mut self, f: impl Fn() -> Option<String> + 'static) -> Self {
        self.note = Some(Rc::new(f));
        self
    }

    pub(crate) fn row(mut self, row: Row) -> Self {
        self.rows.push(row);
        self
    }

    pub(crate) fn rows(mut self, rows: impl IntoIterator<Item = Row>) -> Self {
        self.rows.extend(rows);
        self
    }

    /// What went wrong with these settings, below them.
    pub(crate) fn errors(mut self, f: impl Fn() -> Vec<String> + 'static) -> Self {
        self.errors = Some(Rc::new(f));
        self
    }

    pub(crate) fn view(self) -> Div {
        let Self {
            title,
            note,
            rows,
            errors,
        } = self;
        let title = title.map(|title| {
            TextStyle::TITLE_SMALL
                .apply(div().px(16.0_f32).child(text(title)))
                .dyn_style(|s| s.text_color(color_scheme().primary))
                .access(|a| a.role(Role::Heading).level(3))
        });
        let note = note.map(|line| {
            let shows = line.clone();
            show(
                move || shows().is_some(),
                move || {
                    let line = line.clone();
                    TextStyle::BODY_MEDIUM
                        .apply(
                            div()
                                .px(16.0_f32)
                                .child(text(move || line().unwrap_or_default())),
                        )
                        .dyn_style(|s| s.text_color(color_scheme().on_surface_variant))
                },
            )
        });
        let errors = errors.map(|errors| {
            let errors = memo(move || errors());
            dyn_view(move || {
                let errors = errors.get();
                untrack(|| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(4.0_f32)
                        .px(16.0_f32)
                        .children(errors.into_iter().map(error_text).collect::<Vec<_>>())
                })
            })
        });
        div()
            .flex()
            .flex_col()
            .gap(8.0_f32)
            .children(title)
            .children(note)
            .child(segmented(rows))
            .children(errors)
    }
}

/// `rows` with a little room between them and the group rounded at its ends, whichever rows are
/// visible.
fn segmented(rows: Vec<Row>) -> Div {
    let rows = Rc::new(rows);
    let count = rows.len();
    let views = (0..count)
        .map(|i| {
            let around = rows.clone();
            let corners = move || {
                let first = !around[..i].iter().any(Row::is_visible);
                let last = !around[i + 1..].iter().any(Row::is_visible);
                let top = if first { OUTER } else { INNER };
                let bottom = if last { OUTER } else { INNER };
                [top, top, bottom, bottom]
            };
            let row = rows[i].clone();
            match row.visible.clone() {
                Some(visible) => {
                    show(move || visible(), move || row.view(corners.clone())).into_view()
                }
                None => row.view(corners).into_view(),
            }
        })
        .collect::<Vec<_>>();
    div().flex().flex_col().gap(2.0_f32).children(views)
}

/// A page of groups, the room between them showing where one ends.
pub(crate) fn page(groups: Vec<View>) -> Div {
    div().flex().flex_col().gap(24.0_f32).children(groups)
}
