//! The fonts the settings pick. The interface font replaces the faces behind the app's
//! font handles (regular, bold, italic, bold italic), so every text that uses them,
//! the space's slug texts included, follows without knowing. The memory view's font is a
//! family name tried before the system serif.

use bevy::prelude::*;
use bevy::text::{FontSource, GenericFontFamily};
use fontique::{Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, SourceCache};

use crate::config::Config;
use crate::memory_ui::{MemoryState, SerifFont};
use crate::space::{SlugFont, SpaceText};
use crate::ui::{Fonts, Status};

/// the system's font families, sorted
pub fn system_families() -> Vec<String> {
    let mut c = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    let mut names: Vec<String> = c.family_names().map(String::from).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// a family's regular, bold, italic and bold italic faces (the nearest it has)
fn faces(family: &str) -> Option<[Vec<u8>; 4]> {
    let mut c = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: true,
    });
    let mut cache = SourceCache::default();
    let fam = c.family_by_name(family)?;
    let mut get = |style: FontStyle, weight: FontWeight| {
        let f = fam.match_font(FontWidth::NORMAL, style, weight, false)?;
        // a face inside a collection file can't be told apart once it's a font asset
        if f.index() != 0 {
            return None;
        }
        f.load(Some(&mut cache)).map(|b| b.data().to_vec())
    };
    let regular = get(FontStyle::Normal, FontWeight::NORMAL)?;
    let bold = get(FontStyle::Normal, FontWeight::BOLD).unwrap_or_else(|| regular.clone());
    let italic = get(FontStyle::Italic, FontWeight::NORMAL).unwrap_or_else(|| regular.clone());
    let bold_italic = get(FontStyle::Italic, FontWeight::BOLD).unwrap_or_else(|| bold.clone());
    Some([regular, bold, italic, bold_italic])
}

#[derive(Resource, Default)]
pub struct Typeface {
    /// the built in faces, to go back to
    originals: Option<[Vec<u8>; 4]>,
    /// the family that's in place (none: the built in one)
    applied: Option<Option<String>>,
    /// faces waiting a frame: the text system only reads a font again after it's gone
    pending: Option<[Vec<u8>; 4]>,
    memory_applied: Option<Option<String>>,
}

/// swaps the interface font when the settings change it
pub fn apply_typeface(
    fonts: Res<Fonts>,
    mut assets: ResMut<Assets<Font>>,
    config: Res<Config>,
    mut tf: ResMut<Typeface>,
    mut slug: ResMut<SlugFont>,
    mut texts: Query<&mut SpaceText>,
    mut status: ResMut<Status>,
    time: Res<Time>,
) {
    let ids = [
        fonts.regular.id(),
        fonts.bold.id(),
        fonts.italic.id(),
        fonts.bold_italic.id(),
    ];
    if tf.originals.is_none() {
        let loaded: Option<Vec<Vec<u8>>> = ids
            .iter()
            .map(|id| assets.get(*id).map(|f| f.data.data().to_vec()))
            .collect();
        match loaded {
            Some(v) => tf.originals = v.try_into().ok(),
            None => return,
        }
    }
    if let Some(new) = tf.pending.take() {
        for (id, bytes) in ids.into_iter().zip(new) {
            let _ = assets.insert(id, Font::from_bytes(bytes));
        }
        slug.reset();
        for mut t in texts.iter_mut() {
            t.relayout = true;
        }
        return;
    }
    if tf.applied.as_ref() == Some(&config.font) {
        return;
    }
    let first = tf.applied.is_none();
    tf.applied = Some(config.font.clone());
    let new = match &config.font {
        None if first => return,
        None => tf.originals.clone(),
        Some(family) => {
            let found = faces(family);
            if found.is_none() {
                status.text = format!("couldn't load the font {}", family);
                status.until = time.elapsed_secs_f64() + 4.0;
            }
            found
        }
    };
    if let Some(new) = new {
        for id in ids {
            assets.remove(id);
        }
        tf.pending = Some(new);
    }
}

/// the memory view's font
pub fn apply_memory_font(
    config: Res<Config>,
    mut tf: ResMut<Typeface>,
    mut serif: ResMut<SerifFont>,
    fonts: Res<Fonts>,
    mut mem: ResMut<MemoryState>,
) {
    if tf.memory_applied.as_ref() == Some(&config.memory_font) {
        return;
    }
    tf.memory_applied = Some(config.memory_font.clone());
    let mut list = Vec::new();
    if let Some(f) = &config.memory_font {
        list.push(FontSource::Family(f.clone().into()));
    }
    list.push(FontSource::Family("Times New Roman".into()));
    list.push(FontSource::from(GenericFontFamily::Serif));
    list.push(FontSource::Handle(fonts.regular.clone()));
    serif.0 = FontSource::from(list);
    mem.redraw_results = true;
    mem.redraw_chrome = true;
    mem.redraw_toasts = true;
}
