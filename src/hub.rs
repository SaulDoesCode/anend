//! Talking to an anend hub (see docs/hub-api.md): a self-hosted server where people share
//! their saves. Every call here blocks, so the settings menu runs them on a thread of
//! their own (see hub_ui.rs). What a hub sends is treated as untrusted: names are cleaned
//! before they touch the disk, links must stay on the hub, and sizes are capped.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::clean_name;

/// the biggest save a download may be
pub const MAX_DOWNLOAD: u64 = 256 << 20;
/// the biggest JSON answer read
const MAX_JSON: u64 = 4 << 20;

#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct HubSave {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
    pub notes: u64,
    pub memories: u64,
    pub size: u64,
    pub stars: u64,
    pub downloads: u64,
    pub featured: bool,
    pub created: String,
    pub updated: String,
    pub download_url: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SavesPage {
    pub saves: Vec<HubSave>,
    pub more: bool,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Me {
    pub username: String,
    pub points: i64,
    pub level: i64,
    pub title: String,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct HubInfo {
    pub name: String,
    pub description: String,
    pub saves: u64,
    pub users: u64,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Uploaded {
    pub id: String,
    pub status: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ErrorBody {
    error: String,
}

/// how the browse list is ordered
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Sort {
    #[default]
    New,
    Stars,
    Downloads,
    Featured,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::New, Sort::Stars, Sort::Downloads, Sort::Featured];

    pub fn name(self) -> &'static str {
        match self {
            Sort::New => "new",
            Sort::Stars => "stars",
            Sort::Downloads => "downloads",
            Sort::Featured => "featured",
        }
    }
}

// ------------------------------------------------------------------ urls

/// a typed hub address made tidy: `http(s)://host[:port][/path]` without the trailing `/`
pub fn base(url: &str) -> Result<String, String> {
    let u = url.trim().trim_end_matches('/');
    let rest = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))
        .ok_or("a hub's address starts with http:// or https://")?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err("the hub's address needs a host".into());
    }
    if u.chars().any(|c| c.is_whitespace() || c.is_control()) || u.contains(['?', '#']) {
        return Err("the hub's address has odd characters in it".into());
    }
    Ok(u.to_string())
}

/// a path on the hub; a path the hub sent can't lead anywhere else
pub fn join(hub: &str, path: &str) -> Result<String, String> {
    let b = base(hub)?;
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains("..")
        || path.contains('\\')
        || path.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("the hub sent a bad link".into());
    }
    Ok(format!("{}{}", b, path))
}

/// percent-encodes a query value
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn saves_url(hub: &str, q: &str, sort: Sort, page: u32) -> Result<String, String> {
    join(
        hub,
        &format!("/api/saves?q={}&sort={}&page={}", encode(q.trim()), sort.name(), page),
    )
}

// ------------------------------------------------------------------ http

/// what came back from a call
struct Answer {
    status: u16,
    body: Vec<u8>,
}

/// a request with an optional bearer token and body (its content type and bytes); the
/// answer's body is cut off past `limit`, which is an error
async fn request(
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<(String, Vec<u8>)>,
    limit: u64,
    timeout: Duration,
) -> Result<Answer, String> {
    use std::io::Read;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .user_agent(concat!("anend/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let auth = token.map(|t| format!("Bearer {}", t.trim()));
    let resp = match (method, body) {
        ("POST", Some((kind, bytes))) => {
            let mut req = agent.post(url).content_type(kind);
            if let Some(a) = &auth {
                req = req.header("Authorization", a);
            }
            req.send(&bytes[..])
        }
        _ => {
            let mut req = agent.get(url);
            if let Some(a) = &auth {
                req = req.header("Authorization", a);
            }
            req.call()
        }
    };
    let short = |e: ureq::Error| match e {
        ureq::Error::Timeout(_) => "the hub took too long to answer".to_string(),
        ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => "couldn't reach the hub".into(),
        ureq::Error::Io(e) => format!("couldn't reach the hub: {}", e),
        e => e.to_string(),
    };
    let resp = resp.map_err(short)?;
    let status = resp.status().as_u16();
    let length = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if length.is_some_and(|n| n > limit) {
        return Err(too_much(limit));
    }
    // one byte over the limit shows the hub sent too much
    let mut body = Vec::new();
    resp.into_body()
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("the answer broke off: {}", e))?;
    if body.len() as u64 > limit {
        return Err(too_much(limit));
    }
    Ok(Answer { status, body })
}

fn too_much(limit: u64) -> String {
    if limit >= MAX_DOWNLOAD {
        "that save is over 256 MB; not downloading it".into()
    } else {
        "the hub sent too much".into()
    }
}

/// the hub's own words for a failed call, or its status
fn failure(status: u16, body: &[u8]) -> String {
    let said = serde_json::from_slice::<ErrorBody>(body)
        .map(|b| b.error)
        .unwrap_or_default();
    let said: String = said.chars().filter(|c| !c.is_control()).take(200).collect();
    match (status, said.trim()) {
        (401, "") => "the hub didn't take the token (401)".into(),
        (413, "") => "the save is bigger than the hub takes (413)".into(),
        (_, "") => format!("the hub answered {}", status),
        (_, s) => format!("{} ({})", s, status),
    }
}

fn read_json<T: serde::de::DeserializeOwned>(a: Answer) -> Result<T, String> {
    if !(200..300).contains(&a.status) {
        return Err(failure(a.status, &a.body));
    }
    parse(&a.body)
}

pub fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, String> {
    serde_json::from_slice(body).map_err(|e| format!("the hub sent something odd: {}", e))
}

async fn get_json<T: serde::de::DeserializeOwned>(url: &str, token: Option<&str>) -> Result<T, String> {
    read_json(request("GET", url, token, None, MAX_JSON, Duration::from_secs(20)).await?)
}

/// a token, checked
fn need_token(token: &str) -> Result<Option<&str>, String> {
    let t = token.trim();
    if t.is_empty() {
        return Err("paste an app token from your profile on the hub".into());
    }
    if t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the token has spaces in it".into());
    }
    Ok(Some(t))
}

// ------------------------------------------------------------------ calls

/// what the hub calls itself
pub async fn hub_info(hub: &str) -> Result<HubInfo, String> {
    get_json(&join(hub, "/api/hub")?, None).await
}

/// who the token belongs to
pub async fn me(hub: &str, token: &str) -> Result<Me, String> {
    get_json(&join(hub, "/api/me")?, need_token(token)?).await
}

/// a page of approved saves
pub async fn saves(hub: &str, q: &str, sort: Sort, page: u32) -> Result<SavesPage, String> {
    get_json(&saves_url(hub, q, sort, page)?, None).await
}

/// fetches a save into `dir` under a free name and checks the app can open it; returns
/// where it went
pub async fn download(hub: &str, save: &HubSave, dir: &Path) -> Result<PathBuf, String> {
    let url = join(hub, &save.download_url)?;
    let name = clean_name(&save.name).unwrap_or_else(|| "hub save".into());
    let a = request("GET", &url, None, None, MAX_DOWNLOAD, Duration::from_secs(15 * 60)).await?;
    if !(200..300).contains(&a.status) {
        return Err(failure(a.status, &a.body));
    }
    crate::vfs::create_dir_all(dir).map_err(|e| format!("can't use the saves folder: {}", e))?;
    let part = dir.join(format!(".{}.download", clean_name(&save.id).unwrap_or_default()));
    let got = crate::vfs::write(&part, &a.body)
        .map_err(|e| format!("can't write the download: {}", e))
        .and_then(|_| check_save(&part));
    if let Err(e) = got {
        let _ = crate::vfs::remove(&part);
        return Err(e);
    }
    let to = crate::settings::unique(dir, &name);
    crate::vfs::rename(&part, &to).map_err(|e| {
        let _ = crate::vfs::remove(&part);
        format!("couldn't keep the download: {}", e)
    })?;
    Ok(to)
}

/// a downloaded file must be a save the app opens
fn check_save(path: &Path) -> Result<(), String> {
    let ok = crate::vfs::read(path).is_ok_and(|b| b.starts_with(b"redb\x1A\x0A\xA9\x0D\x0A"));
    if !ok {
        return Err("the hub sent something that isn't a save; deleted it".into());
    }
    // opening it as a store repairs a save copied while it was open and indexes it
    let store =
        crate::store::Store::open(path).map_err(|e| format!("the app can't open that save ({}); deleted it", e))?;
    drop(store);
    crate::store::Store::summary(path)
        .map(|_| ())
        .ok_or_else(|| "the app can't read that save; deleted it".into())
}

/// a multipart/form-data body of text fields and one file
pub fn multipart(boundary: &str, fields: &[(&str, &str)], file: (&str, &str, &[u8])) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n",
                boundary,
                quoted(name),
                value
            )
            .as_bytes(),
        );
    }
    let (field, filename, bytes) = file;
    body.extend_from_slice(
        format!(
            "--{}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n",
            boundary,
            quoted(field),
            quoted(filename)
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
    body
}

/// a header parameter can't hold quotes or line breaks
fn quoted(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '"' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// a boundary that doesn't turn up in what it separates
fn boundary_for(parts: &[&[u8]]) -> String {
    let mut seed = (chrono::Utc::now().timestamp_nanos_opt().unwrap_or(7) as u64) | 1;
    loop {
        // xorshift: plenty for a boundary
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let b = format!("anend-{:016x}", seed);
        if !parts.iter().any(|p| p.windows(b.len()).any(|w| w == b.as_bytes())) {
            return b;
        }
    }
}

/// shares a save: the hub keeps it pending until an admin approves it (unless the
/// uploader is trusted)
pub async fn upload(
    hub: &str,
    token: &str,
    name: &str,
    description: &str,
    tags: &str,
    bytes: &[u8],
) -> Result<Uploaded, String> {
    let token = need_token(token)?;
    let url = join(hub, "/api/saves")?;
    let name = clean_name(name).ok_or("give the shared save a name")?;
    let tags: Vec<String> = tags
        .split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let tags = tags.join(",");
    let description = description.trim();
    let boundary = boundary_for(&[bytes, name.as_bytes(), description.as_bytes(), tags.as_bytes()]);
    let filename = format!("{}.redb", name);
    let body = multipart(
        &boundary,
        &[("name", &name), ("description", description), ("tags", &tags)],
        ("file", &filename, bytes),
    );
    let kind = format!("multipart/form-data; boundary={}", boundary);
    read_json(
        request(
            "POST",
            &url,
            token,
            Some((kind, body)),
            MAX_JSON,
            Duration::from_secs(15 * 60),
        )
        .await?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(base(" http://localhost:8080/ ").unwrap(), "http://localhost:8080");
        assert_eq!(base("https://hub.example/anend").unwrap(), "https://hub.example/anend");
        assert!(base("localhost:8080").is_err());
        assert!(base("http://").is_err());
        assert!(base("http://a b").is_err());
        assert_eq!(
            join("http://h:1/", "/saves/k3/download").unwrap(),
            "http://h:1/saves/k3/download"
        );
        assert_eq!(join("https://h/x", "/api/hub").unwrap(), "https://h/x/api/hub");
        for bad in ["http://evil/x", "//evil/x", "/../etc", "saves", "/a b", "/a\\b"] {
            assert!(join("http://h", bad).is_err(), "{}", bad);
        }
        assert_eq!(
            saves_url("http://h", " the psalms&x=1 ", Sort::Stars, 2).unwrap(),
            "http://h/api/saves?q=the%20psalms%26x%3D1&sort=stars&page=2"
        );
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[test]
    fn multipart_body() {
        let body = multipart(
            "XX",
            &[("name", "the psalms"), ("tags", "bible,hebrew")],
            ("file", "the \"psalms\".redb", b"redb\x00bytes"),
        );
        let want = b"--XX\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nthe psalms\r\n\
--XX\r\nContent-Disposition: form-data; name=\"tags\"\r\n\r\nbible,hebrew\r\n\
--XX\r\nContent-Disposition: form-data; name=\"file\"; filename=\"the _psalms_.redb\"\r\n\
Content-Type: application/octet-stream\r\n\r\nredb\x00bytes\r\n--XX--\r\n";
        assert_eq!(body, want.to_vec());
        let b = boundary_for(&[b"anend-", b"data"]);
        assert!(b.starts_with("anend-") && b.len() == 22);
    }

    #[test]
    fn json() {
        let page: SavesPage = parse(
            br#"{"saves": [{"id": "k3x9q2", "name": "the psalms", "author": "saul",
            "description": "every psalm verse as a memory", "tags": ["bible", "hebrew"],
            "notes": 12, "memories": 2511, "size": 1048576, "stars": 31, "downloads": 204,
            "featured": false, "created": "2026-10-13T08:00:00Z", "updated": "2026-10-13T08:00:00Z",
            "download_url": "/saves/k3x9q2/download", "extra": 1}], "more": true}"#,
        )
        .unwrap();
        assert!(page.more);
        assert_eq!(page.saves[0].memories, 2511);
        assert_eq!(page.saves[0].tags, ["bible", "hebrew"]);
        let me: Me = parse(br#"{"username": "saul", "points": 120, "level": 3, "title": "scribe"}"#).unwrap();
        assert_eq!((me.level, me.points, me.title.as_str()), (3, 120, "scribe"));
        let hub: HubInfo = parse(br#"{"name": "home", "saves": 12}"#).unwrap();
        assert_eq!((hub.name.as_str(), hub.saves, hub.users), ("home", 12, 0));
        let up: Uploaded = parse(br#"{"id": "a1", "status": "pending"}"#).unwrap();
        assert_eq!(up.status, "pending");
        assert!(parse::<SavesPage>(b"<html>").is_err());
        assert_eq!(failure(403, br#"{"error": "not yours"}"#), "not yours (403)");
        assert_eq!(failure(500, b"oops"), "the hub answered 500");
    }

    /// against a running hub with the test saves (`ANEND_TEST_HUB=http://...`, token
    /// `ANEND_TEST_TOKEN`): `cargo test -- --ignored live_hub`
    #[test]
    #[ignore]
    fn live_hub() {
        use bevy::tasks::block_on;
        let hub = std::env::var("ANEND_TEST_HUB").expect("ANEND_TEST_HUB");
        let token = std::env::var("ANEND_TEST_TOKEN").unwrap_or_default();
        let info = block_on(hub_info(&hub)).unwrap();
        assert!(!info.name.is_empty());
        let me = block_on(me(&hub, &token)).unwrap();
        assert!(!me.username.is_empty());
        assert!(block_on(super::me(&hub, "wrong")).unwrap_err().contains("401"));
        let first = block_on(saves(&hub, "", Sort::New, 0)).unwrap();
        assert!(!first.saves.is_empty());
        let dir = std::env::temp_dir().join(format!("anend-hub-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut kept = None;
        for save in &first.saves {
            match block_on(download(&hub, save, &dir)) {
                Ok(path) => {
                    assert!(path.starts_with(&dir) && path.extension().is_some_and(|x| x == "redb"));
                    assert!(crate::store::Store::summary(&path).is_some());
                    kept.get_or_insert(path);
                }
                Err(e) => eprintln!("{}: {}", save.name, e),
            }
        }
        let kept = kept.expect("downloaded a save");
        // nothing half done is left behind
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            assert!(p.extension().is_some_and(|x| x == "redb"), "{}", p.display());
        }
        let bytes = std::fs::read(&kept).unwrap();
        let up = block_on(upload(
            &hub,
            &token,
            "my shared save",
            "a test",
            "Bible, , hebrew",
            &bytes,
        ))
        .unwrap();
        assert!(up.status == "pending" || up.status == "approved");
        assert!(block_on(upload(&hub, &token, "x", "", "", b"not a save")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
