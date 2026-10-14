//! Through the router, for each mode: which routes exist, what the pages offer, the
//! sign-in the web app needs (pages, API, socket), joining by invitation, and the
//! visibility rules (a private or unlisted save or classifier never leaks into a grid,
//! a search, a feed, a profile or the API's lists, and a private one is its owner's alone).

use salvo::test::{ResponseExt, TestClient};

use crate::auth;
use crate::mode::Mode;
use crate::store::{Role, Standing, User};
use crate::testkit::{BASE, body, get_in, person, post_form_in, service, setup};
use crate::web;

/// a member still waiting for an admin, with a session
#[cfg_attr(not(feature = "app"), allow(dead_code))]
fn applicant(name: &str) -> String {
    web::hub()
        .store
        .add_user(User {
            name: name.into(),
            standing: Standing::Applied,
            ..Default::default()
        })
        .unwrap();
    format!(
        "{}={}",
        auth::COOKIE,
        auth::start_session(&web::hub().store, name).unwrap()
    )
}

/// a GET's status, where it redirects, and its body
#[cfg_attr(not(feature = "app"), allow(dead_code))]
async fn fetch(mode: Mode, path: &str, cookie: Option<&str>) -> (u16, String, String) {
    let mut b = TestClient::get(format!("{BASE}{path}")).add_header("host", "127.0.0.1:5800", true);
    if let Some(c) = cookie {
        b = b.add_header("cookie", c, true);
    }
    let mut r = b.send(&service(mode)).await;
    let code = r.status_code.map(|c| c.as_u16()).unwrap_or(200);
    let to = r
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (code, to, r.take_string().await.unwrap_or_default())
}

#[cfg(feature = "app")]
async fn socket(mode: Mode, cookie: Option<&str>) -> u16 {
    let mut b = TestClient::get(format!("{BASE}/app/ws"))
        .add_header("connection", "upgrade", true)
        .add_header("upgrade", "websocket", true)
        .add_header("sec-websocket-version", "13", true)
        .add_header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==", true);
    if let Some(c) = cookie {
        b = b.add_header("cookie", c, true);
    }
    body(b.send(&service(mode)).await).await.0
}

#[cfg(feature = "app")]
async fn app_call(mode: Mode, cookie: Option<&str>) -> u16 {
    let mut b = TestClient::post(format!("{BASE}/app/api/hello"))
        .add_header("host", "127.0.0.1:5800", true)
        .json(&serde_json::json!({}));
    if let Some(c) = cookie {
        b = b.add_header("cookie", c, true);
    }
    body(b.send(&service(mode)).await).await.0
}

/// every mode has the account pages and the pages about the server; the hub's and the
/// app's routes exist only in their modes, and the nav offers only what's there
#[cfg(all(feature = "hub", feature = "app"))]
#[tokio::test]
async fn routes_per_mode() {
    setup();
    let admin = person("mo-admin", Role::Admin);
    let hub_only = [
        "/classifiers",
        "/leaderboard",
        "/upload",
        "/api/saves",
        "/api/classifiers",
        "/api/hub",
        "/classifiers/upload",
    ];
    for m in [Mode::BOTH, Mode::HUB, Mode::APP, Mode::LOCAL] {
        for p in ["/about", "/terms", "/privacy", "/join", "/login", "/guide", "/health"] {
            assert_eq!(get_in(m, p, None).await.0, 200, "{p} in {}", m.name());
        }
        assert_eq!(get_in(m, "/admin", Some(&admin)).await.0, 200, "admin in {}", m.name());
        for p in hub_only {
            let (code, _) = get_in(m, p, Some(&admin)).await;
            assert_eq!(code == 200, m.hub, "{p} in {}: {code}", m.name());
        }
        let (code, _, _) = fetch(m, "/app/", Some(&admin)).await;
        assert_eq!(code == 200, m.app, "/app/ in {}: {code}", m.name());
        assert_eq!(app_call(m, Some(&admin)).await == 200, m.app, "app api in {}", m.name());
        let (_, health) = get_in(m, "/health", None).await;
        assert!(health.contains(&format!("\"mode\":\"{}\"", m.name())), "{health}");
    }
    // the app and the hub meet only in both
    for m in [Mode::HUB, Mode::APP] {
        let r = TestClient::post(format!("{BASE}/app/share"))
            .add_header("cookie", admin.as_str(), true)
            .json(&serde_json::json!({}))
            .send(&service(m))
            .await;
        assert_eq!(body(r).await.0, 404, "app/share in {}", m.name());
    }
    // the nav and the front page follow the mode
    let (_, page) = get_in(Mode::APP, "/", None).await;
    assert!(page.contains("/login?next=/app/") && !page.contains("href=\"/classifiers\""), "{page}");
    let (_, page) = get_in(Mode::HUB, "/", None).await;
    assert!(page.contains("href=\"/classifiers\"") && !page.contains("href=\"/app/\""), "{page}");
    let (_, page) = get_in(Mode::BOTH, "/", None).await;
    assert!(page.contains("href=\"/classifiers\"") && page.contains("href=\"/app/\""));
    let (code, to, _) = fetch(Mode::LOCAL, "/", None).await;
    assert_eq!((code, to.as_str()), (302, "/app/"));
    // a missing page is the hub's own page; an API's is JSON
    let (code, page) = get_in(Mode::APP, "/nowhere", None).await;
    assert!(code == 404 && page.contains("not here"));
    // no secrets in the log's paths
    assert_eq!(crate::ops::loggable("/invite/abcd-efgh-ijkm"), "/invite/…");
}

/// the web app needs a signed-in, admitted member: its page sends others to log in, and
/// its files, calls and socket refuse them; `--local` needs no one
#[cfg(feature = "app")]
#[tokio::test]
async fn app_needs_sign_in() {
    setup();
    let ok = person("si-member", Role::Member);
    let waiting = applicant("si-waiting");
    for m in [Mode::BOTH, Mode::APP] {
        let (code, to, _) = fetch(m, "/app/", None).await;
        assert_eq!((code, to.as_str()), (303, "/login?next=/app/"), "{}", m.name());
        let (code, _, page) = fetch(m, "/app/", Some(&waiting)).await;
        assert!(code == 403 && page.contains("waiting"), "{page}");
        let (code, _, page) = fetch(m, "/app/", Some(&ok)).await;
        assert!(code == 200 && page.contains("<html"), "{code}");
        assert_eq!(fetch(m, "/app/app.js", None).await.0, 401);
        assert_eq!(fetch(m, "/app/app.js", Some(&ok)).await.0, 200);
        assert_eq!(app_call(m, None).await, 401);
        assert_eq!(app_call(m, Some(&waiting)).await, 403);
        assert_eq!(app_call(m, Some(&ok)).await, 200);
        assert_eq!(socket(m, None).await, 401);
        assert_eq!(socket(m, Some(&waiting)).await, 403);
        assert_eq!(fetch(m, "/app/saves/x", None).await.0, 404);
    }
    // the app's page says which server it's on
    let r = TestClient::get(format!("{BASE}/app/"))
        .add_header("cookie", ok.as_str(), true)
        .send(&service(Mode::APP))
        .await;
    assert_eq!(r.headers().get("x-anend").unwrap(), "app");
    // --local: no one needed
    assert_eq!(app_call(Mode::LOCAL, None).await, 200);
    assert_eq!(fetch(Mode::LOCAL, "/app/", None).await.0, 200);
    // logging in goes back where it was asked from, and only to this server
    web::hub()
        .store
        .update_user("si-member", |u| u.password = auth::hash_password("long enough pw").unwrap())
        .unwrap();
    for (next, want) in [("/app/", "/app/"), ("//evil.example", "/"), ("https://evil.example", "/")] {
        let r = TestClient::post(format!("{BASE}/login?next={}", web::url(next)))
            .raw_form("name=si-member&password=long+enough+pw".to_string())
            .send(&service(Mode::APP))
            .await;
        assert_eq!(r.headers().get("location").unwrap(), want, "{next}");
    }
}

/// joining by invitation: the policy, codes with uses and an end, taking them back, who
/// may make them
#[tokio::test]
async fn invites() {
    setup();
    let h = web::hub();
    let admin = person("iv-admin", Role::Admin);
    let trusted = person("iv-trusted", Role::Trusted);
    h.set_join_policy(crate::hub::Join::Invite).unwrap();
    let join = |name: &str, code: &str| {
        format!("name={name}&password=long+enough+pw&invite={}", web::url(code))
    };
    // no code, no account
    let (code, _) = post_form_in(Mode::BOTH, "/join", "", &join("iv-nocode", "")).await;
    assert_eq!(code, 303);
    assert!(h.store.user("iv-nocode").unwrap().is_none());
    let (code, _) = post_form_in(Mode::BOTH, "/join", "", &join("iv-guess", "aaaa-bbbb-cccc")).await;
    assert_eq!(code, 303);
    assert!(h.store.user("iv-guess").unwrap().is_none());
    // only admins invite, until they let trusted members
    let (code, _) = get_in(Mode::BOTH, "/invites", Some(&trusted)).await;
    assert_eq!(code, 403);
    let (code, _) = post_form_in(Mode::BOTH, "/admin/settings", &trusted, "join=open").await;
    assert_eq!(code, 404);
    assert_eq!(h.join_policy(), crate::hub::Join::Invite);
    post_form_in(Mode::BOTH, "/admin/settings", &admin, "join=invite&trusted_invite=on").await;
    assert!(h.trusted_invite());
    assert_eq!(get_in(Mode::BOTH, "/invites", Some(&trusted)).await.0, 200);
    // an invite for two, shown once
    let (code, page) = post_form_in(Mode::BOTH, "/invites", &trusted, "uses=2&days=7&note=friends").await;
    assert_eq!(code, 200);
    let link = page
        .split("id=\"new-invite\">")
        .nth(1)
        .and_then(|s| s.split('<').next())
        .unwrap()
        .to_string();
    let code_text = link.rsplit('/').next().unwrap().to_string();
    assert!(link.starts_with(&format!("{BASE}/invite/")), "{link}");
    let (_, again) = get_in(Mode::BOTH, "/invites", Some(&trusted)).await;
    assert!(!again.contains(&code_text), "a code is shown only once");
    // its link fills in the join form
    let (code, page) = get_in(Mode::BOTH, &format!("/invite/{code_text}"), None).await;
    assert!(code == 200 && page.contains(&code_text) && page.contains("iv-trusted invited you"));
    // two people join with it, a third can't
    post_form_in(Mode::BOTH, "/join", "", &join("iv-one", &code_text.to_uppercase())).await;
    post_form_in(Mode::BOTH, "/join", "", &join("iv-two", &code_text)).await;
    post_form_in(Mode::BOTH, "/join", "", &join("iv-three", &code_text)).await;
    let one = h.store.user("iv-one").unwrap().unwrap();
    assert_eq!((one.standing, one.invited_by.as_str()), (Standing::Active, "iv-trusted"));
    assert!(h.store.user("iv-two").unwrap().is_some());
    assert!(h.store.user("iv-three").unwrap().is_none());
    // the admin panel lists every invite, and takes one back
    let (_, page) = get_in(Mode::BOTH, "/admin", Some(&admin)).await;
    assert!(page.contains("used up") && page.contains("friends"), "listed in admin");
    let (fresh, inv) = h
        .make_invite(&h.store.user("iv-admin").unwrap().unwrap(), 5, 0, "")
        .unwrap();
    // someone else can't take it back; its maker (or an admin) can
    post_form_in(Mode::BOTH, &format!("/invites/{}/revoke", inv.id), &trusted, "").await;
    assert!(h.store.invites().unwrap().iter().any(|i| i.id == inv.id && !i.revoked));
    post_form_in(Mode::BOTH, &format!("/invites/{}/revoke", inv.id), &admin, "").await;
    post_form_in(Mode::BOTH, "/join", "", &join("iv-late", &fresh)).await;
    assert!(h.store.user("iv-late").unwrap().is_none());
    // a code that ran out
    let (old, _) = h
        .make_invite(&h.store.user("iv-admin").unwrap().unwrap(), 1, 1, "")
        .unwrap();
    h.store
        .update_invite(&crate::store::hex(&crate::invites::hash(&old))[..12], |i| {
            i.expires = Some(chrono::Utc::now() - chrono::Duration::minutes(1))
        })
        .unwrap();
    post_form_in(Mode::BOTH, "/join", "", &join("iv-expired", &old)).await;
    assert!(h.store.user("iv-expired").unwrap().is_none());
    // bad numbers are refused
    assert!(h.make_invite(&h.store.user("iv-admin").unwrap().unwrap(), 0, 1, "").is_err());
    assert!(h.make_invite(&h.store.user("iv-admin").unwrap().unwrap(), 1, 9999, "").is_err());
    // under apply, a code skips the wait; without one you apply
    h.set_join_policy(crate::hub::Join::Apply).unwrap();
    let (skip, _) = h
        .make_invite(&h.store.user("iv-admin").unwrap().unwrap(), 1, 7, "")
        .unwrap();
    post_form_in(Mode::BOTH, "/join", "", &join("iv-skip", &skip)).await;
    post_form_in(Mode::BOTH, "/join", "", &join("iv-apply", "")).await;
    assert_eq!(h.store.user("iv-skip").unwrap().unwrap().standing, Standing::Active);
    assert_eq!(h.store.user("iv-apply").unwrap().unwrap().standing, Standing::Applied);
    h.set_join_policy(crate::hub::Join::Invite).unwrap();
}

/// private and unlisted saves and classifiers stay out of every list; a private one is
/// its owner's alone, an unlisted one anyone's with the link
#[cfg(feature = "hub")]
#[tokio::test]
async fn visibility() {
    use crate::hub::Details;
    use crate::store::{ClassifierRec, Status, Visibility};
    let dir = setup();
    let h = web::hub();
    let owner = person("vz-owner", Role::Trusted);
    let other = person("vz-other", Role::Member);
    let admin = person("vz-admin", Role::Admin);
    let token = auth::new_token(&h.store, "vz-owner", "test").unwrap();
    let u = h.store.user("vz-owner").unwrap().unwrap();
    let file = dir.join("vz.redb");
    crate::inspect::tests::make_save(&file, &["truth", "listen"]);
    let mut ids = std::collections::HashMap::new();
    for (name, vis) in [
        ("vzsecretpriv", Visibility::Private),
        ("vzsecretunl", Visibility::Unlisted),
        ("vzopenpub", Visibility::Public),
    ] {
        let d = Details::new(name, "", "vztag").unwrap().seen_by(vis.name()).unwrap();
        let s = h.accept(&u, d, &file, None).await.unwrap();
        assert_eq!(s.visibility, vis);
        ids.insert(name, s.id);
        // a classifier beside each
        let c = ClassifierRec {
            id: format!("vzc{}", &vis.name()[..3]),
            name: format!("{name}cls"),
            author: "vz-owner".into(),
            // the public one waits, so the classifier tests' grid stays theirs
            status: if vis == Visibility::Public { Status::Pending } else { Status::Approved },
            visibility: vis,
            size: 10,
            ..Default::default()
        };
        h.store.put_classifier(&c).unwrap();
        std::fs::write(h.store.classifier_path(&c.id).unwrap(), b"ANENDCLM test").unwrap();
    }
    h.store.log("vz-owner", "shared", &ids["vzsecretpriv"]).unwrap();
    h.store.log("vz-owner", "shared", &ids["vzsecretunl"]).unwrap();
    let (privid, unlid) = (ids["vzsecretpriv"].clone(), ids["vzsecretunl"].clone());
    // nowhere listed: grids, searches, feeds, profiles, the leaderboard, the API
    for (path, cookie) in [
        ("/", None),
        ("/?q=vzsecret", None),
        ("/?q=%23vztag", Some(&admin)),
        ("/?sort=stars", Some(&other)),
        ("/classifiers", None),
        ("/classifiers?q=vzsecret", Some(&admin)),
        ("/leaderboard", None),
        ("/u/vz-owner", Some(&other)),
        ("/u/vz-owner", Some(&admin)),
        ("/api/saves", None),
        ("/api/saves?q=vzsecret", None),
        ("/api/classifiers?q=vzsecret", None),
    ] {
        let (code, page) = get_in(Mode::BOTH, path, cookie.map(|c| c.as_str())).await;
        assert_eq!(code, 200, "{path}");
        assert!(
            !page.contains("vzsecretpriv") && !page.contains("vzsecretunl"),
            "{path} leaks: {page}"
        );
    }
    let (_, page) = get_in(Mode::BOTH, "/api/saves?q=vzopen", None).await;
    assert!(page.contains("vzopenpub"), "the public one is listed");
    // the owner sees theirs on their own page, marked
    let (_, page) = get_in(Mode::BOTH, "/u/vz-owner", Some(&owner)).await;
    assert!(page.contains("vzsecretpriv") && page.contains("private") && page.contains("unlisted"));
    // a private one: its owner only (not other members, not admins, not the API without the token)
    for (path, code_other, code_owner) in [
        (format!("/saves/{privid}"), 404, 200),
        (format!("/saves/{privid}/download"), 404, 200),
        (format!("/api/saves/{privid}"), 404, 200),
        ("/classifiers/vzcpri".to_string(), 404, 200),
        ("/classifiers/vzcpri/download".to_string(), 404, 200),
        ("/api/classifiers/vzcpri".to_string(), 404, 200),
        (format!("/saves/{unlid}"), 200, 200),
        (format!("/api/saves/{unlid}"), 200, 200),
        ("/classifiers/vzcunl".to_string(), 200, 200),
    ] {
        assert_eq!(get_in(Mode::BOTH, &path, Some(&other)).await.0, code_other, "{path} for another");
        assert_eq!(get_in(Mode::BOTH, &path, None).await.0, code_other, "{path} for no one");
        assert_eq!(get_in(Mode::BOTH, &path, Some(&owner)).await.0, code_owner, "{path} for its owner");
    }
    assert_eq!(get_in(Mode::BOTH, &format!("/saves/{privid}"), Some(&admin)).await.0, 404);
    let r = TestClient::get(format!("{BASE}/api/saves/{privid}"))
        .bearer_auth(&token)
        .send(&service(Mode::BOTH))
        .await;
    assert_eq!(body(r).await.0, 200, "the owner's token sees it");
    let r = TestClient::get(format!("{BASE}/api/mine"))
        .bearer_auth(&token)
        .send(&service(Mode::BOTH))
        .await;
    let (_, mine) = body(r).await;
    assert!(mine.contains("vzsecretpriv") && mine.contains("\"visibility\":\"private\""));
    // downloads of unlisted ones aren't counted, so they earn nothing
    get_in(Mode::BOTH, &format!("/saves/{unlid}/download"), Some(&other)).await;
    assert_eq!(h.store.save(&unlid).unwrap().unwrap().downloads, 0);
    // only the owner changes it; going public from a member's account waits for review
    let (code, _) = post_form_in(Mode::BOTH, &format!("/saves/{privid}/visibility"), &other, "visibility=public").await;
    assert_eq!(code, 404);
    let member = person("vz-member", Role::Member);
    let m = h.store.user("vz-member").unwrap().unwrap();
    let s = h
        .accept(&m, Details::new("vzmemberpriv", "", "").unwrap().seen_by("private").unwrap(), &file, None)
        .await
        .unwrap();
    assert_eq!((s.status, s.waiting()), (Status::Pending, false), "private skips the queue");
    let (_, page) = get_in(Mode::BOTH, "/admin", Some(&admin)).await;
    assert!(
        page.contains("vzmemberpriv") && !page.contains(&format!("<a href=\"/saves/{}\">", s.id)),
        "a private one is listed in admin, unlinked"
    );
    post_form_in(Mode::BOTH, &format!("/saves/{}/visibility", s.id), &member, "visibility=public").await;
    let s = h.store.save(&s.id).unwrap().unwrap();
    assert!(s.waiting() && !s.listed(), "public now, waiting for an admin");
    let (_, page) = get_in(Mode::BOTH, "/", None).await;
    assert!(!page.contains("vzmemberpriv"));
    // quotas count private ones
    assert!(h.shared_bytes("vz-owner").unwrap() >= 3 * std::fs::metadata(&file).unwrap().len());
    // a bad visibility is refused
    assert!(Details::new("x", "", "").unwrap().seen_by("secret").is_err());
}
