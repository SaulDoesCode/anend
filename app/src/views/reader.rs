//! (being written)

use dioxus::prelude::*;

#[component]
pub fn Reader() -> Element {
    rsx! { div { class: "page", div { class: "card empty", "coming next" } } }
}
