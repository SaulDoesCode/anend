//! The hub's save pages: the grid of saves (the front page), a save's page, sharing,
//! editing and deleting one, stars and downloads, the leaderboard and the admins'
//! verdicts on saves. Only in a build with the `hub` feature, and only routed in the
//! `hub` and `both` modes.

use askama::Template;
use salvo::prelude::*;

use crate::auth;
use crate::game;
use crate::hub::{Details, Hub, Sort, human_size};
use crate::inspect;
use crate::store::{Role, Save, Standing, Visibility, may_see};
use crate::web::{
    Card, Field, Form, Layout, ago, card, client_ip, field, go, hub, hue, layout, me, mode, need_active, need_admin,
    not_found, render, status_name, url,
};

pub struct EventView {
    pub user: String,
    pub what: String,
    pub save_name: String,
    /// where the save or classifier it's about lives
    pub href: String,
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
    let approved: Vec<&Save> = all.iter().filter(|s| s.listed()).collect();
    let users = h.store.users().unwrap_or_default();
    let classifiers = h.store.classifiers().unwrap_or_default();
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
            // only point at saves and classifiers everyone can see
            let (save_name, href) = if e.save.is_empty() {
                (String::new(), String::new())
            } else if let Some(id) = e.save.strip_prefix(crate::store::CLS) {
                let c = classifiers.iter().find(|c| c.id == id && c.listed())?;
                (format!("{} (a classifier)", c.name), format!("/classifiers/{id}"))
            } else {
                let s = approved.iter().find(|s| s.id == e.save)?;
                (s.name.clone(), format!("/saves/{}", s.id))
            };
            Some(EventView {
                user: e.user,
                what: e.what,
                save_name,
                href,
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
    own: bool,
    visibility: &'static str,
    app_on: bool,
}

/// a save the asker may see (`store::may_see`): a listed one, an unlisted one by its
/// link, their own, or (for an admin) a public one waiting for a verdict
fn visible(depot: &Depot, id: &str) -> Option<Save> {
    let s = hub().store.save(id).ok().flatten()?;
    may_see(s.visibility, s.status, &s.author, me(depot)).then_some(s)
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
            can_star: user.is_some_and(Hub::active) && !own && s.listed(),
            own,
            visibility: s.visibility.name(),
            app_on: mode(depot).app,
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
    hub().downloaded(&s, &who).ok();
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
                hint: "private: only you; unlisted: anyone with its link; public: in the grid once an admin lets it in",
                ..field(
                    "visibility",
                    "who sees it",
                    "visibility",
                    s.map(|s| s.visibility.name()).unwrap_or("public"),
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
                "An admin looks at each public save before it shows in the grid; private and unlisted ones skip that and are never listed."
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
pub(crate) struct Sent {
    pub name: String,
    pub description: String,
    pub tags: String,
    pub visibility: String,
    pub file: Option<(std::path::PathBuf, u64)>,
}

pub(crate) async fn read_upload(req: &mut Request) -> Result<Sent, String> {
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
        visibility: get("visibility"),
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
    let details =
        match Details::new(&sent.name, &sent.description, &sent.tags).and_then(|d| d.seen_by(&sent.visibility)) {
            Ok(d) => d,
            Err(e) => return go(res, "/upload", &format!("!{e}")),
        };
    let Some((path, _)) = sent.file else {
        return go(res, "/upload", "!pick a save to share");
    };
    match hub().accept(&user, details, &path, None).await {
        Ok(s) if s.listed() => go(res, &format!("/saves/{}", s.id), "shared: it's in the grid"),
        Ok(s) if s.visibility == Visibility::Private => {
            go(res, &format!("/saves/{}", s.id), "kept: only you can see it")
        }
        Ok(s) if s.visibility == Visibility::Unlisted => go(
            res,
            &format!("/saves/{}", s.id),
            "kept: anyone with this page's link can see it",
        ),
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
    let details =
        match Details::new(&sent.name, &sent.description, &sent.tags).and_then(|d| d.seen_by(&sent.visibility)) {
            Ok(d) => d,
            Err(e) => return go(res, &back, &format!("!{e}")),
        };
    let page = format!("/saves/{}", s.id);
    match sent.file {
        Some((path, _)) => {
            // the new version is the author's, even when an admin sends it
            let author = hub().store.user(&s.author).ok().flatten().unwrap_or(user);
            match hub().accept(&author, details, &path, Some(&s)).await {
                Ok(n) if !n.waiting() => go(res, &page, "new version is in"),
                Ok(_) => go(res, &page, "new version sent in: an admin will look at it"),
                Err(e) => go(res, &back, &format!("!{e}")),
            }
        }
        None => {
            let vis = details.visibility;
            let r = hub().store.update_save(&s.id, |x| {
                x.name = details.name;
                x.description = details.description;
                x.tags = details.tags;
            });
            let r = match (r, vis) {
                (Ok(_), Some(v)) if v != s.visibility => hub().set_save_visibility(&user, &s.id, v),
                (r, _) => r,
            };
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

/// `POST /saves/{id}/visibility` with `visibility`: private, unlisted or public (its
/// owner; an admin can only remove, not reveal)
#[handler]
pub async fn visibility(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(user) = need_active(res, depot).cloned() else {
        return;
    };
    let Some(s) = hub()
        .store
        .save(&id)
        .ok()
        .flatten()
        .filter(|s| s.author.eq_ignore_ascii_case(&user.name))
    else {
        return not_found(res, depot);
    };
    let back = format!("/saves/{}", s.id);
    let Some(v) = Visibility::parse(&req.form::<String>("visibility").await.unwrap_or_default()) else {
        return go(res, &back, "!visibility is private, unlisted or public");
    };
    match hub().set_save_visibility(&user, &s.id, v) {
        Ok(n) => go(res, &back, &visibility_said(n.visibility, n.listed(), n.waiting())),
        Err(e) => go(res, &back, &format!("!{e}")),
    }
}

/// what changing who sees something did, in words
pub fn visibility_said(v: Visibility, listed: bool, waiting: bool) -> String {
    match v {
        Visibility::Private => "private now: only you can see it".into(),
        Visibility::Unlisted => "unlisted now: anyone with its link can see it; it isn't listed".into(),
        Visibility::Public if listed => "public: it's in the grid".into(),
        Visibility::Public if waiting => "public: an admin will look at it, then it shows in the grid".into(),
        Visibility::Public => "public, but an admin turned this version down: send a new one".into(),
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
    let approved: Vec<&Save> = saves.iter().filter(|s| s.listed()).collect();
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
