//! The hub's classifier pages: the grid of shared classifiers, a classifier's page (its
//! classes, how well it does, what it builds on), sharing a module, editing or deleting
//! your own, and the admins' verdicts. Like the saves' pages in web.rs.

use askama::Template;
use salvo::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, HeaderValue};
use salvo::prelude::*;

use crate::auth;
use crate::clshub::may_edit;
use crate::hub::{Hub, Sort, human_size};
use crate::store::{ClassifierRec, Role, Status};
use crate::web::{
    Field, Form, Layout, ago, client_ip, field, go, hub, hue, layout, me, need_active, need_admin, not_found,
    read_upload, render, status_name, url,
};

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
fn percent(x: Option<f64>) -> String {
    x.filter(|v| v.is_finite())
        .map(|v| format!("{:.0}%", v * 100.0))
        .unwrap_or_default()
}

/// a weight count, short
fn count(n: u64) -> String {
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

#[derive(Template)]
#[template(path = "classifiers.html")]
struct Grid {
    l: Layout,
    q: String,
    q_url: String,
    sort: &'static str,
    sorts: [&'static str; 4],
    cards: Vec<ClsCard>,
    page: usize,
    more: bool,
    total: usize,
    total_classes: usize,
    authors: usize,
}

#[handler]
pub async fn grid(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let q: String = req.query::<String>("q").unwrap_or_default().chars().take(200).collect();
    let sort = Sort::parse(&req.query::<String>("sort").unwrap_or_default());
    let page = req.query::<usize>("page").unwrap_or(0).min(10_000);
    let h = hub();
    let (found, more) = h.find_classifiers(&q, sort, page).unwrap_or_default();
    let all = h.store.classifiers().unwrap_or_default();
    let approved: Vec<&ClassifierRec> = all.iter().filter(|c| c.status == Status::Approved).collect();
    let mut authors: Vec<String> = approved.iter().map(|c| c.author.to_lowercase()).collect();
    authors.sort();
    authors.dedup();
    render(
        res,
        Grid {
            l: layout(depot, "classifiers"),
            q_url: url(&q),
            q,
            sort: sort.name(),
            sorts: ["new", "stars", "downloads", "featured"],
            cards: found.iter().map(cls_card).collect(),
            page,
            more,
            total: approved.len(),
            total_classes: approved.iter().map(|c| c.classes.len()).sum(),
            authors: authors.len(),
        },
    );
}

/// a classifier the asker may see: approved ones, their own, or any for an admin
fn visible(depot: &Depot, id: &str) -> Option<ClassifierRec> {
    let c = hub().store.classifier(id).ok().flatten()?;
    let mine = me(depot).is_some_and(|u| u.is_admin() || u.name.eq_ignore_ascii_case(&c.author));
    (c.status == Status::Approved || mine).then_some(c)
}

/// a classifier the asker may change
fn own(depot: &Depot, id: &str) -> Option<ClassifierRec> {
    let c = hub().store.classifier(id).ok().flatten()?;
    may_edit(me(depot), &c).then_some(c)
}

#[derive(Template)]
#[template(path = "classifier.html")]
struct Page {
    l: Layout,
    c: ClassifierRec,
    status: &'static str,
    updated: String,
    created: String,
    size: String,
    weights: String,
    accuracy: String,
    macro_f1: String,
    hue: u64,
    can_star: bool,
    starred: bool,
    can_take: bool,
    can_edit: bool,
    is_admin: bool,
    missing: Vec<String>,
}

#[handler]
pub async fn detail(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(c) = visible(depot, &id) else {
        return not_found(res, depot);
    };
    let user = me(depot);
    let is_admin = user.is_some_and(|u| u.is_admin());
    let mine = user.is_some_and(|u| u.name.eq_ignore_ascii_case(&c.author));
    let starred = user.is_some_and(|u| hub().store.has_starred(&c.key(), &u.name).unwrap_or(false));
    let active = user.is_some_and(Hub::active);
    render(
        res,
        Page {
            l: layout(depot, "classifiers"),
            status: status_name(c.status),
            updated: ago(c.updated),
            created: ago(c.created),
            size: human_size(c.size),
            weights: count(c.weights),
            accuracy: percent(c.accuracy),
            macro_f1: percent(c.macro_f1),
            hue: hue(&c.name),
            can_star: active && !mine && c.status == Status::Approved,
            starred,
            can_take: active && c.status == Status::Approved,
            can_edit: may_edit(user, &c),
            is_admin,
            missing: c.upstream.iter().filter(|u| !u.1).map(|u| u.0.clone()).collect(),
            c,
        },
    );
}

/// a file name for a module's download
fn file_name(c: &ClassifierRec) -> String {
    let n: String = c
        .name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let n = n.trim_matches('-');
    format!("{}.anendcls", if n.is_empty() { c.id.as_str() } else { n })
}

/// `GET /classifiers/{id}/download` (and `/api/classifiers/{id}/download`): the module
#[handler]
pub async fn download(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(c) = visible(depot, &id) else {
        return not_found(res, depot);
    };
    let Some(path) = hub().store.classifier_path(&c.id) else {
        return not_found(res, depot);
    };
    let who = match me(depot) {
        Some(u) => u.name.to_lowercase(),
        None => auth::address_key(client_ip(req)),
    };
    if c.status == Status::Approved {
        hub().classifier_downloaded(&c, &who).ok();
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            res.headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
            if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", file_name(&c))) {
                res.headers_mut().insert(CONTENT_DISPOSITION, v);
            }
            res.headers_mut()
                .insert("x-content-type-options", HeaderValue::from_static("nosniff"));
            res.write_body(bytes).ok();
        }
        Err(_) => not_found(res, depot),
    }
}

#[handler]
pub async fn star(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(user) = need_active(res, depot) else { return };
    let Some(c) = visible(depot, &id) else {
        return not_found(res, depot);
    };
    let back = format!("/classifiers/{}", c.id);
    match hub().star_classifier(&c, user) {
        Ok((true, _)) => go(res, &back, "starred"),
        Ok((false, _)) => go(res, &back, "unstarred"),
        Err(e) => go(res, &back, &format!("!{e}")),
    }
}

// ------------------------------------------------------------------ sharing and editing

fn share_form(depot: &Depot, action: String, heading: String, c: Option<&ClassifierRec>, intro: String) -> Form {
    let max = human_size(hub().classifier_max());
    Form {
        l: layout(depot, "classifiers"),
        heading,
        intro,
        action,
        multipart: true,
        fields: vec![
            Field {
                hint: "left empty, it's the classifier's own name",
                ..field("name", "name", "text", c.map(|c| c.name.as_str()).unwrap_or(""))
            },
            field(
                "description",
                "what it tells apart, and from what",
                "textarea",
                c.map(|c| c.description.as_str()).unwrap_or(""),
            ),
            Field {
                hint: "a few words, comma separated",
                ..field(
                    "tags",
                    "tags",
                    "text",
                    &c.map(|c| c.tags.join(", ")).unwrap_or_default(),
                )
            },
            Field {
                required: c.is_none(),
                hint: if c.is_some() {
                    "leave empty to keep the module and only change the words"
                } else {
                    "a .anendcls module: in the app, classifiers → export"
                },
                ..field("file", "the module", "clsfile", "")
            },
        ],
        submit: if c.is_some() { "save" } else { "share" },
        after: format!(
            "Up to {max}. The hub reads every module strictly before taking it. {}",
            if me(depot).is_some_and(|u| matches!(u.role, Role::Trusted | Role::Admin)) {
                "Yours go straight in."
            } else {
                "An admin looks at each one before it shows in the grid."
            }
        ),
    }
}

#[handler]
pub async fn upload_page(depot: &mut Depot, res: &mut Response) {
    if need_active(res, depot).is_none() {
        return;
    }
    let f = share_form(
        depot,
        "/classifiers/upload".into(),
        "share a classifier".into(),
        None,
        "Share a trained classifier: its weights and classes, for anyone to take into their own saves. Its classes become lists there, matched by name.".into(),
    );
    render(res, f);
}

#[handler]
pub async fn upload(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(user) = need_active(res, depot).cloned() else {
        return;
    };
    const BACK: &str = "/classifiers/upload";
    if !hub().limiter.allow(client_ip(req), "clsupload", 20, 3600) {
        return go(res, BACK, "!that's a lot of uploads; try again in a while");
    }
    let sent = match read_upload(req).await {
        Ok(s) => s,
        Err(e) => return go(res, BACK, &format!("!{e}")),
    };
    let Some((path, _)) = sent.file else {
        return go(res, BACK, "!pick a module to share");
    };
    let words = (sent.name.as_str(), sent.description.as_str(), sent.tags.as_str());
    match hub().accept_classifier(&user, words, &path, None).await {
        Ok(c) if c.status == Status::Approved => go(res, &format!("/classifiers/{}", c.id), "shared: it's in the grid"),
        Ok(c) => go(
            res,
            &format!("/classifiers/{}", c.id),
            "sent in: an admin will look at it, then it shows in the grid",
        ),
        Err(e) => go(res, BACK, &format!("!{e}")),
    }
}

#[handler]
pub async fn edit_page(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(c) = own(depot, &id) else {
        return not_found(res, depot);
    };
    let f = share_form(
        depot,
        format!("/classifiers/{}/edit", c.id),
        format!("edit “{}”", c.name),
        Some(&c),
        "Change its words, or send a new version of the module (it keeps its stars and downloads).".into(),
    );
    render(res, f);
}

#[handler]
pub async fn edit(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let Some(c) = own(depot, &id) else {
        return not_found(res, depot);
    };
    let Some(user) = need_active(res, depot).cloned() else {
        return;
    };
    let back = format!("/classifiers/{}/edit", c.id);
    let page = format!("/classifiers/{}", c.id);
    if !hub().limiter.allow(client_ip(req), "clsupload", 20, 3600) {
        return go(res, &back, "!that's a lot of uploads; try again in a while");
    }
    let sent = match read_upload(req).await {
        Ok(x) => x,
        Err(e) => return go(res, &back, &format!("!{e}")),
    };
    let words = (sent.name.as_str(), sent.description.as_str(), sent.tags.as_str());
    match sent.file {
        Some((path, _)) => {
            // the new version is the author's, even when an admin sends it
            let author = hub().store.user(&c.author).ok().flatten().unwrap_or(user);
            match hub().accept_classifier(&author, words, &path, Some(&c)).await {
                Ok(n) if n.status == Status::Approved => go(res, &page, "new version is in"),
                Ok(_) => go(res, &page, "new version sent in: an admin will look at it"),
                Err(e) => go(res, &back, &format!("!{e}")),
            }
        }
        None => {
            let name = if words.0.trim().is_empty() {
                c.name.as_str()
            } else {
                words.0
            };
            let details = match crate::hub::Details::new(name, words.1, words.2) {
                Ok(d) => d,
                Err(e) => return go(res, &back, &format!("!{e}")),
            };
            let r = hub().store.update_classifier(&c.id, |x| {
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
    let Some(c) = own(depot, &id) else {
        return not_found(res, depot);
    };
    let who = me(depot).map(|u| u.name.clone()).unwrap_or_default();
    match hub().remove_classifier(&c.id) {
        Ok(()) => {
            hub().store.log(&who, "deleted a classifier", "").ok();
            go(res, "/classifiers", &format!("deleted “{}”", c.name))
        }
        Err(e) => go(res, &format!("/classifiers/{}", c.id), &format!("!{e}")),
    }
}

/// `POST /admin/classifiers/{id}`: approve, reject (with `note`), feature, unfeature, remove
#[handler]
pub async fn admin_action(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(admin) = need_admin(res, depot).cloned() else {
        return;
    };
    let id = req.param::<String>("id").unwrap_or_default();
    let action = req.form::<String>("action").await.unwrap_or_default();
    let note = req.form::<String>("note").await.unwrap_or_default();
    if action.is_empty() {
        return go(res, "/admin#classifiers", "");
    }
    let back = if action == "remove" {
        "/admin#classifiers".to_string()
    } else {
        req.header::<String>("referer")
            .filter(|r| r.contains("/classifiers/"))
            .map(|_| format!("/classifiers/{id}"))
            .unwrap_or_else(|| "/admin#classifiers".into())
    };
    match hub().review_classifier(&admin, &id, &action, &note) {
        Ok(m) => go(res, &back, &m),
        Err(e) => go(res, &back, &format!("!{e}")),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_numbers() {
        let c = ClassifierRec {
            id: "abc".into(),
            name: "../../etc/passwd \"x\"".into(),
            ..Default::default()
        };
        assert_eq!(file_name(&c), "etc-passwd--x.anendcls");
        assert_eq!(
            file_name(&ClassifierRec {
                id: "abc".into(),
                name: "///".into(),
                ..Default::default()
            }),
            "abc.anendcls"
        );
        assert_eq!(percent(Some(0.934)), "93%");
        assert_eq!(percent(Some(f64::NAN)), "");
        assert_eq!(count(5200), "5k");
    }
}
