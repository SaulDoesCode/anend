//! The memory view: weum's memory.html as a native view. A black page with the results
//! area in the middle and toasts under it.
//!
//! Term bar (Enter):
//! * a number: gsearch it in the current bucket, under its prime factor reduction line
//! * text: gsearch its sum in the current bucket and show the text itself (saturating it
//!   when it's new and saturate is on); with gsearch typed text off, an ordinary search
//!   over every variation of the text instead
//! * Ctrl+Enter: gsearch the text's sum and saturate it
//! * Shift+Enter: remember, show the item under the key typed (`3,4` or `note:1000001`)
//! * Delete: forget the memory under the key typed
//! * `Enlist`, `List`, `Delist`, `Unlist`: list commands (see the article clicks)
//!
//! Articles: click puts the text in the term bar, double-click its key, Ctrl+click makes
//! it the list candidate, Shift+click the list, Ctrl+right-click forgets and re-saturates
//! it under a new key, middle-click opens a note in the editor.
//!
//! Underlines: a text that shares its reversed sum with another text in the database is
//! underlined in a colour picked by its reversed sum (the hue, shared by all its matches)
//! and its ordinal sum (the shade). A text alone with its reversed sum gets none.
//!
//! The term bar is the app's command bar (see bar.rs), with the sidecar and the toggles
//! at its right. Escape leaves it, so the single key controls (buckets, sidecar arrows,
//! ...) can be used; `/` goes back to it. All of them can be remapped in the keys menu.

use bevy::ecs::system::SystemParam;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input_focus::{FocusedInput, InputFocus};
use bevy::picking::Pickable;
use bevy::picking::events::{PointerClick, PointerEnter, PointerLeave};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::text::{EditableText, FontSource, GenericFontFamily, TextEdit, TextEditChange, Underline, UnderlineColor};
use bevy::ui::UiTransform;
use bevy::ui_widgets::ScrollArea;

use crate::keys::{Action, Fired, modifiers};
use crate::memory::{Bucket, BucketKey, Hit, Ref, measure};
use crate::ui::{Ctx, Db, Fonts, Route, Session};
use crate::weum::{self, Seg};

const BLACK: Color = Color::BLACK;
const WHITE: Color = Color::WHITE;
/// main.mn
const MAIN_BG: Color = Color::srgb(0xc9 as f32 / 255.0, 0xab as f32 / 255.0, 0x99 as f32 / 255.0);
/// main.mn article
const ARTICLE_BG: Color = Color::srgb(0x28 as f32 / 255.0, 0x26 as f32 / 255.0, 0x26 as f32 / 255.0);
/// .pf
const PF: Color = Color::srgb(229.0 / 255.0, 229.0 / 255.0, 229.0 / 255.0);

#[derive(Resource)]
pub struct MemoryState {
    pub bucket: Bucket,
    /// weum's "do gsearch on what is other than number"
    pub gsearch_text: bool,
    /// weum's "search: also saturate"
    pub also_saturate: bool,
    pub white: bool,
    /// the big line over the results and its reduction line
    pub header: Option<(String, Vec<Seg>)>,
    pub results: Vec<Hit>,
    pub list_id: Option<Ref>,
    pub list_candidate: Option<Ref>,
    pub toasts: Vec<(String, f64)>,
    pub hover: Option<String>,
    pub term: String,
    pub set_term: Option<String>,
    pub redraw_results: bool,
    pub redraw_chrome: bool,
    pub redraw_toasts: bool,
}

impl Default for MemoryState {
    fn default() -> Self {
        MemoryState {
            bucket: Bucket::Ordinal,
            gsearch_text: true,
            also_saturate: true,
            white: false,
            header: None,
            results: Vec::new(),
            list_id: None,
            list_candidate: None,
            toasts: Vec::new(),
            hover: None,
            term: String::new(),
            set_term: None,
            redraw_results: true,
            redraw_chrome: true,
            redraw_toasts: true,
        }
    }
}

#[derive(SystemParam)]
pub struct Mem<'w> {
    pub db: Res<'w, Db>,
    pub state: ResMut<'w, MemoryState>,
    pub time: Res<'w, Time>,
}

impl Mem<'_> {
    fn toast(&mut self, msg: impl Into<String>) {
        let until = self.time.elapsed_secs_f64() + 15.0;
        self.state.toasts.push((msg.into(), until));
        self.state.redraw_toasts = true;
    }

    fn show(&mut self, header: Option<(String, Vec<Seg>)>, results: Vec<Hit>) {
        self.state.header = header;
        self.state.results = results;
        self.state.redraw_results = true;
    }

    /// adds a text to the view unless it's already there (`!mn.innerHTML.includes(v)`)
    fn articulate(&mut self, hit: Hit) {
        if !self.state.results.iter().any(|h| h.text == hit.text) {
            self.state.results.push(hit);
            self.state.redraw_results = true;
        }
    }

    fn header_for(bucket: Bucket, key: &BucketKey) -> (String, Vec<Seg>) {
        match key {
            BucketKey::Sum(n) => (n.to_string(), weum::pf_reduction(*n as u64)),
            BucketKey::Frequency(f) => (
                format!("{} · {}", crate::simd::frequency_letters(f), bucket.name()),
                Vec::new(),
            ),
        }
    }

    /// shows everything filed under a key, with the key's header line
    fn gsearch(&mut self, bucket: Bucket, key: BucketKey) {
        let header = Self::header_for(bucket, &key);
        match self.db.0.gsearch(bucket, &key) {
            Ok(hits) => self.show(Some(header), hits),
            Err(e) => {
                self.show(Some(header), Vec::new());
                self.toast(e);
            }
        }
    }

    /// weum's `envale`: put a value in the term bar and gsearch it
    fn envale(&mut self, bucket: Bucket, key: BucketKey) {
        self.state.bucket = bucket;
        self.state.redraw_chrome = true;
        self.state.set_term = Some(match &key {
            BucketKey::Sum(n) => n.to_string(),
            BucketKey::Frequency(f) => crate::simd::frequency_letters(f),
        });
        self.gsearch(bucket, key);
    }

    fn saturate_and_show(&mut self, text: &str) {
        match self.db.0.saturate(text) {
            Ok((k, _)) => {
                let label = Ref::Memory(k.clone()).label();
                self.toast(label);
                self.articulate(Hit {
                    r: Ref::Memory(k),
                    text: text.to_string(),
                });
            }
            Err(e) => self.toast(e),
        }
    }

    fn list_command(&mut self, cmd: &str) -> bool {
        let (list, cand) = (self.state.list_id.clone(), self.state.list_candidate.clone());
        match cmd {
            "Enlist" => {
                if let (Some(l), Some(c)) = (list, cand) {
                    match self.db.0.enlist(&l, &c) {
                        Ok(()) => self.toast(format!("enlisted {} in {}", c.label(), l.label())),
                        Err(e) => self.toast(e),
                    }
                }
            }
            "List" => {
                if let Some(l) = list {
                    match self.db.0.list(&l) {
                        Ok(hits) => self.show(Some((format!("list {}", l.label()), Vec::new())), hits),
                        Err(e) => self.toast(e),
                    }
                }
            }
            "Delist" => {
                if let (Some(l), Some(c)) = (list, cand) {
                    match self.db.0.delist(&l, &c) {
                        Ok(()) => self.toast(format!("delisted {} from {}", c.label(), l.label())),
                        Err(e) => self.toast(e),
                    }
                }
            }
            "Unlist" => {
                if let Some(l) = list {
                    match self.db.0.unlist(&l) {
                        Ok(n) => self.toast(format!("unlisted {} ({} members)", l.label(), n)),
                        Err(e) => self.toast(e),
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// the term bar's Enter
    pub fn enter(&mut self, raw: &str, ctrl: bool, shift: bool) {
        let v = raw.trim().to_string();
        if v.is_empty() || self.list_command(&v) {
            return;
        }
        let bucket = self.state.bucket;
        let number = v.parse::<u32>().ok();
        if number.is_none() && !shift {
            if ctrl {
                self.gsearch(bucket, measure(&v).key(bucket));
                self.saturate_and_show(&v);
            } else if self.state.gsearch_text {
                self.gsearch(bucket, measure(&v).key(bucket));
                match self.db.0.find_exact(&v) {
                    Ok(Some(k)) => self.articulate(Hit {
                        r: Ref::Memory(k),
                        text: v,
                    }),
                    Ok(None) if self.state.also_saturate => self.saturate_and_show(&v),
                    Ok(None) => {}
                    Err(e) => self.toast(e),
                }
            } else {
                match self.db.0.search_with_variations(&v) {
                    Ok(hits) => {
                        self.show(None, hits);
                        if self.state.also_saturate
                            && matches!(self.db.0.find_exact(&v), Ok(None))
                            && let Err(e) = self.db.0.saturate_new(&v)
                        {
                            self.toast(e);
                        }
                        if self.state.results.is_empty() {
                            self.toast(format!("didn't find anything for {}", v));
                        }
                    }
                    Err(e) => self.toast(e),
                }
            }
        } else if shift {
            match Ref::parse(&v).map(|r| self.db.0.remember(&r)) {
                Some(Ok(hit)) => self.show(None, vec![hit]),
                Some(Err(e)) => self.toast(e),
                None => self.toast("that's not a key"),
            }
        } else if let Some(n) = number {
            let bucket = if bucket == Bucket::Frequency {
                Bucket::Ordinal
            } else {
                bucket
            };
            self.gsearch(bucket, BucketKey::Sum(n));
        }
    }

    /// the term bar's Delete
    pub fn forget_typed(&mut self, raw: &str) {
        match Ref::parse(raw) {
            Some(Ref::Memory(k)) => match self.db.0.forget(&k) {
                Ok(text) => {
                    self.toast(format!("forgot {}", text));
                    self.state.results.retain(|h| h.r != Ref::Memory(k.clone()));
                    self.state.redraw_results = true;
                }
                Err(e) => self.toast(e),
            },
            Some(Ref::Note(_)) => self.toast("notes are deleted from the editor"),
            None => {}
        }
    }

    fn sidecar(&mut self, bucket: Bucket) {
        let term = self.state.term.trim().to_string();
        if !term.is_empty() {
            let m = measure(&term);
            self.envale(bucket, m.key(bucket));
        }
    }
}

// ------------------------------------------------------------------ markers

#[derive(Component, Default, Clone)]
pub struct MemoryView;
#[derive(Component, Default, Clone)]
pub struct MemMain;
#[derive(Component, Default, Clone)]
pub struct MemTerm;
#[derive(Component, Default, Clone)]
pub struct Toasts;
#[derive(Component, Default, Clone)]
pub struct HoverLine;

#[derive(Component, Clone)]
struct Article(Hit);

#[derive(Resource)]
pub struct SerifFont(pub FontSource);

impl FromWorld for SerifFont {
    fn from_world(world: &mut World) -> Self {
        let fallback = world.resource::<Fonts>().regular.clone();
        SerifFont(FontSource::from(vec![
            FontSource::Family("Times New Roman".into()),
            FontSource::from(GenericFontFamily::Serif),
            FontSource::Handle(fallback),
        ]))
    }
}

pub fn view() -> impl Scene {
    bsn! {
        Node {
            flex_grow: 1.0,
            min_height: px(0),
            display: Display::None,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border_radius: BorderRadius::all(px(4)),
        }
        BackgroundColor(BLACK)
        MemoryView
        crate::palette::OwnColors
        Children [
            Node {
                position_type: PositionType::Absolute,
                top: px(6),
                left: px(10),
                right: px(10),
            }
            HoverLine
        crate::zen::Chrome
            --
            Node {
                width: percent(97),
                max_height: percent(75),
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                align_content: AlignContent::FlexStart,
                align_items: AlignItems::Center,
                padding: UiRect::all(px(4)),
                overflow: Overflow::scroll_y(),
            }
            BackgroundColor(MAIN_BG)
            ScrollArea
            MemMain
            --
            Node {
                position_type: PositionType::Absolute,
                bottom: px(8),
                left: px(0),
                right: px(0),
                display: Display::Flex,
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                column_gap: px(10),
            }
            Toasts
        ]
    }
}

// ------------------------------------------------------------------ term bar

pub(crate) fn term_key(
    key: On<FocusedInput<KeyboardInput>>,
    keys: Res<ButtonInput<KeyCode>>,
    terms: Query<&EditableText, With<MemTerm>>,
    mut focus: ResMut<InputFocus>,
    session: Res<Session>,
    mut mem: Mem,
) {
    if !key.input.state.is_pressed() || session.route != Route::Memory {
        return;
    }
    let Ok(term) = terms.get(key.focused_entity) else {
        return;
    };
    let text = term.value().to_string();
    let (ctrl, shift, _) = modifiers(&keys);
    match key.input.logical_key {
        Key::Enter => mem.enter(&text, ctrl, shift),
        Key::Delete => mem.forget_typed(&text),
        // leave the term bar so the single key controls work
        Key::Escape => focus.clear(),
        _ => {}
    }
}

pub(crate) fn term_changed(
    change: On<TextEditChange>,
    terms: Query<&EditableText, With<MemTerm>>,
    session: Res<Session>,
    mut mem: ResMut<MemoryState>,
) {
    if session.route != Route::Memory {
        return;
    }
    if let Ok(term) = terms.get(change.event_target()) {
        let text = term.value().to_string();
        if text != mem.term {
            mem.term = text;
            mem.redraw_chrome = true;
        }
    }
}

pub fn apply_term(
    mut mem: ResMut<MemoryState>,
    mut term: Single<&mut EditableText, With<MemTerm>>,
    session: Res<Session>,
) {
    if session.route != Route::Memory {
        return;
    }
    if let Some(t) = mem.set_term.take() {
        term.editor_mut().set_text(&t);
        term.queue_edit(TextEdit::TextEnd(false));
        mem.term = t;
        mem.redraw_chrome = true;
    }
}

/// focuses the term bar whenever the memory view opens
pub fn focus_on_open(
    session: Res<Session>,
    mut last: Local<Option<Route>>,
    term: Single<Entity, With<MemTerm>>,
    mut focus: ResMut<InputFocus>,
) {
    if *last != Some(session.route) {
        *last = Some(session.route);
        if session.route == Route::Memory {
            focus.set(*term, bevy::input_focus::FocusCause::Navigated);
        }
    }
}

pub fn memory_actions(
    mut fired: MessageReader<Fired>,
    session: Res<Session>,
    mut mem: Mem,
    term: Single<Entity, With<MemTerm>>,
    mut focus: ResMut<InputFocus>,
) {
    for Fired(action) in fired.read() {
        if session.route != Route::Memory {
            continue;
        }
        let s = &mut mem.state;
        match action {
            Action::FocusTerm => focus.set(*term, bevy::input_focus::FocusCause::Navigated),
            Action::BucketOrdinal => s.bucket = Bucket::Ordinal,
            // like weum's 'r', a second press goes back to ordinal
            Action::BucketReversed => {
                s.bucket = if s.bucket == Bucket::Reversed {
                    Bucket::Ordinal
                } else {
                    Bucket::Reversed
                }
            }
            Action::BucketAtbash => s.bucket = Bucket::Atbash,
            Action::BucketFrequency => s.bucket = Bucket::Frequency,
            Action::ToggleGsearch => s.gsearch_text = !s.gsearch_text,
            Action::ToggleSaturate => s.also_saturate = !s.also_saturate,
            Action::WhiteMode => {
                s.white = !s.white;
                s.redraw_results = true;
                s.redraw_toasts = true;
            }
            Action::SideOrdinal => mem.sidecar(Bucket::Ordinal),
            Action::SideReversed => mem.sidecar(Bucket::Reversed),
            Action::SideAtbash => mem.sidecar(Bucket::Atbash),
            Action::SideFrequency => mem.sidecar(Bucket::Frequency),
            _ => continue,
        }
        mem.state.redraw_chrome = true;
    }
}

pub fn expire_toasts(mut mem: ResMut<MemoryState>, time: Res<Time>) {
    let now = time.elapsed_secs_f64();
    if mem.toasts.iter().any(|(_, until)| *until < now) {
        mem.toasts.retain(|(_, until)| *until >= now);
        mem.redraw_toasts = true;
    }
}

// ------------------------------------------------------------------ drawing

fn fg(white: bool) -> Color {
    if white { BLACK } else { WHITE }
}

/// the underline colour: hue from the reversed sum, so matches share it, shade from the
/// ordinal sum
pub fn underline_color(ordinal: u32, reversed: u32) -> Color {
    let hue = (reversed as f32 * 137.507_76) % 360.0;
    let lightness = 0.48 + (ordinal % 5) as f32 * 0.06;
    Color::hsl(hue, 0.85, lightness)
}

fn serif_text(s: impl Into<String>, size: f32, color: Color, serif: &SerifFont) -> impl Bundle {
    (
        Text::new(s),
        TextFont {
            font: serif.0.clone(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

pub fn rebuild_results(
    mut mem: Mem,
    main: Single<(Entity, &mut BackgroundColor), With<MemMain>>,
    mut view: Single<&mut BackgroundColor, (With<MemoryView>, Without<MemMain>)>,
    serif: Res<SerifFont>,
    tints: Res<crate::tint::Tints>,
    mut commands: Commands,
) {
    if !mem.state.redraw_results {
        return;
    }
    mem.state.redraw_results = false;
    let white = mem.state.white;
    let (main, mut main_bg) = main.into_inner();
    let empty = mem.state.header.is_none() && mem.state.results.is_empty();
    main_bg.0 = if empty {
        Color::NONE
    } else if white {
        WHITE
    } else {
        MAIN_BG
    };
    view.0 = if white { WHITE } else { BLACK };
    commands.entity(main).despawn_related::<Children>();

    if let Some((big, segs)) = mem.state.header.clone() {
        let header = commands
            .spawn((
                Node {
                    width: percent(100),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                },
                ChildOf(main),
            ))
            .id();
        let mut big = commands.spawn((serif_text(big, 24.0, fg(white), &serif), ChildOf(header)));
        big.insert(TextFont {
            font: serif.0.clone(),
            font_size: FontSize::Px(24.0),
            weight: FontWeight::BOLD,
            ..default()
        });
        if !segs.is_empty() {
            let row = commands
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    ChildOf(header),
                ))
                .id();
            let pf = if white { BLACK } else { PF };
            commands.spawn((serif_text("(", 12.0, pf, &serif), ChildOf(row)));
            for seg in segs {
                match seg {
                    Seg::Text(t) => {
                        commands.spawn((serif_text(t, 12.0, pf, &serif), ChildOf(row)));
                    }
                    Seg::Num(n) => {
                        commands
                            .spawn((serif_text(n.to_string(), 12.0, pf, &serif), ChildOf(row)))
                            .insert(TextFont {
                                font: serif.0.clone(),
                                font_size: FontSize::Px(12.0),
                                style: FontStyle::Italic,
                                ..default()
                            })
                            .observe(move |_c: On<PointerClick>, mut mem: ResMut<MemoryState>| {
                                mem.set_term = Some(n.to_string())
                            });
                    }
                }
            }
            commands.spawn((serif_text(")", 12.0, pf, &serif), ChildOf(row)));
        }
    }

    // nothing searched yet: say how to start, rather than show an empty page
    if empty {
        let dim = fg(white).with_alpha(0.55);
        for (line, size) in [
            ("type in the bar below", 20.0),
            (
                "a number gsearches it in the bucket · text finds what sums the same · \
                 Shift+Enter remembers it under a key · Ctrl+Enter saturates it",
                14.0,
            ),
        ] {
            commands.spawn((
                serif_text(line, size, dim, &serif),
                Node {
                    width: percent(100),
                    margin: UiRect::top(px(if size > 15.0 { 0.0 } else { 6.0 })),
                    ..default()
                },
                TextLayout::justify(Justify::Center),
                ChildOf(main),
            ));
        }
    }

    // which texts share a reversed sum with something else in the database
    let measures: Vec<_> = mem.state.results.iter().map(|h| measure(&h.text)).collect();
    let sums: Vec<u32> = measures.iter().map(|m| m.reversed).collect();
    let counts = mem.db.0.reversed_counts(&sums).unwrap_or_default();

    for (hit, m) in mem.state.results.clone().into_iter().zip(measures) {
        let shared = counts.get(&m.reversed).copied().unwrap_or(0) >= 2;
        let label = if let Ref::Note(_) = hit.r {
            format!("✎ {}", hit.text)
        } else {
            hit.text.clone()
        };
        // coloured by a classifier: a bar of its class's colour at the left
        let tint = tints.color(&hit.r.label());
        let under = if shared {
            underline_color(m.ordinal, m.reversed)
        } else {
            Color::NONE
        };
        let article = commands
            .spawn((
                Node {
                    padding: UiRect::axes(px(7), px(4)),
                    margin: UiRect::all(px(0.51)),
                    max_width: percent(95),
                    border: UiRect {
                        bottom: px(if shared { 3.0 } else { 0.0 }),
                        left: px(if tint.is_some() { 6.0 } else { 0.0 }),
                        ..default()
                    },
                    ..default()
                },
                BorderColor {
                    bottom: under,
                    left: tint.map(|(c, s)| c.with_alpha(0.35 + 0.65 * s)).unwrap_or(Color::NONE),
                    ..BorderColor::all(under)
                },
                BackgroundColor(if white { WHITE } else { ARTICLE_BG }),
                UiTransform::IDENTITY,
                Article(hit.clone()),
                ChildOf(main),
            ))
            .observe(article_enter)
            .observe(article_leave)
            .observe(article_click)
            .id();
        commands.spawn((
            serif_text(label, 20.0, fg(white), &serif),
            Pickable::IGNORE,
            ChildOf(article),
        ));
    }
}

fn article_enter(
    e: On<PointerEnter>,
    mut articles: Query<(&Article, &mut BackgroundColor, &mut UiTransform, &Children)>,
    mut texts: Query<&mut TextColor>,
    mut mem: ResMut<MemoryState>,
) {
    if let Ok((article, mut bg, mut tf, children)) = articles.get_mut(e.event_target()) {
        tf.scale = Vec2::splat(1.2);
        if !mem.white {
            bg.0 = WHITE;
            for c in children.iter() {
                if let Ok(mut t) = texts.get_mut(c) {
                    t.0 = ARTICLE_BG;
                }
            }
        }
        mem.hover = Some(format!(
            "{} {}",
            article.0.r.label(),
            weum::gsum_detail(&article.0.text)
        ));
        mem.redraw_chrome = true;
    }
}

fn article_leave(
    e: On<PointerLeave>,
    mut articles: Query<(&mut BackgroundColor, &mut UiTransform, &Children), With<Article>>,
    mut texts: Query<&mut TextColor>,
    mut mem: ResMut<MemoryState>,
) {
    if let Ok((mut bg, mut tf, children)) = articles.get_mut(e.event_target()) {
        tf.scale = Vec2::ONE;
        bg.0 = if mem.white { WHITE } else { ARTICLE_BG };
        for c in children.iter() {
            if let Ok(mut t) = texts.get_mut(c) {
                t.0 = fg(mem.white);
            }
        }
        mem.hover = None;
        mem.redraw_chrome = true;
    }
}

fn article_click(
    click: On<PointerClick>,
    articles: Query<&Article>,
    keys: Res<ButtonInput<KeyCode>>,
    mut mem: Mem,
    mut ctx: Ctx,
) {
    let Ok(Article(hit)) = articles.get(click.event_target()) else {
        return;
    };
    let hit = hit.clone();
    let (ctrl, shift, _) = modifiers(&keys);
    match click.button {
        PointerButton::Primary if click.count >= 2 => mem.state.set_term = Some(hit.r.label()),
        PointerButton::Primary if ctrl => {
            mem.state.list_candidate = Some(hit.r.clone());
            mem.toast(format!("list candidate {}", hit.r.label()));
        }
        PointerButton::Primary if shift => {
            mem.state.list_candidate = None;
            mem.state.list_id = Some(hit.r.clone());
            mem.toast(format!("list {}", hit.r.label()));
        }
        PointerButton::Primary => mem.state.set_term = Some(hit.text.clone()),
        PointerButton::Secondary if ctrl => {
            if let Ref::Memory(k) = &hit.r {
                // forget it and saturate it again, giving it a fresh key
                let rekeyed = mem.db.0.forget(k).and_then(|text| mem.db.0.saturate_new(&text));
                match rekeyed {
                    Ok(new) => {
                        mem.toast(Ref::Memory(new.clone()).label());
                        for h in mem.state.results.iter_mut().filter(|h| h.r == hit.r) {
                            h.r = Ref::Memory(new.clone());
                        }
                    }
                    Err(e) => mem.toast(e),
                }
            }
        }
        PointerButton::Middle => {
            if let Ref::Note(k) = &hit.r {
                ctx.open(k);
            }
        }
        _ => {}
    }
}

fn clickable_text(
    commands: &mut Commands,
    parent: Entity,
    s: String,
    color: Color,
    underline: bool,
    serif: &SerifFont,
) -> Entity {
    let mut e = commands.spawn((serif_text(s, 15.0, color, serif), ChildOf(parent)));
    if underline {
        e.insert((Underline, UnderlineColor(color)));
    }
    e.id()
}

/// the sidecar and the toggles (in the command bar) and the hover line
pub fn rebuild_chrome(
    mut mem: ResMut<MemoryState>,
    side: Single<Entity, With<crate::bar::BarSide>>,
    hover: Single<Entity, With<HoverLine>>,
    session: Res<Session>,
    db: Res<Db>,
    serif: Res<SerifFont>,
    mut commands: Commands,
) {
    if !mem.redraw_chrome {
        return;
    }
    mem.redraw_chrome = false;

    // hover line: what weum showed as an article's title
    commands.entity(*hover).despawn_related::<Children>();
    let line = mem.hover.clone().unwrap_or_else(|| {
        let n = db.0.memory_count().unwrap_or(0);
        format!("{} memories · {} bucket", n, mem.bucket.name())
    });
    commands.spawn((
        serif_text(line, 13.0, fg(mem.white).with_alpha(0.7), &serif),
        ChildOf(*hover),
    ));

    if session.route != Route::Memory {
        return;
    }
    let side = *side;
    commands.entity(side).despawn_related::<Children>();
    let (color, dim) = (crate::bar::BAR_TEXT, crate::bar::BAR_DIM);

    // sidecar: (ordinal, reversed, atbash, letters), each one gsearches itself
    let term = mem.term.trim().to_string();
    if !term.is_empty() && term.parse::<u32>().is_err() {
        let sc = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    ..default()
                },
                ChildOf(side),
            ))
            .id();
        let m = measure(&term);
        clickable_text(&mut commands, sc, "(".into(), dim, false, &serif);
        for (i, bucket) in Bucket::ALL.into_iter().enumerate() {
            if i > 0 {
                clickable_text(&mut commands, sc, ", ".into(), dim, false, &serif);
            }
            let shown = match m.key(bucket) {
                BucketKey::Sum(n) => n.to_string(),
                BucketKey::Frequency(f) => crate::simd::frequency_letters(&f),
            };
            let key = m.key(bucket);
            let e = clickable_text(&mut commands, sc, shown, color, bucket == mem.bucket, &serif);
            commands
                .entity(e)
                .observe(move |_c: On<PointerClick>, mut mem: Mem| mem.envale(bucket, key.clone()));
        }
        clickable_text(&mut commands, sc, ")".into(), dim, false, &serif);
    }

    // the buckets, then the toggles
    let buckets = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: px(6),
                ..default()
            },
            ChildOf(side),
        ))
        .id();
    for bucket in Bucket::ALL {
        let on = bucket == mem.bucket;
        let e = clickable_text(
            &mut commands,
            buckets,
            bucket.name().into(),
            if on { color } else { dim },
            on,
            &serif,
        );
        let (key, tip) = match bucket {
            Bucket::Ordinal => (Action::BucketOrdinal, "list memories by their ordinal sums"),
            Bucket::Reversed => (Action::BucketReversed, "list memories by their reversed sums"),
            Bucket::Atbash => (Action::BucketAtbash, "list memories by their atbash sums"),
            Bucket::Frequency => (Action::BucketFrequency, "list memories by their letters"),
        };
        commands
            .entity(e)
            .insert((crate::icons::Tip(tip.into()), crate::icons::TipKey(key)))
            .observe(move |_c: On<PointerClick>, mut mem: ResMut<MemoryState>| {
                mem.bucket = bucket;
                mem.redraw_chrome = true;
            });
    }
    let toggle = |commands: &mut Commands, label: &str, on: bool| {
        clickable_text(commands, side, label.into(), if on { color } else { dim }, on, &serif)
    };
    let tip = |commands: &mut Commands, e: Entity, tip: &str, key: Action| {
        commands
            .entity(e)
            .insert((crate::icons::Tip(tip.into()), crate::icons::TipKey(key)));
    };
    let g = toggle(&mut commands, "gsearch text", mem.gsearch_text);
    tip(&mut commands, g, "also gsearch what's typed", Action::ToggleGsearch);
    commands
        .entity(g)
        .observe(|_c: On<PointerClick>, mut mem: ResMut<MemoryState>| {
            mem.gsearch_text = !mem.gsearch_text;
            mem.redraw_chrome = true;
        });
    let s = toggle(&mut commands, "saturate", mem.also_saturate);
    tip(
        &mut commands,
        s,
        "searches saturate: each found memory searches on",
        Action::ToggleSaturate,
    );
    commands
        .entity(s)
        .observe(|_c: On<PointerClick>, mut mem: ResMut<MemoryState>| {
            mem.also_saturate = !mem.also_saturate;
            mem.redraw_chrome = true;
        });
    let w = toggle(&mut commands, "white", mem.white);
    tip(&mut commands, w, "white mode: dark text on white", Action::WhiteMode);
    commands
        .entity(w)
        .observe(|_c: On<PointerClick>, mut mem: ResMut<MemoryState>| {
            mem.white = !mem.white;
            mem.redraw_results = true;
            mem.redraw_toasts = true;
            mem.redraw_chrome = true;
        });
}

pub fn rebuild_toasts(
    mut mem: ResMut<MemoryState>,
    toasts: Single<Entity, With<Toasts>>,
    serif: Res<SerifFont>,
    mut commands: Commands,
) {
    if !mem.redraw_toasts {
        return;
    }
    mem.redraw_toasts = false;
    commands.entity(*toasts).despawn_related::<Children>();
    for (i, (msg, _)) in mem.toasts.iter().enumerate() {
        let m = msg.clone();
        commands
            .spawn((serif_text(msg.clone(), 16.0, fg(mem.white), &serif), ChildOf(*toasts)))
            .observe(move |c: On<PointerClick>, mut mem: ResMut<MemoryState>| {
                if c.count >= 2 {
                    if i < mem.toasts.len() {
                        mem.toasts.remove(i);
                    }
                    mem.redraw_toasts = true;
                } else {
                    mem.set_term = Some(m.clone());
                }
            });
    }
}
