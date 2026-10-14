//! The JSON API the app uses (docs/hub-api.md): browsing and fetching saves and
//! classifiers, sharing one with an app token, and who the token belongs to.

use salvo::prelude::*;
use serde::Serialize;
use serde_json::{Value, json};

use crate::game;
use crate::hub::{Details, Hub, Sort};
use crate::store::{ClassifierRec, Sample, Save, Standing, Status};
use crate::web::{hub, me};

#[derive(Serialize)]
pub struct SaveJson {
    id: String,
    name: String,
    author: String,
    description: String,
    tags: Vec<String>,
    notes: u64,
    memories: u64,
    size: u64,
    stars: u64,
    downloads: u64,
    featured: bool,
    created: String,
    updated: String,
    download_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sample: Option<Vec<Sample>>,
}

fn json_of(s: &Save, sample: bool) -> SaveJson {
    SaveJson {
        id: s.id.clone(),
        name: s.name.clone(),
        author: s.author.clone(),
        description: s.description.clone(),
        tags: s.tags.clone(),
        notes: s.notes,
        memories: s.memories,
        size: s.size,
        stars: s.stars,
        downloads: s.downloads,
        featured: s.featured,
        created: s.created.to_rfc3339(),
        updated: s.updated.to_rfc3339(),
        download_url: format!("/saves/{}/download", s.id),
        sample: sample.then(|| s.sample.clone()),
    }
}

fn error(res: &mut Response, code: StatusCode, msg: &str) {
    res.status_code(code);
    res.render(Json(json!({ "error": msg })));
}

#[handler]
pub async fn about(res: &mut Response) {
    let h = hub();
    let saves = h.store.saves().unwrap_or_default();
    let users = h.store.users().unwrap_or_default();
    res.render(Json(json!({
        "name": h.config.name,
        "description": h.config.description,
        "saves": saves.iter().filter(|s| s.status == Status::Approved).count(),
        "classifiers": h.store.classifiers().unwrap_or_default().iter().filter(|c| c.status == Status::Approved).count(),
        "users": users.iter().filter(|u| u.standing == Standing::Active).count(),
    })));
}

#[handler]
pub async fn list(req: &mut Request, res: &mut Response) {
    let q = req.query::<String>("q").unwrap_or_default();
    let sort = Sort::parse(&req.query::<String>("sort").unwrap_or_default());
    let page = req.query::<usize>("page").unwrap_or(0).min(10_000);
    match hub().find(&q, sort, page) {
        Ok((saves, more)) => res.render(Json(json!({
            "saves": saves.iter().map(|s| json_of(s, false)).collect::<Vec<_>>(),
            "more": more,
        }))),
        Err(e) => error(res, StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

#[handler]
pub async fn one(req: &mut Request, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    match hub().store.save(&id) {
        Ok(Some(s)) if s.status == Status::Approved => res.render(Json(json_of(&s, true))),
        _ => error(res, StatusCode::NOT_FOUND, "no such save"),
    }
}

#[handler]
pub async fn whoami(depot: &mut Depot, res: &mut Response) {
    match me(depot) {
        Some(u) => {
            let level = game::level(u.points);
            res.render(Json(json!({
                "username": u.name,
                "points": u.points,
                "level": level,
                "title": game::title(level),
                "active": Hub::active(u),
            })));
        }
        None => error(res, StatusCode::UNAUTHORIZED, "that token isn't known here"),
    }
}

#[handler]
pub async fn share(req: &mut Request, res: &mut Response) {
    // a bearer token only: the login cookie would ride along on a form another site posts
    // here (the hub's own pages share through /upload, which checks where a form came from)
    let Some(user) = token_user(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "send an app token (Authorization: Bearer …)");
    };
    if !Hub::active(&user) {
        return error(res, StatusCode::FORBIDDEN, "your account is waiting for an admin");
    }
    if !hub().limiter.allow(crate::web::client_ip(req), "upload", 20, 3600) {
        return error(
            res,
            StatusCode::TOO_MANY_REQUESTS,
            "that's a lot of uploads; try again in a while",
        );
    }
    let max = hub().config.max_upload;
    let form = match req.form_data_max_size(max as usize + 256 * 1024).await {
        Ok(f) => f,
        Err(_) => {
            return error(
                res,
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!("the hub takes saves up to {}", crate::hub::human_size(max)),
            );
        }
    };
    let get = |k: &str| form.fields.get(k).cloned().unwrap_or_default();
    let (name, description, tags) = (get("name"), get("description"), get("tags"));
    let Some(path) = form
        .files
        .get("file")
        .filter(|f| f.size() > 0)
        .map(|f| f.path().clone())
    else {
        return error(res, StatusCode::BAD_REQUEST, "send the save as `file`");
    };
    let details = match Details::new(&name, &description, &tags) {
        Ok(d) => d,
        Err(e) => return error(res, StatusCode::BAD_REQUEST, &e),
    };
    match hub().accept(&user, details, &path, None).await {
        Ok(s) => res.render(Json(json!({
            "id": s.id,
            "status": if s.status == Status::Approved { "approved" } else { "pending" },
        }))),
        Err(e) => error(res, StatusCode::BAD_REQUEST, &e),
    }
}

// ------------------------------------------------------------------ classifiers

#[derive(Serialize)]
pub struct ClassifierJson {
    id: String,
    name: String,
    author: String,
    description: String,
    tags: Vec<String>,
    classes: Vec<String>,
    head: String,
    preset: String,
    weights: u64,
    accuracy: Option<f64>,
    macro_f1: Option<f64>,
    examples: usize,
    size: u64,
    /// the classifiers it builds on: `{name, bundled}`
    upstream: Vec<Value>,
    stars: u64,
    downloads: u64,
    featured: bool,
    version: u32,
    created: String,
    updated: String,
    download_url: String,
    take_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<Value>,
}

pub fn classifier_json(c: &ClassifierRec, detail: bool) -> ClassifierJson {
    ClassifierJson {
        id: c.id.clone(),
        name: c.name.clone(),
        author: c.author.clone(),
        description: c.description.clone(),
        tags: c.tags.clone(),
        classes: c.classes.clone(),
        head: c.head.clone(),
        preset: c.preset.clone(),
        weights: c.weights,
        accuracy: c.accuracy.filter(|x| x.is_finite()),
        macro_f1: c.macro_f1.filter(|x| x.is_finite()),
        examples: c.examples,
        size: c.size,
        upstream: c
            .upstream
            .iter()
            .map(|(n, b)| json!({ "name": n, "bundled": b }))
            .collect(),
        stars: c.stars,
        downloads: c.downloads,
        featured: c.featured,
        version: c.version,
        created: c.created.to_rfc3339(),
        updated: c.updated.to_rfc3339(),
        download_url: format!("/classifiers/{}/download", c.id),
        take_url: format!("/app/classifiers/{}/take", c.id),
        detail: detail.then(|| {
            json!({
                "module": c.module,
                "hash": c.hash,
                "measured_on": c.measured_on,
            })
        }),
    }
}

/// `GET /api/classifiers?q=&sort=&page=`
#[handler]
pub async fn classifiers(req: &mut Request, res: &mut Response) {
    let q: String = req.query::<String>("q").unwrap_or_default().chars().take(200).collect();
    let sort = Sort::parse(&req.query::<String>("sort").unwrap_or_default());
    let page = req.query::<usize>("page").unwrap_or(0).min(10_000);
    match hub().find_classifiers(&q, sort, page) {
        Ok((found, more)) => res.render(Json(json!({
            "classifiers": found.iter().map(|c| classifier_json(c, false)).collect::<Vec<_>>(),
            "more": more,
        }))),
        Err(e) => error(res, StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// `GET /api/classifiers/{id}`
#[handler]
pub async fn classifier(req: &mut Request, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    match hub().store.classifier(&id) {
        Ok(Some(c)) if c.status == Status::Approved => res.render(Json(classifier_json(&c, true))),
        _ => error(res, StatusCode::NOT_FOUND, "no such classifier"),
    }
}

/// the user behind the request's app token (and only a token: a cookie can ride along
/// on a request from another site, a bearer token can't)
fn token_user(req: &Request) -> Option<crate::store::User> {
    let bearer = req
        .header::<String>("authorization")
        .and_then(|h| h.strip_prefix("Bearer ").map(|t| t.trim().to_string()))
        .filter(|t| !t.is_empty())?;
    let t = hub().store.token(&crate::auth::digest(&bearer)).ok().flatten()?;
    hub()
        .store
        .user(&t.user)
        .ok()
        .flatten()
        .filter(|u| u.standing != Standing::Banned)
}

/// `POST /api/classifiers` with an app token: multipart `file` (the .anendcls module),
/// `name`, `description`, `tags`
#[handler]
pub async fn share_classifier(req: &mut Request, res: &mut Response) {
    let Some(user) = token_user(req) else {
        return error(res, StatusCode::UNAUTHORIZED, "send an app token (Authorization: Bearer …)");
    };
    if !Hub::active(&user) {
        return error(res, StatusCode::FORBIDDEN, "your account is waiting for an admin");
    }
    if !hub().limiter.allow(crate::web::client_ip(req), "clsupload", 20, 3600) {
        return error(
            res,
            StatusCode::TOO_MANY_REQUESTS,
            "that's a lot of uploads; try again in a while",
        );
    }
    let max = hub().classifier_max();
    let form = match req.form_data_max_size(max as usize + 256 * 1024).await {
        Ok(f) => f,
        Err(_) => {
            return error(
                res,
                StatusCode::PAYLOAD_TOO_LARGE,
                &format!("the hub takes classifiers up to {}", crate::hub::human_size(max)),
            );
        }
    };
    let get = |k: &str| form.fields.get(k).cloned().unwrap_or_default();
    let (name, description, tags) = (get("name"), get("description"), get("tags"));
    let Some(path) = form
        .files
        .get("file")
        .filter(|f| f.size() > 0)
        .map(|f| f.path().clone())
    else {
        return error(res, StatusCode::BAD_REQUEST, "send the module as `file`");
    };
    match hub()
        .accept_classifier(&user, (&name, &description, &tags), &path, None)
        .await
    {
        Ok(c) => res.render(Json(json!({
            "id": c.id,
            "status": if c.status == Status::Approved { "approved" } else { "pending" },
            "url": format!("/classifiers/{}", c.id),
        }))),
        Err(e) => error(res, StatusCode::BAD_REQUEST, &e),
    }
}
