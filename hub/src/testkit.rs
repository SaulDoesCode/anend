//! What the router tests share: one hub for the whole test run (it's a global, set
//! once), people with sessions, and requests through a router built for a mode. Not
//! every build's tests use every helper.
#![allow(dead_code)]

use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::Value;

use crate::auth::{self, Limiter};
use crate::hub::{Config, Hub};
use crate::mode::Mode;
use crate::store::{Role, Standing, Store, User};
use crate::web;

pub const BASE: &str = "http://127.0.0.1:5800";

/// the hub every router test shares (it's a global, set once)
pub fn setup() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(start).clone()
}

pub fn start() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("anend-hubhttp-{}", rand::random::<u64>()));
    let hub = Hub {
        store: Store::open(&dir).unwrap(),
        config: Config {
            name: "test hub".into(),
            url: BASE.into(),
            max_upload: 8 << 20,
            ..Default::default()
        },
        limiter: Limiter::new(true),
    };
    assert!(web::HUB.set(hub).is_ok(), "one test sets the hub");
    #[cfg(feature = "app")]
    crate::app::WORKSPACES
        .set(crate::app::Workspaces::new(dir.join("users")))
        .ok();
    dir
}

/// an active member with a session; returns their cookie
pub fn person(name: &str, role: Role) -> String {
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

pub async fn body(mut r: Response) -> (u16, String) {
    let code = r.status_code.map(|c| c.as_u16()).unwrap_or(200);
    (code, r.take_string().await.unwrap_or_default())
}

pub async fn get(path: &str, cookie: Option<&str>) -> (u16, String) {
    let mut b = TestClient::get(format!("{BASE}{path}"));
    if let Some(c) = cookie {
        b = b.add_header("cookie", c, true);
    }
    body(b.send(&service(Mode::BOTH)).await).await
}

pub async fn post_json(path: &str, cookie: &str, v: Value, origin: Option<&str>) -> (u16, Value) {
    let mut b = TestClient::post(format!("{BASE}{path}")).add_header("cookie", cookie, true);
    if let Some(o) = origin {
        b = b.add_header("origin", o, true);
    }
    let (code, text) = body(b.json(&v).send(&service(Mode::BOTH)).await).await;
    (code, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

pub async fn post_form(path: &str, cookie: &str, form: &str) -> (u16, String) {
    let b = TestClient::post(format!("{BASE}{path}"))
        .add_header("cookie", cookie, true)
        .raw_form(form.to_string());
    body(b.send(&service(Mode::BOTH)).await).await
}

pub fn multipart(fields: &[(&str, &str)], file: &[u8]) -> (String, Vec<u8>) {
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


/// the server as main builds it, for a mode
pub fn service(mode: Mode) -> Service {
    crate::service(mode)
}

/// the server in both modes
pub fn both() -> Service {
    service(Mode::BOTH)
}

/// a GET in a mode
pub async fn get_in(mode: Mode, path: &str, cookie: Option<&str>) -> (u16, String) {
    let mut b = TestClient::get(format!("{BASE}{path}"));
    if let Some(c) = cookie {
        b = b.add_header("cookie", c, true);
    }
    body(b.send(&service(mode)).await).await
}

/// a form POST in a mode, without a cookie when it's empty
pub async fn post_form_in(mode: Mode, path: &str, cookie: &str, form: &str) -> (u16, String) {
    let mut b = TestClient::post(format!("{BASE}{path}")).raw_form(form.to_string());
    if !cookie.is_empty() {
        b = b.add_header("cookie", cookie, true);
    }
    body(b.send(&service(mode)).await).await
}

/// the cookie a response set, as `name=value`
pub fn cookie_of(r: &Response, name: &str) -> Option<String> {
    r.cookies().get(name).map(|c| format!("{}={}", name, c.value()))
}
