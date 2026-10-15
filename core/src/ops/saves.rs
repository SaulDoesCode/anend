//! A folder of saves (`<name>.redb`, each an entry graph) with one open, answering the
//! `saves.*`, `forge.*` and `hello` ops and handing the rest to [`super::call`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use serde_json::{Value, json};

use super::merge::{self, Effect, Filter};
use super::{Removed, Res, s};
use crate::graph::Graph;

/// most saves one forge call reads
pub const MAX_INGREDIENTS: usize = 32;

/// every graph this process has open, by file: redb holds a file once per process, so a
/// save that's still busy elsewhere (a training thread, another call) is shared, not
/// opened twice
fn open_graphs() -> &'static Mutex<HashMap<PathBuf, Weak<Graph>>> {
    static G: OnceLock<Mutex<HashMap<PathBuf, Weak<Graph>>>> = OnceLock::new();
    G.get_or_init(Default::default)
}

/// the graph in a file, shared with whoever has it open already; a file another holder
/// is letting go of is tried again for a moment
pub fn graph_at(path: &Path) -> Result<Arc<Graph>, String> {
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut tries = 0;
    loop {
        {
            let mut all = open_graphs().lock().unwrap_or_else(|e| e.into_inner());
            all.retain(|_, w| w.strong_count() > 0);
            if let Some(g) = all.get(&key).and_then(Weak::upgrade) {
                return Ok(g);
            }
            match Graph::open(path) {
                Ok(g) => {
                    let g = Arc::new(g);
                    let key = std::fs::canonicalize(path).unwrap_or(key.clone());
                    all.insert(key, Arc::downgrade(&g));
                    return Ok(g);
                }
                Err(e) if tries >= 40 => {
                    return Err(if e.contains("already open") {
                        "that save is busy (a training, or a call still running); try again in a moment".into()
                    } else {
                        e
                    });
                }
                Err(_) => {}
            }
        }
        tries += 1;
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// a save's name made safe for a file name
pub fn clean(name: &str) -> Result<String, String> {
    let n: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let n = n.trim_matches('.').trim().to_string();
    if n.is_empty() || n.chars().count() > 96 {
        Err("a save's name: letters, numbers, spaces, - _ and ., 1 to 96 of them".into())
    } else {
        Ok(n)
    }
}

fn save_name(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

pub struct Open {
    pub name: String,
    pub path: PathBuf,
    pub graph: Arc<Graph>,
}

/// a folder of saves and the open one
pub struct Saves {
    dir: PathBuf,
    /// the save made when there's none
    default: String,
    /// whether the open save's name is kept in `.open` and opened again next time
    remember: bool,
    /// whether `saves.backup` writes its two files into `backups/` (the desktop) or
    /// answers them (a server)
    backups_on_disk: bool,
    /// other files that belong to a save, `<name>.<extension>`: renamed, copied and
    /// deleted with it (the hub's classifiers)
    companions: Vec<String>,
    open: Mutex<Option<Open>>,
    pub removed: Removed,
}

impl Saves {
    pub fn new(dir: impl Into<PathBuf>, default: &str) -> Saves {
        Saves {
            dir: dir.into(),
            default: default.to_string(),
            remember: false,
            backups_on_disk: true,
            companions: Vec::new(),
            open: Mutex::new(None),
            removed: Default::default(),
        }
    }

    /// opens the save at `path` now (its folder becomes the saves folder)
    pub fn at(path: &Path) -> Result<Saves, String> {
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let saves = Saves::new(dir, &save_name(path));
        let graph = graph_at(path)?;
        *saves.open.lock().unwrap_or_else(|e| e.into_inner()) = Some(Open {
            name: save_name(path),
            path: path.to_path_buf(),
            graph,
        });
        Ok(saves)
    }

    /// keeps the open save's name in `.open`, and opens it first next time
    pub fn remembering(mut self) -> Saves {
        self.remember = true;
        self
    }

    /// answers `saves.backup` with the two documents instead of writing files
    pub fn backups_answered(mut self) -> Saves {
        self.backups_on_disk = false;
        self
    }

    pub fn with_companions(mut self, extensions: &[&str]) -> Saves {
        self.companions = extensions.iter().map(|e| e.to_string()).collect();
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self, name: &str) -> Result<PathBuf, String> {
        Ok(self.dir.join(format!("{}.redb", clean(name)?)))
    }

    /// a save's companion file (`<name>.<extension>`)
    pub fn companion(&self, name: &str, extension: &str) -> Result<PathBuf, String> {
        Ok(self.dir.join(format!("{}.{extension}", clean(name)?)))
    }

    /// the saves' names, alphabetically
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "redb"))
            .map(|p| save_name(&p))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    /// the open save, opening the last one (or making the default) the first time
    pub fn current(&self) -> Result<(String, Arc<Graph>), String> {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(o) = open.as_ref() {
            return Ok((o.name.clone(), o.graph.clone()));
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let last = self
            .remember
            .then(|| std::fs::read_to_string(self.dir.join(".open")).ok())
            .flatten();
        let name = last
            .map(|s| s.trim().to_string())
            .filter(|n| self.path(n).is_ok_and(|p| p.exists()))
            .or_else(|| self.path(&self.default).ok().filter(|p| p.exists()).map(|_| self.default.clone()))
            .or_else(|| self.names().into_iter().next())
            .unwrap_or_else(|| self.default.clone());
        let path = self.path(&name)?;
        let graph = graph_at(&path)?;
        *open = Some(Open {
            name: name.clone(),
            path,
            graph: graph.clone(),
        });
        Ok((name, graph))
    }

    /// the open save's file
    pub fn current_path(&self) -> Result<PathBuf, String> {
        let (name, _) = self.current()?;
        self.path(&name)
    }

    /// opens another save (made when it's new)
    pub fn switch(&self, name: &str) -> Result<(String, Arc<Graph>), String> {
        let name = clean(name)?;
        let path = self.path(&name)?;
        {
            let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
            if open.as_ref().is_some_and(|o| o.path == path) {
                let o = open.as_ref().expect("open");
                return Ok((o.name.clone(), o.graph.clone()));
            }
            // let go of the old one first
            *open = None;
            let graph = graph_at(&path)?;
            *open = Some(Open {
                name: name.clone(),
                path,
                graph: graph.clone(),
            });
        }
        self.removed.lock().unwrap_or_else(|e| e.into_inner()).clear();
        if self.remember {
            let _ = std::fs::write(self.dir.join(".open"), &name);
        }
        self.current()
    }

    /// lets go of a save if it's the open one
    pub fn close_if_open(&self, name: &str) -> bool {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.as_ref().is_some_and(|o| o.name.eq_ignore_ascii_case(name)) {
            *open = None;
            return true;
        }
        false
    }

    /// a save's graph: the open one, or another opened for the call
    pub fn graph(&self, name: &str) -> Result<(String, Arc<Graph>), String> {
        let (open, g) = self.current()?;
        if name.trim().is_empty() || name.eq_ignore_ascii_case(&open) {
            return Ok((open, g));
        }
        let path = self.path(name)?;
        if !path.exists() {
            return Err("there's no save by that name".into());
        }
        Ok((clean(name)?, graph_at(&path)?))
    }

    /// a name like `want` that no save has yet (`want 2`, `want 3`, ...)
    pub fn free_name(&self, want: &str) -> Result<String, String> {
        let mut n = clean(want)?;
        let mut i = 2;
        while self.path(&n)?.exists() {
            n = clean(&format!("{want} {i}"))?;
            i += 1;
        }
        Ok(n)
    }

    fn info(&self, name: &str, open: &str, g: &Graph) -> Value {
        let path = self.path(name).unwrap_or_default();
        let meta = std::fs::metadata(&path).ok();
        let is_open = name.eq_ignore_ascii_case(open);
        // the open one is counted; others only when nothing else has them open
        let counts = if is_open {
            g.view().ok().and_then(|v| super::counts(&v).ok())
        } else {
            None
        };
        json!({
            "name": name, "open": is_open,
            "notes": counts.map(|c| c.0), "memories": counts.map(|c| c.1),
            "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
            "modified": meta.and_then(|m| m.modified().ok())
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
        })
    }

    fn move_companions(&self, from: &str, to: &str, copy: bool) -> Result<(), String> {
        for ext in &self.companions {
            let a = self.companion(from, ext)?;
            if a.exists() {
                let b = self.companion(to, ext)?;
                if copy {
                    std::fs::copy(&a, &b).map_err(|e| e.to_string())?;
                } else {
                    std::fs::rename(&a, &b).map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }

    /// a new save from texts rows and, from a whole backup, the rest
    pub fn restore_into(&self, name: &str, texts: &str, other: Option<&str>) -> Result<(String, Value), String> {
        let n = self.free_name(name)?;
        let path = self.path(&n)?;
        let made = (|| {
            let g = graph_at(&path)?;
            let im = match other {
                Some(o) => g.restore(texts, o)?,
                None => g.import_texts(texts)?,
            };
            let v = g.view()?;
            let (notes, memories) = super::counts(&v)?;
            let lists = v
                .all_edges()?
                .iter()
                .filter(|e| e.1 == crate::graph::store::rel::MEMBER)
                .map(|e| e.0)
                .collect::<std::collections::HashSet<_>>()
                .len();
            Ok::<Value, String>(json!({ "save": n, "memories": memories, "notes": notes, "lists": lists,
                "replaced": im.replaced, "skipped": im.skipped, "borrowing": im.borrowing }))
        })();
        match made {
            Ok(v) => Ok((n, v)),
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                Err(e)
            }
        }
    }

    fn forge_contents(&self, args: &Value) -> Result<(Vec<Arc<Graph>>, merge::Contents), String> {
        let ingredients = args
            .get("ingredients")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if ingredients.len() > MAX_INGREDIENTS {
            return Err(format!("the forge takes at most {MAX_INGREDIENTS} saves at once"));
        }
        let mut graphs = Vec::new();
        let mut parts = Vec::new();
        for (i, ing) in ingredients.iter().enumerate() {
            let (_, g) = self.graph(s(ing, "save"))?;
            let c = merge::read(i, &g.view()?)?;
            parts.push((Effect::parse(s(ing, "effect")), c));
            graphs.push(g);
        }
        let f = args.get("filter").cloned().unwrap_or(Value::Null);
        let num = |k: &str| f.get(k).and_then(Value::as_i64);
        let flag = |k: &str| f.get(k).and_then(Value::as_bool).unwrap_or(true);
        let filter = Filter {
            words: s(&f, "words").to_string(),
            ordinal: (num("ordinal_from"), num("ordinal_to")),
            reversed: (num("reversed_from"), num("reversed_to")),
            memories: flag("memories"),
            notes: flag("notes"),
            lists: flag("lists"),
        };
        Ok((graphs, filter.apply(merge::combine(parts))))
    }

    fn forge_into(&self, target: &Graph, args: &Value) -> Result<merge::Written, String> {
        let (graphs, c) = self.forge_contents(args)?;
        let views = graphs.iter().map(|g| g.view()).collect::<Result<Vec<_>, _>>()?;
        let refs: Vec<&crate::graph::View> = views.iter().collect();
        target.write(|tx| merge::write(tx, &refs, &c))
    }

    /// one op of the app API: the saves' own here, the rest on the open save
    pub fn call(&self, op: &str, args: &Value) -> Res {
        if op == "hello" {
            let (open, g) = self.current()?;
            let (notes, memories) = super::counts(&g.view()?)?;
            return Ok(json!({ "save": open, "saves": self.names(), "counts": { "notes": notes, "memories": memories } }));
        }
        if op.starts_with("saves.") || op.starts_with("forge.") {
            return self.saves_call(op, args);
        }
        let (_, g) = self.current()?;
        super::call(&g, &self.removed, op, args)
    }

    fn saves_call(&self, op: &str, args: &Value) -> Res {
        let (open, g) = self.current()?;
        match op {
            "saves.list" => Ok(Value::Array(self.names().iter().map(|n| self.info(n, &open, &g)).collect())),
            "saves.open" => {
                let name = s(args, "name");
                if !self.path(name)?.exists() {
                    return Err("there's no save by that name".into());
                }
                let (n, _) = self.switch(name)?;
                Ok(json!({ "save": n }))
            }
            "saves.new" => {
                let name = clean(s(args, "name"))?;
                if self.path(&name)?.exists() {
                    return Err("there's already a save called that".into());
                }
                let (n, _) = self.switch(&name)?;
                Ok(json!({ "save": n }))
            }
            "saves.copy" => {
                let from = s(args, "name");
                let to = clean(s(args, "to")).or_else(|_| clean(&format!("{from} copy")))?;
                if self.path(&to)?.exists() {
                    return Err("there's already a save called that".into());
                }
                let (from, src) = self.graph(from)?;
                let (texts, other) = src.backup()?;
                drop(src);
                let path = self.path(&to)?;
                if let Err(e) = graph_at(&path).and_then(|g| g.restore(&texts, &other)) {
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
                self.move_companions(&from, &to, true)?;
                Ok(json!({ "save": to }))
            }
            "saves.rename" => {
                let from = s(args, "name").to_string();
                let to = clean(s(args, "to"))?;
                if self.path(&to)?.exists() {
                    return Err("there's already a save called that".into());
                }
                if !self.path(&from)?.exists() {
                    return Err("there's no save by that name".into());
                }
                let was_open = from.eq_ignore_ascii_case(&open);
                drop(g);
                self.close_if_open(&from);
                std::fs::rename(self.path(&from)?, self.path(&to)?).map_err(|e| e.to_string())?;
                self.move_companions(&from, &to, false)?;
                if was_open {
                    self.switch(&to)?;
                }
                Ok(json!({ "save": to }))
            }
            "saves.delete" => {
                let name = s(args, "name");
                if name.eq_ignore_ascii_case(&open) {
                    return Err("open another save before deleting this one".into());
                }
                let path = self.path(name)?;
                if !path.exists() {
                    return Err("there's no save by that name".into());
                }
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
                for ext in &self.companions {
                    let _ = std::fs::remove_file(self.companion(name, ext)?);
                }
                Ok(Value::Null)
            }
            "saves.backup" => {
                // weum's backup: the texts in one file, everything else in another
                let (name, graph) = self.graph(s(args, "name"))?;
                let (texts, other) = graph.backup()?;
                if !self.backups_on_disk {
                    let t: Value = serde_json::from_str(&texts).map_err(|e| e.to_string())?;
                    let o: Value = serde_json::from_str(&other).map_err(|e| e.to_string())?;
                    return Ok(json!({ "paths": [], "texts": t, "other": o }));
                }
                let dir = self.dir.join("backups");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let stamp = chrono::Local::now().format("%Y-%m-%d %H%M%S");
                let tp = dir.join(format!("{name} {stamp} texts.json"));
                let op = dir.join(format!("{name} {stamp} other.json"));
                std::fs::write(&tp, texts).map_err(|e| e.to_string())?;
                std::fs::write(&op, other).map_err(|e| e.to_string())?;
                Ok(json!({ "paths": [tp.display().to_string(), op.display().to_string()] }))
            }
            "saves.import_texts" => {
                // weum's all-of-it.json (or a backup's texts.json), with `other` a whole
                // backup: into a new save `name`, or (texts only) the open one when empty
                let data = s(args, "data");
                let other = args.get("other").and_then(|o| match o {
                    Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
                    Value::Object(_) => Some(o.to_string()),
                    _ => None,
                });
                let new = s(args, "name").trim();
                if new.is_empty() && other.is_none() {
                    let im = g.import_texts(data)?;
                    return Ok(json!({ "save": open, "memories": im.entries, "replaced": im.replaced,
                                      "skipped": im.skipped, "borrowing": im.borrowing }));
                }
                let want = if new.is_empty() { "imported" } else { new };
                if !new.is_empty() && self.path(new)?.exists() {
                    return Err("there's already a save called that".into());
                }
                Ok(self.restore_into(want, data, other.as_deref())?.1)
            }
            "saves.export" => {
                let (name, graph) = self.graph(s(args, "name"))?;
                super::bundle(&name, &graph)
            }
            "saves.import" => {
                // a bundle: a new save (its keys kept), or with `merge` into the open one
                // (a whole backup's entries get fresh keys where new; weum rows keep theirs)
                let b = args.get("bundle").cloned().unwrap_or(Value::Null);
                if !b.is_object() {
                    return Err("that isn't a bundle".into());
                }
                let (texts, other) = super::bundle_parts(&b)?;
                if args.get("merge").and_then(Value::as_bool).unwrap_or(false) {
                    let w = match other {
                        Some(o) => {
                            let tmp = Graph::in_memory()?;
                            tmp.restore(&texts, &o)?;
                            let v = tmp.view()?;
                            g.write(|tx| merge::merge(tx, &v))?
                        }
                        None => {
                            let im = g.import_texts(&texts)?;
                            merge::Written {
                                memories: im.entries,
                                ..Default::default()
                            }
                        }
                    };
                    return Ok(json!({ "merged": true, "memories": w.memories, "notes": w.notes, "lists": w.lists }));
                }
                let want = match s(args, "name") {
                    "" => match s(&b, "name") {
                        "" => "imported",
                        n => n,
                    },
                    n => n,
                };
                Ok(self.restore_into(want, &texts, other.as_deref())?.1)
            }

            // ---- the forge
            "forge.preview" => {
                let (_, c) = self.forge_contents(args)?;
                let sample: Vec<Value> = c
                    .items
                    .iter()
                    .filter(|i| !i.is_note())
                    .take(24)
                    .map(|i| {
                        let m = crate::graph::gematria::Measure::of(&i.text);
                        json!({ "text": i.text, "ordinal": m.ordinal, "reversed": m.reversed })
                    })
                    .collect();
                let titles: Vec<String> = c
                    .items
                    .iter()
                    .filter(|i| i.is_note())
                    .take(12)
                    .map(|i| crate::note::split(&i.text).0)
                    .collect();
                Ok(json!({ "memories": c.memories(), "notes": c.notes(), "lists": c.links.len(),
                           "sample": sample, "titles": titles }))
            }
            "forge.make" => {
                let name = clean(s(args, "name"))?;
                let path = self.path(&name)?;
                if path.exists() {
                    return Err("there's already a save called that".into());
                }
                let made = graph_at(&path).and_then(|out| self.forge_into(&out, args));
                match made {
                    Ok(w) => Ok(json!({ "save": name, "memories": w.memories, "notes": w.notes, "lists": w.lists })),
                    Err(e) => {
                        let _ = std::fs::remove_file(&path);
                        Err(e)
                    }
                }
            }
            "forge.pour" => {
                let w = self.forge_into(&g, args)?;
                Ok(json!({ "memories": w.memories, "notes": w.notes, "lists": w.lists }))
            }
            _ => Err(format!("there's no {op:?} here")),
        }
    }
}

impl Graph {
    /// whether a file is an entry graph save (read only; a file that needs repair is
    /// checked on a writable open). For checking uploads before opening them.
    pub fn is_save(path: &Path) -> bool {
        use redb::ReadableDatabase;
        let check = |txn: redb::ReadTransaction| {
            txn.open_table(crate::graph::store::META)
                .ok()
                .and_then(|t| t.get("format").ok().flatten().map(|_| ()))
                .is_some()
        };
        match redb::ReadOnlyDatabase::open(path) {
            Ok(db) => db.begin_read().is_ok_and(check),
            Err(_) => redb::Database::open(path).is_ok_and(|db| db.begin_read().is_ok_and(check)),
        }
    }
}
