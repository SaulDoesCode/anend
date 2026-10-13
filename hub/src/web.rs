//! The hub's web pages: the grid of saves, a save's page, sharing, joining and logging in,
//! profiles with their levels and badges, the leaderboard and the admin panel. Pages are
//! askama templates; forms post back and redirect with a one-line message in a cookie.

use askama::Template;
use chrono::{DateTime, Utc};
use salvo::http::cookie::{Cookie, SameSite};
use salvo::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use salvo::prelude::*;
use std::net::IpAddr;
use std::sync::OnceLock;

use crate::auth;
use crate::game;
use crate::hub::{Details, Hub, Sort, human_size};
use crate::inspect;
use crate::store::{Role, Save, Standing, Status, User, unhex};

pub static HUB: OnceLock<Hub> = OnceLock::new();

pub fn hub() -> &'static Hub {
    HUB.get().expect("the hub starts before serving")
}

const FLASH: &str = "hub_flash";

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

/// loads the user behind the session cookie (or app token), turns away forms posted from
/// other sites, and picks up the message left by the last redirect
#[handler]
pub async fn who_is_asking(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
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

fn render(res: &mut Response, t: impl Template) {
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
}

fn layout(depot: &Depot, tab: &'static str) -> Layout {
    let flash = depot.get::<String>("flash").cloned().unwrap_or_default();
    let (flash, flash_bad) = match flash.strip_prefix('!') {
        Some(f) => (f.to_string(), true),
        None => (flash, false),
    };
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
        let s = hub().store.saves().unwrap_or_default();
        let u = hub().store.users().unwrap_or_default();
        s.iter().filter(|s| s.status == Status::Pending).count()
            + u.iter().filter(|u| u.standing == Standing::Applied).count()
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
    }
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
            back_label: "to the saves".into(),
        },
    );
}

fn not_found(res: &mut Response, depot: &Depot) {
    message(
        res,
        depot,
        StatusCode::NOT_FOUND,
        "not here",
        "There's nothing at this address.",
    );
}

/// the logged in, active user, or a message saying why not
fn need_active<'a>(res: &mut Response, depot: &'a Depot) -> Option<&'a User> {
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
                "Your account is waiting for an admin to let you in. You can share and star saves once they do.",
            );
            None
        }
        Some(u) => Some(u),
    }
}

fn need_admin<'a>(res: &mut Response, depot: &'a Depot) -> Option<&'a User> {
    match me(depot) {
        Some(u) if u.is_admin() => Some(u),
        _ => {
            not_found(res, depot);
            None
        }
    }
}

// ------------------------------------------------------------------ the grid

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

pub struct EventView {
    pub user: String,
    pub what: String,
    pub save: String,
    pub save_name: String,
    pub ago: String,
}

#[derive(Template)]
#[template(path = "index.html")]
struct Index {
    l: Layout,
    about: String,
    q: String,
    q_url: String,
    sort: &'static str,
    sorts: [&'static str; 4],
    cards: Vec<Card>,
    page: usize,
    more: bool,
    total_saves: usize,
    total_memories: u64,
    total_users: usize,
    events: Vec<EventView>,
}

#[handler]
pub async fn index(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let q = req.query::<String>("q").unwrap_or_default();
    let sort = Sort::parse(&req.query::<String>("sort").unwrap_or_default());
    let page = req.query::<usize>("page").unwrap_or(0).min(10_000);
    let h = hub();
    let (saves, more) = h.find(&q, sort, page).unwrap_or_default();
    let all = h.store.saves().unwrap_or_default();
    let approved: Vec<&Save> = all.iter().filter(|s| s.status == Status::Approved).collect();
    let users = h.store.users().unwrap_or_default();
    let events = h
        .store
        .events(30)
        .unwrap_or_default()
        .into_iter()
        // starring, unstarring and starring again is one thing worth telling
        .fold(Vec::new(), |mut seen: Vec<crate::store::Event>, e| {
            if !seen
                .iter()
                .any(|x| x.user == e.user && x.what == e.what && x.save == e.save && !e.save.is_empty())
            {
                seen.push(e);
            }
            seen
        })
        .into_iter()
        .filter_map(|e| {
            let save_name = if e.save.is_empty() {
                String::new()
            } else {
                // only point at saves everyone can see
                let s = approved.iter().find(|s| s.id == e.save)?;
                s.name.clone()
            };
            Some(EventView {
                user: e.user,
                what: e.what,
                save: e.save,
                save_name,
                ago: ago(e.at),
            })
        })
        .take(12)
        .collect();
    render(
        res,
        Index {
            l: layout(depot, "saves"),
            about: h.config.description.clone(),
            q_url: url(&q),
            q,
            sort: sort.name(),
            sorts: ["new", "stars", "downloads", "featured"],
            cards: saves.iter().map(card).collect(),
            page,
            more,
            total_saves: approved.len(),
            total_memories: approved.iter().map(|s| s.memories).sum(),
            total_users: users.iter().filter(|u| u.standing == Standing::Active).count(),
            events,
        },
    );
}

// ------------------------------------------------------------------ a save

pub struct SaveView {
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
    pub version: u32,
    pub note: String,
    pub status_name: &'static str,
    pub sample: Vec<crate::store::Sample>,
    pub titles: Vec<String>,
}

#[derive(Template)]
#[template(path = "save.html")]
struct SavePage {
    l: Layout,
    s: SaveView,
    updated: String,
    size: String,
    hue: u64,
    ordinal: u64,
    reversed: u64,
    can_star: bool,
    starred: bool,
    can_edit: bool,
    is_admin: bool,
}

pub fn status_name(s: Status) -> &'static str {
    match s {
        Status::Pending => "pending",
        Status::Approved => "approved",
        Status::Rejected => "rejected",
    }
}

/// a save the asker may see: approved ones, or their own, or any for an admin
fn visible(depot: &Depot, id: &str) -> Option<Save> {
    let s = hub().store.save(id).ok().flatten()?;
    let mine = me(depot).is_some_and(|u| u.is_admin() || u.name.eq_ignore_ascii_case(&s.author));
    (s.status == Status::Approved || mine).then_some(s)
}

#[handler]
pub async fn save_page(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(s) = visible(depot, &id) else {
        return not_found(res, depot);
    };
    let user = me(depot);
    let is_admin = user.is_some_and(|u| u.is_admin());
    let own = user.is_some_and(|u| u.name.eq_ignore_ascii_case(&s.author));
    let starred = user.is_some_and(|u| hub().store.has_starred(&s.id, &u.name).unwrap_or(false));
    let (ordinal, reversed) = inspect::sums(&s.name);
    render(
        res,
        SavePage {
            l: layout(depot, "saves"),
            updated: ago(s.updated),
            size: human_size(s.size),
            hue: hue(&s.name),
            ordinal,
            reversed,
            can_star: user.is_some_and(Hub::active) && !own && s.status == Status::Approved,
            starred,
            can_edit: own || is_admin,
            is_admin,
            s: SaveView {
                status_name: status_name(s.status),
                id: s.id,
                name: s.name,
                author: s.author,
                description: s.description,
                tags: s.tags,
                notes: s.notes,
                memories: s.memories,
                stars: s.stars,
                downloads: s.downloads,
                featured: s.featured,
                version: s.version,
                note: s.note,
                sample: s.sample,
                titles: s.titles,
            },
        },
    );
}

/// a file name for a save's download
fn file_name(s: &Save) -> String {
    let n: String = s
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let n = n.trim();
    format!("{}.redb", if n.is_empty() { s.id.as_str() } else { n })
}

#[handler]
pub async fn download(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(s) = visible(depot, &id) else {
        return not_found(res, depot);
    };
    let who = match me(depot) {
        Some(u) => u.name.to_lowercase(),
        None => auth::address_key(client_ip(req)),
    };
    if s.status == Status::Approved {
        hub().downloaded(&s, &who).ok();
    }
    salvo::fs::NamedFile::builder(hub().store.save_path(&s.id))
        .attached_name(file_name(&s))
        .send(req.headers(), res)
        .await;
}

#[handler]
pub async fn star(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(user) = need_active(res, depot) else { return };
    let Some(s) = visible(depot, &id) else {
        return not_found(res, depot);
    };
    let back = format!("/saves/{}", s.id);
    match hub().star(&s, user) {
        Ok((true, _)) => go(res, &back, "starred"),
        Ok((false, _)) => go(res, &back, "unstarred"),
        Err(e) => go(res, &back, &format!("!{e}")),
    }
}

// ------------------------------------------------------------------ forms

pub struct Field {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub value: String,
    pub hint: &'static str,
    pub required: bool,
}

fn field(name: &'static str, label: &'static str, kind: &'static str, value: &str) -> Field {
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
struct Form {
    l: Layout,
    heading: String,
    intro: String,
    action: String,
    multipart: bool,
    fields: Vec<Field>,
    submit: &'static str,
    after: String,
}

fn upload_form(depot: &Depot, action: String, heading: String, s: Option<&Save>, intro: String) -> Form {
    let max = human_size(hub().config.max_upload);
    Form {
        l: layout(depot, "upload"),
        heading,
        intro,
        action,
        multipart: true,
        fields: vec![
            Field {
                required: true,
                ..field("name", "name", "text", s.map(|s| s.name.as_str()).unwrap_or(""))
            },
            field(
                "description",
                "what's in it",
                "textarea",
                s.map(|s| s.description.as_str()).unwrap_or(""),
            ),
            Field {
                hint: "a few words, comma separated",
                ..field(
                    "tags",
                    "tags",
                    "text",
                    &s.map(|s| s.tags.join(", ")).unwrap_or_default(),
                )
            },
            Field {
                required: s.is_none(),
                hint: if s.is_some() {
                    "leave empty to keep the file and only change the words"
                } else {
                    "a save: a .redb file from the app's saves folder"
                },
                ..field("file", "the save", "file", "")
            },
        ],
        submit: if s.is_some() { "save" } else { "share" },
        after: format!(
            "Up to {max}. {}",
            if me(depot).is_some_and(|u| matches!(u.role, Role::Trusted | Role::Admin)) {
                "Yours go straight in."
            } else {
                "An admin looks at each save before it shows in the grid."
            }
        ),
    }
}

#[handler]
pub async fn upload_page(depot: &mut Depot, res: &mut Response) {
    if need_active(res, depot).is_none() {
        return;
    }
    let f = upload_form(
        depot,
        "/upload".into(),
        "share a save".into(),
        None,
        "Share one of your saves: its memories and notes, for anyone to download into the app.".into(),
    );
    render(res, f);
}

/// what an upload form sent
struct Sent {
    name: String,
    description: String,
    tags: String,
    file: Option<(std::path::PathBuf, u64)>,
}

async fn read_upload(req: &mut Request) -> Result<Sent, String> {
    let limit = hub().config.max_upload as usize + 256 * 1024;
    let form = req.form_data_max_size(limit).await.map_err(|e| {
        let e = e.to_string();
        if e.contains("limit") || e.contains("large") || e.contains("size") {
            format!(
                "that file is over the hub's {} limit",
                human_size(hub().config.max_upload)
            )
        } else {
            format!("couldn't read the form: {e}")
        }
    })?;
    let get = |k: &str| form.fields.get(k).cloned().unwrap_or_default();
    Ok(Sent {
        name: get("name"),
        description: get("description"),
        tags: get("tags"),
        file: form
            .files
            .get("file")
            .filter(|f| f.size() > 0)
            .map(|f| (f.path().clone(), f.size())),
    })
}

#[handler]
pub async fn upload(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(user) = need_active(res, depot).cloned() else {
        return;
    };
    if !hub().limiter.allow(client_ip(req), "upload", 20, 3600) {
        return go(res, "/upload", "!that's a lot of uploads; try again in a while");
    }
    let sent = match read_upload(req).await {
        Ok(s) => s,
        Err(e) => return go(res, "/upload", &format!("!{e}")),
    };
    let details = match Details::new(&sent.name, &sent.description, &sent.tags) {
        Ok(d) => d,
        Err(e) => return go(res, "/upload", &format!("!{e}")),
    };
    let Some((path, _)) = sent.file else {
        return go(res, "/upload", "!pick a save to share");
    };
    match hub().accept(&user, details, &path, None).await {
        Ok(s) if s.status == Status::Approved => go(res, &format!("/saves/{}", s.id), "shared: it's in the grid"),
        Ok(s) => go(
            res,
            &format!("/saves/{}", s.id),
            "sent in: an admin will look at it, then it shows in the grid",
        ),
        Err(e) => go(res, "/upload", &format!("!{e}")),
    }
}

fn own_save(depot: &Depot, id: &str) -> Option<Save> {
    let u = me(depot)?;
    let s = hub().store.save(id).ok().flatten()?;
    (u.is_admin() || u.name.eq_ignore_ascii_case(&s.author)).then_some(s)
}

#[handler]
pub async fn edit_page(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(s) = own_save(depot, &id) else {
        return not_found(res, depot);
    };
    let f = upload_form(
        depot,
        format!("/saves/{}/edit", s.id),
        format!("edit “{}”", s.name),
        Some(&s),
        "Change its words, or send a new version of the file (it keeps its stars and downloads).".into(),
    );
    render(res, f);
}

#[handler]
pub async fn edit(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(s) = own_save(depot, &id) else {
        return not_found(res, depot);
    };
    let Some(user) = need_active(res, depot).cloned() else {
        return;
    };
    let back = format!("/saves/{}/edit", s.id);
    let sent = match read_upload(req).await {
        Ok(x) => x,
        Err(e) => return go(res, &back, &format!("!{e}")),
    };
    let details = match Details::new(&sent.name, &sent.description, &sent.tags) {
        Ok(d) => d,
        Err(e) => return go(res, &back, &format!("!{e}")),
    };
    let page = format!("/saves/{}", s.id);
    match sent.file {
        Some((path, _)) => {
            // the new version is the author's, even when an admin sends it
            let author = hub().store.user(&s.author).ok().flatten().unwrap_or(user);
            match hub().accept(&author, details, &path, Some(&s)).await {
                Ok(n) if n.status == Status::Approved => go(res, &page, "new version is in"),
                Ok(_) => go(res, &page, "new version sent in: an admin will look at it"),
                Err(e) => go(res, &back, &format!("!{e}")),
            }
        }
        None => {
            let r = hub().store.update_save(&s.id, |x| {
                x.name = details.name;
                x.description = details.description;
                x.tags = details.tags;
            });
            match r {
                Ok(_) => go(res, &page, "saved"),
                Err(e) => go(res, &back, &format!("!{e}")),
            }
        }
    }
}

#[handler]
pub async fn delete(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(s) = own_save(depot, &id) else {
        return not_found(res, depot);
    };
    let who = me(depot).map(|u| u.name.clone()).unwrap_or_default();
    match hub().remove(&s.id) {
        Ok(()) => {
            hub().store.log(&who, "deleted a save", "").ok();
            go(res, &format!("/u/{}", s.author), &format!("deleted “{}”", s.name))
        }
        Err(e) => go(res, &format!("/saves/{}", s.id), &format!("!{e}")),
    }
}

// ------------------------------------------------------------------ joining and logging in

#[handler]
pub async fn join_page(depot: &mut Depot, res: &mut Response) {
    let open = hub().config.open;
    render(
        res,
        Form {
            l: layout(depot, "join"),
            heading: "join".into(),
            intro: if open {
                "Make an account to share saves, star the ones you like and climb the levels.".into()
            } else {
                "Apply for an account: an admin lets people in. Say a little about why you'd like to join.".into()
            },
            action: "/join".into(),
            multipart: false,
            fields: {
                let mut f = vec![
                    Field {
                        required: true,
                        hint: "letters, digits, - and _",
                        ..field("name", "name", "text", "")
                    },
                    Field {
                        hint: "optional; only admins see it",
                        ..field("email", "email", "email", "")
                    },
                    Field {
                        required: true,
                        hint: "8 characters or more",
                        ..field("password", "password", "password", "")
                    },
                ];
                if !open {
                    f.push(field("why", "why you'd like to join", "textarea", ""));
                }
                f
            },
            submit: if open { "join" } else { "apply" },
            after: String::new(),
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
    let why: String = req
        .form::<String>("why")
        .await
        .unwrap_or_default()
        .chars()
        .take(1000)
        .collect();
    if let Err(e) = auth::name_ok(&name).and(auth::password_ok(&password)) {
        return go(res, "/join", &format!("!{e}"));
    }
    if email.len() > 200 || (!email.is_empty() && !email.contains('@')) {
        return go(res, "/join", "!that email doesn't look right");
    }
    if !email.is_empty() && hub().store.user_by_email(&email).ok().flatten().is_some() {
        return go(res, "/join", "!that email has an account already");
    }
    let hash = match auth::hash_password(&password) {
        Ok(h) => h,
        Err(e) => return go(res, "/join", &format!("!{e}")),
    };
    let open = hub().config.open;
    let user = User {
        name: name.clone(),
        email,
        password: hash,
        why: why.trim().to_string(),
        standing: if open { Standing::Active } else { Standing::Applied },
        joined: Utc::now(),
        ..Default::default()
    };
    match hub().store.add_user(user) {
        Ok(u) => {
            if open {
                hub().store.log(&u.name, "joined", "").ok();
            }
            log_in(res, &u.name);
            if open {
                go(res, &format!("/u/{}", u.name), "welcome! you're level 1, a seeker");
            } else {
                go(res, "/", "applied: you can look around while an admin considers it");
            }
        }
        Err(e) => go(res, "/join", &format!("!{e}")),
    }
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
pub async fn login_page(depot: &mut Depot, res: &mut Response) {
    render(
        res,
        Form {
            l: layout(depot, "login"),
            heading: "log in".into(),
            intro: String::new(),
            action: "/login".into(),
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
            go(res, "/", &format!("welcome back, {}", u.name));
        }
        _ => go(res, "/login", "!that name and password don't match"),
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
    shared: usize,
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
        .filter(|s| s.status == Status::Approved)
        .collect();
    approved.sort_by(|a, b| b.stars.cmp(&a.stars).then(b.updated.cmp(&a.updated)));
    let given = hub().store.stars_given(&u.name).unwrap_or(0);
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
    let pending = theirs
        .iter()
        .filter(|s| s.status != Status::Approved)
        .map(|s| (s.id.clone(), s.name.clone(), status_name(s.status), s.note.clone()))
        .collect();
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
            badges: game::badges(&u, &theirs, given)
                .into_iter()
                .map(|b| BadgeView {
                    name: b.name,
                    about: b.about,
                    glyph: b.glyph,
                })
                .collect(),
            cards: approved.iter().map(|s| card(s)).collect(),
            shared: approved.len(),
            stars: approved.iter().map(|s| s.stars).sum(),
            downloads: approved.iter().map(|s| s.downloads).sum(),
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

// ------------------------------------------------------------------ the leaderboard

pub struct Ranked {
    pub name: String,
    pub hue: u64,
    pub initial: String,
    pub level: u32,
    pub title: &'static str,
    pub points: i64,
    pub saves: usize,
    pub stars: u64,
}

#[derive(Template)]
#[template(path = "leaderboard.html")]
struct Leaderboard {
    l: Layout,
    people: Vec<Ranked>,
    top: Vec<Card>,
    levels: Vec<(u32, &'static str, i64)>,
}

#[handler]
pub async fn leaderboard(depot: &mut Depot, res: &mut Response) {
    let saves = hub().store.saves().unwrap_or_default();
    let approved: Vec<&Save> = saves.iter().filter(|s| s.status == Status::Approved).collect();
    let mut people: Vec<Ranked> = hub()
        .store
        .users()
        .unwrap_or_default()
        .into_iter()
        .filter(|u| u.standing == Standing::Active)
        .map(|u| {
            let theirs: Vec<&&Save> = approved
                .iter()
                .filter(|s| s.author.eq_ignore_ascii_case(&u.name))
                .collect();
            let level = game::level(u.points);
            Ranked {
                hue: hue(&u.name),
                initial: u
                    .name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default(),
                level,
                title: game::title(level),
                points: u.points,
                saves: theirs.len(),
                stars: theirs.iter().map(|s| s.stars).sum(),
                name: u.name,
            }
        })
        .collect();
    people.sort_by(|a, b| b.points.cmp(&a.points).then(a.name.cmp(&b.name)));
    people.truncate(50);
    let mut top: Vec<&Save> = approved.clone();
    top.sort_by(|a, b| b.stars.cmp(&a.stars).then(b.downloads.cmp(&a.downloads)));
    render(
        res,
        Leaderboard {
            l: layout(depot, "leaderboard"),
            people,
            top: top.iter().take(10).map(|s| card(s)).collect(),
            levels: (1..=10).map(|l| (l, game::title(l), game::points_for(l))).collect(),
        },
    );
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
}

pub struct AdminUser {
    pub name: String,
    pub email: String,
    pub why: String,
    pub ago: String,
    pub role: &'static str,
    pub standing: &'static str,
    pub points: i64,
}

#[derive(Template)]
#[template(path = "admin.html")]
struct Admin {
    l: Layout,
    waiting_saves: Vec<AdminSave>,
    applied: Vec<AdminUser>,
    users: Vec<AdminUser>,
    all_saves: Vec<AdminSave>,
    open: bool,
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
    }
}

#[handler]
pub async fn admin_page(depot: &mut Depot, res: &mut Response) {
    if need_admin(res, depot).is_none() {
        return;
    }
    let mut saves = hub().store.saves().unwrap_or_default();
    saves.sort_by_key(|s| std::cmp::Reverse(s.updated));
    let mut users = hub().store.users().unwrap_or_default();
    users.sort_by_key(|u| u.number);
    render(
        res,
        Admin {
            l: layout(depot, "admin"),
            waiting_saves: saves
                .iter()
                .filter(|s| s.status == Status::Pending)
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
            open: hub().config.open,
        },
    );
}

#[handler]
pub async fn admin_save_action(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(admin) = need_admin(res, depot).cloned() else {
        return;
    };
    let id = req.param::<String>("id").unwrap_or_default();
    let action = req.form::<String>("action").await.unwrap_or_default();
    let note = req.form::<String>("note").await.unwrap_or_default();
    if action.is_empty() {
        return go(res, "/admin", "");
    }
    // back where the admin came from, unless the save is gone
    let back = if action == "remove" {
        "/admin".to_string()
    } else {
        req.header::<String>("referer")
            .filter(|r| r.contains("/saves/"))
            .map(|_| format!("/saves/{id}"))
            .unwrap_or_else(|| "/admin".into())
    };
    match hub().review(&admin, &id, &action, &note) {
        Ok(m) => go(res, &back, &m),
        Err(e) => go(res, &back, &format!("!{e}")),
    }
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
pub async fn css(res: &mut Response) {
    res.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"));
    res.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
    res.write_body(include_str!("../static/hub.css")).ok();
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

    #[test]
    fn flash_round_trip() {
        for s in ["", "plain", "!bad news: “quoted” & 100%", "%zz", "%4"] {
            assert_eq!(decode(&encode(s)), s);
        }
        assert_eq!(decode("%zz"), "%zz");
    }
}
