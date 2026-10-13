//! The web app's side of the hub: its files (hub/web, built into the binary), the
//! WebSocket and the plain posts that reach app.rs, and moving saves between a
//! workspace, the hub and your computer.

use futures_util::{SinkExt, StreamExt};
use rust_embed::RustEmbed;
use salvo::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use salvo::prelude::*;
use salvo::websocket::{Message, WebSocketUpgrade};
use serde_json::{Value, json};

use crate::app::{self, owner, workspaces};
use crate::hub::Details;
use crate::store::Status;
use crate::web::{hub, me};

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

/// the web app's files; `/app/` is its page
#[handler]
pub async fn files(req: &mut Request, res: &mut Response) {
    let rest = req.param::<String>("rest").unwrap_or_default();
    let path = if rest.is_empty() {
        "index.html".to_string()
    } else {
        rest
    };
    serve(res, &path);
}

fn serve(res: &mut Response, path: &str) {
    match Files::get(path) {
        Some(f) => {
            res.headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(kind(path)));
            res.headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            // how the page knows a hub served it (api.js), and not a static server
            res.headers_mut()
                .insert("x-anend", HeaderValue::from_static("hub"));
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
pub async fn app_root(req: &mut Request, res: &mut Response) {
    if req.uri().path().ends_with('/') {
        serve(res, "index.html");
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
    let args: Value = req.parse_json_with_max_size(64 << 20).await.unwrap_or(Value::Null);
    let who = owner(me(depot));
    let r = run(who, op, args).await;
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
    let who = owner(me(depot));
    WebSocketUpgrade::new()
        .max_message_size(64 << 20)
        .upgrade(req, res, move |ws| async move {
            let (mut tx, mut rx) = ws.split();
            while let Some(Ok(msg)) = rx.next().await {
                if msg.is_close() {
                    break;
                }
                let Ok(text) = msg.as_str() else { continue };
                let call: Value = serde_json::from_str(text).unwrap_or(Value::Null);
                let id = call.get("id").cloned().unwrap_or(Value::Null);
                let op = call.get("op").and_then(Value::as_str).unwrap_or("").to_string();
                let args = call.get("args").cloned().unwrap_or(Value::Null);
                let answer = envelope(&id, run(who.clone(), op, args).await);
                if tx.send(Message::text(answer.to_string())).await.is_err() {
                    break;
                }
            }
        })
        .await
}

// ------------------------------------------------------------------ moving saves

/// `GET /app/saves/{name}`: a workspace save to take home
#[handler]
pub async fn download(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let name = req.param::<String>("name").unwrap_or_default();
    let found = owner(me(depot)).and_then(|o| workspaces().get(&o).path(&name));
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
    let who = owner(me(depot));
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
#[handler]
pub async fn take(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let id = req.param::<String>("id").unwrap_or_default();
    let merge = req.query::<String>("merge").is_some_and(|m| m == "1" || m == "true");
    let who = owner(me(depot));
    let got = async {
        let o = who?;
        let save = hub()
            .store
            .save(&id)?
            .filter(|s| s.status == Status::Approved)
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
#[handler]
pub async fn share(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let args: Value = req.parse_json().await.unwrap_or(Value::Null);
    let user = me(depot).cloned();
    let got = async {
        let user = user.ok_or("log in to share on the hub")?;
        let o = owner(Some(&user))?;
        let ws = workspaces().get(&o);
        let (open, _) = ws.store()?;
        let path = ws.path(&open)?;
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let name = if s("name").is_empty() { open.clone() } else { s("name") };
        let details = Details::new(&name, &s("description"), &s("tags"))?;
        let saved = hub().accept(&user, details, &path, None).await?;
        Ok(json!({ "id": saved.id, "status": if saved.status == Status::Approved { "approved" } else { "pending" } }))
    }
    .await;
    if got.is_err() {
        res.status_code(StatusCode::BAD_REQUEST);
    }
    res.render(Json(envelope(&Value::Null, got)));
}
