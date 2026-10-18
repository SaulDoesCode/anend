//! The web app's side of the hub: its files (hub/web, built into the binary), the
//! WebSocket and the plain posts that reach app.rs, and moving saves between a
//! workspace, the hub and your computer.

use futures_util::{SinkExt, StreamExt};
use rust_embed::RustEmbed;
use salvo::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use salvo::prelude::*;
use salvo::websocket::{Message, WebSocketUpgrade};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::app::{self, workspaces};
#[cfg(feature = "hub")]
use crate::hub::Details;
#[cfg(feature = "hub")]
use crate::store::{Status, Visibility, may_see};
use crate::store::User;
use crate::web::{hub, me, mode};

/// whose saves a call works on (`app::owner`), for this request's mode
fn owner(depot: &Depot, user: Option<&User>) -> Result<String, String> {
    app::owner(user, mode(depot).local)
}

/// the status for a refusal: 401 for no one, 403 for an account still waiting
fn refusal(depot: &Depot) -> StatusCode {
    if me(depot).is_none() { StatusCode::UNAUTHORIZED } else { StatusCode::FORBIDDEN }
}

#[derive(RustEmbed)]
#[folder = "web/"]
struct Files;

fn kind(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// the web app's files, for signed-in members only (or anyone on a `--local` server);
/// `/app/` is its page
#[handler]
pub async fn files(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let rest = req.param::<String>("rest").unwrap_or_default();
    let path = if rest.is_empty() {
        "index.html".to_string()
    } else {
        rest
    };
    if path == "index.html" {
        return page(depot, res);
    }
    if owner(depot, me(depot)).is_err() {
        res.status_code(refusal(depot));
        return res.render(Json(json!({ "ok": false, "error": "log in to use the app here" })));
    }
    serve(res, &path, mode(depot));
}

/// the app's page: signed in and let in, or off to log in (or a page saying it waits)
fn page(depot: &Depot, res: &mut Response) {
    match owner(depot, me(depot)) {
        Ok(_) => serve(res, "index.html", mode(depot)),
        Err(_) if me(depot).is_none() => res.render(Redirect::other("/login?next=/app/")),
        Err(e) => crate::web::message(res, depot, StatusCode::FORBIDDEN, "not yet", &format!("{}{}.", e[..1].to_uppercase(), &e[1..])),
    }
}

fn serve(res: &mut Response, path: &str, m: crate::mode::Mode) {
    match Files::get(path) {
        Some(f) => {
            res.headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(kind(path)));
            res.headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            // how the page knows a server served it (api.js), and not a static server:
            // "hub" when the hub is here too; "local" for a --local server (no accounts,
            // nothing to share on); "app" for the app alone (no hub to share on)
            let what = match (m.local, m.hub) {
                (true, _) => "local",
                (_, true) => "hub",
                _ => "app",
            };
            res.headers_mut().insert("x-anend", HeaderValue::from_static(what));
            res.write_body(f.data.into_owned()).ok();
        }
        None => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Text::Plain("not here"));
        }
    }
}

/// `/app` goes to `/app/` (so the page's own links resolve), and `/app/` is the page:
/// the router matches both here
#[handler]
pub async fn app_root(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if req.uri().path().ends_with('/') {
        page(depot, res);
    } else {
        res.render(Redirect::found("/app/"));
    }
}

fn envelope(id: &Value, r: app::Res) -> Value {
    match r {
        Ok(data) => json!({ "id": id, "ok": true, "data": data }),
        Err(e) => json!({ "id": id, "ok": false, "error": e }),
    }
}

/// how many calls one member's app may have running at once, and how many sockets it
/// may hold open: one person's tabs can't take every thread the hub has
const CALLS_AT_ONCE: usize = 4;
const SOCKETS: usize = 8;

/// each owner's turnstile for calls, and how many sockets they have open
type Gates = std::sync::Mutex<std::collections::HashMap<String, (Arc<Semaphore>, usize)>>;

fn gates() -> &'static Gates {
    static G: std::sync::OnceLock<Gates> = std::sync::OnceLock::new();
    G.get_or_init(Default::default)
}

fn gate(owner: &str) -> Arc<Semaphore> {
    let local = crate::mode::local();
    let mut g = gates().lock().unwrap_or_else(|e| e.into_inner());
    g.entry(owner.to_string())
        .or_insert_with(|| (Arc::new(Semaphore::new(if local { 64 } else { CALLS_AT_ONCE })), 0))
        .0
        .clone()
}

/// waits for a turn to run one of `owner`'s calls
async fn turn(owner: &str) -> Result<OwnedSemaphorePermit, String> {
    gate(owner).acquire_owned().await.map_err(|e| e.to_string())
}

/// counts a socket in (false: too many open) or out
fn socket_count(owner: &str, opened: bool) -> bool {
    let local = crate::mode::local();
    let mut g = gates().lock().unwrap_or_else(|e| e.into_inner());
    let e = g
        .entry(owner.to_string())
        .or_insert_with(|| (Arc::new(Semaphore::new(if local { 64 } else { CALLS_AT_ONCE })), 0));
    if opened {
        if !local && e.1 >= SOCKETS {
            return false;
        }
        e.1 += 1;
    } else {
        e.1 = e.1.saturating_sub(1);
    }
    true
}

/// runs an operation for whoever's asking
async fn run(owner: Result<String, String>, op: String, args: Value) -> app::Res {
    let owner = owner?;
    tokio::task::spawn_blocking(move || {
        let ws = workspaces().get(&owner);
        app::call(&ws, &op, &args)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `POST /app/api/{op}` with the arguments as JSON
#[handler]
pub async fn api(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let op = req.param::<String>("op").unwrap_or_default();
    // who's asking before anything is read: a body from no one isn't parsed at all
    let who = owner(depot, me(depot));
    if let Err(e) = &who {
        res.status_code(refusal(depot));
        return res.render(Json(envelope(&Value::Null, Err(e.clone()))));
    }
    let r = match &who {
        Ok(o) => match turn(o).await {
            Ok(_permit) => {
                let args: Value = req.parse_json_with_max_size(64 << 20).await.unwrap_or(Value::Null);
                run(who, op, args).await
            }
            Err(e) => Err(e),
        },
        Err(e) => Err(e.clone()),
    };
    if r.is_err() {
        res.status_code(StatusCode::BAD_REQUEST);
    }
    res.render(Json(envelope(&Value::Null, r)));
}

/// whether a request comes from one of the hub's own pages (a WebSocket carries the
/// login cookie, so another site mustn't open one)
fn same_origin(req: &Request) -> bool {
    let host = req.header::<String>("host").unwrap_or_default();
    req.header::<String>("origin")
        .map(|o| o.split("://").nth(1).unwrap_or("") == host)
        .unwrap_or(true)
}

/// `GET /app/ws`: a WebSocket of calls and answers
#[handler]
pub async fn socket(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    if !same_origin(req) {
        return Err(StatusError::forbidden().brief("only the hub's own pages"));
    }
    // no socket for no one (the app posts instead, and hears why)
    let who = owner(depot, me(depot)).map_err(|e| {
        if me(depot).is_none() {
            StatusError::unauthorized().brief(e)
        } else {
            StatusError::forbidden().brief(e)
        }
    })?;
    if !socket_count(&who, true) {
        return Err(StatusError::too_many_requests().brief("too many of your app's tabs are open"));
    }
    let counted = who.clone();
    let r = WebSocketUpgrade::new()
        .max_message_size(64 << 20)
        .upgrade(req, res, move |ws| async move {
            let (mut tx, mut rx) = ws.split();
            while let Some(Ok(msg)) = rx.next().await {
                if msg.is_close() {
                    break;
                }
                let Ok(text) = msg.as_str() else { continue };
                let Ok(_permit) = turn(&who).await else { break };
                let call: Value = serde_json::from_str(text).unwrap_or(Value::Null);
                let id = call.get("id").cloned().unwrap_or(Value::Null);
                let op = call.get("op").and_then(Value::as_str).unwrap_or("").to_string();
                let args = call.get("args").cloned().unwrap_or(Value::Null);
                let answer = envelope(&id, run(Ok(who.clone()), op, args).await);
                if tx.send(Message::text(answer.to_string())).await.is_err() {
                    break;
                }
            }
            socket_count(&who, false);
        })
        .await;
    if r.is_err() {
        socket_count(&counted, false);
    }
    r
}

// ------------------------------------------------------------------ moving saves

/// `GET /app/saves/{name}`: a workspace save to take home
#[handler]
pub async fn download(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let name = req.param::<String>("name").unwrap_or_default();
    let found = owner(depot, me(depot)).and_then(|o| workspaces().get(&o).path(&name));
    match found {
        Ok(p) if p.exists() => {
            salvo::fs::NamedFile::builder(&p)
                .attached_name(format!("{}.redb", app::clean(&name).unwrap_or(name)))
                .send(req.headers(), res)
                .await
        }
        _ => {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(json!({ "ok": false, "error": "no such save" })));
        }
    }
}

/// `POST /app/saves`: a save from your computer into the workspace (multipart `file`)
#[handler]
pub async fn upload(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let who = owner(depot, me(depot));
    let max = hub().config.max_upload as usize + 64 * 1024;
    let got = async {
        let o = who?;
        let form = req
            .form_data_max_size(max)
            .await
            .map_err(|_| "that file is too big".to_string())?;
        let f = form.files.get("file").ok_or("send the save as `file`")?;
        let name = form
            .fields
            .get("name")
            .cloned()
            .or_else(|| f.name().map(|n| n.trim_end_matches(".redb").to_string()))
            .unwrap_or_else(|| "uploaded".into());
        let src = f.path().clone();
        tokio::task::spawn_blocking(move || {
            // it must be a save: the hub's own check, on a copy
            let ws = workspaces().get(&o);
            crate::inspect::inspect(&src)?;
            app::take_hub_save(&ws, &src, &name, false)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    .await;
    if got.is_err() {
        res.status_code(StatusCode::BAD_REQUEST);
    }
    res.render(Json(envelope(&Value::Null, got)));
}

/// `POST /app/hub/{id}?merge=1`: a hub save into the workspace, beside the others or
/// merged into the open one
#[cfg(feature = "hub")]
#[handler]
pub async fn take(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let merge = req.query::<String>("merge").is_some_and(|m| m == "1" || m == "true");
    let who = owner(depot, me(depot));
    let got = async {
        let o = who?;
        let save = hub()
            .store
            .save(&id)?
            .filter(|s| may_see(s.visibility, s.status, &s.author, me(depot)))
            .ok_or("no such save on the hub")?;
        let who_took = me(depot)
            .map(|u| u.name.to_lowercase())
            .unwrap_or_else(|| "local".into());
        hub().downloaded(&save, &who_took).ok();
        let file = hub().store.save_path(&save.id);
        tokio::task::spawn_blocking(move || app::take_hub_save(&workspaces().get(&o), &file, &save.name, merge))
            .await
            .map_err(|e| e.to_string())?
    }
    .await;
    if got.is_err() {
        res.status_code(StatusCode::BAD_REQUEST);
    }
    res.render(Json(envelope(&Value::Null, got)));
}

/// `POST /app/share`: the open save onto the hub (`name`, `description`, `tags`)
#[cfg(feature = "hub")]
#[handler]
pub async fn share(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let args: Value = req.parse_json().await.unwrap_or(Value::Null);
    let user = me(depot).cloned();
    let got = async {
        let user = user.ok_or("log in to share on the hub")?;
        let o = owner(depot, Some(&user))?;
        let ws = workspaces().get(&o);
        let (open, _) = ws.graph()?;
        let path = ws.path(&open)?;
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let name = if s("name").is_empty() { open.clone() } else { s("name") };
        let details = Details::new(&name, &s("description"), &s("tags"))?.seen_by(&s("visibility"))?;
        let saved = hub().accept(&user, details, &path, None).await?;
        Ok(json!({
            "id": saved.id,
            "status": if saved.status == Status::Approved { "approved" } else { "pending" },
            "visibility": saved.visibility.name(),
            "url": format!("/saves/{}", saved.id),
        }))
    }
    .await;
    if got.is_err() {
        res.status_code(StatusCode::BAD_REQUEST);
    }
    res.render(Json(envelope(&Value::Null, got)));
}

// ------------------------------------------------------------------ moving classifiers

/// `POST /app/classifiers/{id}/take`: an approved hub classifier into the open save; its
/// classes go to lists by name. Optional JSON `{as, lists: {class: key}}`.
#[cfg(feature = "hub")]
#[handler]
pub async fn take_classifier(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let args: Value = req.parse_json_with_max_size(64 * 1024).await.unwrap_or(Value::Null);
    let who = owner(depot, me(depot));
    let ip = crate::web::client_ip(req);
    let got = async {
        let o = who?;
        if !hub().limiter.allow(ip, "clstake", 120, 3600) {
            return Err("that's a lot of classifiers; try again in a while".to_string());
        }
        let c = hub()
            .store
            .classifier(&id)?
            .filter(|c| may_see(c.visibility, c.status, &c.author, me(depot)))
            .ok_or("no such classifier on the hub")?;
        let file = hub()
            .store
            .classifier_path(&c.id)
            .ok_or("no such classifier on the hub")?;
        let max = hub().classifier_max();
        let took =
            tokio::task::spawn_blocking(move || crate::clshub::take_into(&workspaces().get(&o), &file, max, &args))
                .await
                .map_err(|e| e.to_string())??;
        let who_took = me(depot)
            .map(|u| u.name.to_lowercase())
            .unwrap_or_else(|| "local".into());
        hub().classifier_downloaded(&c, &who_took).ok();
        Ok(took)
    }
    .await;
    if got.is_err() {
        res.status_code(StatusCode::BAD_REQUEST);
    }
    res.render(Json(envelope(&Value::Null, got)));
}

/// `POST /app/classifiers/share`: a classifier from the open save onto the hub (`name`
/// of the classifier in the save; `title`, `description`, `tags` for the hub)
#[cfg(feature = "hub")]
#[handler]
pub async fn share_classifier(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let args: Value = req.parse_json_with_max_size(64 * 1024).await.unwrap_or(Value::Null);
    let user = me(depot).cloned();
    let ip = crate::web::client_ip(req);
    let got = async {
        let user = user.ok_or("log in to share on the hub")?;
        let o = owner(depot, Some(&user))?;
        if !hub().limiter.allow(ip, "clsupload", 20, 3600) {
            return Err("that's a lot of uploads; try again in a while".to_string());
        }
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let name = s("name");
        if name.trim().is_empty() {
            return Err("say which classifier (`name`)".into());
        }
        let (description, author) = (s("description"), user.name.clone());
        let bytes = {
            let name = name.clone();
            let description = description.clone();
            tokio::task::spawn_blocking(move || {
                crate::clshub::export_from(&workspaces().get(&o), &name, &description, &author)
            })
            .await
            .map_err(|e| e.to_string())??
        };
        let title = if s("title").trim().is_empty() { name } else { s("title") };
        let vis = match s("visibility").trim() {
            "" => None,
            v => Some(Visibility::parse(v).ok_or("visibility is private, unlisted or public")?),
        };
        let c = hub()
            .accept_classifier_bytes(&user, (&title, &description, &s("tags")), vis, bytes, None)
            .await?;
        Ok(json!({
            "id": c.id,
            "status": if c.status == Status::Approved { "approved" } else { "pending" },
            "visibility": c.visibility.name(),
            "url": format!("/classifiers/{}", c.id),
        }))
    }
    .await;
    if got.is_err() {
        res.status_code(StatusCode::BAD_REQUEST);
    }
    res.render(Json(envelope(&Value::Null, got)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// one member's tabs hold at most SOCKETS sockets and CALLS_AT_ONCE calls
    #[test]
    fn turnstiles() {
        let who = format!("gate-test-{}", rand::random::<u32>());
        for _ in 0..SOCKETS {
            assert!(socket_count(&who, true));
        }
        assert!(!socket_count(&who, true), "one too many");
        socket_count(&who, false);
        assert!(socket_count(&who, true));
        let g = gate(&who);
        let held: Vec<_> = (0..CALLS_AT_ONCE)
            .map(|_| g.clone().try_acquire_owned().unwrap())
            .collect();
        assert!(g.clone().try_acquire_owned().is_err(), "a fifth call waits");
        drop(held);
        assert!(g.try_acquire_owned().is_ok());
    }
}
