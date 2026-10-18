//! The pages about the server itself: the web app's front page (when the hub isn't
//! served), `/about` (what this server offers, how to join, its limits), and the terms
//! of use and privacy note, which are the operator's own markdown files (`--terms`,
//! `--privacy`, or `terms.md` and `privacy.md` in the data folder; a placeholder until
//! they're written).

use askama::Template;
use salvo::prelude::*;

use crate::hub::human_size;
use crate::web::{Layout, hub, layout, me, mode, render};

#[derive(Template)]
#[template(path = "landing.html")]
struct Landing {
    l: Layout,
    about: String,
    signed_in: bool,
    waiting: bool,
}

/// `/` when only the web app is served: what it is and the way in (`--local` goes
/// straight to the app)
#[handler]
pub async fn landing(depot: &mut Depot, res: &mut Response) {
    if mode(depot).local {
        return res.render(Redirect::found("/app/"));
    }
    let user = me(depot);
    render(
        res,
        Landing {
            l: layout(depot, "home"),
            about: hub().config.description.clone(),
            signed_in: user.is_some(),
            waiting: user.is_some_and(|u| !crate::hub::Hub::active(u)),
        },
    );
}

#[derive(Template)]
#[template(path = "about.html")]
struct About {
    l: Layout,
    about: String,
    contact: String,
    join_about: &'static str,
    max_upload: String,
    share_quota: String,
    app_quota: String,
    url: String,
    version: &'static str,
}

/// what a quota allows, for people
fn limit(bytes: u64) -> String {
    if bytes == 0 {
        "no limit".into()
    } else {
        human_size(bytes)
    }
}

/// `GET /about`
#[handler]
pub async fn about(depot: &mut Depot, res: &mut Response) {
    let h = hub();
    #[cfg(feature = "app")]
    let app_quota = limit(crate::app::QUOTA.get().copied().unwrap_or(0));
    #[cfg(not(feature = "app"))]
    let app_quota = String::new();
    render(
        res,
        About {
            l: layout(depot, "about"),
            about: h.config.description.clone(),
            contact: h.config.contact.clone(),
            join_about: h.join_policy().about(),
            max_upload: human_size(h.config.max_upload),
            share_quota: limit(h.config.share_quota),
            app_quota,
            url: h.config.url.clone(),
            version: env!("CARGO_PKG_VERSION"),
        },
    );
}

#[derive(Template)]
#[template(path = "doc.html")]
struct Doc {
    l: Layout,
    title: &'static str,
    body: String,
}

const TERMS: &str = "# Terms of use\n\nWhoever runs this server hasn't written its terms yet. They go in `terms.md` in the server's data folder (or the file given with `--terms`), as markdown; `hub/config/terms.example.md` in the anend repository is a start.\n";
const PRIVACY: &str = "# Privacy\n\nWhoever runs this server hasn't written its privacy note yet. It goes in `privacy.md` in the server's data folder (or the file given with `--privacy`), as markdown; `hub/config/privacy.example.md` in the anend repository is a start.\n";

fn doc(depot: &Depot, res: &mut Response, path: &std::path::Path, fallback: &str, tab: &'static str) {
    let md = std::fs::read_to_string(path)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string());
    render(
        res,
        Doc {
            l: layout(depot, tab),
            title: tab,
            body: crate::guide::to_html(&md),
        },
    );
}

/// `GET /terms`
#[handler]
pub async fn terms(depot: &mut Depot, res: &mut Response) {
    doc(depot, res, &hub().config.terms, TERMS, "terms");
}

/// `GET /privacy`
#[handler]
pub async fn privacy(depot: &mut Depot, res: &mut Response) {
    doc(depot, res, &hub().config.privacy, PRIVACY, "privacy");
}
