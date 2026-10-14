//! The pages every mode has, and what they share: who's asking, the layout, joining and
//! logging in, profiles (with levels and badges on a hub), app tokens, the admin panel,
//! the static files. The hub's own pages are in saves.rs and clsweb.rs. Pages are askama
//! templates; forms post back and redirect with a one-line message in a cookie.

use askama::Template;
use chrono::{DateTime, Utc};
use salvo::http::cookie::{Cookie, SameSite};
use salvo::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use salvo::prelude::*;
use std::net::IpAddr;
use std::sync::OnceLock;

use crate::auth;
use crate::game;
use crate::hub::{Hub, Join, human_size};
use crate::inspect;
use crate::mode::Mode;
use crate::store::{ClassifierRec, Role, Save, Standing, Status, User, Visibility, unhex};

pub static HUB: OnceLock<Hub> = OnceLock::new();

pub fn hub() -> &'static Hub {
    HUB.get().expect("the hub starts before serving")
}

const FLASH: &str = "hub_flash";

/// puts the router's mode where every handler can read it (`mode`)
pub struct SetMode(pub Mode);

#[handler]
impl SetMode {
    async fn handle(&self, depot: &mut Depot) {
        depot.insert("mode", self.0);
    }
}

/// what the server runs, for this request
pub fn mode(depot: &Depot) -> Mode {
    depot.get::<Mode>("mode").copied().unwrap_or_else(|_| crate::mode::get())
}

// ------------------------------------------------------------------ who's asking

/// trust X-Forwarded-For (the hub sits behind a proxy)
pub static BEHIND_PROXY: OnceLock<bool> = OnceLock::new();

pub fn client_ip(req: &Request) -> IpAddr {
    if *BEHIND_PROXY.get().unwrap_or(&false)
        && let Some(ip) = req
            .header::<String>("x-forwarded-for")
            .and_then(|v| v.split(',').next().and_then(|s| s.trim().parse().ok()))
    {
        return ip;
    }
    req.remote_addr().ip().unwrap_or(IpAddr::from([0, 0, 0, 0]))
}

/// whether a request's `Host` may reach a `--local` hub, which answers anyone without a
/// login. A page on another site can make the browser send requests to a name it
/// controls that resolves to this machine (DNS rebinding); such a request carries that
/// name as its Host. So a local hub answers only to loopback names (`localhost`,
/// `*.localhost`), IP addresses (which no other site can rebind), and the host of its
/// own `--url`.
pub fn local_host_ok(host: &str, url: &str) -> bool {
    fn name(h: &str) -> String {
        let h = h.trim().to_ascii_lowercase();
        if let Some(rest) = h.strip_prefix('[') {
            // [v6]:port
            return rest.split(']').next().unwrap_or("").to_string();
        }
        match h.rsplit_once(':') {
            Some((n, port)) if port.bytes().all(|b| b.is_ascii_digit()) && !n.contains(':') => n.to_string(),
            _ => h,
        }
    }
    let host = name(host);
    if host.is_empty() {
        return false;
    }
    let own = url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("");
    host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok()
        || (!own.is_empty() && host == name(own))
}

/// headers every answer carries: no framing by other sites (the admin panel's buttons
/// can't be clicked through a disguise), no sniffing a download into a page, and
/// addresses kept to the hub when following a link out. The web app's page also gets a
/// content security policy: scripts only from the hub, so a note that slipped some html
/// past the markdown renderer still couldn't run anything.
fn guard_headers(req: &Request, res: &mut Response) {
    let h = res.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    let csp = if req.uri().path().starts_with("/app") {
        APP_CSP
    } else {
        "frame-ancestors 'none'; base-uri 'self'; object-src 'none'"
    };
    h.insert("content-security-policy", HeaderValue::from_static(csp));
}

/// the web app's policy: its own scripts only; styles inline too (the editor's mirror and
/// the views set them); pictures in notes from anywhere on the web; calls to the hub
pub const APP_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; connect-src 'self' ws: wss:; font-src 'self' data:; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

/// loads the user behind the session cookie (or app token), turns away forms posted from
/// other sites, and picks up the message left by the last redirect
#[handler]
pub async fn who_is_asking(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
    guard_headers(req, res);
    if mode(depot).local
        && !local_host_ok(&req.header::<String>("host").unwrap_or_default(), &hub().config.url)
    {
        res.status_code(StatusCode::FORBIDDEN);
        res.render(Text::Plain(
            "this hub runs with --local and only answers to localhost, an IP address or its --url",
        ));
        ctrl.skip_rest();
        return;
    }
    if req.method() == salvo::http::Method::POST && !req.uri().path().starts_with("/api/") {
        let cross = req
            .header::<String>("sec-fetch-site")
            .is_some_and(|s| s == "cross-site");
        let host = req.header::<String>("host").unwrap_or_default();
        let origin_ok = req
            .header::<String>("origin")
            .map(|o| o.split("://").nth(1).unwrap_or("") == host)
            .unwrap_or(true);
        if cross || !origin_ok {
            res.status_code(StatusCode::FORBIDDEN);
            res.render(Text::Plain("forms are only taken from the hub's own pages"));
            ctrl.skip_rest();
            return;
        }
    }
    let user = if let Some(c) = req.cookie(auth::COOKIE) {
        let hash = auth::digest(c.value());
        hub()
            .store
            .session(&hash)
            .ok()
            .flatten()
            .and_then(|s| hub().store.user(&s.user).ok().flatten())
    } else if let Some(bearer) = req
        .header::<String>("authorization")
        .and_then(|h| h.strip_prefix("Bearer ").map(|t| t.trim().to_string()))
    {
        hub()
            .store
            .token(&auth::digest(&bearer))
            .ok()
            .flatten()
            .and_then(|t| hub().store.user(&t.user).ok().flatten())
    } else {
        None
    };
    if let Some(u) = user.filter(|u| u.standing != Standing::Banned) {
        depot.insert("me", u);
    }
    if let Some(f) = req.cookie(FLASH) {
        depot.insert("flash", decode(f.value()));
        res.add_cookie(
            Cookie::build((FLASH, ""))
                .path("/")
                .max_age(salvo::http::cookie::time::Duration::ZERO)
                .build(),
        );
    }
}

pub fn me(depot: &Depot) -> Option<&User> {
    depot.get::<User>("me").ok()
}

// ------------------------------------------------------------------ helpers

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        if b[i] == b'%'
            && let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// a url query value
pub fn url(s: &str) -> String {
    encode(s)
}

/// leaves a message for the next page and goes there; a message starting with `!` is bad news
pub fn go(res: &mut Response, to: &str, msg: &str) {
    if !msg.is_empty() {
        res.add_cookie(
            Cookie::build((FLASH, encode(msg)))
                .path("/")
                .http_only(true)
                .same_site(SameSite::Lax)
                .build(),
        );
    }
    res.render(Redirect::other(to));
}

pub fn ago(t: DateTime<Utc>) -> String {
    let s = (Utc::now() - t).num_seconds().max(0);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        86400..2_592_000 => format!("{} days ago", s / 86400),
        _ => t.format("%d %b %Y").to_string(),
    }
}

/// a colour for a name, from its gematria
pub fn hue(name: &str) -> u64 {
    let (o, r) = inspect::sums(name);
    (o * 37 + r * 11) % 360
}

pub fn render(res: &mut Response, t: impl Template) {
    match t.render() {
        Ok(html) => res.render(Text::Html(html)),
        Err(e) => {
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Text::Plain(format!("couldn't draw the page: {e}")));
        }
    }
}

pub struct MeView {
    pub name: String,
    pub level: u32,
    pub title: &'static str,
    pub points: i64,
    pub admin: bool,
}

pub struct Layout {
    pub hub: String,
    pub tab: &'static str,
    pub me: Option<MeView>,
    pub flash: String,
    pub flash_bad: bool,
    /// saves and people waiting for an admin
    pub waiting: usize,
    /// the sharing site is served
    pub hub_on: bool,
    /// the web app is served
    pub app_on: bool,
    /// the web app alone, for one person, with no login
    pub local: bool,
    /// how people join: open, apply or invite
    pub join: &'static str,
    /// the asker may make invite codes
    pub can_invite: bool,
    /// the server's line about itself, for the pages' description
    pub about: String,
}

pub fn layout(depot: &Depot, tab: &'static str) -> Layout {
    let flash = depot.get::<String>("flash").cloned().unwrap_or_default();
    let (flash, flash_bad) = match flash.strip_prefix('!') {
        Some(f) => (f.to_string(), true),
        None => (flash, false),
    };
    let m = mode(depot);
    let can_invite = me(depot).is_some_and(|u| hub().may_invite(u));
    let me = me(depot).map(|u| {
        let l = game::level(u.points);
        MeView {
            name: u.name.clone(),
            level: l,
            title: game::title(l),
            points: u.points,
            admin: u.is_admin(),
        }
    });
    let waiting = if me.as_ref().is_some_and(|m| m.admin) {
        let u = hub().store.users().unwrap_or_default();
        let shared = if m.hub {
            let s = hub().store.saves().unwrap_or_default();
            let c = hub().store.classifiers().unwrap_or_default();
            s.iter().filter(|s| s.waiting()).count() + c.iter().filter(|c| c.waiting()).count()
        } else {
            0
        };
        shared + u.iter().filter(|u| u.standing == Standing::Applied).count()
    } else {
        0
    };
    Layout {
        hub: hub().config.name.clone(),
        tab,
        me,
        flash,
        flash_bad,
        waiting,
        hub_on: m.hub,
        app_on: m.app,
        local: m.local,
        join: hub().join_policy().name(),
        can_invite,
        about: hub().config.description.clone(),
    }
}

/// where "back" goes from a message: the front page, named for what's on it
fn home_label(depot: &Depot) -> &'static str {
    if mode(depot).hub { "to the saves" } else { "to the start" }
}

#[derive(Template)]
#[template(path = "message.html")]
struct Message {
    l: Layout,
    heading: String,
    text: String,
    back: String,
    back_label: String,
}

pub fn message(res: &mut Response, depot: &Depot, code: StatusCode, heading: &str, text: &str) {
    res.status_code(code);
    render(
        res,
        Message {
            l: layout(depot, ""),
            heading: heading.into(),
            text: text.into(),
            back: "/".into(),
            back_label: home_label(depot).into(),
        },
    );
}

pub(crate) fn not_found(res: &mut Response, depot: &Depot) {
    message(
        res,
        depot,
        StatusCode::NOT_FOUND,
        "not here",
        "There's nothing at this address.",
    );
}

/// the logged in, active user, or a message saying why not
#[cfg_attr(not(feature = "hub"), allow(dead_code))]
pub(crate) fn need_active<'a>(res: &mut Response, depot: &'a Depot) -> Option<&'a User> {
    match me(depot) {
        None => {
            go(res, "/login", "!log in first");
            None
        }
        Some(u) if u.standing != Standing::Active => {
            message(
                res,
                depot,
                StatusCode::FORBIDDEN,
                "not yet",
                if mode(depot).hub {
                    "Your account is waiting for an admin to let you in. You can share and star saves once they do."
                } else {
                    "Your account is waiting for an admin to let you in. You can use the app once they do."
                },
            );
            None
        }
        Some(u) => Some(u),
    }
}

pub fn need_admin<'a>(res: &mut Response, depot: &'a Depot) -> Option<&'a User> {
    match me(depot) {
        Some(u) if u.is_admin() => Some(u),
        _ => {
            not_found(res, depot);
            None
        }
    }
}

// ------------------------------------------------------------------ cards and rows
//
// what the grids, profiles and the admin panel show of a save or a classifier

pub struct Card {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
    pub notes: u64,
    pub memories: u64,
    pub stars: u64,
    pub downloads: u64,
    pub featured: bool,
    pub hue: u64,
    pub ordinal: u64,
    pub reversed: u64,
    pub ago: String,
}

pub fn card(s: &Save) -> Card {
    let (ordinal, reversed) = inspect::sums(&s.name);
    let description: String = s.description.chars().take(160).collect();
    Card {
        id: s.id.clone(),
        name: s.name.clone(),
        author: s.author.clone(),
        description,
        tags: s.tags.clone(),
        notes: s.notes,
        memories: s.memories,
        stars: s.stars,
        downloads: s.downloads,
        featured: s.featured,
        hue: hue(&s.name),
        ordinal,
        reversed,
        ago: ago(s.updated),
    }
}

pub fn status_name(s: Status) -> &'static str {
    match s {
        Status::Pending => "pending",
        Status::Approved => "approved",
        Status::Rejected => "rejected",
    }
}

/// a save the asker may see: approved ones, or their own, or any for an admin
/// a classifier in the grid
pub struct ClsCard {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
    pub classes: Vec<String>,
    /// classes not shown on the card
    pub more: usize,
    pub n_classes: usize,
    pub accuracy: String,
    pub preset: String,
    pub weights: String,
    pub stars: u64,
    pub downloads: u64,
    pub featured: bool,
    pub hue: u64,
    pub ago: String,
}

/// a share as a whole percent, or empty
pub fn percent(x: Option<f64>) -> String {
    x.filter(|v| v.is_finite())
        .map(|v| format!("{:.0}%", v * 100.0))
        .unwrap_or_default()
}

/// a weight count, short
pub fn count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.0}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

pub fn cls_card(c: &ClassifierRec) -> ClsCard {
    const SHOWN: usize = 6;
    ClsCard {
        id: c.id.clone(),
        name: c.name.clone(),
        author: c.author.clone(),
        description: c.description.chars().take(160).collect(),
        tags: c.tags.clone(),
        classes: c.classes.iter().take(SHOWN).cloned().collect(),
        more: c.classes.len().saturating_sub(SHOWN),
        n_classes: c.classes.len(),
        accuracy: percent(c.accuracy),
        preset: c.preset.clone(),
        weights: count(c.weights),
        stars: c.stars,
        downloads: c.downloads,
        featured: c.featured,
        hue: hue(&c.name),
        ago: ago(c.updated),
    }
}

/// a classifier's row in the admin panel
pub struct AdminCls {
    pub id: String,
    pub name: String,
    pub author: String,
    pub classes: String,
    pub accuracy: String,
    pub preset: String,
    pub size: String,
    pub ago: String,
    pub version: u32,
    pub status: &'static str,
    pub featured: bool,
    pub stars: u64,
    pub downloads: u64,
    pub needs: usize,
    pub visibility: &'static str,
}

pub fn admin_cls(c: &ClassifierRec) -> AdminCls {
    let mut classes = c.classes.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    if c.classes.len() > 5 {
        classes.push_str(&format!(" +{}", c.classes.len() - 5));
    }
    AdminCls {
        id: c.id.clone(),
        name: c.name.clone(),
        author: c.author.clone(),
        classes,
        accuracy: percent(c.accuracy),
        preset: c.preset.clone(),
        size: human_size(c.size),
        ago: ago(c.updated),
        version: c.version,
        status: status_name(c.status),
        featured: c.featured,
        stars: c.stars,
        downloads: c.downloads,
        needs: c.upstream.iter().filter(|u| !u.1).count(),
        visibility: c.visibility.name(),
    }
}

// ------------------------------------------------------------------ forms

/// one of the three visibilities, as the controls show it
pub struct VisChoice {
    pub name: &'static str,
    pub icon: &'static str,
    pub tip: &'static str,
}

/// private, unlisted and public, with their icons and what each means
pub fn vis_choices() -> [VisChoice; 3] {
    [
        VisChoice {
            name: "private",
            icon: "eye-off",
            tip: "private: only you see it, and only you can download it",
        },
        VisChoice {
            name: "unlisted",
            icon: "link",
            tip: "unlisted: anyone with its link; never in the grid, search or feeds",
        },
        VisChoice {
            name: "public",
            icon: "hub",
            tip: "public: in the grid once an admin lets it in",
        },
    ]
}

pub struct Field {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub value: String,
    pub hint: &'static str,
    pub required: bool,
}

pub(crate) fn field(name: &'static str, label: &'static str, kind: &'static str, value: &str) -> Field {
    Field {
        name,
        label,
        kind,
        value: value.to_string(),
        hint: "",
        required: false,
    }
}

#[derive(Template)]
#[template(path = "form.html")]
pub(crate) struct Form {
    pub l: Layout,
    pub heading: String,
    pub intro: String,
    pub action: String,
    pub multipart: bool,
    pub fields: Vec<Field>,
    pub submit: &'static str,
    pub after: String,
}

// ------------------------------------------------------------------ joining and logging in

#[handler]
pub async fn join_page(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let code = req.query::<String>("invite").unwrap_or_default();
    join_form(depot, res, &code);
}

/// the join form, with an invite code filled in when there is one
pub fn join_form(depot: &Depot, res: &mut Response, code: &str) {
    let policy = hub().join_policy();
    let m = mode(depot);
    let what = match (m.hub, m.app) {
        (true, true) => "to keep your saves here and use the app in the browser, share saves, star the ones you like and climb the levels",
        (true, false) => "to share saves, star the ones you like and climb the levels",
        _ => "to keep your saves here and use the app in the browser",
    };
    // who sent the invite, when it's good
    let from = (!code.is_empty())
        .then(|| hub().store.invite(&crate::invites::hash(code)).ok().flatten())
        .flatten()
        .filter(|i| i.spent().is_none())
        .map(|i| i.by);
    let intro = match (policy, &from) {
        (_, Some(by)) => format!("{by} invited you. Make an account {what}."),
        (Join::Open, _) => format!("Make an account {what}."),
        (Join::Apply, _) => format!(
            "Apply for an account {what}: an admin lets people in. Say a little about why you'd like to join. (With an invite code you're in at once.)"
        ),
        (Join::Invite, _) => format!(
            "This server is invite-only: you need an invite code from a member. Make an account {what}."
        ),
    };
    let mut f = vec![
        Field {
            required: true,
            hint: "letters, digits, - and _",
            ..field("name", "name", "text", "")
        },
        Field {
            hint: "optional; only admins see it, and nothing is ever sent to it",
            ..field("email", "email", "email", "")
        },
        Field {
            required: true,
            hint: "8 characters or more",
            ..field("password", "password", "password", "")
        },
    ];
    if policy != Join::Open {
        f.push(Field {
            required: policy == Join::Invite,
            hint: if policy == Join::Invite {
                "from the invite link you were given"
            } else {
                "optional: with one you're in at once"
            },
            ..field("invite", "invite code", "text", code.trim())
        });
    }
    if policy == Join::Apply && from.is_none() {
        f.push(field("why", "why you'd like to join", "textarea", ""));
    }
    render(
        res,
        Form {
            l: layout(depot, "join"),
            heading: "join".into(),
            intro,
            action: "/join".into(),
            multipart: false,
            fields: f,
            submit: if policy == Join::Apply && from.is_none() { "apply" } else { "join" },
            after: "By joining you agree to the terms (see the foot of the page). Lost your password? An admin can make you a login link.".into(),
        },
    );
}

#[handler]
pub async fn join(req: &mut Request, res: &mut Response) {
    if !hub().limiter.allow(client_ip(req), "join", 5, 3600) {
        return go(res, "/join", "!too many tries from here; try again later");
    }
    let name = req.form::<String>("name").await.unwrap_or_default().trim().to_string();
    let email = req.form::<String>("email").await.unwrap_or_default().trim().to_string();
    let password = req.form::<String>("password").await.unwrap_or_default();
    let code = req.form::<String>("invite").await.unwrap_or_default().trim().to_string();
    let why: String = req
        .form::<String>("why")
        .await
        .unwrap_or_default()
        .chars()
        .take(1000)
        .collect();
    let back = if code.is_empty() {
        "/join".to_string()
    } else {
        format!("/join?invite={}", url(&code))
    };
    if let Err(e) = auth::name_ok(&name).and(auth::password_ok(&password)) {
        return go(res, &back, &format!("!{e}"));
    }
    if email.len() > 200 || (!email.is_empty() && !email.contains('@')) {
        return go(res, &back, "!that email doesn't look right");
    }
    if !email.is_empty() && hub().store.user_by_email(&email).ok().flatten().is_some() {
        return go(res, &back, "!that email has an account already");
    }
    let policy = hub().join_policy();
    // an invite code: checked before anything is made (a wrong one counts against the
    // address, so codes can't be guessed)
    let hash = crate::invites::hash(&code);
    let invited = if policy == Join::Open || code.is_empty() {
        None
    } else {
        if !hub().limiter.allow(client_ip(req), "invite", 10, 3600) {
            return go(res, &back, "!too many invite codes tried from here; try again later");
        }
        match hub().store.invite(&hash).ok().flatten() {
            Some(i) => match i.spent() {
                None => Some(i),
                Some(why) => return go(res, &back, &format!("!{why}")),
            },
            None => return go(res, &back, "!that invite code isn't known here"),
        }
    };
    if policy == Join::Invite && invited.is_none() {
        return go(res, &back, "!this server is invite-only: you need an invite code");
    }
    if hub().store.user(&name).ok().flatten().is_some() {
        return go(res, &back, "!that name is taken");
    }
    let pw = match auth::hash_password(&password) {
        Ok(h) => h,
        Err(e) => return go(res, &back, &format!("!{e}")),
    };
    // the invite's use is taken before the account is made: two people can't share its last use
    let invited_by = match invited {
        Some(_) => match hub().store.use_invite(&hash, &name) {
            Ok(i) => i.by,
            Err(e) => return go(res, &back, &format!("!{e}")),
        },
        None => String::new(),
    };
    let active = policy == Join::Open || !invited_by.is_empty();
    let user = User {
        name: name.clone(),
        email,
        password: pw,
        why: why.trim().to_string(),
        standing: if active { Standing::Active } else { Standing::Applied },
        joined: Utc::now(),
        invited_by,
        ..Default::default()
    };
    match hub().store.add_user(user) {
        Ok(u) => {
            if active {
                hub().store.log(&u.name, "joined", "").ok();
            }
            log_in(res, &u.name);
            if active {
                go(res, &format!("/u/{}", u.name), "welcome! you're in");
            } else {
                go(res, "/", "applied: an admin will look at it soon");
            }
        }
        Err(e) => go(res, &back, &format!("!{e}")),
    }
}

/// a page on this server to go to after logging in (`next`), or none
fn local_next(next: &str) -> Option<&str> {
    (next.starts_with('/') && !next.starts_with("//") && !next.contains('\\') && next.len() < 200).then_some(next)
}

fn log_in(res: &mut Response, name: &str) {
    if let Ok(secret) = auth::start_session(&hub().store, name) {
        let secure = hub().config.url.starts_with("https://");
        res.add_cookie(
            Cookie::build((auth::COOKIE, secret))
                .path("/")
                .http_only(true)
                .secure(secure)
                .same_site(SameSite::Lax)
                .max_age(salvo::http::cookie::time::Duration::days(auth::SESSION_DAYS))
                .build(),
        );
    }
}

#[handler]
pub async fn login_page(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let next = req.query::<String>("next").unwrap_or_default();
    let next = local_next(&next).unwrap_or("").to_string();
    render(
        res,
        Form {
            l: layout(depot, "login"),
            heading: "log in".into(),
            intro: String::new(),
            action: if next.is_empty() {
                "/login".into()
            } else {
                format!("/login?next={}", url(&next))
            },
            multipart: false,
            fields: vec![
                Field {
                    required: true,
                    ..field("name", "name or email", "text", "")
                },
                Field {
                    required: true,
                    ..field("password", "password", "password", "")
                },
            ],
            submit: "log in",
            after: "Forgot your password? An admin can make you a login link.".into(),
        },
    );
}

#[handler]
pub async fn login(req: &mut Request, res: &mut Response) {
    if !hub().limiter.allow(client_ip(req), "login", 10, 600) {
        return go(res, "/login", "!too many tries; wait a few minutes");
    }
    let next = req.query::<String>("next").unwrap_or_default();
    let next = local_next(&next).unwrap_or("/").to_string();
    let name = req.form::<String>("name").await.unwrap_or_default();
    let password = req.form::<String>("password").await.unwrap_or_default();
    let user = match hub().store.user(name.trim()) {
        Ok(Some(u)) => Some(u),
        _ => hub().store.user_by_email(&name).ok().flatten(),
    };
    match user {
        Some(u)
            if u.standing != Standing::Banned
                && !u.password.is_empty()
                && auth::check_password(&password, &u.password) =>
        {
            log_in(res, &u.name);
            go(res, &next, &format!("welcome back, {}", u.name));
        }
        other => {
            // as slow as a real check, so the answer's time doesn't say who has an account
            if other.is_none_or(|u| u.password.is_empty()) {
                auth::check_password(&password, auth::dummy_hash());
            }
            let back = if next == "/" { "/login".to_string() } else { format!("/login?next={}", url(&next)) };
            go(res, &back, "!that name and password don't match")
        }
    }
}

/// a one time link from the terminal or an admin
#[handler]
pub async fn login_link(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let secret = req.param::<String>("secret").unwrap_or_default();
    match hub().store.take_link(&auth::digest(&secret)) {
        Ok(Some(link)) => {
            log_in(res, &link.user);
            go(
                res,
                &format!("/u/{}", link.user),
                "logged in: set a password below if you haven't",
            );
        }
        _ => message(
            res,
            depot,
            StatusCode::FORBIDDEN,
            "that link's used up",
            "A login link works once, within an hour. Ask for a new one.",
        ),
    }
}

#[handler]
pub async fn logout(req: &mut Request, res: &mut Response) {
    if let Some(c) = req.cookie(auth::COOKIE) {
        hub().store.drop_session(&auth::digest(c.value())).ok();
    }
    res.add_cookie(
        Cookie::build((auth::COOKIE, ""))
            .path("/")
            .max_age(salvo::http::cookie::time::Duration::ZERO)
            .build(),
    );
    go(res, "/", "logged out");
}

// ------------------------------------------------------------------ people

pub struct BadgeView {
    pub name: &'static str,
    pub about: &'static str,
    pub glyph: &'static str,
}

#[derive(Template)]
#[template(path = "profile.html")]
struct Profile {
    l: Layout,
    u: User,
    hue: u64,
    initial: String,
    level: u32,
    title: &'static str,
    progress: u32,
    next: i64,
    joined: String,
    standing: &'static str,
    badges: Vec<BadgeView>,
    cards: Vec<Card>,
    cls_cards: Vec<ClsCard>,
    shared: usize,
    shared_cls: usize,
    stars: u64,
    downloads: u64,
    own: bool,
    pending: Vec<(String, String, &'static str, String)>,
    tokens: Vec<(String, String, String, String)>,
    new_token: String,
    no_password: bool,
    approved_pts: i64,
    star_pts: i64,
    download_pts: i64,
    featured_pts: i64,
    curate_pts: i64,
}

fn standing_name(s: Standing) -> &'static str {
    match s {
        Standing::Applied => "applied",
        Standing::Active => "active",
        Standing::Banned => "banned",
    }
}

#[handler]
pub async fn profile(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let name = req.param::<String>("name").unwrap_or_default();
    show_profile(&name, depot, res);
}

fn show_profile(name: &str, depot: &Depot, res: &mut Response) {
    let Some(mut u) = hub().store.user(name).ok().flatten() else {
        return not_found(res, depot);
    };
    let viewer = me(depot);
    let own = viewer.is_some_and(|v| v.name.eq_ignore_ascii_case(&u.name));
    let admin = viewer.is_some_and(|v| v.is_admin());
    if u.standing != Standing::Active && !own && !admin {
        return not_found(res, depot);
    }
    let all = hub().store.saves().unwrap_or_default();
    let theirs: Vec<&Save> = all.iter().filter(|s| s.author.eq_ignore_ascii_case(&u.name)).collect();
    let mut approved: Vec<&Save> = theirs
        .iter()
        .copied()
        .filter(|s| s.listed())
        .collect();
    approved.sort_by(|a, b| b.stars.cmp(&a.stars).then(b.updated.cmp(&a.updated)));
    let given = hub().store.stars_given(&u.name).unwrap_or(0);
    let all_cls = hub().store.classifiers().unwrap_or_default();
    let their_cls: Vec<&ClassifierRec> = all_cls
        .iter()
        .filter(|c| c.author.eq_ignore_ascii_case(&u.name))
        .collect();
    let mut cls_approved: Vec<&ClassifierRec> = their_cls
        .iter()
        .copied()
        .filter(|c| c.listed())
        .collect();
    cls_approved.sort_by(|a, b| b.stars.cmp(&a.stars).then(b.updated.cmp(&a.updated)));
    let level = game::level(u.points);
    let new_token = depot.get::<String>("new_token").cloned().unwrap_or_default();
    let tokens = if own {
        hub()
            .store
            .tokens_of(&u.name)
            .unwrap_or_default()
            .into_iter()
            .map(|(h, t)| (h, t.label, t.hint, ago(t.created)))
            .collect()
    } else {
        Vec::new()
    };
    // the owner's own that aren't in the grid: private, unlisted, waiting or turned down
    let pill = |v: Visibility, st: Status| {
        if v == Visibility::Public {
            status_name(st)
        } else {
            v.name()
        }
    };
    let pending = if own {
        theirs
            .iter()
            .filter(|s| !s.listed())
            .map(|s| {
                (
                    format!("/saves/{}", s.id),
                    s.name.clone(),
                    pill(s.visibility, s.status),
                    s.note.clone(),
                )
            })
            .chain(their_cls.iter().filter(|c| !c.listed()).map(|c| {
                (
                    format!("/classifiers/{}", c.id),
                    format!("{} (a classifier)", c.name),
                    pill(c.visibility, c.status),
                    c.note.clone(),
                )
            }))
            .collect()
    } else {
        Vec::new()
    };
    let listed_saves: Vec<&Save> = theirs.iter().copied().filter(|s| s.listed()).collect();
    let listed_cls: Vec<&ClassifierRec> = their_cls.iter().copied().filter(|c| c.listed()).collect();
    let no_password = u.password.is_empty();
    if !own && !admin {
        u.email.clear();
    }
    u.password.clear();
    render(
        res,
        Profile {
            l: layout(depot, ""),
            hue: hue(&u.name),
            initial: u
                .name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default(),
            level,
            title: game::title(level),
            progress: game::progress(u.points),
            next: game::points_for(level + 1) - u.points,
            joined: ago(u.joined),
            standing: standing_name(u.standing),
            badges: game::badges(&u, &listed_saves, &listed_cls, given)
                .into_iter()
                .map(|b| BadgeView {
                    name: b.name,
                    about: b.about,
                    glyph: b.glyph,
                })
                .collect(),
            cards: approved.iter().map(|s| card(s)).collect(),
            cls_cards: cls_approved.iter().map(|c| cls_card(c)).collect(),
            shared: approved.len(),
            shared_cls: cls_approved.len(),
            stars: approved.iter().map(|s| s.stars).sum::<u64>() + cls_approved.iter().map(|c| c.stars).sum::<u64>(),
            downloads: approved.iter().map(|s| s.downloads).sum::<u64>()
                + cls_approved.iter().map(|c| c.downloads).sum::<u64>(),
            own,
            pending,
            tokens,
            new_token,
            no_password,
            approved_pts: game::APPROVED,
            star_pts: game::STAR,
            download_pts: game::DOWNLOAD,
            featured_pts: game::FEATURED,
            curate_pts: game::CURATE,
            u,
        },
    );
}

#[handler]
pub async fn update_me(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(user) = me(depot).cloned() else {
        return go(res, "/login", "!log in first");
    };
    let page = format!("/u/{}", user.name);
    let bio: String = req
        .form::<String>("bio")
        .await
        .unwrap_or_default()
        .chars()
        .take(2000)
        .collect();
    let email = req.form::<String>("email").await.unwrap_or_default().trim().to_string();
    let password = req.form::<String>("password").await.unwrap_or_default();
    if email.len() > 200 || (!email.is_empty() && !email.contains('@')) {
        return go(res, &page, "!that email doesn't look right");
    }
    let hash = if password.is_empty() {
        None
    } else {
        if let Err(e) = auth::password_ok(&password) {
            return go(res, &page, &format!("!{e}"));
        }
        match auth::hash_password(&password) {
            Ok(h) => Some(h),
            Err(e) => return go(res, &page, &format!("!{e}")),
        }
    };
    let changed_pw = hash.is_some();
    let r = hub().store.update_user(&user.name, |u| {
        u.bio = bio.trim().to_string();
        u.email = email;
        if let Some(h) = hash {
            u.password = h;
        }
    });
    if let Err(e) = r {
        return go(res, &page, &format!("!{e}"));
    }
    if changed_pw {
        // other sessions end; this one starts again
        hub().store.drop_sessions_of(&user.name).ok();
        log_in(res, &user.name);
    }
    go(
        res,
        &page,
        if changed_pw {
            "saved, with your new password"
        } else {
            "saved"
        },
    );
}

#[handler]
pub async fn make_token(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(user) = me(depot).cloned() else {
        return go(res, "/login", "!log in first");
    };
    let label: String = req
        .form::<String>("label")
        .await
        .unwrap_or_default()
        .trim()
        .chars()
        .take(60)
        .collect();
    let label = if label.is_empty() { "app".to_string() } else { label };
    match auth::new_token(&hub().store, &user.name, &label) {
        Ok(t) => {
            // shown once, on this page
            depot.insert("new_token", t);
            depot.insert(
                "flash",
                "here's your token: copy it into the app now, it won't be shown again".to_string(),
            );
            show_profile(&user.name, depot, res);
        }
        Err(e) => go(res, &format!("/u/{}", user.name), &format!("!{e}")),
    }
}

#[handler]
pub async fn revoke_token(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(user) = me(depot).cloned() else {
        return go(res, "/login", "!log in first");
    };
    let page = format!("/u/{}", user.name);
    let Some(hash) = unhex(&req.param::<String>("hash").unwrap_or_default()) else {
        return go(res, &page, "!no such token");
    };
    match hub().store.token(&hash) {
        Ok(Some(t)) if t.user.eq_ignore_ascii_case(&user.name) => {
            hub().store.drop_token(&hash).ok();
            go(res, &page, &format!("revoked “{}”", t.label));
        }
        _ => go(res, &page, "!no such token"),
    }
}

/// `GET /api/me` (every mode): who the token or session belongs to
#[handler]
pub async fn whoami(depot: &mut Depot, res: &mut Response) {
    match me(depot) {
        Some(u) => {
            let level = game::level(u.points);
            res.render(salvo::writing::Json(serde_json::json!({
                "username": u.name,
                "points": u.points,
                "level": level,
                "title": game::title(level),
                "active": Hub::active(u),
                "admin": u.is_admin(),
                "can_invite": hub().may_invite(u),
            })));
        }
        None => {
            res.status_code(StatusCode::UNAUTHORIZED);
            res.render(salvo::writing::Json(serde_json::json!({ "error": "that token isn't known here" })));
        }
    }
}

// ------------------------------------------------------------------ the admin panel

pub struct AdminSave {
    pub id: String,
    pub name: String,
    pub author: String,
    pub memories: u64,
    pub notes: u64,
    pub size: String,
    pub ago: String,
    pub version: u32,
    pub status: &'static str,
    pub featured: bool,
    pub stars: u64,
    pub downloads: u64,
    pub visibility: &'static str,
}

pub struct AdminUser {
    pub name: String,
    pub email: String,
    pub why: String,
    pub ago: String,
    pub role: &'static str,
    pub standing: &'static str,
    pub points: i64,
    pub invited_by: String,
}

#[derive(Template)]
#[template(path = "admin.html")]
struct Admin {
    l: Layout,
    waiting_saves: Vec<AdminSave>,
    applied: Vec<AdminUser>,
    users: Vec<AdminUser>,
    all_saves: Vec<AdminSave>,
    waiting_cls: Vec<AdminCls>,
    all_cls: Vec<AdminCls>,
    join: &'static str,
    join_about: &'static str,
    trusted_invite: bool,
    invites: Vec<crate::invites::InviteRow>,
    new_invite: String,
}

fn admin_save(s: &Save) -> AdminSave {
    AdminSave {
        id: s.id.clone(),
        name: s.name.clone(),
        author: s.author.clone(),
        memories: s.memories,
        notes: s.notes,
        size: human_size(s.size),
        ago: ago(s.updated),
        version: s.version,
        status: status_name(s.status),
        featured: s.featured,
        stars: s.stars,
        downloads: s.downloads,
        visibility: s.visibility.name(),
    }
}

fn admin_user(u: &User) -> AdminUser {
    AdminUser {
        name: u.name.clone(),
        email: u.email.clone(),
        why: u.why.clone(),
        ago: ago(u.joined),
        role: match u.role {
            Role::Member => "member",
            Role::Trusted => "trusted",
            Role::Admin => "admin",
        },
        standing: standing_name(u.standing),
        points: u.points,
        invited_by: u.invited_by.clone(),
    }
}

#[handler]
pub async fn admin_page(depot: &mut Depot, res: &mut Response) {
    show_admin(depot, res);
}

/// the admin panel (with a just-made invite link shown once, when there is one)
pub fn show_admin(depot: &Depot, res: &mut Response) {
    if need_admin(res, depot).is_none() {
        return;
    }
    let mut saves = hub().store.saves().unwrap_or_default();
    saves.sort_by_key(|s| std::cmp::Reverse(s.updated));
    let mut users = hub().store.users().unwrap_or_default();
    users.sort_by_key(|u| u.number);
    let mut cls = hub().store.classifiers().unwrap_or_default();
    cls.sort_by_key(|c| std::cmp::Reverse(c.updated));
    render(
        res,
        Admin {
            l: layout(depot, "admin"),
            waiting_saves: saves
                .iter()
                .filter(|s| s.waiting())
                .map(admin_save)
                .collect(),
            applied: users
                .iter()
                .filter(|u| u.standing == Standing::Applied)
                .map(admin_user)
                .collect(),
            users: users
                .iter()
                .filter(|u| u.standing != Standing::Applied)
                .map(admin_user)
                .collect(),
            all_saves: saves.iter().map(admin_save).collect(),
            waiting_cls: cls
                .iter()
                .filter(|c| c.waiting())
                .map(admin_cls)
                .collect(),
            all_cls: cls.iter().map(admin_cls).collect(),
            join: hub().join_policy().name(),
            join_about: hub().join_policy().about(),
            trusted_invite: hub().trusted_invite(),
            invites: hub()
                .store
                .invites()
                .unwrap_or_default()
                .iter()
                .map(crate::invites::row)
                .collect(),
            new_invite: depot.get::<String>("new_invite").cloned().unwrap_or_default(),
        },
    );
}

#[handler]
pub async fn admin_user_action(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(admin) = need_admin(res, depot).cloned() else {
        return;
    };
    let name = req.param::<String>("name").unwrap_or_default();
    let action = req.form::<String>("action").await.unwrap_or_default();
    if action.is_empty() {
        return go(res, "/admin", "");
    }
    if action == "link" {
        return match hub().store.user(&name).ok().flatten() {
            Some(u) if u.standing != Standing::Banned => match auth::new_link(&hub().store, &u.name) {
                Ok(s) => message(
                    res,
                    depot,
                    StatusCode::OK,
                    &format!("a login link for {}", u.name),
                    &format!(
                        "Give them this; it works once, within an hour: {}/login/{}",
                        hub().config.url.trim_end_matches('/'),
                        s
                    ),
                ),
                Err(e) => go(res, "/admin", &format!("!{e}")),
            },
            _ => go(res, "/admin", "!no such user, or they're banned"),
        };
    }
    match hub().manage(&admin, &name, &action) {
        Ok(m) => go(res, "/admin", &m),
        Err(e) => go(res, "/admin", &format!("!{e}")),
    }
}

// ------------------------------------------------------------------ files

#[handler]
pub async fn css(req: &mut Request, res: &mut Response) {
    res.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"));
    versioned(req, res, CSS_V);
    res.write_body(include_str!("../static/hub.css")).ok();
}

/// a version for each static file, from what's in it, so pages ask for `?v=` and the
/// files can be cached for long: a new build that changes them changes the version
const fn fnv(b: &[u8]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < b.len() {
        h = (h ^ b[i] as u32).wrapping_mul(0x0100_0193);
        i += 1;
    }
    h
}
pub const CSS_V: u32 = fnv(include_bytes!("../static/hub.css"));
pub const ICONS_V: u32 = fnv(include_bytes!("../static/icons.svg"));

fn versioned(req: &Request, res: &mut Response, version: u32) {
    // asked for with this build's version: keep it for a year; otherwise for an hour
    let long = req.query::<String>("v") == Some(version.to_string());
    res.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static(if long {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=3600"
        }),
    );
}

/// an icon of the shared set (assets/icons), drawn from the sprite: in a template,
/// `{{ crate::web::icon("star") }}`. It is only a picture: the control it sits in carries
/// the name (its words, or an aria-label and a data-tip when it is only the icon)
pub fn icon(name: &str) -> askama::filters::Safe<String> {
    debug_assert!(name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'), "{name}");
    askama::filters::Safe(format!(
        r#"<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><use href="/static/icons.svg?v={ICONS_V}#i-{name}"/></svg>"#
    ))
}

/// the icon beside a form's button, by what the button says
pub fn submit_icon(submit: &str) -> &'static str {
    match submit {
        "save" => "save",
        "share" => "upload",
        "log in" => "keys",
        _ => "new",
    }
}

/// the shared icon set (assets/icons) as a sprite, made by hub/web/tools/icons.mjs
#[handler]
pub async fn icons(req: &mut Request, res: &mut Response) {
    res.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
    res.headers_mut()
        .insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    versioned(req, res, ICONS_V);
    res.write_body(include_str!("../static/icons.svg")).ok();
}

#[handler]
pub async fn favicon(res: &mut Response) {
    res.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
    res.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400"));
    res.write_body(include_str!("../static/favicon.svg")).ok();
}

#[handler]
pub async fn missing(depot: &mut Depot, res: &mut Response) {
    not_found(res, depot);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a --local hub answers to loopback names, addresses and its own --url only
    #[test]
    fn local_hosts() {
        let url = "http://notes.example:8080";
        for ok in [
            "localhost:8231",
            "LOCALHOST",
            "app.localhost:1",
            "127.0.0.1:8231",
            "[::1]:8231",
            "192.168.1.4:80",
            "notes.example:8080",
            "notes.example",
        ] {
            assert!(local_host_ok(ok, url), "{ok}");
        }
        for bad in [
            "",
            "evil.example:8231",
            "localhost.evil.example",
            "127.0.0.1.nip.io:8231",
            "notes.example.evil:8080",
        ] {
            assert!(!local_host_ok(bad, url), "{bad}");
        }
        assert!(!local_host_ok("notes.example", "http://127.0.0.1:8080"));
    }

    #[test]
    fn flash_round_trip() {
        for s in ["", "plain", "!bad news: “quoted” & 100%", "%zz", "%4"] {
            assert_eq!(decode(&encode(s)), s);
        }
        assert_eq!(decode("%zz"), "%zz");
    }

    /// every icon a page asks for is in the sprite (hub/web/tools/icons.mjs makes it from
    /// assets/icons), and the sprite is only shapes
    #[test]
    fn icons_in_the_sprite() {
        let sprite = include_str!("../static/icons.svg");
        assert!(!sprite.contains("<script") && !sprite.contains("href=") && !sprite.contains("<foreignObject"));
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
        let mut asked = vec![];
        for e in std::fs::read_dir(dir).unwrap() {
            let page = std::fs::read_to_string(e.unwrap().path()).unwrap();
            for part in page.split("crate::web::icon(\"").skip(1) {
                asked.push(part.split('"').next().unwrap().to_string());
            }
        }
        for s in ["save", "share", "log in", "join", "apply"] {
            asked.push(submit_icon(s).to_string());
        }
        assert!(asked.len() > 20, "{asked:?}");
        for name in asked {
            assert!(
                sprite.contains(&format!("<symbol id=\"i-{name}\"")),
                "no icon {name} in the sprite"
            );
        }
        // the sprite has the whole set: run hub/web/tools/icons.mjs when it changes
        let set = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/icons");
        for e in std::fs::read_dir(set).unwrap() {
            let f = e.unwrap().file_name().into_string().unwrap();
            if let Some(name) = f.strip_suffix(".svg") {
                assert!(
                    sprite.contains(&format!("<symbol id=\"i-{name}\"")),
                    "{name} isn't in the sprite: run node hub/web/tools/icons.mjs"
                );
            }
        }
        assert!(icon("star").0.contains(&format!("icons.svg?v={ICONS_V}#i-star")));
    }
}
