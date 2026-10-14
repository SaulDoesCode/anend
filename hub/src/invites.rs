//! Invites: codes that let people join. Admins make them, and trusted members too when
//! the admins allow it; each has a number of uses and may run out after some days. Only
//! a code's hash is kept, so its link is shown once, when it's made. `--join invite`
//! (the default for a new server) makes a code the only way in; under `apply` one skips
//! the wait. A code typed at the join form counts against the address (10 an hour), so
//! codes can't be guessed.

use askama::Template;
use chrono::{Duration, Utc};
use rand::Rng;
use salvo::prelude::*;

use crate::auth;
use crate::hub::{Hub, Join};
use crate::store::{Invite, User, hex};
use crate::web::{Layout, ago, client_ip, go, hub, layout, me, message, need_admin, render};

/// the most uses one invite may have, and the longest it may last
pub const MAX_USES: u32 = 100;
pub const MAX_DAYS: i64 = 365;

/// a code as typed: spaces around it and the case don't matter
pub fn hash(code: &str) -> Vec<u8> {
    auth::digest(&code.trim().to_lowercase())
}

/// a fresh code: twelve letters and digits easy to read aloud, in threes of four
fn fresh_code() -> String {
    const ABC: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut rng = rand::rng();
    let c: String = (0..12).map(|_| ABC[rng.random_range(0..ABC.len())] as char).collect();
    format!("{}-{}-{}", &c[..4], &c[4..8], &c[8..])
}

/// where an invite's code takes people
pub fn link(code: &str) -> String {
    format!("{}/invite/{}", hub().config.url.trim_end_matches('/'), code)
}

impl Hub {
    /// makes an invite for `uses` people, lasting `days` (0: until used up or taken back);
    /// returns its code (shown once)
    pub fn make_invite(&self, by: &User, uses: u32, days: i64, note: &str) -> Result<(String, Invite), String> {
        if !self.may_invite(by) {
            return Err("you can't make invites here".into());
        }
        if !(1..=MAX_USES).contains(&uses) {
            return Err(format!("an invite is for 1 to {MAX_USES} people"));
        }
        if !(0..=MAX_DAYS).contains(&days) {
            return Err(format!("an invite lasts up to {MAX_DAYS} days (0: until it's used up)"));
        }
        let code = fresh_code();
        let h = hash(&code);
        let inv = Invite {
            id: hex(&h)[..12].to_string(),
            hint: code[..4].to_string(),
            by: by.name.clone(),
            note: note.trim().chars().take(120).collect(),
            created: Utc::now(),
            expires: (days > 0).then(|| Utc::now() + Duration::days(days)),
            max_uses: uses,
            used_by: Vec::new(),
            revoked: false,
        };
        self.store.put_invite(&h, &inv)?;
        self.store.log(&by.name, "made an invite", "")?;
        Ok((code, inv))
    }

    /// takes an invite back: its maker or an admin
    pub fn revoke_invite(&self, user: &User, id: &str) -> Result<Invite, String> {
        let mine = self
            .store
            .invites()?
            .into_iter()
            .find(|i| i.id == id)
            .filter(|i| user.is_admin() || i.by.eq_ignore_ascii_case(&user.name))
            .ok_or("no such invite")?;
        self.store.update_invite(&mine.id, |i| i.revoked = true)
    }
}

/// an invite in a list
pub struct InviteRow {
    pub id: String,
    pub hint: String,
    pub by: String,
    pub note: String,
    pub made: String,
    pub expires: String,
    pub uses: String,
    pub used_by: String,
    /// live, used up, run out or taken back
    pub state: &'static str,
    pub live: bool,
}

pub fn row(i: &Invite) -> InviteRow {
    let state = match i.spent() {
        None => "live",
        Some(_) if i.revoked => "taken back",
        Some(_) if i.used_by.len() as u32 >= i.max_uses => "used up",
        Some(_) => "run out",
    };
    InviteRow {
        id: i.id.clone(),
        hint: i.hint.clone(),
        by: i.by.clone(),
        note: i.note.clone(),
        made: ago(i.created),
        expires: match i.expires {
            Some(e) if e > Utc::now() => format!("{} days left", (e - Utc::now()).num_days().max(0) + 1),
            Some(e) => e.format("%d %b %Y").to_string(),
            None => "never".into(),
        },
        uses: format!("{} of {}", i.used_by.len(), i.max_uses),
        used_by: i.used_by.join(", "),
        state,
        live: state == "live",
    }
}

#[derive(Template)]
#[template(path = "invites.html")]
struct Page {
    l: Layout,
    invites: Vec<InviteRow>,
    new_invite: String,
    join: &'static str,
    max_uses: u32,
    max_days: i64,
}

fn show(depot: &Depot, res: &mut Response, user: &User) {
    let invites = hub()
        .store
        .invites()
        .unwrap_or_default()
        .iter()
        .filter(|i| i.by.eq_ignore_ascii_case(&user.name))
        .map(row)
        .collect();
    render(
        res,
        Page {
            l: layout(depot, "invites"),
            invites,
            new_invite: depot.get::<String>("new_invite").cloned().unwrap_or_default(),
            join: hub().join_policy().name(),
            max_uses: MAX_USES,
            max_days: MAX_DAYS,
        },
    );
}

/// someone who may make invites, or a page saying why not
fn inviter<'a>(res: &mut Response, depot: &'a Depot) -> Option<&'a User> {
    match me(depot) {
        None => {
            go(res, "/login?next=/invites", "!log in first");
            None
        }
        Some(u) if hub().may_invite(u) => Some(u),
        Some(_) => {
            message(
                res,
                depot,
                StatusCode::FORBIDDEN,
                "invites",
                "Only the admins make invites here. Ask one if you'd like to bring someone in.",
            );
            None
        }
    }
}

/// `GET /invites`: your invites, and a form for a new one
#[handler]
pub async fn page(depot: &mut Depot, res: &mut Response) {
    let Some(u) = inviter(res, depot).cloned() else { return };
    show(depot, res, &u);
}

/// `POST /invites`: `uses`, `days` (0: no end), `note`; `from=admin` shows the admin panel after
#[handler]
pub async fn make(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(u) = inviter(res, depot).cloned() else { return };
    let admin = req.form::<String>("from").await.as_deref() == Some("admin") && u.is_admin();
    let back = if admin { "/admin#invites" } else { "/invites" };
    if !hub().limiter.allow(client_ip(req), "makeinvite", 60, 3600) {
        return go(res, back, "!that's a lot of invites; try again in a while");
    }
    let uses = req.form::<String>("uses").await.unwrap_or_default();
    let days = req.form::<String>("days").await.unwrap_or_default();
    let note = req.form::<String>("note").await.unwrap_or_default();
    let (Ok(uses), Ok(days)) = (uses.trim().parse::<u32>(), days.trim().parse::<i64>()) else {
        return go(res, back, "!uses and days are numbers");
    };
    match hub().make_invite(&u, uses, days, &note) {
        Ok((code, _)) => {
            // shown once, on this page
            depot.insert("new_invite", link(&code));
            depot.insert(
                "flash",
                "here's the invite link: copy it now, it won't be shown again".to_string(),
            );
            if admin {
                crate::web::show_admin(depot, res);
            } else {
                show(depot, res, &u);
            }
        }
        Err(e) => go(res, back, &format!("!{e}")),
    }
}

/// `POST /invites/{id}/revoke`: its maker or an admin takes it back
#[handler]
pub async fn revoke(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(u) = me(depot).cloned() else {
        return go(res, "/login", "!log in first");
    };
    let id = req.param::<String>("id").unwrap_or_default();
    let back = if req.header::<String>("referer").is_some_and(|r| r.contains("/admin")) {
        "/admin#invites"
    } else {
        "/invites"
    };
    match hub().revoke_invite(&u, &id) {
        Ok(i) => go(res, back, &format!("took back the invite {}…", i.hint)),
        Err(e) => go(res, back, &format!("!{e}")),
    }
}

/// `GET /invite/{code}`: the join form with the code filled in
#[handler]
pub async fn follow(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let code = req.param::<String>("code").unwrap_or_default();
    if me(depot).is_some() {
        return go(res, "/", "you're logged in already: that invite is for someone new");
    }
    crate::web::join_form(depot, res, &code);
}

/// `POST /admin/settings`: `join` (open, apply, invite) and `trusted_invite` (on or not)
#[handler]
pub async fn settings(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(admin) = need_admin(res, depot).cloned() else { return };
    let join = req.form::<String>("join").await.unwrap_or_default();
    let trusted = req.form::<String>("trusted_invite").await.is_some_and(|v| v == "on" || v == "1");
    let Some(j) = Join::parse(&join) else {
        return go(res, "/admin#invites", "!joining is open, apply or invite");
    };
    let r = hub().set_join_policy(j).and_then(|_| hub().set_trusted_invite(trusted));
    match r {
        Ok(()) => {
            hub().store.log(&admin.name, format!("set joining to {}", j.name()), "").ok();
            go(res, "/admin#invites", &format!("joining: {}", j.about()))
        }
        Err(e) => go(res, "/admin#invites", &format!("!{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        let c = fresh_code();
        assert_eq!(c.len(), 14);
        assert_eq!(hash(&c), hash(&format!("  {}\n", c.to_uppercase())));
        assert_ne!(fresh_code(), c);
    }
}
