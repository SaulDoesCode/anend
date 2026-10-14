//! The server's own settings: the https certificate and key. Set with --tls-cert and
//! --tls-key, or on /admin/server, which only answers an admin on the machine the hub
//! runs on (it names files on that machine). Saved in server.json in the data folder;
//! the hub reads it when it starts.

use askama::Template;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::web::{Layout, go, layout, need_admin, render};

pub static DATA: OnceLock<PathBuf> = OnceLock::new();

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerFile {
    pub tls_cert: String,
    pub tls_key: String,
}

impl ServerFile {
    fn path(data: &Path) -> PathBuf {
        data.join("server.json")
    }

    pub fn load(data: &Path) -> ServerFile {
        std::fs::read(Self::path(data))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, data: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(Self::path(data), bytes).map_err(|e| e.to_string())
    }

    pub fn cert(&self) -> Option<PathBuf> {
        (!self.tls_cert.trim().is_empty()).then(|| PathBuf::from(self.tls_cert.trim()))
    }

    pub fn key(&self) -> Option<PathBuf> {
        (!self.tls_key.trim().is_empty()).then(|| PathBuf::from(self.tls_key.trim()))
    }
}

#[derive(Template)]
#[template(path = "server.html")]
struct Page {
    l: Layout,
    cert: String,
    key: String,
}

/// a browser on the hub's own machine: the connection comes from loopback and no proxy
/// passed it on (a reverse proxy on the same machine makes every request look local, so
/// anything carrying a forwarding header isn't, whatever --behind-proxy says)
fn local(req: &Request) -> bool {
    is_local(req.remote_addr().ip(), req.headers())
}

fn is_local(from: Option<std::net::IpAddr>, headers: &salvo::http::HeaderMap) -> bool {
    let forwarded = ["x-forwarded-for", "forwarded", "x-real-ip"]
        .iter()
        .any(|h| headers.contains_key(*h));
    !forwarded && from.is_some_and(|ip| ip.is_loopback())
}

#[handler]
pub async fn page(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if need_admin(res, depot).is_none() {
        return;
    }
    if !local(req) {
        return crate::web::message(
            res,
            depot,
            StatusCode::FORBIDDEN,
            "only from the server itself",
            "The server's settings name files on its machine, so they're only shown to an admin browsing from that machine (localhost).",
        );
    }
    let f = ServerFile::load(DATA.get().expect("data folder"));
    render(
        res,
        Page {
            l: layout(depot, "admin"),
            cert: f.tls_cert,
            key: f.tls_key,
        },
    );
}

/// a PEM file that holds what it should
fn check(path: &str, what: &str, marker: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("can't read the {what} at {path}: {e}"))?;
    if text.contains(marker) {
        Ok(())
    } else {
        Err(format!("{path} doesn't look like a PEM {what}"))
    }
}

#[handler]
pub async fn save(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    if need_admin(res, depot).is_none() {
        return;
    }
    if !local(req) {
        return go(
            res,
            "/admin",
            "!the server's settings only change from the server itself",
        );
    }
    let cert = req.form::<String>("cert").await.unwrap_or_default().trim().to_string();
    let key = req.form::<String>("key").await.unwrap_or_default().trim().to_string();
    if !cert.is_empty() || !key.is_empty() {
        let ok = check(&cert, "certificate", "BEGIN CERTIFICATE").and_then(|_| check(&key, "key", "PRIVATE KEY"));
        if let Err(e) = ok {
            return go(res, "/admin/server", &format!("!{e}"));
        }
    }
    let f = ServerFile {
        tls_cert: cert.clone(),
        tls_key: key,
    };
    match f.save(DATA.get().expect("data folder")) {
        Ok(()) if cert.is_empty() => go(res, "/admin/server", "saved: plain http after the next restart"),
        Ok(()) => go(res, "/admin/server", "saved: restart the hub to serve https"),
        Err(e) => go(res, "/admin/server", &format!("!{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trip() {
        let dir = std::env::temp_dir().join(format!("anend-server-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(ServerFile::load(&dir).cert().is_none());
        ServerFile {
            tls_cert: "/a.pem".into(),
            tls_key: " ".into(),
        }
        .save(&dir)
        .unwrap();
        let f = ServerFile::load(&dir);
        assert_eq!(f.cert(), Some(PathBuf::from("/a.pem")));
        assert!(f.key().is_none());
        assert!(check("/nonexistent", "key", "PRIVATE KEY").is_err());
        // only a browser on this machine, not one a proxy here passed on
        let mut h = salvo::http::HeaderMap::new();
        let lo = Some("127.0.0.1".parse().unwrap());
        assert!(is_local(lo, &h) && is_local(Some("::1".parse().unwrap()), &h));
        assert!(!is_local(Some("10.0.0.2".parse().unwrap()), &h) && !is_local(None, &h));
        h.insert("x-forwarded-for", "127.0.0.1".parse().unwrap());
        assert!(!is_local(lo, &h));
        std::fs::remove_dir_all(dir).ok();
    }
}
