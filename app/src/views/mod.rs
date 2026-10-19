//! The views. Every view stays mounted (the space keeps its scene, each view its page and
//! scroll) and loads when first shown; only the open one is visible.

pub mod classifiers;
pub mod clsmore;
pub mod flt;
pub mod judge;
pub mod lists;
pub mod memory;
pub mod models;
pub mod notes;
pub mod pictures;
pub mod reader;
pub mod settings;
pub mod space;
pub mod specs;
pub mod stats;
pub mod tags;

use dioxus::prelude::*;

use crate::ui::*;

#[component]
pub fn ViewHost(view: View) -> Element {
    let ui = ui();
    let open = (ui.view)() == view;
    // a view is built the first time it's shown, then kept
    let mut seen = use_signal(|| false);
    if open && !*seen.peek() {
        seen.set(true);
    }
    if !seen() {
        return rsx! {};
    }
    rsx! {
        section {
            class: "view {view.name()}",
            hidden: !open,
            "aria-label": "{view.name()}",
            match view {
                View::Memory => rsx! { memory::Memory {} },
                View::Space => rsx! { space::Space {} },
                View::Lists => rsx! { lists::Lists {} },
                View::Reader => rsx! { reader::Reader {} },
                View::Notes => rsx! { notes::Notes {} },
                View::Tags => rsx! { tags::Tags {} },
                View::Stats => rsx! { stats::Stats {} },
                View::Models => rsx! { classifiers::Models {} },
                View::Filters => rsx! { models::Filters {} },
            }
        }
    }
}

/// what a view adds at the command bar's right end: the memory view's and the space's sums
/// and toggles, the other views' counts and hints
#[component]
pub fn Side(view: View) -> Element {
    let ui = ui();
    match view {
        View::Memory => rsx! { memory::MemoryStrip {} },
        View::Space => rsx! { space::SpaceStrip {} },
        _ => {
            let note = ui.side.read().get(&view).cloned().unwrap_or_default();
            rsx! { span { class: "sidenote", "{note}" } }
        }
    }
}

/// registers a view's key handler with the shell
pub fn use_keys(view: View, f: impl FnMut(KeyPress) -> bool + 'static) {
    let mut ui = ui();
    let cb = use_callback(f);
    use_hook(move || {
        ui.keys.write().push((view, cb));
    });
}

/// a toggle as small as an icon or a letter; its name and key in the tip
#[component]
pub fn TChip(
    label: String,
    #[props(default)] icon: String,
    #[props(default)] letter: String,
    on: bool,
    onclick: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            class: if on { "tchip on tip-up tip-left" } else { "tchip tip-up tip-left" },
            "data-tip": "{label}",
            "aria-label": "{label}",
            "aria-pressed": "{on}",
            onclick: move |_| onclick.call(()),
            if !icon.is_empty() {
                Icon { name: icon.clone() }
            } else {
                "{letter}"
            }
        }
    }
}

/// a section that folds, remembered by name
#[component]
pub fn Fold(name: String, title: String, children: Element) -> Element {
    let ui = ui();
    let folded = ui.prefs.read().folded(&name);
    let n = name.clone();
    rsx! {
        div { class: if folded { "section folded" } else { "section" },
            button {
                class: "shead",
                "aria-expanded": "{!folded}",
                onclick: move |_| {
                    crate::shell::set_pref(ui, |p| {
                        if !p.collapsed.remove(&n) {
                            p.collapsed.insert(n.clone());
                        }
                    })
                },
                Icon { name: "chevron-right", class: "chev" }
                "{title}"
            }
            if !folded {
                div { class: "body", {children} }
            }
        }
    }
}
