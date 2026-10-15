//! (being written)

use dioxus::prelude::*;

#[component]
pub fn Notes() -> Element {
    rsx! { div { class: "page", div { class: "card empty", "coming next" } } }
}
