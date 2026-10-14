//! The classifier sharing site end to end, through the router: sharing from a workspace
//! save, review, the grid and the API, downloads, taking into another save where it
//! predicts, and the doors that must stay shut (other sites, other people, bad files,
//! odd ids).

use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::app::{self, Workspaces};
use crate::auth::{self, Limiter};
use crate::hub::{Config, Hub};
use crate::store::{Role, Standing, Store, User};
use crate::web;

const BASE: &str = "http://127.0.0.1:5800";

/// the hub every router test shares (it's a global, set once)
fn setup() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(start).clone()
}

fn start() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("anend-hubhttp-{}", rand::random::<u64>()));
    let hub = Hub {
        store: Store::open(&dir).unwrap(),
        config: Config {
            name: "test hub".into(),
            description: String::new(),
            url: BASE.into(),
            open: false,
            max_upload: 8 << 20,
        },
        limiter: Limiter::new(true),
    };
    assert!(web::HUB.set(hub).is_ok(), "one test sets the hub");
    app::WORKSPACES.set(Workspaces::new(dir.join("users"))).ok();
    dir
}

fn person(name: &str, role: Role) -> String {
    web::hub()
        .store
        .add_user(User {
            name: name.into(),
            role,
            standing: Standing::Active,
            ..Default::default()
        })
        .unwrap();
    format!(
        "{}={}",
        auth::COOKIE,
        auth::start_session(&web::hub().store, name).unwrap()
    )
}

async fn body(mut r: Response) -> (u16, String) {
    let code = r.status_code.map(|c| c.as_u16()).unwrap_or(200);
    (code, r.take_string().await.unwrap_or_default())
}

async fn get(path: &str, cookie: Option<&str>) -> (u16, String) {
    let mut b = TestClient::get(format!("{BASE}{path}"));
    if let Some(c) = cookie {
        b = b.add_header("cookie", c, true);
    }
    body(b.send(crate::router()).await).await
}

async fn post_json(path: &str, cookie: &str, v: Value, origin: Option<&str>) -> (u16, Value) {
    let mut b = TestClient::post(format!("{BASE}{path}")).add_header("cookie", cookie, true);
    if let Some(o) = origin {
        b = b.add_header("origin", o, true);
    }
    let (code, text) = body(b.json(&v).send(crate::router()).await).await;
    (code, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

async fn post_form(path: &str, cookie: &str, form: &str) -> (u16, String) {
    let b = TestClient::post(format!("{BASE}{path}"))
        .add_header("cookie", cookie, true)
        .raw_form(form.to_string());
    body(b.send(crate::router()).await).await
}

fn multipart(fields: &[(&str, &str)], file: &[u8]) -> (String, Vec<u8>) {
    let boundary = "anendboundary7";
    let mut b = Vec::new();
    for (k, v) in fields {
        b.extend(format!("--{boundary}\r\ncontent-disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    b.extend(
        format!(
            "--{boundary}\r\ncontent-disposition: form-data; name=\"file\"; filename=\"x.anendcls\"\r\ncontent-type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    b.extend(file);
    b.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), b)
}

#[tokio::test]
async fn classifier_site() {
    let dir = setup();
    let saul = person("saul", Role::Member);
    let ann = person("ann", Role::Member);
    let root = person("root", Role::Admin);
    // saul's workspace has a trained classifier (made, then let go so the hub opens it)
    drop(crate::clshub::tests::trained(
        &dir.join("users").join("saul").join("saves"),
    ));

    // another site can't make saul share
    let (code, _) = post_json(
        "/app/classifiers/share",
        &saul,
        json!({ "name": "kind" }),
        Some("http://evil.example"),
    )
    .await;
    assert_eq!(code, 403);
    assert!(web::hub().store.classifiers().unwrap().is_empty());
    // nor can nobody
    let (code, v) = post_json("/app/classifiers/share", "", json!({ "name": "kind" }), None).await;
    assert_eq!(code, 400, "{v}");
    let (code, v) = post_json("/app/classifiers/share", &saul, json!({ "name": "nope" }), None).await;
    assert_eq!(code, 400, "{v}");

    let (code, v) = post_json(
        "/app/classifiers/share",
        &saul,
        json!({ "name": "kind", "title": "fruit or rock", "description": "tells them apart", "tags": "nature" }),
        None,
    )
    .await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["data"]["status"], "pending", "{v}");
    let id = v["data"]["id"].as_str().unwrap().to_string();

    // waiting: not in the grid, not for others to see, nor to take
    let (_, list) = get("/api/classifiers", None).await;
    assert_eq!(serde_json::from_str::<Value>(&list).unwrap()["classifiers"], json!([]));
    assert_eq!(get(&format!("/classifiers/{id}"), None).await.0, 404);
    assert_eq!(get(&format!("/classifiers/{id}"), Some(&saul)).await.0, 200);
    let (code, _) = post_json(&format!("/app/classifiers/{id}/take"), &ann, json!({}), None).await;
    assert_eq!(code, 400);
    // the admin panel shows it waiting
    let (code, page) = get("/admin", Some(&root)).await;
    assert_eq!(code, 200);
    assert!(
        page.contains("classifiers waiting (1)") && page.contains("fruit or rock"),
        "{page}"
    );
    // only an admin's verdict counts
    post_form(&format!("/admin/classifiers/{id}"), &ann, "action=approve").await;
    assert_eq!(get(&format!("/classifiers/{id}"), None).await.0, 404);
    post_form(&format!("/admin/classifiers/{id}"), &root, "action=approve").await;

    let (_, list) = get("/api/classifiers?q=rock&sort=stars", None).await;
    let list: Value = serde_json::from_str(&list).unwrap();
    assert_eq!(list["classifiers"][0]["id"], id.as_str(), "{list}");
    assert_eq!(list["classifiers"][0]["classes"], json!(["fruit", "rocks"]));
    let (code, one) = get(&format!("/api/classifiers/{id}"), None).await;
    assert_eq!(code, 200);
    assert_eq!(serde_json::from_str::<Value>(&one).unwrap()["detail"]["module"], "kind");
    let (code, page) = get("/classifiers?q=%23nature", None).await;
    assert!(code == 200 && page.contains("fruit or rock"), "{page}");
    let (code, page) = get(&format!("/classifiers/{id}"), Some(&ann)).await;
    assert!(code == 200 && page.contains("take into my save") && page.contains("rocks"));
    assert_eq!(get("/classifiers/upload", Some(&ann)).await.0, 200);

    let mut r = TestClient::get(format!("{BASE}/classifiers/{id}/download"))
        .send(crate::router())
        .await;
    let bytes = r.take_bytes(None).await.unwrap();
    assert!(bytes.starts_with(b"ANENDCLM"));
    assert_eq!(web::hub().store.classifier(&id).unwrap().unwrap().downloads, 1);

    // ann takes it into her save, where it predicts
    let (code, v) = post_json(&format!("/app/classifiers/{id}/take"), &ann, json!({}), None).await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["data"]["name"], "kind");
    let ws = app::workspaces().get("ann");
    let p = app::call(
        &ws,
        "classify.predict",
        &json!({ "name": "kind", "texts": ["granite slate flint", "plum peach apple"] }),
    )
    .unwrap();
    assert_eq!(p["results"][0]["class"], "rocks", "{p}");
    assert_eq!(p["results"][1]["class"], "fruit", "{p}");
    let (code, _) = post_json(
        &format!("/app/classifiers/{id}/take"),
        &ann,
        json!({}),
        Some("http://evil.example"),
    )
    .await;
    assert_eq!(code, 403);

    // stars: ann may, saul (its author) may not
    post_form(&format!("/classifiers/{id}/star"), &ann, "").await;
    post_form(&format!("/classifiers/{id}/star"), &saul, "").await;
    assert_eq!(web::hub().store.classifier(&id).unwrap().unwrap().stars, 1);

    // ann can't change or delete saul's
    assert_eq!(get(&format!("/classifiers/{id}/edit"), Some(&ann)).await.0, 404);
    let (code, _) = post_form(&format!("/classifiers/{id}/delete"), &ann, "").await;
    assert_eq!(code, 404);
    let (ct, b) = multipart(&[("name", "mine now")], b"");
    let r = TestClient::post(format!("{BASE}/classifiers/{id}/edit"))
        .add_header("cookie", ann.as_str(), true)
        .add_header("content-type", ct.as_str(), true)
        .bytes(b)
        .send(crate::router())
        .await;
    assert_eq!(body(r).await.0, 404);
    assert_eq!(web::hub().store.classifier(&id).unwrap().unwrap().name, "fruit or rock");
    // saul can
    let (ct, b) = multipart(&[("name", "fruit or stone"), ("tags", "nature")], b"");
    let r = TestClient::post(format!("{BASE}/classifiers/{id}/edit"))
        .add_header("cookie", saul.as_str(), true)
        .add_header("content-type", ct.as_str(), true)
        .bytes(b)
        .send(crate::router())
        .await;
    assert_eq!(body(r).await.0, 303);
    assert_eq!(
        web::hub().store.classifier(&id).unwrap().unwrap().name,
        "fruit or stone"
    );

    // the API takes uploads with a token only, and refuses bad files
    let token = auth::new_token(&web::hub().store, "ann", "test").unwrap();
    let mut bad = bytes.to_vec();
    let n = bad.len();
    bad[n / 2] ^= 0x40;
    for (file, auth, want) in [
        (bytes.to_vec(), None, 401),
        (bad.clone(), Some(token.as_str()), 400),
        (b"ANENDCLM junk".to_vec(), Some(token.as_str()), 400),
        (bytes.to_vec(), Some("anend_wrong"), 401),
    ] {
        let (ct, b) = multipart(&[("name", "copy")], &file);
        let mut q = TestClient::post(format!("{BASE}/api/classifiers"))
            .add_header("cookie", ann.as_str(), true)
            .add_header("content-type", ct.as_str(), true);
        if let Some(t) = auth {
            q = q.bearer_auth(t);
        }
        let (code, text) = body(q.bytes(b).send(crate::router()).await).await;
        assert_eq!(code, want, "{text}");
    }
    let (ct, b) = multipart(&[("name", "ann's copy")], &bytes);
    let r = TestClient::post(format!("{BASE}/api/classifiers"))
        .bearer_auth(token)
        .add_header("content-type", ct.as_str(), true)
        .bytes(b)
        .send(crate::router())
        .await;
    let (code, text) = body(r).await;
    assert_eq!(code, 200, "{text}");
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["status"], "pending");

    // ids that aren't ids
    for bad in ["..%2F..%2Fhub", "..x", "ABC", "x%00y"] {
        assert_eq!(get(&format!("/classifiers/{bad}"), Some(&root)).await.0, 404, "{bad}");
        assert_eq!(get(&format!("/api/classifiers/{bad}"), None).await.0, 404, "{bad}");
        assert_eq!(
            get(&format!("/classifiers/{bad}/download"), Some(&root)).await.0,
            404,
            "{bad}"
        );
    }

    // saul deletes his; it's gone, file and all
    post_form(&format!("/classifiers/{id}/delete"), &saul, "").await;
    assert!(web::hub().store.classifier(&id).unwrap().is_none());
    assert!(!web::hub().store.classifier_path(&id).unwrap().exists());
}

/// the doors around the app and the API: who may post what, from where, and what every
/// answer says about framing
#[tokio::test]
async fn doors() {
    let dir = setup();
    let bea = person("bea", Role::Member);

    // sharing a save through the API takes a token: the login cookie alone, which another
    // site's form would carry, is turned away
    let save = dir.join("bea-share.redb");
    crate::inspect::tests::make_save(&save, &["truth", "listen"]);
    let bytes = std::fs::read(&save).unwrap();
    let send = |auth: Option<String>| {
        let (ct, b) = multipart(&[("name", "bea's save")], &bytes);
        let mut q = TestClient::post(format!("{BASE}/api/saves"))
            .add_header("cookie", bea.as_str(), true)
            .add_header("content-type", ct.as_str(), true);
        if let Some(t) = auth {
            q = q.bearer_auth(t);
        }
        q.bytes(b)
    };
    let (code, text) = body(send(None).send(crate::router()).await).await;
    assert_eq!(code, 401, "{text}");
    assert!(!web::hub().store.saves().unwrap().iter().any(|s| s.author == "bea"));
    let token = auth::new_token(&web::hub().store, "bea", "test").unwrap();
    let (code, text) = body(send(Some(token)).send(crate::router()).await).await;
    assert_eq!(code, 200, "{text}");
    assert!(web::hub().store.saves().unwrap().iter().any(|s| s.author == "bea"));

    // no one gets the app's calls or a socket
    let (code, v) = post_json("/app/api/hello", "", json!({}), None).await;
    assert_eq!(code, 400);
    assert!(v["error"].as_str().unwrap().contains("log in"), "{v}");
    let r = TestClient::get(format!("{BASE}/app/ws"))
        .add_header("connection", "upgrade", true)
        .add_header("upgrade", "websocket", true)
        .add_header("sec-websocket-version", "13", true)
        .add_header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==", true)
        .send(crate::router())
        .await;
    assert_eq!(body(r).await.0, 401);
    // bea does, from the hub's own page only
    let (code, v) = post_json("/app/api/hello", &bea, json!({}), None).await;
    assert_eq!(code, 200, "{v}");
    let (code, _) = post_json("/app/api/hello", &bea, json!({}), Some("http://evil.example")).await;
    assert_eq!(code, 403);

    // nothing is framed by another site; the app's page runs only the hub's scripts
    let r = TestClient::get(format!("{BASE}/")).send(crate::router()).await;
    assert_eq!(r.headers().get("x-frame-options").unwrap(), "DENY");
    assert!(
        r.headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    let r = TestClient::get(format!("{BASE}/app/")).send(crate::router()).await;
    assert_eq!(r.headers().get("content-security-policy").unwrap(), web::APP_CSP);
    assert_eq!(r.headers().get("x-content-type-options").unwrap(), "nosniff");
    // the app's page has no inline script for that policy to stop
    let (_, page) = body(r).await;
    assert!(!page.contains("<script>"), "an inline script in index.html");
}
