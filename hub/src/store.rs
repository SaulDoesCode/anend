//! The hub's own database: accounts, sessions, app tokens, login links, the saves' details,
//! stars and downloads, and a log of what happened. The save files themselves sit next to
//! it in `saves/`. Everything is JSON in redb tables, like the app's notes.
//!
//! Every build has all of it (a hub-only or app-only build reads the same database), so
//! the parts one build doesn't use aren't dead code to fix.
#![cfg_attr(not(all(feature = "hub", feature = "app")), allow(dead_code))]

use chrono::{DateTime, Utc};
use redb::{
    Database, MultimapTableDefinition, ReadableDatabase, ReadableMultimapTable, ReadableTable, ReadableTableMetadata,
    TableDefinition,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

const USERS: TableDefinition<&str, &[u8]> = TableDefinition::new("users");
/// sha256 of a session cookie → session
const SESSIONS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("sessions");
/// sha256 of an app token → token
const TOKENS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("tokens");
/// sha256 of a one time login link → link
const LINKS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("links");
const SAVES: TableDefinition<&str, &[u8]> = TableDefinition::new("saves");
/// save id (or `c:` and a classifier's id) → user names that starred it
const STARS: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("stars");
/// user name → save ids (and `c:` classifier ids) they starred
const STARRED: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("starred");
/// save id (or `c:` id) → who downloaded it (a user name, or a hash of an address), counted once each
const DOWNLOADERS: MultimapTableDefinition<&str, &str> = MultimapTableDefinition::new("downloaders");
/// shared classifier id → its record (the module sits in `classifiers/{id}.anendcls`)
const CLASSIFIERS: TableDefinition<&str, &[u8]> = TableDefinition::new("classifiers");
/// sequence number → event, newest last
const EVENTS: TableDefinition<u64, &[u8]> = TableDefinition::new("events");
/// settings kept in the database (the join policy, who may invite)
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");
/// sha256 of an invite code → invite
const INVITES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("invites");

pub type Result<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Member,
    /// their uploads skip the review
    Trusted,
    Admin,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Standing {
    /// waiting for an admin to let them in
    #[default]
    Applied,
    Active,
    Banned,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct User {
    pub name: String,
    pub email: String,
    /// argon2 hash; empty until they set one (an admin made by a login link)
    pub password: String,
    pub role: Role,
    pub standing: Standing,
    pub bio: String,
    /// what they wrote when applying
    pub why: String,
    pub joined: DateTime<Utc>,
    pub points: i64,
    /// the order they joined in, from 1
    pub number: u64,
    /// who made the invite they joined with
    pub invited_by: String,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin && self.standing == Standing::Active
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub user: String,
    pub expires: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Token {
    pub user: String,
    pub label: String,
    pub created: DateTime<Utc>,
    /// the first characters, so a list of tokens can be told apart
    pub hint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link {
    pub user: String,
    pub expires: DateTime<Utc>,
}

/// who may see a save or a classifier on the hub
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// only its owner: never listed, never reviewed, downloaded by its owner alone
    Private,
    /// anyone with its link; never listed, never reviewed
    Unlisted,
    /// listed in the grids, search, feeds and the API once an admin lets it in (everything
    /// shared before visibility existed is public)
    #[default]
    Public,
}

impl Visibility {
    pub fn parse(s: &str) -> Option<Visibility> {
        match s.trim() {
            "private" => Some(Visibility::Private),
            "unlisted" => Some(Visibility::Unlisted),
            "public" => Some(Visibility::Public),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Unlisted => "unlisted",
            Visibility::Public => "public",
        }
    }
}

/// whether `user` may see a thing `author` shared with `vis` and `status`: a private one
/// only its author; an unlisted one anyone with the link; a public one anyone once it's
/// approved, and its author and the admins before
pub fn may_see(vis: Visibility, status: Status, author: &str, user: Option<&User>) -> bool {
    let own = user.is_some_and(|u| u.name.eq_ignore_ascii_case(author));
    match vis {
        Visibility::Private => own,
        Visibility::Unlisted => true,
        Visibility::Public => status == Status::Approved || own || user.is_some_and(|u| u.is_admin()),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Pending,
    Approved,
    Rejected,
}

/// a memory from a save, shown on its page
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Sample {
    pub text: String,
    pub ordinal: u64,
    pub reversed: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Save {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
    pub status: Status,
    /// why an admin turned it down
    pub note: String,
    pub featured: bool,
    pub notes: u64,
    pub memories: u64,
    pub size: u64,
    pub stars: u64,
    pub downloads: u64,
    pub sample: Vec<Sample>,
    pub titles: Vec<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub version: u32,
    /// its uploader got the points for its first approval
    pub rewarded: bool,
    /// and for its first feature
    pub feature_rewarded: bool,
    pub visibility: Visibility,
}

impl Save {
    /// in the grid, search, feeds and the API's lists: public and let in
    pub fn listed(&self) -> bool {
        self.visibility == Visibility::Public && self.status == Status::Approved
    }

    /// waiting for an admin: public and not yet looked at (private and unlisted ones skip the review)
    pub fn waiting(&self) -> bool {
        self.visibility == Visibility::Public && self.status == Status::Pending
    }
}

/// a classifier module shared on the hub: what its uploader said and what its file holds
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClassifierRec {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
    pub status: Status,
    /// why an admin turned it down
    pub note: String,
    pub featured: bool,
    /// the classifier's own name inside the module
    pub module: String,
    pub classes: Vec<String>,
    /// single, multi or regress
    pub head: String,
    pub preset: String,
    /// weights bundled in the module
    pub weights: u64,
    pub accuracy: Option<f64>,
    pub macro_f1: Option<f64>,
    pub examples: usize,
    pub measured_on: String,
    /// the classifiers it builds on, and whether each is bundled
    pub upstream: Vec<(String, bool)>,
    /// its weights' hash
    pub hash: String,
    pub size: u64,
    pub stars: u64,
    pub downloads: u64,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub version: u32,
    pub rewarded: bool,
    pub feature_rewarded: bool,
    pub visibility: Visibility,
}

impl ClassifierRec {
    /// in the grid, search, feeds and the API's lists: public and let in
    pub fn listed(&self) -> bool {
        self.visibility == Visibility::Public && self.status == Status::Approved
    }

    /// waiting for an admin: public and not yet looked at
    pub fn waiting(&self) -> bool {
        self.visibility == Visibility::Public && self.status == Status::Pending
    }

    /// its key in the stars and downloads tables, apart from the saves'
    pub fn key(&self) -> String {
        format!("{CLS}{}", self.id)
    }
}

/// the prefix of a classifier's key in the stars and downloads tables
pub const CLS: &str = "c:";

/// a record with stars and downloads counted on it
trait Counted: Serialize + DeserializeOwned {
    fn set_stars(&mut self, n: u64);
    fn add_download(&mut self);
}

impl Counted for Save {
    fn set_stars(&mut self, n: u64) {
        self.stars = n;
    }
    fn add_download(&mut self) {
        self.downloads += 1;
    }
}

impl Counted for ClassifierRec {
    fn set_stars(&mut self, n: u64) {
        self.stars = n;
    }
    fn add_download(&mut self) {
        self.downloads += 1;
    }
}

/// an id the hub made (letters and digits only), so it's safe in a file's name
pub fn id_ok(id: &str) -> bool {
    (1..=16).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// an invite: a code (kept only as its hash) that lets people join an invite-only hub
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Invite {
    /// the start of the code's hash, as hex: how lists and links name it
    pub id: String,
    /// the code's first characters, so a list can be told apart
    pub hint: String,
    /// who made it
    pub by: String,
    /// what it's for, for whoever made it
    pub note: String,
    pub created: DateTime<Utc>,
    /// none: it doesn't run out by time
    pub expires: Option<DateTime<Utc>>,
    /// how many people may join with it
    pub max_uses: u32,
    /// who joined with it
    pub used_by: Vec<String>,
    pub revoked: bool,
}

impl Invite {
    /// why it can't be used now, if it can't
    pub fn spent(&self) -> Option<&'static str> {
        if self.revoked {
            Some("that invite was taken back")
        } else if self.expires.is_some_and(|e| e <= Utc::now()) {
            Some("that invite has run out")
        } else if self.used_by.len() as u32 >= self.max_uses {
            Some("that invite has been used up")
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub at: DateTime<Utc>,
    pub user: String,
    pub what: String,
    /// a save id, when it's about one
    #[serde(default)]
    pub save: String,
}

pub struct Store {
    db: Database,
    dir: PathBuf,
}

fn get<T: DeserializeOwned>(v: &[u8]) -> Result<T> {
    serde_json::from_slice(v).map_err(err)
}

fn put<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(v).map_err(err)
}

impl Store {
    pub fn open(dir: &Path) -> Result<Store> {
        std::fs::create_dir_all(dir.join("saves")).map_err(err)?;
        std::fs::create_dir_all(dir.join("classifiers")).map_err(err)?;
        let db = Database::create(dir.join("hub.redb")).map_err(err)?;
        let txn = db.begin_write().map_err(err)?;
        txn.open_table(USERS).map_err(err)?;
        txn.open_table(SESSIONS).map_err(err)?;
        txn.open_table(TOKENS).map_err(err)?;
        txn.open_table(LINKS).map_err(err)?;
        txn.open_table(SAVES).map_err(err)?;
        txn.open_multimap_table(STARS).map_err(err)?;
        txn.open_multimap_table(STARRED).map_err(err)?;
        txn.open_multimap_table(DOWNLOADERS).map_err(err)?;
        txn.open_table(CLASSIFIERS).map_err(err)?;
        txn.open_table(EVENTS).map_err(err)?;
        txn.open_table(META).map_err(err)?;
        txn.open_table(INVITES).map_err(err)?;
        txn.commit().map_err(err)?;
        Ok(Store {
            db,
            dir: dir.to_path_buf(),
        })
    }

    /// where a save's file lives
    pub fn save_path(&self, id: &str) -> PathBuf {
        self.dir.join("saves").join(format!("{id}.redb"))
    }

    /// where a shared classifier's module lives (`None` for an id the hub couldn't have made)
    pub fn classifier_path(&self, id: &str) -> Option<PathBuf> {
        id_ok(id).then(|| self.dir.join("classifiers").join(format!("{id}.anendcls")))
    }

    /// a scratch place for uploads being checked
    pub fn incoming(&self) -> PathBuf {
        self.dir.join("incoming")
    }

    // -------------------------------------------------------------- users

    pub fn user(&self, name: &str) -> Result<Option<User>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(USERS).map_err(err)?;
        t.get(name.to_lowercase().as_str())
            .map_err(err)?
            .map(|v| get(v.value()))
            .transpose()
    }

    pub fn users(&self) -> Result<Vec<User>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(USERS).map_err(err)?;
        let mut out = Vec::new();
        for row in t.iter().map_err(err)? {
            let (_, v) = row.map_err(err)?;
            out.push(get(v.value())?);
        }
        Ok(out)
    }

    pub fn user_by_email(&self, email: &str) -> Result<Option<User>> {
        let email = email.trim().to_lowercase();
        Ok(self
            .users()?
            .into_iter()
            .find(|u| !u.email.is_empty() && u.email.to_lowercase() == email))
    }

    /// adds a user, numbering them; fails if the name is taken
    pub fn add_user(&self, mut user: User) -> Result<User> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(USERS).map_err(err)?;
            let key = user.name.to_lowercase();
            if t.get(key.as_str()).map_err(err)?.is_some() {
                return Err("that name is taken".into());
            }
            user.number = t.len().map_err(err)? + 1;
            t.insert(key.as_str(), put(&user)?.as_slice()).map_err(err)?;
        }
        txn.commit().map_err(err)?;
        Ok(user)
    }

    /// changes a user in place
    pub fn update_user(&self, name: &str, f: impl FnOnce(&mut User)) -> Result<User> {
        let txn = self.db.begin_write().map_err(err)?;
        let user = {
            let mut t = txn.open_table(USERS).map_err(err)?;
            let key = name.to_lowercase();
            let mut user: User = match t.get(key.as_str()).map_err(err)? {
                Some(v) => get(v.value())?,
                None => return Err("no such user".into()),
            };
            f(&mut user);
            t.insert(key.as_str(), put(&user)?.as_slice()).map_err(err)?;
            user
        };
        txn.commit().map_err(err)?;
        Ok(user)
    }

    /// removes a user (a turned down application)
    pub fn remove_user(&self, name: &str) -> Result<()> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(USERS)
            .map_err(err)?
            .remove(name.to_lowercase().as_str())
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    pub fn add_points(&self, name: &str, n: i64) -> Result<()> {
        if name.is_empty() || n == 0 {
            return Ok(());
        }
        self.update_user(name, |u| u.points += n).map(|_| ())
    }

    // -------------------------------------------------------------- sessions, tokens, links

    pub fn put_session(&self, hash: &[u8], s: &Session) -> Result<()> {
        self.put_bytes(SESSIONS, hash, &put(s)?)
    }

    pub fn session(&self, hash: &[u8]) -> Result<Option<Session>> {
        let s: Option<Session> = self.get_bytes(SESSIONS, hash)?;
        Ok(s.filter(|s| s.expires > Utc::now()))
    }

    pub fn drop_session(&self, hash: &[u8]) -> Result<()> {
        self.remove_bytes(SESSIONS, hash).map(|_| ())
    }

    /// ends every session of a user (banned, or their password changed)
    pub fn drop_sessions_of(&self, name: &str) -> Result<()> {
        let name = name.to_lowercase();
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(SESSIONS).map_err(err)?;
            t.retain(|_, v| {
                serde_json::from_slice::<Session>(v)
                    .map(|s| s.user.to_lowercase() != name && s.expires > Utc::now())
                    .unwrap_or(false)
            })
            .map_err(err)?;
        }
        txn.commit().map_err(err)
    }

    pub fn put_token(&self, hash: &[u8], t: &Token) -> Result<()> {
        self.put_bytes(TOKENS, hash, &put(t)?)
    }

    pub fn token(&self, hash: &[u8]) -> Result<Option<Token>> {
        self.get_bytes(TOKENS, hash)
    }

    /// a user's tokens, with their hashes (as hex) to revoke them by
    pub fn tokens_of(&self, name: &str) -> Result<Vec<(String, Token)>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(TOKENS).map_err(err)?;
        let mut out = Vec::new();
        for row in t.iter().map_err(err)? {
            let (k, v) = row.map_err(err)?;
            let tok: Token = get(v.value())?;
            if tok.user.eq_ignore_ascii_case(name) {
                out.push((hex(k.value()), tok));
            }
        }
        out.sort_by_key(|t| std::cmp::Reverse(t.1.created));
        Ok(out)
    }

    pub fn drop_token(&self, hash: &[u8]) -> Result<Option<Token>> {
        self.remove_bytes(TOKENS, hash)?.map(|v| get(&v)).transpose()
    }

    pub fn put_link(&self, hash: &[u8], l: &Link) -> Result<()> {
        self.put_bytes(LINKS, hash, &put(l)?)
    }

    /// uses up a login link
    pub fn take_link(&self, hash: &[u8]) -> Result<Option<Link>> {
        let l: Option<Link> = self.remove_bytes(LINKS, hash)?.map(|v| get(&v)).transpose()?;
        Ok(l.filter(|l| l.expires > Utc::now()))
    }

    fn put_bytes(&self, table: TableDefinition<&[u8], &[u8]>, k: &[u8], v: &[u8]) -> Result<()> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(table).map_err(err)?.insert(k, v).map_err(err)?;
        txn.commit().map_err(err)
    }

    fn get_bytes<T: DeserializeOwned>(&self, table: TableDefinition<&[u8], &[u8]>, k: &[u8]) -> Result<Option<T>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(table).map_err(err)?;
        t.get(k).map_err(err)?.map(|v| get(v.value())).transpose()
    }

    fn remove_bytes(&self, table: TableDefinition<&[u8], &[u8]>, k: &[u8]) -> Result<Option<Vec<u8>>> {
        let txn = self.db.begin_write().map_err(err)?;
        let old = txn
            .open_table(table)
            .map_err(err)?
            .remove(k)
            .map_err(err)?
            .map(|g| g.value().to_vec());
        txn.commit().map_err(err)?;
        Ok(old)
    }

    // -------------------------------------------------------------- saves

    pub fn save(&self, id: &str) -> Result<Option<Save>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(SAVES).map_err(err)?;
        t.get(id).map_err(err)?.map(|v| get(v.value())).transpose()
    }

    pub fn saves(&self) -> Result<Vec<Save>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(SAVES).map_err(err)?;
        let mut out = Vec::new();
        for row in t.iter().map_err(err)? {
            let (_, v) = row.map_err(err)?;
            out.push(get(v.value())?);
        }
        Ok(out)
    }

    pub fn put_save(&self, s: &Save) -> Result<()> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(SAVES)
            .map_err(err)?
            .insert(s.id.as_str(), put(s)?.as_slice())
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    pub fn update_save(&self, id: &str, f: impl FnOnce(&mut Save)) -> Result<Save> {
        let txn = self.db.begin_write().map_err(err)?;
        let save = {
            let mut t = txn.open_table(SAVES).map_err(err)?;
            let mut s: Save = match t.get(id).map_err(err)? {
                Some(v) => get(v.value())?,
                None => return Err("no such save".into()),
            };
            f(&mut s);
            t.insert(id, put(&s)?.as_slice()).map_err(err)?;
            s
        };
        txn.commit().map_err(err)?;
        Ok(save)
    }

    /// removes a save's record, its stars and downloads (the file is the caller's)
    pub fn remove_save(&self, id: &str) -> Result<Option<Save>> {
        self.remove_in(SAVES, id, id)
    }

    fn remove_in<T: DeserializeOwned>(
        &self,
        table: TableDefinition<&str, &[u8]>,
        id: &str,
        key: &str,
    ) -> Result<Option<T>> {
        let txn = self.db.begin_write().map_err(err)?;
        let old = {
            let mut t = txn.open_table(table).map_err(err)?;
            let old = t.remove(id).map_err(err)?.map(|g| g.value().to_vec());
            let mut stars = txn.open_multimap_table(STARS).map_err(err)?;
            let who: Vec<String> = stars
                .get(key)
                .map_err(err)?
                .filter_map(|v| v.ok().map(|v| v.value().to_string()))
                .collect();
            stars.remove_all(key).map_err(err)?;
            let mut starred = txn.open_multimap_table(STARRED).map_err(err)?;
            for u in who {
                starred.remove(u.as_str(), key).map_err(err)?;
            }
            txn.open_multimap_table(DOWNLOADERS)
                .map_err(err)?
                .remove_all(key)
                .map_err(err)?;
            old
        };
        txn.commit().map_err(err)?;
        old.map(|v| get(&v)).transpose()
    }

    /// stars or unstars a save; returns whether it's starred now and its star count
    pub fn toggle_star(&self, id: &str, user: &str) -> Result<(bool, u64)> {
        self.toggle_star_in::<Save>(SAVES, id, id, user)
    }

    fn toggle_star_in<T: Counted>(
        &self,
        table: TableDefinition<&str, &[u8]>,
        id: &str,
        key: &str,
        user: &str,
    ) -> Result<(bool, u64)> {
        let user = user.to_lowercase();
        let txn = self.db.begin_write().map_err(err)?;
        let result = {
            let mut t = txn.open_table(table).map_err(err)?;
            let mut s: T = match t.get(id).map_err(err)? {
                Some(v) => get(v.value())?,
                None => return Err("no such thing".into()),
            };
            let mut stars = txn.open_multimap_table(STARS).map_err(err)?;
            let mut starred = txn.open_multimap_table(STARRED).map_err(err)?;
            let on = if stars.remove(key, user.as_str()).map_err(err)? {
                starred.remove(user.as_str(), key).map_err(err)?;
                false
            } else {
                stars.insert(key, user.as_str()).map_err(err)?;
                starred.insert(user.as_str(), key).map_err(err)?;
                true
            };
            let count = stars.get(key).map_err(err)?.count() as u64;
            s.set_stars(count);
            t.insert(id, put(&s)?.as_slice()).map_err(err)?;
            (on, count)
        };
        txn.commit().map_err(err)?;
        Ok(result)
    }

    /// whether a user starred a save (its id) or a classifier (`ClassifierRec::key`)
    pub fn has_starred(&self, key: &str, user: &str) -> Result<bool> {
        let txn = self.db.begin_read().map_err(err)?;
        let stars = txn.open_multimap_table(STARS).map_err(err)?;
        let user = user.to_lowercase();
        for v in stars.get(key).map_err(err)? {
            if v.map_err(err)?.value() == user {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// how many saves, and how many classifiers, a user has starred
    pub fn stars_given_all(&self, user: &str) -> Result<(u64, u64)> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_multimap_table(STARRED).map_err(err)?;
        let (mut saves, mut cls) = (0, 0);
        for v in t.get(user.to_lowercase().as_str()).map_err(err)? {
            if v.map_err(err)?.value().starts_with(CLS) {
                cls += 1;
            } else {
                saves += 1;
            }
        }
        Ok((saves, cls))
    }

    /// how many saves a user has starred
    pub fn stars_given(&self, user: &str) -> Result<u64> {
        Ok(self.stars_given_all(user)?.0)
    }

    /// counts a download; true the first time this downloader takes this save
    pub fn count_download(&self, id: &str, who: &str) -> Result<bool> {
        self.count_download_in::<Save>(SAVES, id, id, who)
    }

    fn count_download_in<T: Counted>(
        &self,
        table: TableDefinition<&str, &[u8]>,
        id: &str,
        key: &str,
        who: &str,
    ) -> Result<bool> {
        let txn = self.db.begin_write().map_err(err)?;
        let first = {
            let mut t = txn.open_table(table).map_err(err)?;
            let mut s: T = match t.get(id).map_err(err)? {
                Some(v) => get(v.value())?,
                None => return Err("no such thing".into()),
            };
            let mut d = txn.open_multimap_table(DOWNLOADERS).map_err(err)?;
            let first = !d.insert(key, who).map_err(err)?;
            s.add_download();
            t.insert(id, put(&s)?.as_slice()).map_err(err)?;
            first
        };
        txn.commit().map_err(err)?;
        Ok(first)
    }

    // -------------------------------------------------------------- classifiers

    pub fn classifier(&self, id: &str) -> Result<Option<ClassifierRec>> {
        if !id_ok(id) {
            return Ok(None);
        }
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(CLASSIFIERS).map_err(err)?;
        t.get(id).map_err(err)?.map(|v| get(v.value())).transpose()
    }

    pub fn classifiers(&self) -> Result<Vec<ClassifierRec>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(CLASSIFIERS).map_err(err)?;
        let mut out = Vec::new();
        for row in t.iter().map_err(err)? {
            let (_, v) = row.map_err(err)?;
            out.push(get(v.value())?);
        }
        Ok(out)
    }

    pub fn put_classifier(&self, c: &ClassifierRec) -> Result<()> {
        if !id_ok(&c.id) {
            return Err("a classifier's id is letters and digits".into());
        }
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(CLASSIFIERS)
            .map_err(err)?
            .insert(c.id.as_str(), put(c)?.as_slice())
            .map_err(err)?;
        txn.commit().map_err(err)
    }

    pub fn update_classifier(&self, id: &str, f: impl FnOnce(&mut ClassifierRec)) -> Result<ClassifierRec> {
        let txn = self.db.begin_write().map_err(err)?;
        let c = {
            let mut t = txn.open_table(CLASSIFIERS).map_err(err)?;
            let mut c: ClassifierRec = match t.get(id).map_err(err)? {
                Some(v) => get(v.value())?,
                None => return Err("no such classifier".into()),
            };
            f(&mut c);
            t.insert(id, put(&c)?.as_slice()).map_err(err)?;
            c
        };
        txn.commit().map_err(err)?;
        Ok(c)
    }

    /// removes a classifier's record, its stars and downloads (the file is the caller's)
    pub fn remove_classifier(&self, id: &str) -> Result<Option<ClassifierRec>> {
        self.remove_in(CLASSIFIERS, id, &format!("{CLS}{id}"))
    }

    pub fn toggle_classifier_star(&self, id: &str, user: &str) -> Result<(bool, u64)> {
        self.toggle_star_in::<ClassifierRec>(CLASSIFIERS, id, &format!("{CLS}{id}"), user)
    }

    /// counts a download; true the first time this downloader takes this classifier
    pub fn count_classifier_download(&self, id: &str, who: &str) -> Result<bool> {
        self.count_download_in::<ClassifierRec>(CLASSIFIERS, id, &format!("{CLS}{id}"), who)
    }

    // -------------------------------------------------------------- settings

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(META).map_err(err)?;
        Ok(t.get(key).map_err(err)?.map(|v| v.value().to_string()))
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.open_table(META).map_err(err)?.insert(key, value).map_err(err)?;
        txn.commit().map_err(err)
    }

    // -------------------------------------------------------------- invites

    pub fn put_invite(&self, hash: &[u8], i: &Invite) -> Result<()> {
        self.put_bytes(INVITES, hash, &put(i)?)
    }

    pub fn invite(&self, hash: &[u8]) -> Result<Option<Invite>> {
        self.get_bytes(INVITES, hash)
    }

    /// every invite, newest first
    pub fn invites(&self) -> Result<Vec<Invite>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(INVITES).map_err(err)?;
        let mut out: Vec<Invite> = Vec::new();
        for row in t.iter().map_err(err)? {
            let (_, v) = row.map_err(err)?;
            out.push(get(v.value())?);
        }
        out.sort_by_key(|i| std::cmp::Reverse(i.created));
        Ok(out)
    }

    /// uses an invite for `user` if it's still good, in one transaction, so two people
    /// can't both take its last use
    pub fn use_invite(&self, hash: &[u8], user: &str) -> Result<Invite> {
        let txn = self.db.begin_write().map_err(err)?;
        let inv = {
            let mut t = txn.open_table(INVITES).map_err(err)?;
            let mut inv: Invite = match t.get(hash).map_err(err)? {
                Some(v) => get(v.value())?,
                None => return Err("that invite code isn't known here".into()),
            };
            if let Some(why) = inv.spent() {
                return Err(why.into());
            }
            inv.used_by.push(user.to_string());
            t.insert(hash, put(&inv)?.as_slice()).map_err(err)?;
            inv
        };
        txn.commit().map_err(err)?;
        Ok(inv)
    }

    /// changes the invite with this id in place
    pub fn update_invite(&self, id: &str, f: impl FnOnce(&mut Invite)) -> Result<Invite> {
        let txn = self.db.begin_write().map_err(err)?;
        let inv = {
            let mut t = txn.open_table(INVITES).map_err(err)?;
            let mut found = None;
            for row in t.iter().map_err(err)? {
                let (k, v) = row.map_err(err)?;
                let inv: Invite = get(v.value())?;
                if !id.is_empty() && inv.id == id {
                    found = Some((k.value().to_vec(), inv));
                    break;
                }
            }
            let Some((k, mut inv)) = found else {
                return Err("no such invite".into());
            };
            f(&mut inv);
            t.insert(k.as_slice(), put(&inv)?.as_slice()).map_err(err)?;
            inv
        };
        txn.commit().map_err(err)?;
        Ok(inv)
    }

    // -------------------------------------------------------------- events

    pub fn log(&self, user: &str, what: impl Into<String>, save: &str) -> Result<()> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut t = txn.open_table(EVENTS).map_err(err)?;
            let next = t.last().map_err(err)?.map(|(k, _)| k.value() + 1).unwrap_or(0);
            let e = Event {
                at: Utc::now(),
                user: user.to_string(),
                what: what.into(),
                save: save.to_string(),
            };
            t.insert(next, put(&e)?.as_slice()).map_err(err)?;
        }
        txn.commit().map_err(err)
    }

    /// the newest events first
    pub fn events(&self, n: usize) -> Result<Vec<Event>> {
        let txn = self.db.begin_read().map_err(err)?;
        let t = txn.open_table(EVENTS).map_err(err)?;
        let mut out = Vec::new();
        for row in t.iter().map_err(err)?.rev().take(n) {
            let (_, v) = row.map_err(err)?;
            out.push(get(v.value())?);
        }
        Ok(out)
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("anend-hub-test-{}", rand::random::<u64>()));
        (Store::open(&dir).unwrap(), dir)
    }

    #[test]
    fn users_stars_downloads() {
        let (s, dir) = temp();
        let a = s
            .add_user(User {
                name: "Saul".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(a.number, 1);
        assert!(
            s.add_user(User {
                name: "saul".into(),
                ..Default::default()
            })
            .is_err()
        );
        let b = s
            .add_user(User {
                name: "ann".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(b.number, 2);
        assert_eq!(s.user("SAUL").unwrap().unwrap().name, "Saul");
        s.add_points("saul", 50).unwrap();
        assert_eq!(s.user("saul").unwrap().unwrap().points, 50);

        s.put_save(&Save {
            id: "x1".into(),
            name: "psalms".into(),
            author: "Saul".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.toggle_star("x1", "ann").unwrap(), (true, 1));
        assert!(s.has_starred("x1", "Ann").unwrap());
        assert_eq!(s.stars_given("ann").unwrap(), 1);
        assert_eq!(s.toggle_star("x1", "ann").unwrap(), (false, 0));
        s.toggle_star("x1", "ann").unwrap();
        assert!(s.count_download("x1", "ann").unwrap());
        assert!(!s.count_download("x1", "ann").unwrap());
        let x = s.save("x1").unwrap().unwrap();
        assert_eq!((x.stars, x.downloads), (1, 2));
        // a classifier's stars and downloads sit beside the saves', apart
        s.put_classifier(&ClassifierRec {
            id: "x1".into(),
            name: "kinds".into(),
            author: "Saul".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.toggle_classifier_star("x1", "ann").unwrap(), (true, 1));
        assert!(s.has_starred("c:x1", "ann").unwrap());
        assert_eq!(s.stars_given_all("ann").unwrap(), (1, 1));
        assert!(s.count_classifier_download("x1", "ann").unwrap());
        assert_eq!(s.save("x1").unwrap().unwrap().downloads, 2);
        assert!(
            s.put_classifier(&ClassifierRec {
                id: "../x".into(),
                ..Default::default()
            })
            .is_err()
        );
        assert!(s.classifier("../x").unwrap().is_none());
        assert!(s.classifier_path("a/b").is_none());
        s.remove_classifier("x1").unwrap();
        assert_eq!(s.stars_given_all("ann").unwrap(), (1, 0));
        assert!(s.has_starred("x1", "ann").unwrap());
        s.remove_save("x1").unwrap();
        assert!(s.save("x1").unwrap().is_none());
        assert_eq!(s.stars_given("ann").unwrap(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sessions_and_links_expire() {
        let (s, dir) = temp();
        let soon = Utc::now() + chrono::Duration::minutes(5);
        let past = Utc::now() - chrono::Duration::minutes(5);
        s.put_session(
            b"a",
            &Session {
                user: "x".into(),
                expires: soon,
            },
        )
        .unwrap();
        s.put_session(
            b"b",
            &Session {
                user: "x".into(),
                expires: past,
            },
        )
        .unwrap();
        assert!(s.session(b"a").unwrap().is_some());
        assert!(s.session(b"b").unwrap().is_none());
        s.drop_sessions_of("X").unwrap();
        assert!(s.session(b"a").unwrap().is_none());
        s.put_link(
            b"l",
            &Link {
                user: "x".into(),
                expires: soon,
            },
        )
        .unwrap();
        assert!(s.take_link(b"l").unwrap().is_some());
        assert!(s.take_link(b"l").unwrap().is_none());
        s.log("x", "joined", "").unwrap();
        s.log("x", "uploaded", "x1").unwrap();
        assert_eq!(s.events(10).unwrap()[0].what, "uploaded");
        assert_eq!(unhex(&hex(&[1, 255, 16])).unwrap(), vec![1, 255, 16]);
        std::fs::remove_dir_all(dir).ok();
    }
}
