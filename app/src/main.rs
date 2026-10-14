//! anend's app with Dioxus: the desktop app (the system webview) and, from the same code,
//! the web app (WebAssembly). Work in progress; see docs/next.md, "The Dioxus app".

// the backend and the numbers are written ahead of the views that use them
#![allow(dead_code)]

mod backend;
mod gem;

use dioxus::prelude::*;

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! { div { "anend" } }
}
