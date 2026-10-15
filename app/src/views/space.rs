//! (being written)

use dioxus::prelude::*;

#[component]
pub fn Space() -> Element {
    rsx! { div { class: "page", div { class: "card empty", "coming next" } } }
}

#[component]
pub fn SpaceStrip() -> Element {
    rsx! { div { class: "page", div { class: "card empty", "coming next" } } }
}
