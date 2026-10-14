//! anend-hub: a self-hosted place to share anend saves and classifiers. People join (an admin lets them
//! in), share saves, star and download each other's, and earn points, levels and badges
//! for saves others use. The app browses and shares through its JSON API.

mod api;
mod app;
mod appweb;
mod auth;
mod classify;
mod clshub;
mod clsweb;
#[cfg(test)]
mod clstests;
mod game;
mod hub;
mod inspect;
mod server;
mod store;
mod web;

use salvo::prelude::*;
use std::path::PathBuf;

use crate::auth::Limiter;
use crate::hub::{Config, Hub};
use crate::store::Store;

const HELP: &str = "anend-hub: share anend saves and classifiers

usage: anend-hub [options]

  --addr HOST:PORT     where to listen (default 127.0.0.1:8080)
  --url URL            the address people reach the hub at, for links (default http://ADDR)
  --data DIR           where the hub keeps its database and saves (default ./hub-data)
  --name NAME          what the hub calls itself (default \"anend hub\")
  --about TEXT         a line about the hub for its front page
  --open               new accounts are active at once (default: an admin lets them in)
  --max-upload MB      largest save taken, in megabytes (default 64)
  --behind-proxy       trust X-Forwarded-For for the client's address
  --admin NAME         make NAME an admin (creating them if needed), print a login link, carry on
  --no-ratelimit       don't limit login, join and upload tries
  --local              one workspace for the web app and no login needed for it: the hub
                       as the app's daemon on your own machine (keep --addr on localhost)
  --tls-cert FILE      serve https with this certificate chain (PEM)...
  --tls-key FILE       ...and this private key (PEM); both can be set in the admin panel too
  -h, --help           this
";

struct Args {
    addr: String,
    url: Option<String>,
    data: PathBuf,
    name: String,
    about: String,
    open: bool,
    max_upload: u64,
    behind_proxy: bool,
    admin: Option<String>,
    no_ratelimit: bool,
    local: bool,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        addr: "127.0.0.1:8080".into(),
        url: None,
        data: PathBuf::from("hub-data"),
        name: "anend hub".into(),
        about:
            "Saves from the anend notes app: memories, notes and their gematria, shared by the people who made them."
                .into(),
        open: false,
        max_upload: 64,
        behind_proxy: false,
        admin: None,
        no_ratelimit: false,
        local: false,
        tls_cert: None,
        tls_key: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |what: &str| it.next().ok_or(format!("{what} needs a value"));
        match arg.as_str() {
            "--addr" => a.addr = value("--addr")?,
            "--url" => a.url = Some(value("--url")?),
            "--data" => a.data = PathBuf::from(value("--data")?),
            "--name" => a.name = value("--name")?,
            "--about" => a.about = value("--about")?,
            "--open" => a.open = true,
            "--max-upload" => {
                a.max_upload = value("--max-upload")?
                    .parse()
                    .map_err(|_| "--max-upload is a number of megabytes")?
            }
            "--behind-proxy" => a.behind_proxy = true,
            "--admin" => a.admin = Some(value("--admin")?),
            "--no-ratelimit" => a.no_ratelimit = true,
            "--local" => a.local = true,
            "--tls-cert" => a.tls_cert = Some(PathBuf::from(value("--tls-cert")?)),
            "--tls-key" => a.tls_key = Some(PathBuf::from(value("--tls-key")?)),
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}\n\n{HELP}")),
        }
    }
    Ok(a)
}

pub fn router() -> Router {
    Router::new()
        .hoop(web::who_is_asking)
        .get(web::index)
        .push(Router::with_path("static/hub.css").get(web::css))
        .push(Router::with_path("static/favicon.svg").get(web::favicon))
        .push(Router::with_path("favicon.ico").get(web::favicon))
        .push(Router::with_path("saves/{id}").get(web::save_page))
        .push(Router::with_path("saves/{id}/download").get(web::download))
        .push(Router::with_path("saves/{id}/star").post(web::star))
        .push(Router::with_path("saves/{id}/edit").get(web::edit_page).post(web::edit))
        .push(Router::with_path("saves/{id}/delete").post(web::delete))
        .push(Router::with_path("upload").get(web::upload_page).post(web::upload))
        .push(Router::with_path("classifiers").get(clsweb::grid))
        .push(
            Router::with_path("classifiers/upload")
                .get(clsweb::upload_page)
                .post(clsweb::upload),
        )
        .push(Router::with_path("classifiers/{id}").get(clsweb::detail))
        .push(Router::with_path("classifiers/{id}/download").get(clsweb::download))
        .push(Router::with_path("classifiers/{id}/star").post(clsweb::star))
        .push(
            Router::with_path("classifiers/{id}/edit")
                .get(clsweb::edit_page)
                .post(clsweb::edit),
        )
        .push(Router::with_path("classifiers/{id}/delete").post(clsweb::delete))
        .push(Router::with_path("join").get(web::join_page).post(web::join))
        .push(Router::with_path("login").get(web::login_page).post(web::login))
        .push(Router::with_path("login/{secret}").get(web::login_link))
        .push(Router::with_path("logout").post(web::logout))
        .push(Router::with_path("u/{name}").get(web::profile))
        .push(Router::with_path("me").post(web::update_me))
        .push(Router::with_path("me/tokens").post(web::make_token))
        .push(Router::with_path("me/tokens/{hash}/revoke").post(web::revoke_token))
        .push(Router::with_path("leaderboard").get(web::leaderboard))
        .push(Router::with_path("app").get(appweb::app_root))
        .push(Router::with_path("app/ws").goal(appweb::socket))
        .push(Router::with_path("app/api/{op}").post(appweb::api))
        .push(Router::with_path("app/saves").post(appweb::upload))
        .push(Router::with_path("app/saves/{name}").get(appweb::download))
        .push(Router::with_path("app/hub/{id}").post(appweb::take))
        .push(Router::with_path("app/share").post(appweb::share))
        .push(Router::with_path("app/classifiers/share").post(appweb::share_classifier))
        .push(Router::with_path("app/classifiers/{id}/take").post(appweb::take_classifier))
        .push(Router::with_path("app/{**rest}").get(appweb::files))
        .push(Router::with_path("admin").get(web::admin_page))
        .push(Router::with_path("admin/saves/{id}").post(web::admin_save_action))
        .push(Router::with_path("admin/users/{name}").post(web::admin_user_action))
        .push(Router::with_path("admin/classifiers/{id}").post(clsweb::admin_action))
        .push(Router::with_path("admin/server").get(server::page).post(server::save))
        .push(Router::with_path("api/hub").get(api::about))
        .push(Router::with_path("api/saves").get(api::list).post(api::share))
        .push(Router::with_path("api/saves/{id}").get(api::one))
        .push(Router::with_path("api/me").get(api::whoami))
        .push(
            Router::with_path("api/classifiers")
                .get(api::classifiers)
                .post(api::share_classifier),
        )
        .push(Router::with_path("api/classifiers/{id}").get(api::classifier))
        .push(Router::with_path("api/classifiers/{id}/download").get(clsweb::download))
        .push(Router::with_path("{**rest}").goal(web::missing))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let store = match Store::open(&args.data) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("couldn't open the hub's data in {}: {e}", args.data.display());
            std::process::exit(1);
        }
    };
    // uploads that were being checked when the hub stopped
    std::fs::remove_dir_all(store.incoming()).ok();
    // https: the flags, or what the admin panel saved
    let file = server::ServerFile::load(&args.data);
    let cert = args.tls_cert.clone().or_else(|| file.cert());
    let key = args.tls_key.clone().or_else(|| file.key());
    let tls = cert.zip(key);
    let scheme = if tls.is_some() { "https" } else { "http" };
    let url = args.url.clone().unwrap_or_else(|| format!("{scheme}://{}", args.addr));
    let hub = Hub {
        store,
        config: Config {
            name: args.name,
            description: args.about,
            url: url.clone(),
            open: args.open,
            max_upload: args.max_upload.max(1) * 1024 * 1024,
        },
        limiter: Limiter::new(args.no_ratelimit),
    };
    web::BEHIND_PROXY.set(args.behind_proxy).ok();
    app::LOCAL.set(args.local).ok();
    app::WORKSPACES.set(app::Workspaces::new(args.data.join("users"))).ok();
    server::DATA.set(args.data.clone()).ok();
    if web::HUB.set(hub).is_err() {
        unreachable!("the hub starts once");
    }
    if let Some(name) = args.admin {
        match web::hub().make_admin(&name) {
            Ok(link) => {
                println!("\n  {name} is an admin. Log in with this link (once, within an hour):\n\n    {link}\n")
            }
            Err(e) => {
                eprintln!("couldn't make {name} an admin: {e}");
                std::process::exit(1);
            }
        }
    }
    let users = web::hub().store.users().map(|u| u.len()).unwrap_or(0);
    if users == 0 {
        println!("\n  No one has joined yet. Run with --admin YOURNAME to make yourself the first admin.\n");
    }
    println!(
        "  the hub is at {url} (data in {}); the app at {url}/app/",
        args.data.display()
    );
    if args.local {
        println!("  --local: the web app needs no login; keep the hub on localhost");
    }
    match tls {
        Some((cert, key)) => {
            let keycert = salvo::conn::rustls::Keycert::new()
                .cert_from_path(&cert)
                .and_then(|k| k.key_from_path(&key));
            let keycert = match keycert {
                Ok(k) => k,
                Err(e) => {
                    eprintln!(
                        "couldn't read the certificate or key ({}, {}): {e}",
                        cert.display(),
                        key.display()
                    );
                    std::process::exit(1);
                }
            };
            let config = salvo::conn::rustls::RustlsConfig::new(keycert);
            let acceptor = TcpListener::new(args.addr.clone()).rustls(config).bind().await;
            Server::new(acceptor).serve(router()).await;
        }
        None => {
            let acceptor = TcpListener::new(args.addr.clone()).bind().await;
            Server::new(acceptor).serve(router()).await;
        }
    }
}
