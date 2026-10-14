//! The shared icon set and the tooltips that name what an icon does.
//!
//! The icons are drawn as 24x24 line SVGs in `assets/icons/` (the web app uses those),
//! rasterized white on transparent by `scripts/icons.mjs` into `assets/icons/png/`, and
//! embedded here so the binary stays self-contained. An icon takes the colour its text
//! would have: inside a feathers button it follows the button's text colour, elsewhere
//! the `TextColor` it's spawned with, so the palette and zen's fade reach it like text.
//!
//! Anything with a [`Tip`] shows it in a small tooltip once the pointer has rested on it
//! for a moment, with the key bound to its [`TipKey`] action if it has one.

use bevy::asset::RenderAssetUsages;
use bevy::feathers::controls::{ButtonVariant, FeathersButton};
use bevy::feathers::theme::ThemedText;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::picking::Pickable;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::collections::HashMap;

use crate::keys::{Action, Keymap};

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        /// an icon of the shared set
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub enum Icon {
            #[default]
            $($variant),*
        }

        impl Icon {
            pub const ALL: &'static [Icon] = &[$(Icon::$variant),*];

            /// the file's name in `assets/icons/`
            pub fn name(self) -> &'static str {
                match self {
                    $(Icon::$variant => $file),*
                }
            }

            fn png(self) -> &'static [u8] {
                match self {
                    $(Icon::$variant => include_bytes!(concat!("../assets/icons/png/", $file, ".png"))),*
                }
            }
        }
    };
}

icons! {
    New => "new",
    Save => "save",
    Delete => "delete",
    View => "view",
    Edit => "edit",
    Settings => "settings",
    Close => "close",
    Search => "search",
    Tag => "tag",
    Tags => "tags",
    Bucket => "bucket",
    List => "list",
    Memory => "memory",
    Space => "space",
    Stats => "stats",
    Reader => "reader",
    Editor => "editor",
    Hub => "hub",
    Upload => "upload",
    Download => "download",
    Copy => "copy",
    Rename => "rename",
    Forge => "forge",
    Merge => "merge",
    Undo => "undo",
    Pick => "pick",
    Filter => "filter",
    Help => "help",
    Keys => "keys",
    Font => "font",
    Palette => "palette",
    Zen => "zen",
    Folder => "folder",
    Refresh => "refresh",
    Star => "star",
    Check => "check",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    Sidebar => "sidebar",
    Pin => "pin",
    Play => "play",
    Pause => "pause",
    Export => "export",
    Link => "link",
    Sliders => "sliders",
    Text => "text",
    Home => "home",
    And => "and",
    Or => "or",
    Sort => "sort",
    EyeOff => "eye-off",
    Swap => "swap",
    Open => "open",
    Hash => "hash",
    Classifier => "classifier",
    Shield => "shield",
    Alert => "alert",
    Bolt => "bolt",
    Sun => "sun",
}

/// the decoded icons
#[derive(Resource)]
pub struct Icons(HashMap<Icon, Handle<Image>>);

impl FromWorld for Icons {
    fn from_world(world: &mut World) -> Self {
        let mut images = world.resource_mut::<Assets<Image>>();
        let mut map = HashMap::new();
        for &icon in Icon::ALL {
            match Image::from_buffer(
                icon.png(),
                ImageType::Extension("png"),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::linear(),
                RenderAssetUsages::RENDER_WORLD,
            ) {
                Ok(img) => {
                    map.insert(icon, images.add(img));
                }
                Err(e) => warn!("icon {} didn't decode: {}", icon.name(), e),
            }
        }
        Icons(map)
    }
}

impl Icons {
    pub fn get(&self, icon: Icon) -> Handle<Image> {
        self.0.get(&icon).cloned().unwrap_or_default()
    }
}

/// shows an icon: becomes an image node once spawned, tinted like text
#[derive(Component, Default, Clone)]
#[require(Node)]
pub struct IconImg(pub Icon);

/// what the tooltip says about an element
#[derive(Component, Default, Clone)]
pub struct Tip(pub String);

/// the action whose key the tooltip mentions
#[derive(Component, Default, Clone)]
pub struct TipKey(pub Action);

/// the tooltip itself
#[derive(Component)]
pub struct Tooltip;

/// an icon inside a button, sized to the button's text
fn icon_node(size: f32) -> Node {
    Node {
        width: px(size),
        height: px(size),
        flex_shrink: 0.0,
        ..default()
    }
}

/// an icon button for scenes, its name shown as a tooltip
pub fn icon_button(icon: Icon, tip: &'static str) -> impl Scene {
    bsn! {
        @FeathersButton {
            @caption: bsn! {
                IconImg(icon)
                Node { width: px(16), height: px(16), flex_shrink: 0.0 }
                ThemedText
                Pickable::IGNORE
            }
        }
        Node { padding: UiRect::horizontal(px(6)), flex_shrink: 0.0 }
        Tip({tip.to_string()})
    }
}

/// an icon button for code that spawns buttons as it goes
pub fn icon_scene_button(icon: Icon, tip: String, variant: ButtonVariant) -> impl Scene {
    bsn! {
        @FeathersButton {
            @caption: bsn! {
                IconImg(icon)
                Node { width: px(16), height: px(16), flex_shrink: 0.0 }
                ThemedText
                Pickable::IGNORE
            },
            @variant: variant,
        }
        Node { padding: UiRect::horizontal(px(6)), flex_shrink: 0.0 }
        Tip(tip)
    }
}

/// a button with an icon and a short label beside it
pub fn icon_label_button(icon: Icon, label: String, variant: ButtonVariant) -> impl Scene {
    bsn! {
        @FeathersButton {
            @caption: bsn_list! {
                IconImg(icon)
                Node { width: px(15), height: px(15), flex_shrink: 0.0, margin: UiRect::right(px(5)) }
                ThemedText
                Pickable::IGNORE
                --
                @bevy::feathers::display::caption(label)
            },
            @variant: variant,
        }
        Node { flex_shrink: 0.0 }
    }
}

/// spawns an icon as a label under `parent`, in `color` (a palette colour is repainted
/// with the palette)
pub fn spawn_icon(commands: &mut Commands, parent: Entity, icon: Icon, size: f32, color: Color) -> Entity {
    commands
        .spawn((IconImg(icon), icon_node(size), TextColor(color), ChildOf(parent)))
        .id()
}

/// what a strip chip shows: an icon, or a letter or two
#[derive(Clone, Copy)]
pub enum ChipFace {
    Icon(Icon),
    Letter(&'static str),
}

/// a toggle in the strip over the command bar: the size of an icon button, lit while
/// it's on, its full name (and key, if it has one) in its tooltip
pub fn spawn_chip<'a>(
    commands: &'a mut Commands,
    parent: Entity,
    face: ChipFace,
    on: bool,
    tip: impl Into<String>,
    key: Option<Action>,
    fonts: &crate::ui::Fonts,
) -> EntityCommands<'a> {
    let ink = if on { crate::bar::BAR_TEXT } else { crate::bar::BAR_DIM };
    let chip = commands
        .spawn((
            Node {
                width: px(28),
                height: px(24),
                flex_shrink: 0.0,
                display: Display::Flex,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(4)),
                ..default()
            },
            BackgroundColor(if on {
                crate::bar::WHITE.with_alpha(0.16)
            } else {
                Color::NONE
            }),
            BorderColor::all(if on {
                crate::bar::WHITE.with_alpha(0.35)
            } else {
                Color::NONE
            }),
            Tip(tip.into()),
            bevy::picking::cursor::EntityCursor::System(bevy::window::SystemCursorIcon::Pointer),
            ChildOf(parent),
        ))
        .id();
    if let Some(k) = key {
        commands.entity(chip).insert(TipKey(k));
    }
    match face {
        ChipFace::Icon(icon) => {
            let i = spawn_icon(commands, chip, icon, 16.0, ink);
            commands.entity(i).insert(Pickable::IGNORE);
        }
        ChipFace::Letter(l) => {
            commands.spawn((
                Text::new(l),
                TextFont {
                    font: (&fonts.bold).into(),
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextColor(ink),
                Pickable::IGNORE,
                ChildOf(chip),
            ));
        }
    }
    commands.entity(chip)
}

/// gives each new icon its image
pub fn attach_icons(
    icons: Res<Icons>,
    fresh: Query<(Entity, &IconImg, Option<&TextColor>), Added<IconImg>>,
    mut commands: Commands,
) {
    for (e, icon, color) in &fresh {
        let mut image = ImageNode::new(icons.get(icon.0));
        if let Some(c) = color {
            image.color = c.0;
        }
        commands.entity(e).try_insert(image);
    }
}

/// an icon takes its text colour: a button's (propagated to it), or its own; checked
/// every frame (there are few icons), since zen and the palette change colours in place.
/// A button's propagated colour is put back over zen's fade, so an icon in the chrome
/// fades here, with zen
pub fn tint_icons(
    mut q: Query<(Entity, Option<&TextColor>, &mut ImageNode), With<IconImg>>,
    parents: Query<&ChildOf>,
    chrome: Query<(), With<crate::zen::Chrome>>,
    zen: Res<crate::zen::Zen>,
) {
    for (e, c, mut img) in &mut q {
        let Some(c) = c else { continue };
        let mut want = c.0;
        // its own colour was faded by zen already; a propagated one comes back whole
        if zen.fade < 1.0 && want.alpha() > zen.fade + 0.01 && parents.iter_ancestors(e).any(|a| chrome.contains(a)) {
            want = want.with_alpha(want.alpha() * zen.fade);
        }
        if img.color != want {
            img.color = want;
        }
    }
}

pub fn spawn_tooltip(mut commands: Commands, fonts: Res<crate::ui::Fonts>) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            padding: UiRect::axes(px(8), px(4)),
            border_radius: BorderRadius::all(px(4)),
            max_width: px(360),
            ..default()
        },
        BackgroundColor(crate::theme::colors::NAV),
        GlobalZIndex(1000),
        Pickable::IGNORE,
        Tooltip,
        children![(
            Text::new(""),
            TextFont {
                font: (&fonts.regular).into(),
                font_size: FontSize::Px(12.0),
                ..default()
            },
            TextColor(crate::bar::BAR_TEXT),
            Pickable::IGNORE,
        )],
    ));
}

/// the element under the pointer that has a tip, and since when
#[derive(Default)]
pub struct TipHover {
    target: Option<Entity>,
    since: f64,
}

/// shows the tip of what the pointer rests on, after a short delay
pub fn tooltips(
    hover_map: Option<Res<HoverMap>>,
    parents: Query<&ChildOf>,
    tips: Query<(
        &Tip,
        Option<&TipKey>,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
    )>,
    keymap: Res<Keymap>,
    time: Res<Time>,
    zen: Res<crate::zen::Zen>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    tooltip: Single<(&mut Node, &ComputedNode, &Children), With<Tooltip>>,
    mut texts: Query<&mut Text>,
    mut state: Local<TipHover>,
) {
    let (mut node, tip_computed, children) = tooltip.into_inner();
    // the nearest element with a tip, from what the mouse is over upwards
    let mut target = None;
    if let Some(map) = hover_map.as_ref().and_then(|m| m.get(&PointerId::Mouse)) {
        'outer: for hit in map.keys() {
            for e in std::iter::once(*hit).chain(parents.iter_ancestors(*hit)) {
                if tips.contains(e) {
                    target = Some(e);
                    break 'outer;
                }
            }
        }
    }
    let now = time.elapsed_secs_f64();
    if target != state.target || mouse.get_just_pressed().next().is_some() {
        // a click hides the tip until the pointer moves to something else
        state.since = if target == state.target { f64::INFINITY } else { now };
        state.target = target;
    }
    let shown = target
        .and_then(|t| tips.get(t).ok())
        .filter(|(_, _, c, _, vis)| now - state.since > 0.45 && zen.fade > 0.99 && vis.get() && !c.is_empty());
    let Some((tip, key, computed, transform, _)) = shown else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    };
    let label = match key {
        Some(k) => format!("{}  ·  {}", tip.0, keymap.get(k.0)),
        None => tip.0.clone(),
    };
    for c in children.iter() {
        if let Ok(mut t) = texts.get_mut(c)
            && t.0 != label
        {
            t.0 = label.clone();
        }
    }
    // below the element, or above it near the window's bottom; kept inside the window
    let k = computed.inverse_scale_factor();
    let center = transform.translation * k;
    let half = computed.size() * k / 2.0;
    let size = tip_computed.size() * tip_computed.inverse_scale_factor();
    let s = scale.0.max(0.01);
    let (w, h) = (window.width() / s, window.height() / s);
    let mut top = center.y + half.y + 6.0;
    if top + size.y > h - 4.0 {
        top = center.y - half.y - 6.0 - size.y;
    }
    let left = (center.x - size.x / 2.0).clamp(4.0, (w - size.x - 4.0).max(4.0));
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    if node.left != px(left) {
        node.left = px(left);
    }
    if node.top != px(top.max(4.0)) {
        node.top = px(top.max(4.0));
    }
}

/// where the app's own icon (assets/icon.png, full colour) goes: the left of the top bar
#[derive(Component, Default, Clone)]
pub struct AppMark;

/// gives each new [`AppMark`] the app's icon, decoded once
pub fn fill_app_mark(
    mut commands: Commands,
    marks: Query<Entity, (With<AppMark>, Without<ImageNode>)>,
    mut images: ResMut<Assets<Image>>,
    mut handle: Local<Option<Handle<Image>>>,
) {
    if marks.is_empty() {
        return;
    }
    if handle.is_none() {
        match Image::from_buffer(
            include_bytes!("../assets/icon.png"),
            ImageType::Extension("png"),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::linear(),
            RenderAssetUsages::RENDER_WORLD,
        ) {
            Ok(img) => *handle = Some(images.add(img)),
            Err(e) => {
                warn!("the app's icon didn't decode: {}", e);
                return;
            }
        }
    }
    let Some(h) = handle.clone() else { return };
    for e in &marks {
        commands.entity(e).insert(ImageNode::new(h.clone()));
    }
}
