//! What the hub does, apart from how it's asked: taking uploads in, finding and sorting
//! saves, stars, downloads, the review queue. The web pages and the API both call these.

use chrono::Utc;
use std::path::Path;

use crate::auth::{self, Limiter};
use crate::game;
use crate::inspect;
use crate::store::{Role, Save, Standing, Status, Store, User};

pub struct Config {
    pub name: String,
    pub description: String,
    /// the address people reach the hub at, for links printed in the terminal
    pub url: String,
    /// new accounts are active at once instead of waiting for an admin
    pub open: bool,
    /// largest upload in bytes
    pub max_upload: u64,
}

pub struct Hub {
    pub store: Store,
    pub config: Config,
    pub limiter: Limiter,
}

pub const PAGE: usize = 24;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    New,
    Stars,
    Downloads,
    Featured,
}

impl Sort {
    pub fn parse(s: &str) -> Sort {
        match s {
            "stars" => Sort::Stars,
            "downloads" => Sort::Downloads,
            "featured" => Sort::Featured,
            _ => Sort::New,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sort::New => "new",
            Sort::Stars => "stars",
            Sort::Downloads => "downloads",
            Sort::Featured => "featured",
        }
    }
}

/// what an uploader says about a save
pub struct Details {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
}

/// tags typed as `a, b c` → `["a", "b-c"]`, lowercase, at most 8
pub fn parse_tags(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in s.split(',') {
        let t: String = t
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let t = t.trim_matches('-').chars().take(24).collect::<String>();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
        if out.len() == 8 {
            break;
        }
    }
    out
}

impl Details {
    pub fn new(name: &str, description: &str, tags: &str) -> Result<Details, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("give the save a name".into());
        }
        if name.chars().count() > 80 {
            return Err("a name is at most 80 characters".into());
        }
        let description = description.trim();
        if description.chars().count() > 4000 {
            return Err("a description is at most 4000 characters".into());
        }
        Ok(Details {
            name: name.to_string(),
            description: description.to_string(),
            tags: parse_tags(tags),
        })
    }
}

impl Hub {
    /// whether a user may share saves and star them
    pub fn active(user: &User) -> bool {
        user.standing == Standing::Active
    }

    /// takes an uploaded file in: checks it's a save, files it, and queues it for review
    /// (or lets it straight in for trusted users). `replace` uploads a new version.
    pub async fn accept(
        &self,
        user: &User,
        details: Details,
        file: &Path,
        replace: Option<&Save>,
    ) -> Result<Save, String> {
        if !Hub::active(user) {
            return Err("your account isn't active yet".into());
        }
        let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
        if size > self.config.max_upload {
            return Err(format!(
                "that file is {}; the hub takes up to {}",
                human_size(size),
                human_size(self.config.max_upload)
            ));
        }
        // check a copy, so nothing outside the hub's folder is opened
        let incoming = self.store.incoming();
        std::fs::create_dir_all(&incoming).map_err(|e| e.to_string())?;
        let tmp = incoming.join(format!("{}.redb", auth::secret()));
        std::fs::copy(file, &tmp).map_err(|e| e.to_string())?;
        let check = tmp.clone();
        let inside = tokio::task::spawn_blocking(move || inspect::inspect(&check))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        let inside = match inside {
            Ok(i) => i,
            Err(e) => {
                std::fs::remove_file(&tmp).ok();
                return Err(e);
            }
        };
        let trusted = matches!(user.role, Role::Trusted | Role::Admin);
        let now = Utc::now();
        // the stored record, not the caller's copy: stars and downloads may have moved since
        let current = match replace {
            Some(old) => Some(self.store.save(&old.id)?.ok_or("that save is gone")?),
            None => None,
        };
        let mut save = match current.as_ref() {
            Some(old) => Save {
                version: old.version + 1,
                updated: now,
                note: String::new(),
                ..old.clone()
            },
            None => Save {
                id: self.fresh_id()?,
                author: user.name.clone(),
                created: now,
                updated: now,
                version: 1,
                ..Default::default()
            },
        };
        save.name = details.name;
        save.description = details.description;
        save.tags = details.tags;
        save.notes = inside.notes;
        save.memories = inside.memories;
        save.sample = inside.sample;
        save.titles = inside.titles;
        save.size = size;
        save.status = if trusted { Status::Approved } else { Status::Pending };
        if let Err(e) = std::fs::rename(&tmp, self.store.save_path(&save.id)) {
            std::fs::remove_file(&tmp).ok();
            return Err(e.to_string());
        }
        if save.status == Status::Approved && !save.rewarded {
            save.rewarded = true;
            self.store.add_points(&save.author, game::APPROVED)?;
        }
        self.store.put_save(&save)?;
        let what = match (replace.is_some(), save.status) {
            (false, Status::Approved) => "shared",
            (false, _) => "sent in for review",
            (true, _) => "updated",
        };
        self.store.log(&user.name, what, &save.id)?;
        Ok(save)
    }

    fn fresh_id(&self) -> Result<String, String> {
        loop {
            let id = auth::short_id();
            if self.store.save(&id)?.is_none() {
                return Ok(id);
            }
        }
    }

    /// approved saves matching a search, sorted, one page of them; and whether there's more
    pub fn find(&self, q: &str, sort: Sort, page: usize) -> Result<(Vec<Save>, bool), String> {
        let q = q.trim().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let tag = q.strip_prefix('#');
        let mut saves: Vec<Save> = self
            .store
            .saves()?
            .into_iter()
            .filter(|s| s.status == Status::Approved)
            .filter(|s| match tag {
                Some(t) => s.tags.iter().any(|x| x == t),
                None => words.iter().all(|w| {
                    s.name.to_lowercase().contains(w)
                        || s.description.to_lowercase().contains(w)
                        || s.author.to_lowercase().contains(w)
                        || s.tags.iter().any(|t| t.contains(w))
                }),
            })
            .filter(|s| sort != Sort::Featured || s.featured)
            .collect();
        match sort {
            Sort::New | Sort::Featured => saves.sort_by_key(|s| std::cmp::Reverse(s.updated)),
            Sort::Stars => saves.sort_by(|a, b| b.stars.cmp(&a.stars).then(b.updated.cmp(&a.updated))),
            Sort::Downloads => saves.sort_by(|a, b| b.downloads.cmp(&a.downloads).then(b.updated.cmp(&a.updated))),
        }
        let start = page.saturating_mul(PAGE);
        let more = saves.len() > start + PAGE;
        Ok((saves.into_iter().skip(start).take(PAGE).collect(), more))
    }

    /// stars or unstars; the save's author gets points for it
    pub fn star(&self, save: &Save, user: &User) -> Result<(bool, u64), String> {
        if !Hub::active(user) {
            return Err("your account isn't active yet".into());
        }
        if save.author.eq_ignore_ascii_case(&user.name) {
            return Err("that's your own save".into());
        }
        let (on, n) = self.store.toggle_star(&save.id, &user.name)?;
        let sign = if on { 1 } else { -1 };
        self.store.add_points(&save.author, sign * game::STAR)?;
        self.store.add_points(&user.name, sign * game::CURATE)?;
        if on {
            self.store.log(&user.name, "starred", &save.id)?;
        }
        Ok((on, n))
    }

    /// counts a download; the author gets a point the first time each person takes it
    pub fn downloaded(&self, save: &Save, who: &str) -> Result<(), String> {
        let first = self.store.count_download(&save.id, who)?;
        if first && !save.author.eq_ignore_ascii_case(who) {
            self.store.add_points(&save.author, game::DOWNLOAD)?;
        }
        Ok(())
    }

    /// an admin's verdict on a save
    pub fn review(&self, admin: &User, id: &str, action: &str, note: &str) -> Result<String, String> {
        let save = self.store.save(id)?.ok_or("no such save")?;
        match action {
            "approve" => {
                let s = self.store.update_save(id, |s| {
                    s.status = Status::Approved;
                    s.note.clear();
                    if !s.rewarded {
                        s.rewarded = true;
                    }
                })?;
                if !save.rewarded {
                    self.store.add_points(&s.author, game::APPROVED)?;
                }
                self.store.log(&admin.name, "approved", id)?;
                Ok(format!("let “{}” in", s.name))
            }
            "reject" => {
                self.store.update_save(id, |s| {
                    s.status = Status::Rejected;
                    s.note = note.trim().chars().take(500).collect();
                })?;
                self.store.log(&admin.name, "turned down", id)?;
                Ok(format!("turned down “{}”", save.name))
            }
            "feature" | "unfeature" => {
                let on = action == "feature";
                self.store.update_save(id, |s| {
                    s.featured = on;
                    if on {
                        s.feature_rewarded = true;
                    }
                })?;
                if on && !save.feature_rewarded {
                    self.store.add_points(&save.author, game::FEATURED)?;
                }
                if on {
                    self.store.log(&admin.name, "featured", id)?;
                }
                Ok(format!(
                    "{} “{}”",
                    if on { "featured" } else { "unfeatured" },
                    save.name
                ))
            }
            "remove" => {
                self.remove(id)?;
                self.store.log(&admin.name, "removed", "")?;
                Ok(format!("removed “{}”", save.name))
            }
            _ => Err("unknown action".into()),
        }
    }

    /// deletes a save and its file
    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.store.remove_save(id)?;
        std::fs::remove_file(self.store.save_path(id)).ok();
        Ok(())
    }

    /// an admin's verdict on an account
    pub fn manage(&self, admin: &User, name: &str, action: &str) -> Result<String, String> {
        let user = self.store.user(name)?.ok_or("no such user")?;
        if user.name.eq_ignore_ascii_case(&admin.name) && action != "approve" {
            return Err("that's you".into());
        }
        let set = |f: &dyn Fn(&mut User)| self.store.update_user(name, |u| f(u));
        let msg = match action {
            "approve" | "unban" => {
                set(&|u| u.standing = Standing::Active)?;
                self.store.log(&user.name, "joined", "")?;
                format!("{} is in", user.name)
            }
            "reject" => {
                if user.standing != Standing::Applied {
                    return Err("only an application can be turned down".into());
                }
                self.store.remove_user(name)?;
                format!("turned down {}", user.name)
            }
            "ban" => {
                set(&|u| u.standing = Standing::Banned)?;
                self.store.drop_sessions_of(name)?;
                format!("banned {}", user.name)
            }
            "member" => {
                set(&|u| u.role = Role::Member)?;
                format!("{} is a member", user.name)
            }
            "trusted" => {
                set(&|u| u.role = Role::Trusted)?;
                format!("{}'s uploads skip the review now", user.name)
            }
            "admin" => {
                set(&|u| u.role = Role::Admin)?;
                format!("{} is an admin", user.name)
            }
            _ => return Err("unknown action".into()),
        };
        Ok(msg)
    }

    /// makes or promotes an admin from the command line and returns a login link
    pub fn make_admin(&self, name: &str) -> Result<String, String> {
        match self.store.user(name)? {
            Some(_) => {
                self.store.update_user(name, |u| {
                    u.role = Role::Admin;
                    u.standing = Standing::Active;
                })?;
            }
            None => {
                auth::name_ok(name)?;
                self.store.add_user(User {
                    name: name.to_string(),
                    role: Role::Admin,
                    standing: Standing::Active,
                    joined: Utc::now(),
                    ..Default::default()
                })?;
            }
        }
        let secret = auth::new_link(&self.store, name)?;
        Ok(format!("{}/login/{}", self.config.url.trim_end_matches('/'), secret))
    }
}

pub fn human_size(n: u64) -> String {
    let n = n as f64;
    if n >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", n / (1024.0 * 1024.0 * 1024.0))
    } else if n >= 1024.0 * 1024.0 {
        format!("{:.1} MB", n / (1024.0 * 1024.0))
    } else if n >= 1024.0 {
        format!("{:.0} KB", n / 1024.0)
    } else {
        format!("{} B", n as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::tests::make_save;

    fn hub() -> (Hub, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("anend-hubcore-{}", rand::random::<u64>()));
        let hub = Hub {
            store: Store::open(&dir).unwrap(),
            config: Config {
                name: "test".into(),
                description: String::new(),
                url: "http://localhost:8080".into(),
                open: false,
                max_upload: 8 << 20,
            },
            limiter: Limiter::new(true),
        };
        (hub, dir)
    }

    fn user(hub: &Hub, name: &str, role: Role) -> User {
        hub.store
            .add_user(User {
                name: name.into(),
                role,
                standing: Standing::Active,
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn tags() {
        assert_eq!(
            parse_tags(" Bible, hebrew words ,, bible,#x!"),
            ["bible", "hebrew-words", "x"]
        );
        assert!(Details::new("  ", "", "").is_err());
    }

    #[tokio::test]
    async fn upload_review_star_download() {
        let (hub, dir) = hub();
        let saul = user(&hub, "saul", Role::Member);
        let ann = user(&hub, "ann", Role::Member);
        let admin = user(&hub, "root", Role::Admin);
        let file = dir.join("upload.redb");
        make_save(&file, &["truth", "listen"]);

        let s = hub
            .accept(&saul, Details::new("psalms", "verses", "bible").unwrap(), &file, None)
            .await
            .unwrap();
        assert_eq!(s.status, Status::Pending);
        assert_eq!(s.memories, 2);
        assert!(hub.store.save_path(&s.id).exists());
        assert!(hub.find("", Sort::New, 0).unwrap().0.is_empty());

        hub.review(&admin, &s.id, "approve", "").unwrap();
        assert_eq!(hub.store.user("saul").unwrap().unwrap().points, game::APPROVED);
        // approving again pays nothing more
        hub.review(&admin, &s.id, "approve", "").unwrap();
        assert_eq!(hub.store.user("saul").unwrap().unwrap().points, game::APPROVED);
        assert_eq!(hub.find("PSA", Sort::New, 0).unwrap().0.len(), 1);
        assert_eq!(hub.find("#bible", Sort::Stars, 0).unwrap().0.len(), 1);
        assert!(hub.find("nothing", Sort::New, 0).unwrap().0.is_empty());

        let s = hub.store.save(&s.id).unwrap().unwrap();
        assert!(hub.star(&s, &saul).is_err());
        assert_eq!(hub.star(&s, &ann).unwrap(), (true, 1));
        hub.downloaded(&s, "ann").unwrap();
        hub.downloaded(&s, "ann").unwrap();
        let points = hub.store.user("saul").unwrap().unwrap().points;
        assert_eq!(points, game::APPROVED + game::STAR + game::DOWNLOAD);
        assert_eq!(hub.store.user("ann").unwrap().unwrap().points, game::CURATE);

        // a trusted user's new version goes straight in
        hub.manage(&admin, "saul", "trusted").unwrap();
        let saul = hub.store.user("saul").unwrap().unwrap();
        let v2 = hub
            .accept(&saul, Details::new("psalms 2", "", "").unwrap(), &file, Some(&s))
            .await
            .unwrap();
        assert_eq!((v2.version, v2.status, v2.stars), (2, Status::Approved, 1));

        let junk = dir.join("junk.redb");
        std::fs::write(&junk, b"junk").unwrap();
        assert!(
            hub.accept(&saul, Details::new("x", "", "").unwrap(), &junk, None)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(hub.store.incoming()).unwrap().count(), 0);

        hub.review(&admin, &s.id, "remove", "").unwrap();
        assert!(!hub.store.save_path(&s.id).exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn accounts() {
        let (hub, dir) = hub();
        let admin = user(&hub, "root", Role::Admin);
        hub.store
            .add_user(User {
                name: "new".into(),
                ..Default::default()
            })
            .unwrap();
        hub.manage(&admin, "new", "approve").unwrap();
        assert!(Hub::active(&hub.store.user("new").unwrap().unwrap()));
        assert!(hub.manage(&admin, "new", "reject").is_err());
        hub.manage(&admin, "new", "ban").unwrap();
        assert!(hub.manage(&admin, "root", "ban").is_err());
        let link = hub.make_admin("other").unwrap();
        assert!(link.starts_with("http://localhost:8080/login/"));
        assert!(hub.store.user("other").unwrap().unwrap().is_admin());
        std::fs::remove_dir_all(dir).ok();
    }
}
