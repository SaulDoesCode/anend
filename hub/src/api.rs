//! The JSON API the app uses (docs/hub-api.md): browsing and fetching saves, sharing one
//! with an app token, and who the token belongs to.

use salvo::prelude::*;
use serde::Serialize;
use serde_json::json;

use crate::game;
use crate::hub::{Details, Hub, Sort};
use crate::store::{Sample, Save, Standing, Status};
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
pub async fn share(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(user) = me(depot).cloned() else {
        return error(res, StatusCode::UNAUTHORIZED, "that token isn't known here");
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
