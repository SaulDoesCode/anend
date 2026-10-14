//! Passwords (argon2), sessions (a random cookie whose hash is stored), app tokens (the
//! same, sent as a bearer token by the app), one time login links, and a small limit on
//! how often an address may try to log in, join or upload.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use chrono::{Duration, Utc};
use rand::Rng;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

use crate::store::{Link, Session, Store, Token, hex};

pub const COOKIE: &str = "hub_session";
pub const SESSION_DAYS: i64 = 30;

/// a random url-safe secret
pub fn secret() -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..40).map(|_| ABC[rng.random_range(0..ABC.len())] as char).collect()
}

/// a short id for a save
pub fn short_id() -> String {
    const ABC: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut rng = rand::rng();
    (0..7).map(|_| ABC[rng.random_range(0..ABC.len())] as char).collect()
}

pub fn digest(s: &str) -> Vec<u8> {
    Sha256::digest(s.as_bytes()).to_vec()
}

pub fn hash_password(pw: &str) -> Result<String, String> {
    Argon2::default()
        .hash_password(pw.as_bytes())
        .map(|h: PasswordHash| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn check_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

/// a hash to check a password against when there's no account by that name, so a login
/// for nobody takes as long as one for someone
pub fn dummy_hash() -> &'static str {
    static H: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    H.get_or_init(|| hash_password("no one has this password").unwrap_or_default())
}

/// starts a session; returns the cookie's value
pub fn start_session(store: &Store, user: &str) -> Result<String, String> {
    let s = secret();
    store.put_session(
        &digest(&s),
        &Session {
            user: user.to_string(),
            expires: Utc::now() + Duration::days(SESSION_DAYS),
        },
    )?;
    Ok(s)
}

/// makes an app token; returns it (it's shown once, only its hash is kept)
pub fn new_token(store: &Store, user: &str, label: &str) -> Result<String, String> {
    let s = format!("anend_{}", secret());
    store.put_token(
        &digest(&s),
        &Token {
            user: user.to_string(),
            label: label.to_string(),
            created: Utc::now(),
            hint: s.chars().take(10).collect(),
        },
    )?;
    Ok(s)
}

/// makes a login link's secret, good once within an hour
pub fn new_link(store: &Store, user: &str) -> Result<String, String> {
    let s = secret();
    store.put_link(
        &digest(&s),
        &Link {
            user: user.to_string(),
            expires: Utc::now() + Duration::hours(1),
        },
    )?;
    Ok(s)
}

/// who an address is, kept as a hash so the hub never stores addresses
pub fn address_key(ip: IpAddr) -> String {
    format!("ip:{}", &hex(&digest(&ip.to_string()))[..16])
}

/// a password worth having
pub fn password_ok(pw: &str) -> Result<(), &'static str> {
    if pw.chars().count() < 8 {
        Err("a password needs at least 8 characters")
    } else if pw.len() > 256 {
        Err("that password is too long")
    } else {
        Ok(())
    }
}

/// a user name: letters, digits, - and _, 2 to 24 of them
pub fn name_ok(name: &str) -> Result<(), &'static str> {
    let n = name.chars().count();
    if !(2..=24).contains(&n) {
        Err("a name is 2 to 24 characters")
    } else if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        Err("a name is letters, digits, - and _")
    } else if ["admin", "api", "static", "login", "logout", "join", "upload", "me"]
        .contains(&name.to_lowercase().as_str())
    {
        Err("that name is taken")
    } else {
        Ok(())
    }
}

/// allows `n` tries per `secs` for each address and kind of thing
pub struct Limiter {
    seen: Mutex<HashMap<(IpAddr, &'static str), Vec<Instant>>>,
    pub off: bool,
}

impl Limiter {
    pub fn new(off: bool) -> Limiter {
        Limiter {
            seen: Mutex::new(HashMap::new()),
            off,
        }
    }

    pub fn allow(&self, ip: IpAddr, what: &'static str, n: usize, secs: u64) -> bool {
        if self.off {
            return true;
        }
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if seen.len() > 50_000 {
            seen.clear();
        }
        let now = Instant::now();
        let times = seen.entry((ip, what)).or_default();
        times.retain(|t| now.duration_since(*t).as_secs() < secs);
        if times.len() >= n {
            return false;
        }
        times.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords() {
        let h = hash_password("correct horse").unwrap();
        assert!(check_password("correct horse", &h));
        assert!(!check_password("wrong horse", &h));
        assert!(!check_password("x", "not a hash"));
        assert!(dummy_hash().starts_with("$argon2") && !check_password("", dummy_hash()));
        assert!(password_ok("short").is_err());
        assert!(password_ok("long enough").is_ok());
    }

    #[test]
    fn names_and_limits() {
        assert!(name_ok("saul").is_ok());
        assert!(name_ok("Ünïcode_ok-1").is_ok());
        assert!(name_ok("a").is_err());
        assert!(name_ok("has space").is_err());
        assert!(name_ok("Admin").is_err());
        let l = Limiter::new(false);
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(l.allow(ip, "login", 2, 60));
        assert!(l.allow(ip, "login", 2, 60));
        assert!(!l.allow(ip, "login", 2, 60));
        assert!(l.allow(ip, "join", 2, 60));
        assert_ne!(secret(), secret());
        assert_eq!(short_id().len(), 7);
    }
}
