//! Talking to an anend hub over HTTP (docs/hub-api.md), as the Bevy app's hub.rs did: what
//! a hub sends is untrusted (links must stay on the hub, sizes are capped, names are
//! cleaned before they touch the disk). The desktop calls with ureq on a blocking thread;
//! the web build with the page's `fetch` (its own hub, or another that allows it).

use serde_json::Value;

/// the biggest save a download may be
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const MAX_DOWNLOAD: u64 = 256 << 20;
/// the biggest JSON answer read
const MAX_JSON: u64 = 4 << 20;

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
    Ok(format!("{b}{path}"))
}

/// percent-encodes a query value
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// a hub's ids are lowercase letters and digits
pub fn id_ok(id: &str) -> bool {
    (1..=16).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// a multipart/form-data body of text fields and one file, and its content type
pub fn multipart(fields: &[(&str, &str)], file: (&str, &str, &[u8])) -> (String, Vec<u8>) {
    let mut seed = (crate::ui::now_ms() as u64).wrapping_mul(2654435761) | 1;
    let boundary = loop {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let b = format!("anend-{seed:016x}");
        if !file.2.windows(b.len()).any(|w| w == b.as_bytes()) {
            break b;
        }
    };
    let q = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c == '"' || c == '\\' || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect()
    };
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{value}\r\n",
                q(name)
            )
            .as_bytes(),
        );
    }
    let (field, filename, bytes) = file;
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
            q(field),
            q(filename)
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// the hub's own words for a failed call, or its status
fn failure(status: u16, body: &[u8]) -> String {
    let said = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string))
        .unwrap_or_default();
    let said: String = said.chars().filter(|c| !c.is_control()).take(200).collect();
    match (status, said.trim()) {
        (401, "") => "the hub didn't take the token (401)".into(),
        (413, "") => "that's bigger than the hub takes (413)".into(),
        (_, "") => format!("the hub answered {status}"),
        (_, s) => format!("{s} ({status})"),
    }
}

/// one call: the status and the body (cut off past `limit`, which is an error)
#[cfg(not(target_arch = "wasm32"))]
async fn request(
    method: &'static str,
    url: String,
    token: Option<String>,
    body: Option<(String, Vec<u8>)>,
    limit: u64,
) -> Result<(u16, Vec<u8>), String> {
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        use std::time::Duration;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(15 * 60)))
            .http_status_as_error(false)
            .user_agent(concat!("anend/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        let auth = token.map(|t| format!("Bearer {}", t.trim()));
        let resp = match (method, body) {
            ("POST", Some((kind, bytes))) => {
                let mut req = agent.post(&url).content_type(kind);
                if let Some(a) = &auth {
                    req = req.header("Authorization", a);
                }
                req.send(&bytes[..])
            }
            ("POST", None) => {
                let mut req = agent.post(&url);
                if let Some(a) = &auth {
                    req = req.header("Authorization", a);
                }
                req.send_empty()
            }
            _ => {
                let mut req = agent.get(&url);
                if let Some(a) = &auth {
                    req = req.header("Authorization", a);
                }
                req.call()
            }
        };
        let resp = resp.map_err(|e| match e {
            ureq::Error::Timeout(_) => "the hub took too long to answer".to_string(),
            ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => "couldn't reach the hub".into(),
            e => format!("couldn't reach the hub: {e}"),
        })?;
        let status = resp.status().as_u16();
        let mut out = Vec::new();
        resp.into_body()
            .into_reader()
            .take(limit + 1)
            .read_to_end(&mut out)
            .map_err(|e| format!("the answer broke off: {e}"))?;
        if out.len() as u64 > limit {
            return Err("the hub sent too much".to_string());
        }
        Ok((status, out))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(target_arch = "wasm32")]
async fn request(
    method: &'static str,
    url: String,
    token: Option<String>,
    body: Option<(String, Vec<u8>)>,
    limit: u64,
) -> Result<(u16, Vec<u8>), String> {
    // the page's fetch, the body going in and out as base64
    let b64 = body.as_ref().map(|(_, b)| crate::fonts::base64(b)).unwrap_or_default();
    let kind = body.as_ref().map(|(k, _)| k.clone()).unwrap_or_default();
    let js = format!(
        r#"(async () => {{
  try {{
    const h = {{}};
    const tok = {tok}; if (tok) h['Authorization'] = 'Bearer ' + tok;
    const kind = {kind}; const b64 = {b64};
    let body;
    if (b64) {{ h['Content-Type'] = kind; body = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)); }}
    const r = await fetch({url}, {{ method: {method}, headers: h, body, credentials: 'same-origin' }});
    const buf = new Uint8Array(await r.arrayBuffer());
    if (buf.length > {limit}) {{ dioxus.send({{ error: 'the hub sent too much' }}); return; }}
    let s = ''; for (let i = 0; i < buf.length; i += 32768) s += String.fromCharCode.apply(null, buf.subarray(i, i + 32768));
    dioxus.send({{ status: r.status, body: btoa(s) }});
  }} catch (e) {{ dioxus.send({{ error: 'couldn\'t reach the hub: ' + (e && e.message || e) }}); }}
}})();"#,
        tok = serde_json::to_string(&token.unwrap_or_default()).unwrap_or_default(),
        kind = serde_json::to_string(&kind).unwrap_or_default(),
        b64 = serde_json::to_string(&b64).unwrap_or_default(),
        url = serde_json::to_string(&url).unwrap_or_default(),
        method = serde_json::to_string(method).unwrap_or_default(),
    );
    let mut ev = dioxus::prelude::document::eval(&js);
    let v: Value = ev.recv().await.map_err(|e| format!("{e:?}"))?;
    if let Some(e) = v["error"].as_str() {
        return Err(e.to_string());
    }
    let body = crate::fonts::unbase64(v["body"].as_str().unwrap_or("")).ok_or("the answer didn't read")?;
    Ok((v["status"].as_u64().unwrap_or(0) as u16, body))
}

pub async fn get_json(url: String, token: Option<String>) -> Result<Value, String> {
    let (s, b) = request("GET", url, token, None, MAX_JSON).await?;
    if !(200..300).contains(&s) {
        return Err(failure(s, &b));
    }
    serde_json::from_slice(&b).map_err(|e| format!("the hub sent something odd: {e}"))
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub async fn get_bytes(url: String, limit: u64) -> Result<Vec<u8>, String> {
    let (s, b) = request("GET", url, None, None, limit).await?;
    if !(200..300).contains(&s) {
        return Err(failure(s, &b));
    }
    Ok(b)
}

pub async fn post(url: String, token: Option<String>, body: Option<(String, Vec<u8>)>) -> Result<Value, String> {
    let (s, b) = request("POST", url, token, body, MAX_JSON).await?;
    if !(200..300).contains(&s) {
        return Err(failure(s, &b));
    }
    Ok(serde_json::from_slice(&b).unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_stay_on_the_hub() {
        assert_eq!(base(" http://a.b:8080/ ").unwrap(), "http://a.b:8080");
        assert!(base("ftp://a").is_err() && base("http://").is_err() && base("http://a b").is_err());
        assert_eq!(join("http://a.b", "/api/saves").unwrap(), "http://a.b/api/saves");
        for bad in ["api", "//evil.org/x", "/../x", "/a b"] {
            assert!(join("http://a.b", bad).is_err(), "{bad}");
        }
        assert_eq!(encode("a b&c"), "a%20b%26c");
        assert!(id_ok("ab12") && !id_ok("AB") && !id_ok("../x"));
        let (kind, body) = multipart(&[("name", "x\"y")], ("file", "f.redb", b"data"));
        let s = String::from_utf8_lossy(&body);
        assert!(kind.starts_with("multipart/form-data; boundary=anend-"));
        assert!(
            s.contains("name=\"name\"\r\n\r\nx\"y") && s.contains("filename=\"f.redb\"") && s.contains("data\r\n--")
        );
    }
}
