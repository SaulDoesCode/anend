//! Running the server: `/health` for load balancers and monitors, a log line for every
//! request (as text or JSON, `--log json`), the error pages for answers that have no
//! body of their own, and stopping gracefully on SIGTERM or Ctrl-C.

use salvo::prelude::*;
use serde_json::json;
use std::time::Instant;

use crate::web::{hub, message, mode};

/// starts the log: plain lines, or one JSON object a line for a log collector
pub fn start_log(json: bool) {
    let f = tracing_subscriber::fmt().with_target(false);
    if json {
        f.json().flatten_event(true).init();
    } else {
        f.init();
    }
}

/// a path fit for the log: the secrets in login links and invite links left out
pub fn loggable(path: &str) -> String {
    for p in ["/login/", "/invite/"] {
        if let Some(rest) = path.strip_prefix(p)
            && !rest.is_empty()
        {
            return format!("{p}…");
        }
    }
    path.chars().take(200).collect()
}

/// logs each request when its answer is ready: method, path (no query, no secrets),
/// status and time. No addresses or names are logged.
#[handler]
pub async fn log_requests(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = loggable(req.uri().path());
    ctrl.call_next(req, depot, res).await;
    let status = res.status_code.map(|s| s.as_u16()).unwrap_or(200);
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    if status >= 500 {
        tracing::warn!(method, path, status, ms, "request");
    } else {
        tracing::info!(method, path, status, ms, "request");
    }
}

/// `GET /health`: 200 `{"status": "ok", ...}` when the database answers, 503 when not
#[handler]
pub async fn health(depot: &mut Depot, res: &mut Response) {
    let db = hub().store.meta("join").is_ok();
    res.status_code(if db {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    });
    res.render(Json(json!({
        "status": if db { "ok" } else { "database unavailable" },
        "mode": mode(depot).name(),
        "version": env!("CARGO_PKG_VERSION"),
    })));
}

/// an error answer with nothing in it (a refused socket, a method not allowed, a panic)
/// gets the hub's own page, or a JSON error for the APIs
#[handler]
pub async fn errors(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
    let Some(code) = res.status_code.filter(|c| c.is_client_error() || c.is_server_error()) else {
        return;
    };
    if !(res.body.is_none() || res.body.is_error()) {
        return;
    }
    let what = match code.as_u16() {
        401 => "log in first",
        403 => "that isn't open to you",
        404 => "there's nothing at this address",
        405 => "that address doesn't take that kind of request",
        413 => "that's too big",
        429 => "too many tries: wait a little",
        500..=599 => "something went wrong on the server; try again in a moment",
        _ => "that didn't work",
    };
    let path = req.uri().path();
    if path.starts_with("/api/") || path.starts_with("/app/") || path == "/health" {
        res.render(Json(json!({ "ok": false, "error": what })));
    } else {
        let heading = match code.as_u16() {
            404 => "not here",
            500..=599 => "a fault",
            _ => "can't do that",
        };
        let text = format!("{}{}.", what[..1].to_uppercase(), &what[1..]);
        message(res, depot, code, heading, &text);
    }
    ctrl.skip_rest();
}

/// waits for SIGTERM or Ctrl-C, then lets the requests in flight finish (up to 20 s)
pub async fn stop_on_signal(handle: salvo::server::ServerHandle) {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("stopping: letting requests in flight finish");
    handle.stop_graceful(std::time::Duration::from_secs(20));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_stay_out_of_the_log() {
        assert_eq!(loggable("/login/abcdef"), "/login/…");
        assert_eq!(loggable("/invite/k3x9-q2mf-7hpd"), "/invite/…");
        assert_eq!(loggable("/login"), "/login");
        assert_eq!(loggable("/saves/x1"), "/saves/x1");
    }
}
