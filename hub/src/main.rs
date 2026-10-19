//! anend-hub: a self-hosted server for the anend notes app. As a hub it's a place to
//! share saves and classifiers: people join (by invitation, application or freely), share
//! saves, star and download each other's, and earn points, levels and badges for saves
//! others use; the app browses and shares through its JSON API. As the app's server it
//! runs the web app on each member's own saves. `--mode` picks either or both (mode.rs).

#[cfg(not(any(feature = "hub", feature = "app")))]
compile_error!("anend-hub needs the `hub` feature, the `app` feature, or both");

#[cfg(feature = "hub")]
mod api;
#[cfg(feature = "app")]
mod app;
#[cfg(feature = "app")]
mod appweb;
mod auth;
#[cfg(feature = "hub")]
mod clshub;
#[cfg(all(test, feature = "hub", feature = "app"))]
mod clstests;
#[cfg(feature = "hub")]
mod clsweb;
mod game;
mod guide;
mod hub;
mod inspect;
mod invites;
mod mode;
#[cfg(test)]
mod modetests;
mod ops;
mod pages;
#[cfg(feature = "hub")]
mod saves;
mod server;
mod store;
#[cfg(test)]
mod testkit;
mod web;

use salvo::prelude::*;
use std::path::PathBuf;

use crate::auth::Limiter;
use crate::hub::{Config, Hub, Join};
use crate::mode::Mode;
use crate::store::Store;

const HELP: &str = "anend-hub: the anend server: a hub to share saves and classifiers, the web app, or both

usage: anend-hub [options]

what it runs
  --mode MODE          hub (the sharing site and its API), app (the web app and its API)
                       or both (the default); one account system either way
  --local              the web app alone for one person, no login: a daemon on your own
                       machine (keep --addr on localhost)

where
  --addr HOST:PORT     where to listen (default 127.0.0.1:8080)
  --url URL            the address people reach it at, for links (default http://ADDR)
  --data DIR           where it keeps its database and files (default ./hub-data)
  --behind-proxy       trust X-Forwarded-For for the client's address
  --tls-cert FILE      serve https with this certificate chain (PEM)...
  --tls-key FILE       ...and this private key (PEM); both can be set in the admin panel too

who gets in
  --join POLICY        invite (only with an invite code), apply (an admin lets people in)
                       or open; kept in the database and changeable in the admin panel.
                       A new server starts invite-only.
  --open               the same as --join open
  --admin NAME         make NAME an admin (creating them if needed), print a login link, carry on
  --invite             print a one-use invite link (good for 7 days), carry on (needs an admin)
  --no-ratelimit       don't limit login, join, invite and upload tries

limits
  --max-upload MB      largest save or classifier taken (default 64)
  --share-quota MB     how much each member may keep shared on the hub, private ones
                       included (default 512; 0 for no limit)
  --quota MB           how much disk each member's saves in the web app may fill
                       (default 1024; 0 for no limit; --local has none)
  --pretrained DIR     a folder of pretrained models (each a HuggingFace folder) that spec
                       models in the web app may start from; repeatable. Without one a
                       shared hub loads none (--local loads any folder)

the pages
  --name NAME          what the server calls itself (default \"anend hub\")
  --about TEXT         a line about it for its front page
  --contact TEXT       how to reach whoever runs it, for the about page
  --terms FILE         the terms of use, markdown (default DATA/terms.md)
  --privacy FILE       the privacy note, markdown (default DATA/privacy.md)
  --log FORMAT         text (default) or json, one object a line

  -h, --help           this
";

struct Args {
    mode: Mode,
    addr: String,
    url: Option<String>,
    data: PathBuf,
    name: String,
    about: Option<String>,
    contact: String,
    join: Option<Join>,
    invite: bool,
    max_upload: u64,
    share_quota: u64,
    #[cfg_attr(not(feature = "app"), allow(dead_code))]
    quota: u64,
    #[cfg_attr(not(feature = "app"), allow(dead_code))]
    pretrained: Vec<PathBuf>,
    behind_proxy: bool,
    admin: Option<String>,
    no_ratelimit: bool,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    terms: Option<PathBuf>,
    privacy: Option<PathBuf>,
    json_log: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        mode: Mode::built(),
        addr: "127.0.0.1:8080".into(),
        url: None,
        data: PathBuf::from("hub-data"),
        name: "anend hub".into(),
        about: None,
        contact: String::new(),
        join: None,
        invite: false,
        max_upload: 64,
        share_quota: 512,
        quota: 1024,
        pretrained: Vec::new(),
        behind_proxy: false,
        admin: None,
        no_ratelimit: false,
        tls_cert: None,
        tls_key: None,
        terms: None,
        privacy: None,
        json_log: false,
    };
    let (mut mode_set, mut local) = (false, false);
    let mb = |v: String, what: &str| v.parse::<u64>().map_err(|_| format!("{what} is a number of megabytes"));
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |what: &str| it.next().ok_or(format!("{what} needs a value"));
        match arg.as_str() {
            "--mode" => {
                a.mode = Mode::parse(&value("--mode")?)?;
                mode_set = true;
            }
            "--local" => local = true,
            "--addr" => a.addr = value("--addr")?,
            "--url" => a.url = Some(value("--url")?),
            "--data" => a.data = PathBuf::from(value("--data")?),
            "--name" => a.name = value("--name")?,
            "--about" => a.about = Some(value("--about")?),
            "--contact" => a.contact = value("--contact")?,
            "--join" => {
                let v = value("--join")?;
                a.join = Some(Join::parse(&v).ok_or(format!("--join is invite, apply or open (not {v})"))?);
            }
            "--open" => a.join = Some(Join::Open),
            "--invite" => a.invite = true,
            "--max-upload" => a.max_upload = mb(value("--max-upload")?, "--max-upload")?,
            "--share-quota" => a.share_quota = mb(value("--share-quota")?, "--share-quota")?,
            "--quota" => a.quota = mb(value("--quota")?, "--quota")?,
            "--pretrained" => a.pretrained.push(PathBuf::from(value("--pretrained")?)),
            "--behind-proxy" => a.behind_proxy = true,
            "--admin" => a.admin = Some(value("--admin")?),
            "--no-ratelimit" => a.no_ratelimit = true,
            "--tls-cert" => a.tls_cert = Some(PathBuf::from(value("--tls-cert")?)),
            "--tls-key" => a.tls_key = Some(PathBuf::from(value("--tls-key")?)),
            "--terms" => a.terms = Some(PathBuf::from(value("--terms")?)),
            "--privacy" => a.privacy = Some(PathBuf::from(value("--privacy")?)),
            "--log" => match value("--log")?.as_str() {
                "json" => a.json_log = true,
                "text" => a.json_log = false,
                v => return Err(format!("--log is text or json (not {v})")),
            },
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}\n\n{HELP}")),
        }
    }
    if local {
        if mode_set && a.mode != Mode::APP {
            return Err("--local is the web app alone: leave out --mode (or say --mode app)".into());
        }
        a.mode = Mode::LOCAL;
        a.mode.check()?;
    }
    Ok(a)
}

/// the server's routes for a mode: a route the mode doesn't serve doesn't exist
pub fn router(mode: Mode) -> Router {
    let mut r = Router::new()
        .hoop(ops::log_requests)
        .hoop(web::SetMode(mode))
        .hoop(web::who_is_asking);
    // every mode: the front page, files, the pages about the server, accounts, admin
    #[cfg(feature = "hub")]
    {
        r = if mode.hub {
            r.get(saves::index)
        } else {
            r.get(pages::landing)
        };
    }
    #[cfg(not(feature = "hub"))]
    {
        r = r.get(pages::landing);
    }
    r = r
        .push(Router::with_path("static/hub.css").get(web::css))
        .push(Router::with_path("static/icons.svg").get(web::icons))
        .push(Router::with_path("static/favicon.svg").get(web::favicon))
        .push(Router::with_path("favicon.ico").get(web::favicon))
        .push(Router::with_path("health").get(ops::health))
        .push(Router::with_path("about").get(pages::about))
        .push(Router::with_path("terms").get(pages::terms))
        .push(Router::with_path("privacy").get(pages::privacy))
        .push(Router::with_path("guide").get(guide::page))
        .push(Router::with_path("guide/{name}").get(guide::picture))
        .push(Router::with_path("join").get(web::join_page).post(web::join))
        .push(Router::with_path("invite/{code}").get(invites::follow))
        .push(Router::with_path("invites").get(invites::page).post(invites::make))
        .push(Router::with_path("invites/{id}/revoke").post(invites::revoke))
        .push(Router::with_path("login").get(web::login_page).post(web::login))
        .push(Router::with_path("login/{secret}").get(web::login_link))
        .push(Router::with_path("logout").post(web::logout))
        .push(Router::with_path("u/{name}").get(web::profile))
        .push(Router::with_path("me").post(web::update_me))
        .push(Router::with_path("me/tokens").post(web::make_token))
        .push(Router::with_path("me/tokens/{hash}/revoke").post(web::revoke_token))
        .push(Router::with_path("admin").get(web::admin_page))
        .push(Router::with_path("admin/users/{name}").post(web::admin_user_action))
        .push(Router::with_path("admin/settings").post(invites::settings))
        .push(Router::with_path("admin/server").get(server::page).post(server::save))
        .push(Router::with_path("api/me").get(web::whoami));
    #[cfg(feature = "hub")]
    if mode.hub {
        r = r
            .push(Router::with_path("saves/{id}").get(saves::save_page))
            .push(Router::with_path("saves/{id}/download").get(saves::download))
            .push(Router::with_path("saves/{id}/star").post(saves::star))
            .push(Router::with_path("saves/{id}/visibility").post(saves::visibility))
            .push(
                Router::with_path("saves/{id}/edit")
                    .get(saves::edit_page)
                    .post(saves::edit),
            )
            .push(Router::with_path("saves/{id}/delete").post(saves::delete))
            .push(Router::with_path("upload").get(saves::upload_page).post(saves::upload))
            .push(Router::with_path("leaderboard").get(saves::leaderboard))
            .push(Router::with_path("classifiers").get(clsweb::grid))
            .push(
                Router::with_path("classifiers/upload")
                    .get(clsweb::upload_page)
                    .post(clsweb::upload),
            )
            .push(Router::with_path("classifiers/{id}").get(clsweb::detail))
            .push(Router::with_path("classifiers/{id}/download").get(clsweb::download))
            .push(Router::with_path("classifiers/{id}/star").post(clsweb::star))
            .push(Router::with_path("classifiers/{id}/visibility").post(clsweb::visibility))
            .push(
                Router::with_path("classifiers/{id}/edit")
                    .get(clsweb::edit_page)
                    .post(clsweb::edit),
            )
            .push(Router::with_path("classifiers/{id}/delete").post(clsweb::delete))
            .push(Router::with_path("admin/saves/{id}").post(saves::admin_save_action))
            .push(Router::with_path("admin/classifiers/{id}").post(clsweb::admin_action))
            .push(Router::with_path("api/hub").get(api::about))
            .push(Router::with_path("api/saves").get(api::list).post(api::share))
            .push(Router::with_path("api/saves/{id}").get(api::one))
            .push(Router::with_path("api/mine").get(api::mine))
            .push(
                Router::with_path("api/classifiers")
                    .get(api::classifiers)
                    .post(api::share_classifier),
            )
            .push(Router::with_path("api/classifiers/{id}").get(api::classifier))
            .push(Router::with_path("api/classifiers/{id}/download").get(clsweb::download));
    }
    #[cfg(feature = "app")]
    if mode.app {
        // between the app and the hub: only when both are served
        #[cfg(feature = "hub")]
        if mode.hub {
            r = r
                .push(Router::with_path("app/hub/{id}").post(appweb::take))
                .push(Router::with_path("app/share").post(appweb::share))
                .push(Router::with_path("app/classifiers/share").post(appweb::share_classifier))
                .push(Router::with_path("app/classifiers/{id}/take").post(appweb::take_classifier));
        }
        r = r
            .push(Router::with_path("app").get(appweb::app_root))
            .push(Router::with_path("app/ws").goal(appweb::socket))
            .push(Router::with_path("app/api/{op}").post(appweb::api))
            .push(Router::with_path("app/saves").post(appweb::upload))
            .push(Router::with_path("app/saves/{name}").get(appweb::download))
            .push(Router::with_path("app/{**rest}").get(appweb::files));
    }
    r.push(Router::with_path("{**rest}").goal(web::missing))
}

/// the router with the error pages
pub fn service(mode: Mode) -> Service {
    Service::new(router(mode)).catcher(salvo::catcher::Catcher::default().hoop(ops::errors))
}

/// the join policy at start: `--join` when given; else what the database keeps; else,
/// for a server that has people already (from before the policy was kept), `apply`, which
/// is how it behaved; and for a new one, invite-only
fn settle_join(h: &Hub, flag: Option<Join>) -> Result<Join, String> {
    let j = match (flag, h.store.meta("join")?.and_then(|s| Join::parse(&s))) {
        (Some(j), _) => j,
        (None, Some(j)) => j,
        (None, None) if !h.store.users()?.is_empty() => Join::Apply,
        (None, None) => Join::Invite,
    };
    h.set_join_policy(j)?;
    Ok(j)
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    ops::start_log(args.json_log);
    let store = match Store::open(&args.data) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("couldn't open the data in {}: {e}", args.data.display());
            std::process::exit(1);
        }
    };
    // uploads that were being checked when the server stopped
    std::fs::remove_dir_all(store.incoming()).ok();
    // https: the flags, or what the admin panel saved
    let file = server::ServerFile::load(&args.data);
    let cert = args.tls_cert.clone().or_else(|| file.cert());
    let key = args.tls_key.clone().or_else(|| file.key());
    let tls = cert.zip(key);
    let scheme = if tls.is_some() { "https" } else { "http" };
    let url = args.url.clone().unwrap_or_else(|| format!("{scheme}://{}", args.addr));
    let mode = args.mode;
    let about = args.about.clone().unwrap_or_else(|| {
        if mode.hub {
            "Saves from the anend notes app: memories, notes and their gematria, shared by the people who made them."
                .into()
        } else {
            String::new()
        }
    });
    let hub = Hub {
        store,
        config: Config {
            name: args.name.clone(),
            description: about,
            url: url.clone(),
            contact: args.contact.clone(),
            max_upload: args.max_upload.max(1) * 1024 * 1024,
            share_quota: args.share_quota.saturating_mul(1024 * 1024),
            terms: args.terms.clone().unwrap_or_else(|| args.data.join("terms.md")),
            privacy: args.privacy.clone().unwrap_or_else(|| args.data.join("privacy.md")),
        },
        limiter: Limiter::new(args.no_ratelimit),
    };
    crate::mode::set(mode);
    web::BEHIND_PROXY.set(args.behind_proxy).ok();
    #[cfg(feature = "app")]
    {
        app::QUOTA.set(args.quota.saturating_mul(1024 * 1024)).ok();
        anend_core::ops::models::allow_pretrained(args.pretrained.clone());
        app::WORKSPACES.set(app::Workspaces::new(args.data.join("users"))).ok();
    }
    server::DATA.set(args.data.clone()).ok();
    if web::HUB.set(hub).is_err() {
        unreachable!("the server starts once");
    }
    let h = web::hub();
    let join = match settle_join(h, args.join) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("couldn't keep the join policy: {e}");
            std::process::exit(1);
        }
    };
    if let Some(name) = &args.admin {
        match h.make_admin(name) {
            Ok(link) => {
                println!("\n  {name} is an admin. Log in with this link (once, within an hour):\n\n    {link}\n")
            }
            Err(e) => {
                eprintln!("couldn't make {name} an admin: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.invite {
        // made in the first admin's name
        let admin = h
            .store
            .users()
            .unwrap_or_default()
            .into_iter()
            .filter(|u| u.is_admin())
            .min_by_key(|u| u.number);
        match admin.map(|a| h.make_invite(&a, 1, 7, "from the command line")) {
            Some(Ok((code, _))) => println!(
                "\n  An invite for one person, good for 7 days:\n\n    {}\n",
                invites::link(&code)
            ),
            Some(Err(e)) => eprintln!("couldn't make an invite: {e}"),
            None => eprintln!("--invite needs an admin first: add --admin YOURNAME"),
        }
    }
    let users = h.store.users().map(|u| u.len()).unwrap_or(0);
    if users == 0 && !mode.local {
        println!("\n  No one has joined yet. Run with --admin YOURNAME to make yourself the first admin.\n");
    }
    let serves = match (mode.hub, mode.app) {
        (true, true) => format!("the hub at {url}, the app at {url}/app/"),
        (true, false) => format!("the hub at {url}"),
        _ => format!("the app at {url}/app/"),
    };
    println!(
        "  {} mode: {serves} (data in {}); joining: {}",
        mode.name(),
        args.data.display(),
        join.about()
    );
    tracing::info!(mode = mode.name(), url, join = join.name(), "starting");
    if mode.local {
        println!("  --local: the web app needs no login; keep it on localhost");
        let loopback = args
            .addr
            .rsplit_once(':')
            .map(|(h, _)| h.trim_matches(['[', ']']))
            .is_some_and(|h| h == "localhost" || h.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()));
        if !loopback {
            println!(
                "  WARNING: --local on {}: anyone who can reach this address can read and change every save, with no login",
                args.addr
            );
        }
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
            let server = Server::new(acceptor);
            tokio::spawn(ops::stop_on_signal(server.handle()));
            server.serve(service(mode)).await;
        }
        None => {
            let acceptor = TcpListener::new(args.addr.clone()).bind().await;
            let server = Server::new(acceptor);
            tokio::spawn(ops::stop_on_signal(server.handle()));
            server.serve(service(mode)).await;
        }
    }
    tracing::info!("stopped");
}
