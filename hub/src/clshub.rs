//! Sharing classifiers on the hub, beside the saves: a `.anendcls` module (core's
//! `clsmodule`) is uploaded, read strictly off the async threads, filed under
//! `classifiers/{id}.anendcls` and queued for review like a save; people browse, star
//! and download them, and the web app takes one into its open save or shares one from
//! it. Points and badges come the same way as for saves.

use chrono::Utc;
use serde_json::{Value, json};
use std::path::Path;

use anend_core::clsmodule;

use crate::app::Workspace;
use crate::auth;
use crate::game;
use crate::hub::{Details, Hub, PAGE, Sort, human_size};
use crate::store::{ClassifierRec, Role, Status, User};

/// the largest module the hub reads, whatever the upload limit
pub const MODULE_CAP: u64 = 64 << 20;

/// a module read and summed up, off the async threads; a bad file (or a panic while
/// reading it) is an error, never a crash
pub async fn read_module(bytes: Vec<u8>, max: u64) -> Result<(Vec<u8>, clsmodule::Module), String> {
    if bytes.len() as u64 > max {
        return Err(format!(
            "that file is {}; the hub takes classifiers up to {}",
            human_size(bytes.len() as u64),
            human_size(max)
        ));
    }
    tokio::task::spawn_blocking(move || {
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| clsmodule::read(&bytes, max as usize)));
        match read {
            Ok(Ok(m)) => Ok((bytes, m)),
            Ok(Err(e)) => Err(format!("that isn't a classifier module the hub can read: {e}")),
            Err(_) => Err("that isn't a classifier module the hub can read".to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

impl Hub {
    /// the largest classifier module taken
    pub fn classifier_max(&self) -> u64 {
        self.config.max_upload.min(MODULE_CAP)
    }

    /// takes an uploaded module file in (see `accept_classifier_bytes`)
    pub async fn accept_classifier(
        &self,
        user: &User,
        words: (&str, &str, &str),
        file: &Path,
        replace: Option<&ClassifierRec>,
    ) -> Result<ClassifierRec, String> {
        if !Hub::active(user) {
            return Err("your account isn't active yet".into());
        }
        let max = self.classifier_max();
        let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
        if size > max {
            return Err(format!(
                "that file is {}; the hub takes classifiers up to {}",
                human_size(size),
                human_size(max)
            ));
        }
        let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
        self.accept_classifier_bytes(user, words, bytes, replace).await
    }

    /// takes a module in: reads it strictly, files it, and queues it for review (or lets
    /// it straight in for trusted users). `words` are its name, description and tags; an
    /// empty name or description comes from the module. `replace` is a new version.
    pub async fn accept_classifier_bytes(
        &self,
        user: &User,
        words: (&str, &str, &str),
        bytes: Vec<u8>,
        replace: Option<&ClassifierRec>,
    ) -> Result<ClassifierRec, String> {
        if !Hub::active(user) {
            return Err("your account isn't active yet".into());
        }
        let (bytes, module) = read_module(bytes, self.classifier_max()).await?;
        let sum = clsmodule::summary(&module);
        let (name, description, tags) = words;
        let name = if name.trim().is_empty() {
            sum.name.as_str()
        } else {
            name
        };
        let description = if description.trim().is_empty() {
            sum.description.chars().take(4000).collect()
        } else {
            description.to_string()
        };
        let details = Details::new(name, &description, tags).map_err(|e| e.replace("the save", "the classifier"))?;
        let trusted = matches!(user.role, Role::Trusted | Role::Admin);
        let now = Utc::now();
        let current = match replace {
            Some(old) => Some(self.store.classifier(&old.id)?.ok_or("that classifier is gone")?),
            None => None,
        };
        let mut c = match current.as_ref() {
            Some(old) => ClassifierRec {
                version: old.version + 1,
                updated: now,
                note: String::new(),
                ..old.clone()
            },
            None => ClassifierRec {
                id: self.fresh_classifier_id()?,
                author: user.name.clone(),
                created: now,
                updated: now,
                version: 1,
                ..Default::default()
            },
        };
        let main = module.manifest.main();
        c.name = details.name;
        c.description = details.description;
        c.tags = details.tags;
        c.module = sum.name;
        c.classes = sum.classes;
        c.head = sum.head;
        c.preset = sum.preset;
        c.weights = sum.weights;
        c.accuracy = sum.accuracy;
        c.macro_f1 = sum.macro_f1;
        c.examples = sum.examples;
        c.measured_on = sum.measured_on;
        c.upstream = sum.upstream;
        c.hash = main.hash.clone();
        c.size = bytes.len() as u64;
        c.status = if trusted { Status::Approved } else { Status::Pending };
        let path = self.store.classifier_path(&c.id).ok_or("a bad id")?;
        // written beside, then moved in, so a half written file is never served
        let incoming = self.store.incoming();
        std::fs::create_dir_all(&incoming).map_err(|e| e.to_string())?;
        let tmp = incoming.join(format!("{}.anendcls", auth::secret()));
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        if let Err(e) = std::fs::rename(&tmp, &path) {
            std::fs::remove_file(&tmp).ok();
            return Err(e.to_string());
        }
        if c.status == Status::Approved && !c.rewarded {
            c.rewarded = true;
            self.store.add_points(&c.author, game::APPROVED)?;
        }
        self.store.put_classifier(&c)?;
        let what = match (replace.is_some(), c.status) {
            (false, Status::Approved) => "shared a classifier",
            (false, _) => "sent in a classifier",
            (true, _) => "updated a classifier",
        };
        self.store.log(&user.name, what, &c.key())?;
        Ok(c)
    }

    fn fresh_classifier_id(&self) -> Result<String, String> {
        loop {
            let id = auth::short_id();
            if self.store.classifier(&id)?.is_none() {
                return Ok(id);
            }
        }
    }

    /// approved classifiers matching a search (words in its name, description, author,
    /// classes or tags; `#tag` for a tag), sorted, one page; and whether there's more
    pub fn find_classifiers(&self, q: &str, sort: Sort, page: usize) -> Result<(Vec<ClassifierRec>, bool), String> {
        let q = q.trim().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let tag = q.strip_prefix('#');
        let mut all: Vec<ClassifierRec> = self
            .store
            .classifiers()?
            .into_iter()
            .filter(|c| c.status == Status::Approved)
            .filter(|c| match tag {
                Some(t) => c.tags.iter().any(|x| x == t),
                None => words.iter().all(|w| {
                    c.name.to_lowercase().contains(w)
                        || c.description.to_lowercase().contains(w)
                        || c.author.to_lowercase().contains(w)
                        || c.module.to_lowercase().contains(w)
                        || c.classes.iter().any(|x| x.to_lowercase().contains(w))
                        || c.tags.iter().any(|t| t.contains(w))
                }),
            })
            .filter(|c| sort != Sort::Featured || c.featured)
            .collect();
        match sort {
            Sort::New | Sort::Featured => all.sort_by_key(|c| std::cmp::Reverse(c.updated)),
            Sort::Stars => all.sort_by(|a, b| b.stars.cmp(&a.stars).then(b.updated.cmp(&a.updated))),
            Sort::Downloads => all.sort_by(|a, b| b.downloads.cmp(&a.downloads).then(b.updated.cmp(&a.updated))),
        }
        let start = page.saturating_mul(PAGE);
        let more = all.len() > start + PAGE;
        Ok((all.into_iter().skip(start).take(PAGE).collect(), more))
    }

    /// stars or unstars a classifier; its author gets points for it
    pub fn star_classifier(&self, c: &ClassifierRec, user: &User) -> Result<(bool, u64), String> {
        if !Hub::active(user) {
            return Err("your account isn't active yet".into());
        }
        if c.author.eq_ignore_ascii_case(&user.name) {
            return Err("that's your own classifier".into());
        }
        if c.status != Status::Approved {
            return Err("it isn't in yet".into());
        }
        let (on, n) = self.store.toggle_classifier_star(&c.id, &user.name)?;
        let sign = if on { 1 } else { -1 };
        self.store.add_points(&c.author, sign * game::STAR)?;
        self.store.add_points(&user.name, sign * game::CURATE)?;
        if on {
            self.store.log(&user.name, "starred", &c.key())?;
        }
        Ok((on, n))
    }

    /// counts a download (or a take into a save); the author gets a point the first time
    /// each person takes it
    pub fn classifier_downloaded(&self, c: &ClassifierRec, who: &str) -> Result<(), String> {
        let first = self.store.count_classifier_download(&c.id, who)?;
        if first && !c.author.eq_ignore_ascii_case(who) {
            self.store.add_points(&c.author, game::DOWNLOAD)?;
        }
        Ok(())
    }

    /// an admin's verdict on a classifier: approve, reject (with a note), feature,
    /// unfeature, remove
    pub fn review_classifier(&self, admin: &User, id: &str, action: &str, note: &str) -> Result<String, String> {
        let c = self.store.classifier(id)?.ok_or("no such classifier")?;
        let key = c.key();
        match action {
            "approve" => {
                self.store.update_classifier(id, |x| {
                    x.status = Status::Approved;
                    x.note.clear();
                    x.rewarded = true;
                })?;
                if !c.rewarded {
                    self.store.add_points(&c.author, game::APPROVED)?;
                }
                self.store.log(&admin.name, "approved", &key)?;
                Ok(format!("let “{}” in", c.name))
            }
            "reject" => {
                self.store.update_classifier(id, |x| {
                    x.status = Status::Rejected;
                    x.featured = false;
                    x.note = note.trim().chars().take(500).collect();
                })?;
                self.store.log(&admin.name, "turned down", &key)?;
                Ok(format!("turned down “{}”", c.name))
            }
            "feature" | "unfeature" => {
                let on = action == "feature";
                if on && c.status != Status::Approved {
                    return Err("let it in before featuring it".into());
                }
                self.store.update_classifier(id, |x| {
                    x.featured = on;
                    if on {
                        x.feature_rewarded = true;
                    }
                })?;
                if on && !c.feature_rewarded {
                    self.store.add_points(&c.author, game::FEATURED)?;
                }
                if on {
                    self.store.log(&admin.name, "featured", &key)?;
                }
                Ok(format!("{} “{}”", if on { "featured" } else { "unfeatured" }, c.name))
            }
            "remove" => {
                self.remove_classifier(id)?;
                self.store.log(&admin.name, "removed a classifier", "")?;
                Ok(format!("removed “{}”", c.name))
            }
            _ => Err("unknown action".into()),
        }
    }

    /// deletes a shared classifier and its file
    pub fn remove_classifier(&self, id: &str) -> Result<(), String> {
        self.store.remove_classifier(id)?;
        if let Some(p) = self.store.classifier_path(id) {
            std::fs::remove_file(p).ok();
        }
        Ok(())
    }
}

/// whether a user may change or delete a shared classifier: its author or an admin
pub fn may_edit(user: Option<&User>, c: &ClassifierRec) -> bool {
    user.is_some_and(|u| Hub::active(u) && (u.is_admin() || u.name.eq_ignore_ascii_case(&c.author)))
}

/// takes a hub classifier's module into a workspace's open save: classes go to lists by
/// name (or `lists`: {class: key}), and `as` names it. Runs on a blocking thread.
pub fn take_into(ws: &Workspace, file: &Path, max: u64, args: &Value) -> Result<Value, String> {
    let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    if size > max {
        return Err("that module is too big".into());
    }
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let m = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| clsmodule::read(&bytes, max as usize)))
        .map_err(|_| "that module can't be read".to_string())??;
    ws.room(false)?;
    let (open, store) = ws.store()?;
    let mut done = crate::classify::import_opts_and_take(&store, &m, args)?;
    done["save"] = json!(open);
    Ok(done)
}

/// a classifier from a workspace's open save as module bytes, everything it builds on
/// bundled. Runs on a blocking thread.
pub fn export_from(ws: &Workspace, name: &str, description: &str, author: &str) -> Result<Vec<u8>, String> {
    let (_, store) = ws.store()?;
    store.export_module(
        name,
        &clsmodule::ExportOpts {
            description: description.chars().take(4000).collect(),
            author: author.to_string(),
            upstream: true,
        },
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::app::call as app_call;
    use crate::auth::Limiter;
    use crate::hub::Config;
    use crate::store::{Standing, Store};

    fn hub() -> (Hub, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("anend-hubcls-{}", rand::random::<u64>()));
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

    /// a workspace with a trained classifier "kind" telling fruit from rocks
    pub fn trained(dir: &Path) -> Workspace {
        let ws = Workspace::new(dir.to_path_buf());
        let c = |op: &str, a: Value| app_call(&ws, op, &a).unwrap();
        let key = |t: &str| {
            c("mem.saturate", json!({ "text": t }))["key"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let (fruit, rocks) = (key("fruit"), key("rocks"));
        let f = ["apple", "banana", "cherry", "mango", "peach", "plum", "grape", "melon"];
        let r = [
            "rock", "stone", "granite", "basalt", "flint", "slate", "quartz", "shale",
        ];
        for i in 0..20 {
            let a = key(&format!("{} {} {}", f[i % 8], f[(i * 3 + 1) % 8], f[(i / 8 + 4) % 8]));
            let b = key(&format!("{} {} {}", r[i % 8], r[(i * 5 + 2) % 8], r[(i / 8 + 4) % 8]));
            c("lists.enlist", json!({ "list": fruit, "member": a }));
            c("lists.enlist", json!({ "list": rocks, "member": b }));
        }
        c(
            "classify.create",
            json!({ "name": "kind", "classes": [fruit, rocks], "preset": "tiny" }),
        );
        c(
            "classify.train",
            json!({ "name": "kind", "epochs": 6, "threads": 2, "wait": true }),
        );
        ws
    }

    #[tokio::test]
    async fn share_review_take_predict() {
        let (hub, dir) = hub();
        let saul = user(&hub, "saul", Role::Member);
        let ann = user(&hub, "ann", Role::Member);
        let admin = user(&hub, "root", Role::Admin);
        let ws = trained(&dir.join("saul"));
        let bytes = export_from(&ws, "kind", "fruit or rocks", "saul").unwrap();

        let c = hub
            .accept_classifier_bytes(&saul, ("", "", "nature, test"), bytes.clone(), None)
            .await
            .unwrap();
        assert_eq!(c.status, Status::Pending);
        assert_eq!((c.name.as_str(), c.description.as_str()), ("kind", "fruit or rocks"));
        assert_eq!(c.classes, ["fruit", "rocks"]);
        assert_eq!(c.preset, "tiny");
        assert!(c.accuracy.is_some() && c.weights > 0);
        assert!(hub.store.classifier_path(&c.id).unwrap().exists());
        assert!(hub.find_classifiers("", Sort::New, 0).unwrap().0.is_empty());
        assert!(hub.star_classifier(&c, &ann).is_err(), "not in yet");

        hub.review_classifier(&admin, &c.id, "approve", "").unwrap();
        assert_eq!(hub.store.user("saul").unwrap().unwrap().points, game::APPROVED);
        hub.review_classifier(&admin, &c.id, "approve", "").unwrap();
        assert_eq!(hub.store.user("saul").unwrap().unwrap().points, game::APPROVED);
        assert_eq!(hub.find_classifiers("rocks", Sort::New, 0).unwrap().0.len(), 1);
        assert_eq!(hub.find_classifiers("#nature", Sort::Stars, 0).unwrap().0.len(), 1);
        assert!(hub.find_classifiers("nothing", Sort::New, 0).unwrap().0.is_empty());

        let c = hub.store.classifier(&c.id).unwrap().unwrap();
        assert!(hub.star_classifier(&c, &saul).is_err());
        assert_eq!(hub.star_classifier(&c, &ann).unwrap(), (true, 1));
        hub.classifier_downloaded(&c, "ann").unwrap();
        hub.classifier_downloaded(&c, "ann").unwrap();
        assert_eq!(
            hub.store.user("saul").unwrap().unwrap().points,
            game::APPROVED + game::STAR + game::DOWNLOAD
        );
        // the save stars aren't the classifier's
        assert_eq!(hub.store.stars_given_all("ann").unwrap(), (0, 1));

        // taken into another save, where it predicts
        let other = Workspace::new(dir.join("ann"));
        let file = hub.store.classifier_path(&c.id).unwrap();
        let took = take_into(&other, &file, hub.classifier_max(), &json!({})).unwrap();
        assert_eq!(took["name"], "kind", "{took}");
        let p = app_call(
            &other,
            "classify.predict",
            &json!({ "name": "kind", "texts": ["granite slate flint", "plum peach apple"] }),
        )
        .unwrap();
        assert_eq!(p["results"][0]["class"], "rocks", "{p}");
        assert_eq!(p["results"][1]["class"], "fruit", "{p}");
        // its classes became lists of the same names there, new ones
        let lists = took["lists"].as_array().unwrap();
        assert_eq!(lists.len(), 2, "{took}");
        assert!(lists.iter().all(|l| l[2] == true), "{took}");
        assert_eq!(took["save"], "notes");

        // a new version keeps its stars and downloads; a trusted one goes straight in
        hub.manage(&admin, "saul", "trusted").unwrap();
        let saul = hub.store.user("saul").unwrap().unwrap();
        let v2 = hub
            .accept_classifier_bytes(&saul, ("kind 2", "", ""), bytes.clone(), Some(&c))
            .await
            .unwrap();
        assert_eq!(
            (v2.version, v2.status, v2.stars, v2.downloads),
            (2, Status::Approved, 1, 2)
        );

        hub.review_classifier(&admin, &c.id, "feature", "").unwrap();
        assert_eq!(hub.find_classifiers("", Sort::Featured, 0).unwrap().0.len(), 1);
        hub.review_classifier(&admin, &c.id, "reject", "too small").unwrap();
        let r = hub.store.classifier(&c.id).unwrap().unwrap();
        assert_eq!(
            (r.status, r.note.as_str(), r.featured),
            (Status::Rejected, "too small", false)
        );

        // only its author or an admin may change it
        assert!(may_edit(Some(&saul), &r));
        assert!(may_edit(Some(&admin), &r));
        assert!(!may_edit(Some(&ann), &r));
        assert!(!may_edit(None, &r));

        hub.review_classifier(&admin, &c.id, "remove", "").unwrap();
        assert!(!hub.store.classifier_path(&c.id).unwrap().exists());
        assert!(hub.store.classifier(&c.id).unwrap().is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn forged_and_corrupt_modules_are_refused() {
        let (hub, dir) = hub();
        let saul = user(&hub, "saul", Role::Trusted);
        let ws = trained(&dir.join("saul"));
        let good = export_from(&ws, "kind", "", "saul").unwrap();
        let mut bad: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"junk".to_vec(),
            b"ANENDCLM".to_vec(),
            good[..good.len() / 2].to_vec(),
        ];
        // one flipped byte anywhere breaks the checksum
        for at in [9, 20, good.len() / 2, good.len() - 1] {
            let mut b = good.clone();
            b[at] ^= 0x5a;
            bad.push(b);
        }
        // forged with the checksum fixed up: a huge manifest length, weights that don't
        // match their hash, a manifest that isn't JSON
        let reseal = |mut b: Vec<u8>| {
            let n = b.len() - 8;
            let h = anend_core::classify::math::fnv(0, &b[..n]);
            b[n..].copy_from_slice(&h.to_le_bytes());
            b
        };
        let mut f = good.clone();
        f[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        bad.push(reseal(f));
        let mut f = good.clone();
        let at = f.len() - 40;
        f[at] ^= 0x01;
        bad.push(reseal(f));
        let mut f = good.clone();
        f[16] = b'!';
        bad.push(reseal(f));
        for b in bad {
            let r = hub.accept_classifier_bytes(&saul, ("x", "", ""), b, None).await;
            assert!(r.is_err());
        }
        // too big for the hub
        let big = vec![0u8; (hub.classifier_max() + 1) as usize];
        let r = hub.accept_classifier_bytes(&saul, ("x", "", ""), big, None).await;
        assert!(r.unwrap_err().contains("up to"));
        // nothing filed, nothing left in the scratch place
        assert!(hub.store.classifiers().unwrap().is_empty());
        assert_eq!(std::fs::read_dir(dir.join("classifiers")).unwrap().count(), 0);
        // the good one still goes in
        assert!(
            hub.accept_classifier_bytes(&saul, ("x", "", ""), good, None)
                .await
                .is_ok()
        );
        // a waiting account can't share
        let new = hub
            .store
            .add_user(User {
                name: "new".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(
            hub.accept_classifier_bytes(&new, ("x", "", ""), b"x".to_vec(), None)
                .await
                .is_err()
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
