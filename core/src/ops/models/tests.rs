use super::*;

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "anend-models-{tag}-{}-{}",
        std::process::id(),
        crate::graph::store::system_ms()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

const FRUIT: [&str; 8] = ["apple", "banana", "cherry", "mango", "peach", "plum", "grape", "melon"];
const ROCKS: [&str; 8] = [
    "rock", "stone", "granite", "basalt", "flint", "slate", "quartz", "shale",
];

/// a save with lists "fruit" and "rocks" of 20 texts each
fn save(ws: &Saves) {
    let c = |op: &str, a: Value| ws.call(op, &a).unwrap();
    let key = |t: &str| {
        c("mem.saturate", json!({ "text": t }))["key"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (fruit, rocks) = (key("fruit"), key("rocks"));
    for i in 0..20 {
        let a = key(&format!(
            "{} {} {}",
            FRUIT[i % 8],
            FRUIT[(i * 3 + 1) % 8],
            FRUIT[(i / 8 + 4) % 8]
        ));
        let b = key(&format!(
            "{} {} {}",
            ROCKS[i % 8],
            ROCKS[(i * 5 + 2) % 8],
            ROCKS[(i / 8 + 4) % 8]
        ));
        c("lists.enlist", json!({ "list": fruit, "member": a }));
        c("lists.enlist", json!({ "list": rocks, "member": b }));
    }
}

fn kind_spec() -> Value {
    json!({
        "name": "kind",
        "input": {"type": "text"},
        "trunk": {"preset": "test"},
        "heads": [{"name": "kind", "kind": "classify", "classes": ["fruit", "rock"]}],
        "data": [
            {"from": "list", "list": "fruit", "head": "kind", "class": "fruit"},
            {"from": "list", "list": "rocks", "head": "kind", "class": "rock"}
        ],
        "train": {"epochs": 30, "batch": 8, "lr": 0.003, "val_fraction": 0.0, "threads": 2}
    })
}

fn writer_spec() -> Value {
    json!({
        "name": "writer",
        "input": {"type": "text"},
        "trunk": {"preset": "test", "max_len": 32},
        "heads": [{"name": "next", "kind": "generate"}],
        "data": [{"from": "list", "list": "fruit", "head": "next"}],
        "critics": [{"model": "kind", "head": "kind", "want": {"class": "fruit"}}],
        "constraints": {"max_len": 40},
        "sampling": {"temperature": 1.0, "max_len": 20, "seed": 3},
        "train": {"epochs": 2, "batch": 8, "lr": 0.003, "val_fraction": 0.0, "threads": 2}
    })
}

fn train(ws: &Saves, name: &str) -> Value {
    let j = ws.call("models.train", &json!({ "name": name, "wait": true })).unwrap();
    assert_eq!(j["state"], "done", "{j}");
    j
}

#[test]
fn spec_models_live_in_the_save() {
    let dir = temp("calls");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    save(&ws);
    let c = |op: &str, a: Value| ws.call(op, &a);

    // made, listed, refused twice
    let made = c("models.create", json!({ "spec": kind_spec() })).unwrap();
    assert_eq!(made["name"], "kind");
    assert_eq!(made["trained"], false);
    assert!(made["source"].as_str().unwrap().contains("\"fruit\""));
    assert!(
        c("models.create", json!({ "spec": kind_spec() }))
            .unwrap_err()
            .contains("at name")
    );
    // the spec as text works too
    let text = writer_spec().to_string();
    c("models.create", json!({ "spec": text })).unwrap();
    assert_eq!(c("models.list", json!({})).unwrap().as_array().unwrap().len(), 2);

    // validation errors say where
    let check = |spec: Value| c("models.check", json!({ "spec": spec })).unwrap();
    let mut bad = kind_spec();
    bad["heads"][0]["classes"] = json!(["fruit"]);
    let e = check(bad.clone());
    assert_eq!(e["ok"], false);
    assert_eq!(e["path"], "heads[0].classes");
    assert!(e["message"].as_str().unwrap().contains("two classes"), "{e}");
    let err = c("models.create", json!({ "spec": bad })).unwrap_err();
    assert!(err.contains("at heads[0].classes"), "{err}");
    let mut bad = kind_spec();
    bad["data"][1]["class"] = json!("pebble");
    assert_eq!(check(bad)["path"], "data[1].class");
    let mut bad = kind_spec();
    bad["heads"][0]["kind"] = json!("sort");
    let e = check(bad);
    assert_eq!(e["path"], "heads[0].kind");
    assert!(e["message"].as_str().unwrap().contains("unknown variant"), "{e}");
    let mut bad = kind_spec();
    bad["heads"][0]["clases"] = json!(["a", "b"]);
    let e = check(bad);
    assert!(e["path"].as_str().unwrap().starts_with("heads[0]"), "{e}");
    assert!(e["message"].as_str().unwrap().contains("unknown field"), "{e}");
    let mut bad = kind_spec();
    bad["trunk"]["preset"] = json!("huge");
    assert_eq!(check(bad)["path"], "trunk.preset");
    let e = c("models.check", json!({ "spec": "{\"name\": \"x\", " })).unwrap();
    assert!(e["message"].as_str().unwrap().contains("doesn't read"), "{e}");
    assert_eq!(check(kind_spec())["ok"], true);

    // trained, in the background, then waited for
    let j = c("models.train", json!({ "name": "kind" })).unwrap();
    assert_eq!(j["state"], "running");
    assert!(
        c("models.update", json!({ "name": "kind", "spec": kind_spec() })).is_err() || {
            // it may have finished already
            c("models.job", json!({ "name": "kind" })).unwrap()["state"] != "running"
        }
    );
    let mut done = Value::Null;
    for _ in 0..3000 {
        done = c("models.job", json!({ "name": "kind" })).unwrap();
        if done["state"] != "running" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(done["state"], "done", "{done}");
    assert!(!done["losses"].as_array().unwrap().is_empty());
    assert_eq!(done["model"]["trained"], true);

    // run: the head's answer
    let run = |t: &str| c("models.run", json!({ "name": "kind", "text": t })).unwrap();
    let a = run("banana plum cherry");
    assert_eq!(a["trained"], true);
    assert_eq!(a["outputs"]["kind"]["kind"], "classes");
    assert_eq!(a["outputs"]["kind"]["top"], "fruit", "{a}");
    assert_eq!(run("granite slate flint")["outputs"]["kind"]["top"], "rock");

    // generate: the critic is the other model; the same seed, the same candidates
    train(&ws, "writer");
    let g = |seed: u64| {
        c(
            "models.generate",
            json!({ "name": "writer", "prompt": "", "candidates": 4, "seed": seed, "max_len": 16 }),
        )
        .unwrap()
    };
    let (a, b, other) = (g(7), g(7), g(8));
    assert_eq!(a["candidates"], b["candidates"]);
    assert_ne!(a["candidates"], other["candidates"]);
    let cands = a["candidates"].as_array().unwrap();
    assert_eq!(cands.len(), 4);
    for k in cands {
        assert!(k["text"].as_str().unwrap().len() <= 16 * 4);
        let s = k["scores"][0].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&s), "{k}");
    }
    assert_eq!(a["critics"][0]["model"], "kind");
    // a generator's run writes after the input
    let w = c("models.run", json!({ "name": "writer", "text": "app", "seed": 1 })).unwrap();
    assert_eq!(w["outputs"]["next"]["kind"], "generated");
    // a classifier of the save judges too: its head is its name
    let mut w2 = writer_spec();
    w2["name"] = json!("writer2");
    w2["critics"][0] = json!({"model": "nobody", "head": "x", "want": {"class": "y"}});
    c("models.create", json!({ "spec": w2 })).unwrap();
    let e = c("models.generate", json!({ "name": "writer2", "candidates": 2 })).unwrap_err();
    assert!(e.contains("no model or classifier"), "{e}");
    let lists = c("lists.stats", json!({})).unwrap()["lists"].clone();
    let keys: Vec<Value> = lists.as_array().unwrap().iter().map(|l| l["list"].clone()).collect();
    c(
        "classify.create",
        json!({ "name": "sorter", "classes": keys, "preset": "tiny" }),
    )
    .unwrap();
    let e = c("models.generate", json!({ "name": "writer2", "candidates": 2 })).unwrap_err();
    assert!(e.contains("no model or classifier"), "{e}");
    let mut w3 = writer_spec();
    w3["name"] = json!("writer3");
    w3["critics"][0] = json!({"model": "sorter", "head": "sorter", "want": {"class": "fruit"}});
    c("models.create", json!({ "spec": w3 })).unwrap();
    let e = c("models.generate", json!({ "name": "writer3", "candidates": 2 })).unwrap_err();
    assert!(e.contains("isn't trained"), "{e}");
    c("classify.train", json!({ "name": "sorter", "epochs": 2, "wait": true })).unwrap();
    let k = c("models.generate", json!({ "name": "writer3", "candidates": 2 })).unwrap();
    assert_eq!(k["candidates"].as_array().unwrap().len(), 2);
    assert!(k["candidates"][0]["scores"][0].as_f64().unwrap() > 0.0);
    c("models.delete", json!({ "name": "writer3" })).unwrap();
    // a spec model can't take a classifier's name
    let mut w4 = writer_spec();
    w4["name"] = json!("sorter");
    assert!(
        c("models.create", json!({ "spec": w4 }))
            .unwrap_err()
            .contains("classifier's name")
    );

    // an edit that keeps the shape keeps the weights; one that changes it drops them
    let mut s = kind_spec();
    s["train"]["epochs"] = json!(3);
    let u = c("models.update", json!({ "name": "kind", "spec": s })).unwrap();
    assert_eq!(u["weights_kept"], true);
    assert_eq!(u["trained"], true);
    let before = run("banana plum cherry");

    // across reopening, the models and their weights are there
    drop(ws);
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    let c = |op: &str, a: Value| ws.call(op, &a);
    let names: Vec<String> = c("models.list", json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["kind", "writer", "writer2"]);
    let again = c("models.run", json!({ "name": "kind", "text": "banana plum cherry" })).unwrap();
    assert_eq!(again["outputs"], before["outputs"]);
    assert_eq!(
        c(
            "models.generate",
            json!({ "name": "writer", "prompt": "", "candidates": 4, "seed": 7, "max_len": 16 })
        )
        .unwrap()["candidates"],
        a["candidates"]
    );

    // in a bundle, out and back into a new save
    let b = c("saves.export", json!({})).unwrap();
    assert_eq!(b["models"].as_array().unwrap().len(), 3);
    let got = c("saves.import", json!({ "bundle": b, "name": "copy" })).unwrap();
    assert_eq!(got["models"], 3);
    c("saves.open", json!({ "name": "copy" })).unwrap();
    let there = c("models.run", json!({ "name": "kind", "text": "banana plum cherry" })).unwrap();
    assert_eq!(there["outputs"], before["outputs"]);
    c("saves.open", json!({ "name": "notes" })).unwrap();

    // a shape change drops the weights
    let mut s = kind_spec();
    s["seed"] = json!(9);
    let u = c("models.update", json!({ "name": "kind", "spec": s })).unwrap();
    assert_eq!(u["weights_kept"], false);
    assert_eq!(u["trained"], false);
    // renamed by its spec
    let mut s = kind_spec();
    s["name"] = json!("sort");
    c("models.update", json!({ "name": "kind", "spec": s })).unwrap();
    assert!(c("models.get", json!({ "name": "kind" })).is_err());

    // deleted
    c("models.delete", json!({ "name": "sort" })).unwrap();
    assert!(c("models.get", json!({ "name": "sort" })).is_err());
    assert!(c("models.delete", json!({ "name": "sort" })).is_err());
    drop(ws);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cancel_writes_nothing() {
    let dir = temp("cancel");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    save(&ws);
    let mut s = kind_spec();
    s["train"]["epochs"] = json!(5000);
    ws.call("models.create", &json!({ "spec": s })).unwrap();
    ws.call("models.train", &json!({ "name": "kind" })).unwrap();
    ws.call("models.cancel", &json!({ "name": "kind" })).unwrap();
    let mut j = Value::Null;
    for _ in 0..3000 {
        j = ws.call("models.job", &json!({ "name": "kind" })).unwrap();
        if j["state"] != "running" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(j["state"], "cancelled", "{j}");
    let g = ws.call("models.get", &json!({ "name": "kind" })).unwrap();
    assert_eq!(g["trained"], false);
    drop(ws);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pretrained_folders_and_shared_limits() {
    let dir = temp("pre");
    let models = dir.join("pretrained");
    let ck = models.join("tiny-llama");
    std::fs::create_dir_all(&ck).unwrap();
    crate::classify::pretrained::synthetic_checkpoint(&ck, 4);
    let spec = |from: &str| {
        json!({
            "name": "smol",
            "from": {"pretrained": from},
            "input": {"type": "text"},
            "trunk": {"max_len": 32},
            "heads": [{"name": "next", "kind": "generate"}],
            "sampling": {"temperature": 1.0, "max_len": 8, "seed": 2}
        })
    };

    // one's own machine loads any folder, untrained (as the checkpoint has it) or tuned
    let mine = Saves::new(dir.join("mine"), "notes").with_models(Limits::NONE);
    mine.call("models.create", &json!({ "spec": spec(&ck.to_string_lossy()) }))
        .unwrap();
    let r = mine
        .call("models.run", &json!({ "name": "smol", "text": "hello" }))
        .unwrap();
    assert_eq!(r["trained"], false);
    assert_eq!(r["outputs"]["next"]["kind"], "generated");

    // a shared hub only its own folders, by name
    let shared = Saves::new(dir.join("shared"), "notes").with_models(Limits::shared());
    let e = shared
        .call("models.check", &json!({ "spec": spec(&ck.to_string_lossy()) }))
        .unwrap();
    assert_eq!(e["path"], "from.pretrained", "{e}");
    allow_pretrained(vec![models.clone()]);
    let offered = shared.call("models.pretrained", &json!({})).unwrap();
    assert_eq!(offered["folders"], json!(["tiny-llama"]));
    assert_eq!(
        shared
            .call("models.check", &json!({ "spec": spec("tiny-llama") }))
            .unwrap()["ok"],
        true
    );
    let e = shared
        .call("models.check", &json!({ "spec": spec("../mine") }))
        .unwrap();
    assert_eq!(e["path"], "from.pretrained");
    shared
        .call("models.create", &json!({ "spec": spec("tiny-llama") }))
        .unwrap();
    let r = shared
        .call("models.run", &json!({ "name": "smol", "text": "hello" }))
        .unwrap();
    assert_eq!(r["outputs"]["next"]["kind"], "generated");
    assert!(
        shared
            .call(
                "models.run",
                &json!({ "name": "smol", "text": "hello", "max_len": 100000 })
            )
            .unwrap_err()
            .contains("at most")
    );
    // too big a trunk, the GPU
    let big = json!({"name": "big", "input": {"type": "text"},
        "trunk": {"d_model": 4096, "layers": 48, "heads": 32, "head_dim": 128, "d_ff": 16384},
        "heads": [{"name": "next", "kind": "generate"}]});
    let e = shared.call("models.check", &json!({ "spec": big })).unwrap();
    assert_eq!(e["path"], "trunk", "{e}");
    let gpu = json!({"name": "g", "input": {"type": "text"}, "device": "gpu",
        "trunk": {"preset": "test"}, "heads": [{"name": "next", "kind": "generate"}]});
    assert_eq!(
        shared.call("models.check", &json!({ "spec": gpu })).unwrap()["path"],
        "device"
    );
    allow_pretrained(vec![]);
    drop((mine, shared));
    let _ = std::fs::remove_dir_all(dir);
}

/// a save from before records lived in it: its models file holds records and weights;
/// opening it moves the records into the save, and the trained model answers as before
#[test]
fn old_saves_move_their_specs_into_the_save() {
    let dir = temp("migrate");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    save(&ws);
    ws.call("models.create", &json!({ "spec": kind_spec() })).unwrap();
    train(&ws, "kind");
    let before = ws
        .call("models.run", &json!({ "name": "kind", "text": "banana plum cherry" }))
        .unwrap();
    let rec = ws.call("models.get", &json!({ "name": "kind" })).unwrap();
    drop(ws);
    // put the layout back as it was: the record in the models file, no fit row
    {
        let g = crate::graph::Graph::open(&dir.join("notes.redb")).unwrap();
        let bytes = g.spec_record("kind").unwrap().unwrap();
        g.delete_spec_record("kind").unwrap();
        let st = Store::open(&dir.join("notes.models")).unwrap();
        let txn = st.db.begin_write().unwrap();
        {
            use redb::ReadableTable;
            txn.open_table(crate::specs::SPECS)
                .unwrap()
                .insert("kind", bytes.as_slice())
                .unwrap();
            let mut f = txn.open_table(crate::specs::SPEC_FIT).unwrap();
            f.remove("kind").unwrap();
            assert!(f.get("kind").unwrap().is_none());
        }
        txn.commit().unwrap();
    }
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    let got = ws.call("models.get", &json!({ "name": "kind" })).unwrap();
    for k in ["source", "created", "modified", "version", "history", "trained"] {
        assert_eq!(got[k], rec[k], "{k}");
    }
    let after = ws
        .call("models.run", &json!({ "name": "kind", "text": "banana plum cherry" }))
        .unwrap();
    assert_eq!(after["outputs"], before["outputs"]);
    let (_, g) = ws.current().unwrap();
    assert!(g.spec_record("kind").unwrap().is_some(), "in the save now");
    drop((g, ws));
    let _ = std::fs::remove_dir_all(dir);
}

/// the records travel in the save: backups, copies, and merges (a clash comes in as
/// "<name> 2"; the same spec is the same model)
#[test]
fn specs_travel_with_the_save() {
    let dir = temp("travel");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    save(&ws);
    let c = |op: &str, a: Value| ws.call(op, &a).unwrap_or_else(|e| panic!("{op}: {e}"));
    c("models.create", json!({ "spec": kind_spec() }));
    c("models.create", json!({ "spec": writer_spec() }));
    // a backup's other.json carries them
    let b = c("saves.backup", json!({}));
    let saved = b["paths"][1].as_str().unwrap();
    let other: Value = serde_json::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
    assert_eq!(other["specs"].as_array().unwrap().len(), 2);
    c("saves.copy", json!({ "name": "notes", "to": "twin" }));
    c("saves.open", json!({ "name": "twin" }));
    assert_eq!(c("models.list", json!({})).as_array().unwrap().len(), 2);
    // a different "kind" in the twin, then the twin merged into notes
    let mut k = kind_spec();
    k["seed"] = json!(5);
    c("models.update", json!({ "name": "kind", "spec": k }));
    let bundle = c("saves.export", json!({}));
    c("saves.open", json!({ "name": "notes" }));
    let m = c("saves.import", json!({ "bundle": bundle, "merge": true }));
    assert_eq!(m["models"], 1, "{m}");
    let names: Vec<String> = c("models.list", json!({}))
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["kind", "kind 2", "writer"]);
    let k2 = c("models.get", json!({ "name": "kind 2" }));
    assert!(k2["source"].as_str().unwrap().contains("\"kind 2\""));
    drop(ws);
    let _ = std::fs::remove_dir_all(dir);
}

/// a picture model learns from the save's pictures by list, and reads one by id
#[test]
fn picture_models_learn_from_the_save() {
    use crate::classifiers::base64;
    let dir = temp("pictures");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    let c = |op: &str, a: Value| ws.call(op, &a).unwrap_or_else(|e| panic!("{op}: {e}"));
    let png = |seed: u32, warm: bool| {
        let im = image::RgbImage::from_fn(24, 24, |x, y| {
            let n = ((x * 7 + y * 13 + seed * 31) % 40) as u8;
            if warm {
                image::Rgb([200 + n / 2, 40 + n, 20])
            } else {
                image::Rgb([20, 40 + n, 200 + n / 2])
            }
        });
        let mut out = Vec::new();
        im.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    };
    let mut ids = vec![];
    for i in 0..12 {
        for (warm, list) in [(true, "warm"), (false, "cold")] {
            let p = c("pictures.add", json!({ "data": base64(&png(i, warm)), "list": list }));
            ids.push((warm, p["id"].as_str().unwrap().to_string()));
        }
    }
    let spec = json!({
        "name": "tone",
        "input": {"type": "image", "width": 16, "height": 16, "patch": 8, "codes": 8},
        "trunk": {"preset": "test"},
        "heads": [{"name": "tone", "kind": "classify", "classes": ["warm", "cold"]}],
        "data": [
            {"from": "pictures", "list": "warm", "head": "tone", "class": "warm"},
            {"from": "pictures", "list": "cold", "head": "tone", "class": "cold"}
        ],
        "train": {"epochs": 40, "batch": 8, "lr": 0.003, "val_fraction": 0.0, "threads": 2}
    });
    c("models.create", json!({ "spec": spec }));
    let j = train(&ws, "tone");
    assert_eq!(j["examples"], 24, "{j}");
    let top = |id: &str| {
        c("models.run", json!({ "name": "tone", "input": {"picture": id} }))["outputs"]["tone"]["top"].clone()
    };
    assert_eq!(top(&ids[0].1), "warm");
    assert_eq!(top(&ids[1].1), "cold");
    // pictures data needs a picture input, and a list that has pictures
    let mut bad = kind_spec();
    bad["data"][0] = json!({"from": "pictures", "list": "warm", "head": "kind", "class": "fruit"});
    let e = c("models.check", json!({ "spec": bad }));
    assert_eq!(e["path"], "data[0].from", "{e}");
    let mut empty = spec.clone();
    empty["name"] = json!("tone2");
    empty["data"][0]["list"] = json!("nothing");
    c("models.create", json!({ "spec": empty }));
    let e = ws
        .call("models.train", &json!({ "name": "tone2", "wait": true }))
        .unwrap_err();
    assert!(e.contains("no pictures in list \"nothing\""), "{e}");
    drop(ws);
    let _ = std::fs::remove_dir_all(dir);
}

/// std:learn: a program asks classifiers, spec models and searches through the save
#[test]
fn programs_classify_and_generate() {
    let dir = temp("learn");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    save(&ws);
    let c = |op: &str, a: Value| ws.call(op, &a);
    c("models.create", json!({ "spec": kind_spec() })).unwrap();
    train(&ws, "kind");
    c("models.create", json!({ "spec": writer_spec() })).unwrap();
    train(&ws, "writer");
    let lists = c("lists.stats", json!({})).unwrap()["lists"].clone();
    let keys: Vec<Value> = lists.as_array().unwrap().iter().map(|l| l["list"].clone()).collect();
    c(
        "classify.create",
        json!({ "name": "sorter", "classes": keys, "preset": "tiny" }),
    )
    .unwrap();
    c(
        "classify.train",
        json!({ "name": "sorter", "epochs": 30, "wait": true }),
    )
    .unwrap();

    let code = "std/learn.s.\n\
        {sorter} {banana plum cherry} class\n{sorted ~c~} >\n{trained} >?\n{p ~c/p~} >\n\
        {sorter} {granite slate} class {sorted ~c~} >\n\
        {kind} {granite slate} model\n{kind says ~m~} >\n\
        {writer} {} 2 generate\n{wrote ~g~} >\n\
        {cherry} search\n{found} #l {found ~=~} >\n\
        {sorter} {fruit} 3 likely\n{likely} #l {likely ~=~} >\n\
        {sorter} {granite slate} {rocks} feedback {taken} >?\n\
        {sorter} {fruit} 0.5 steer {steered} >?\n\
        {nobody} {x} class {no ~err~} >!";
    let r = c("s.run", json!({ "code": code })).unwrap();
    assert_eq!(r["err"], Value::Null, "{r}");
    let out = r["out"].as_str().unwrap();
    for want in [
        "sorted fruit",
        "trained",
        "sorted rocks",
        "kind says rock",
        "wrote ",
        "found 6",
        "likely 3",
        "taken",
        "steered",
        "no ",
    ] {
        assert!(out.contains(want), "{want:?} not in {out}");
    }
    let p: f64 = out.split("p ").nth(1).unwrap().lines().next().unwrap().parse().unwrap();
    assert!(p > 0.5 && p <= 1.0, "{out}");
    drop(ws);
    let _ = std::fs::remove_dir_all(dir);
}

/// taxonomies: classifiers in a tree, routing a text, marked by hand per node
#[test]
fn taxonomies_route_and_count_marks() {
    let dir = temp("taxonomy");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    save(&ws);
    let c = |op: &str, a: Value| ws.call(op, &a);
    let lists = c("lists.stats", json!({})).unwrap()["lists"].clone();
    let keys: Vec<Value> = lists.as_array().unwrap().iter().map(|l| l["list"].clone()).collect();
    for name in ["sorter", "again"] {
        c(
            "classify.create",
            json!({ "name": name, "classes": keys, "preset": "tiny" }),
        )
        .unwrap();
        c("classify.train", json!({ "name": name, "epochs": 30, "wait": true })).unwrap();
    }
    let tree = json!({ "name": "things", "root": { "classifier": "sorter", "routes": { "fruit": { "classifier": "again" } } } });

    // checked against the classifiers
    let bad =
        json!({ "name": "x", "root": { "classifier": "sorter", "routes": { "pebbles": { "classifier": "again" } } } });
    let e = c("taxonomy.save", json!({ "taxonomy": bad })).unwrap_err();
    assert!(e.contains("no class \"pebbles\""), "{e}");
    let bad =
        json!({ "name": "x", "root": { "classifier": "sorter", "routes": { "fruit": { "classifier": "nobody" } } } });
    let e = c("taxonomy.save", json!({ "taxonomy": bad })).unwrap_err();
    assert!(e.contains("after fruit") && e.contains("\"nobody\""), "{e}");

    let made = c("taxonomy.save", json!({ "taxonomy": tree })).unwrap();
    assert_eq!(made["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(made["nodes"][1]["path"], json!(["fruit"]));
    assert!(made["nodes"][0]["held_out"].is_number(), "{made}");
    assert_eq!(c("taxonomy.list", json!({})).unwrap()[0]["nodes"], 2);
    // a new one can't take a name that's there
    let e = c("taxonomy.save", json!({ "taxonomy": tree, "new": true })).unwrap_err();
    assert!(e.contains("already"), "{e}");
    // kept out of search and counts
    assert_eq!(c("mem.search", json!({ "q": "sorter" })).unwrap(), json!([]));

    // a fruit goes on to the second classifier; a rock stops at the first
    let r = c(
        "taxonomy.route",
        json!({ "name": "things", "text": "banana plum cherry" }),
    )
    .unwrap();
    assert_eq!(r["steps"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["steps"][1]["classifier"], "again");
    assert_eq!(r["class"], "fruit");
    assert_eq!(r["reached"], true);
    let r = c("taxonomy.route", json!({ "name": "things", "text": "granite slate" })).unwrap();
    assert_eq!(r["steps"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["class"], "rocks");

    // marks count per node and are the classifier's feedback
    let m = c(
        "taxonomy.mark",
        json!({ "name": "things", "text": "granite slate", "path": [], "right": true }),
    )
    .unwrap();
    assert_eq!(m["nodes"][0]["right"], 1);
    let m = c(
        "taxonomy.mark",
        json!({ "name": "things", "text": "banana plum", "path": ["fruit"], "right": false, "class": "rocks" }),
    )
    .unwrap();
    assert_eq!(m["nodes"][1]["wrong"], 1);
    assert_eq!(m["nodes"][1]["by_hand"], 0.0);
    let fb = c("classify.get", json!({ "name": "again" })).unwrap();
    assert_eq!(fb["feedback_count"], 2, "a no and a yes: {fb}");
    assert!(
        c(
            "taxonomy.mark",
            json!({ "name": "things", "text": "x", "path": ["rocks"], "right": true })
        )
        .is_err()
    );

    // an edit keeps the marks where the same classifier stands; a renamed one keeps them too
    let mut t2 = tree.clone();
    t2["name"] = json!("kinds");
    t2["about"] = json!("fruit, then fruit again");
    let s = c("taxonomy.save", json!({ "taxonomy": t2, "was": "things" })).unwrap();
    assert_eq!(s["nodes"][0]["right"], 1);
    assert_eq!(s["nodes"][1]["wrong"], 1);
    let mut t3 = t2.clone();
    t3["root"]["routes"]["fruit"]["classifier"] = json!("sorter");
    let s = c("taxonomy.save", json!({ "taxonomy": t3 })).unwrap();
    assert_eq!(s["nodes"][1]["wrong"], 0);
    assert!(c("taxonomy.get", json!({ "name": "things" })).is_err());

    // from s
    let r = c(
        "s.run",
        json!({ "code": "std/learn.s.\n{kinds} {banana plum cherry} route\n{ended in ~t~} >\n{t} {all} { then } l/ {~all~} >" }),
    )
    .unwrap();
    assert_eq!(r["err"], Value::Null, "{r}");
    let out = r["out"].as_str().unwrap();
    assert!(
        out.contains("ended in fruit") && out.contains("fruit then fruit"),
        "{out}"
    );

    c("taxonomy.delete", json!({ "name": "kinds" })).unwrap();
    assert_eq!(c("taxonomy.list", json!({})).unwrap(), json!([]));
    drop(ws);
    let _ = std::fs::remove_dir_all(dir);
}

/// pipes: a filter's texts with stacked effects, and actions done on demand
#[test]
fn pipes_stack_effects_and_act() {
    let dir = temp("pipes");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    save(&ws);
    let c = |op: &str, a: Value| ws.call(op, &a);
    let lists = c("lists.stats", json!({})).unwrap()["lists"].clone();
    let keys: Vec<Value> = lists.as_array().unwrap().iter().map(|l| l["list"].clone()).collect();
    c(
        "classify.create",
        json!({ "name": "sorter", "classes": keys, "preset": "tiny" }),
    )
    .unwrap();
    c(
        "classify.train",
        json!({ "name": "sorter", "epochs": 30, "wait": true }),
    )
    .unwrap();
    let fruity = json!({
        "name": "fruity",
        "from": { "op": "class", "classifier": "sorter", "class": "fruit", "top": true },
        "effects": [
            { "kind": "colour", "colour": "#ff8800" },
            { "kind": "label", "text": "fruit" },
            { "kind": "size", "scale": 2.0 },
            { "kind": "gather" }
        ],
        "actions": [ { "kind": "enlist", "list": "fruit bowl" }, { "kind": "tag", "tag": "ripe" } ]
    });
    let everything = json!({
        "name": "everything",
        "effects": [ { "kind": "label", "text": "seen" }, { "kind": "size", "scale": 0.5 }, { "kind": "colour_by", "classifier": "sorter" } ]
    });

    // checked
    let mut bad = fruity.clone();
    bad["effects"][0]["colour"] = json!("orange");
    assert!(
        c("pipes.save", json!({ "pipe": bad }))
            .unwrap_err()
            .contains("isn't a colour")
    );
    let mut bad = fruity.clone();
    bad["from"]["classifier"] = json!("nobody");
    assert!(c("pipes.save", json!({ "pipe": bad })).is_err());
    let mut bad = fruity.clone();
    bad["actions"] = json!([{ "kind": "run", "program": "nothing here" }]);
    assert!(
        c("pipes.save", json!({ "pipe": bad }))
            .unwrap_err()
            .contains("no s program")
    );

    c("pipes.save", json!({ "pipe": fruity, "new": true })).unwrap();
    c("pipes.save", json!({ "pipe": everything, "new": true })).unwrap();
    assert!(
        c("pipes.save", json!({ "pipe": everything, "new": true }))
            .unwrap_err()
            .contains("already")
    );
    assert_eq!(c("pipes.list", json!({})).unwrap().as_array().unwrap().len(), 2);
    assert_eq!(c("mem.search", json!({ "q": "fruity" })).unwrap(), json!([]));

    // a fruit and a rock, as a view would ask
    let find = |q: &str| {
        c("mem.search", json!({ "q": q })).unwrap()[0]["key"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (fruit, rock) = (find("apple"), find("granite"));
    let fx = c(
        "pipes.effects",
        json!({ "names": ["everything", "fruity", "gone"], "keys": [fruit, rock] }),
    )
    .unwrap();
    assert_eq!(fx["missing"], json!(["gone"]));
    let of = |k: &str| {
        fx["effects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == k)
            .cloned()
            .unwrap()
    };
    let (f, r) = (of(&fruit), of(&rock));
    assert_eq!(f["colour"], "#ff8800", "the later pipe's colour wins: {f}");
    assert_eq!(f["label"], "seen · fruit");
    assert_eq!(f["size"], 1.0, "sizes multiply: {f}");
    assert_eq!(f["gather"], "fruity");
    assert_eq!(f["pipes"], json!(["everything", "fruity"]));
    assert_eq!(r["label"], "seen");
    assert_eq!(r["size"], 0.5);
    assert!(
        r["colour"].as_str().unwrap().starts_with('#'),
        "the classifier's colour: {r}"
    );
    assert!(r.get("gather").is_none());

    // a pipe not yet kept runs as given
    let mut stony = fruity.clone();
    stony["from"]["class"] = json!("rocks");
    let run = c("pipes.run", json!({ "pipe": stony, "limit": 5 })).unwrap();
    assert!(run["matched"].as_u64().unwrap() >= 5, "{run}");
    assert_eq!(run["results"].as_array().unwrap().len(), 5);
    assert_eq!(run["effects"].as_array().unwrap().len(), 5);

    // actions: a list and a tag; a program on each
    let done = c("pipes.apply", json!({ "name": "fruity" })).unwrap();
    let n = done["texts"].as_u64().unwrap();
    assert!(n >= 15, "{done}");
    assert_eq!(done["done"][0]["texts"], n);
    let bowl = done["done"][0]["key"].as_str().unwrap().to_string();
    let stats = c("lists.stats", json!({})).unwrap();
    let held = stats["lists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["list"] == bowl.as_str())
        .unwrap()["count"]
        .clone();
    assert_eq!(held.as_u64().unwrap(), n, "{stats}");
    assert_eq!(
        c("tags.entries", json!({ "tags": ["ripe"] }))
            .unwrap()
            .as_array()
            .unwrap()
            .len() as u64,
        n
    );
    c(
        "notes.save",
        json!({ "note": { "tags": ["s"] }, "text": "# shout\n\nargs 0 it [l]\n{~it~!} >" }),
    )
    .unwrap();
    let mut loud = fruity.clone();
    loud["actions"] = json!([{ "kind": "run", "program": "shout" }]);
    c("pipes.save", json!({ "pipe": loud })).unwrap();
    let done = c("pipes.apply", json!({ "name": "fruity" })).unwrap();
    assert_eq!(done["done"][0]["failed"], 0, "{done}");
    assert!(done["done"][0]["out"][0].as_str().unwrap().ends_with('!'), "{done}");
    // a program can't apply a pipe itself
    let r = c(
        "s.run",
        json!({ "code": "std/anend.s.\n{pipes.apply} {{\"name\": \"fruity\"}} op {~err~} >" }),
    )
    .unwrap();
    assert!(r["out"].as_str().unwrap().contains("can't start pipes.apply"), "{r}");

    c("pipes.delete", json!({ "name": "fruity" })).unwrap();
    assert_eq!(c("pipes.list", json!({})).unwrap().as_array().unwrap().len(), 1);
    drop(ws);
    let _ = std::fs::remove_dir_all(dir);
}

/// views as programs: the reader's presets, and views said by s programs
#[test]
fn views_are_presets_and_programs() {
    let dir = temp("views");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    save(&ws);
    let c = |op: &str, a: Value| ws.call(op, &a);
    let lists = c("lists.stats", json!({})).unwrap()["lists"].clone();
    let keys: Vec<Value> = lists.as_array().unwrap().iter().map(|l| l["list"].clone()).collect();
    c(
        "classify.create",
        json!({ "name": "sorter", "classes": keys, "preset": "tiny" }),
    )
    .unwrap();
    c(
        "classify.train",
        json!({ "name": "sorter", "epochs": 30, "wait": true }),
    )
    .unwrap();
    c(
        "pipes.save",
        json!({ "pipe": {
        "name": "fruity", "from": { "op": "class", "classifier": "sorter", "class": "fruit", "top": true },
        "effects": [{ "kind": "colour", "colour": "#ff8800" }] } }),
    )
    .unwrap();
    let prog = |text: &str| c("notes.save", json!({ "note": { "tags": ["s"] }, "text": text })).unwrap();
    prog("# bowl\n\nstd/view.s.\n{fruity} from\n{list} layout\n{alphabetical} order\n{the fruit bowl} named");
    prog("# sorted\n\nstd/view.s.\n{texts} kind\n{sorter} group\n{ordinal} order");
    prog("# broken\n\nstd/view.s.\n{sideways} layout");
    prog("# not a view\n\n{hello} >");

    let list = c("views.list", json!({})).unwrap();
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["everything", "notes", "texts", "bowl", "broken", "sorted"],
        "{list}"
    );

    // a preset is the query it always was
    let p = c("views.run", json!({ "name": "texts", "words": "apple" })).unwrap();
    let q = c(
        "entries.query",
        json!({ "words": "apple", "kind": "memories", "limit": 20000 }),
    )
    .unwrap();
    assert_eq!(p["hits"].as_array().unwrap().len(), q["hits"].as_array().unwrap().len());
    assert_eq!(p["hits"][0]["key"], q["hits"][0]["key"]);
    assert_eq!(p["layout"], "auto");
    let all = c("views.run", json!({ "name": "everything" })).unwrap();
    assert!(all["total"].as_u64().unwrap() > 40, "{}", all["total"]);

    // a program: its pipe's texts, in its order and layout, with the pipe's effects
    let b = c("views.run", json!({ "name": "bowl" })).unwrap();
    assert_eq!(b["layout"], "list");
    assert_eq!(b["title"], "the fruit bowl");
    let texts: Vec<String> = b["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["text"].as_str().unwrap().to_string())
        .collect();
    assert!(texts.len() >= 15, "{b}");
    let mut sorted = texts.clone();
    sorted.sort_by_key(|t| t.to_lowercase());
    assert_eq!(texts, sorted);
    assert_eq!(b["hits"][0]["fx"]["colour"], "#ff8800");
    // the bar's words narrow it
    let n = c("views.run", json!({ "name": "bowl", "words": "apple" })).unwrap();
    assert!(
        n["hits"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["text"].as_str().unwrap().contains("apple"))
    );
    assert!(n["hits"].as_array().unwrap().len() < texts.len());

    // grouped by a classifier's class
    let g = c("views.run", json!({ "name": "sorted" })).unwrap();
    let groups: Vec<&str> = g["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert!(groups.contains(&"fruit") && groups.contains(&"rocks"), "{groups:?}");
    let counted: u64 = g["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["count"].as_u64().unwrap())
        .sum();
    assert_eq!(counted as usize, g["hits"].as_array().unwrap().len());

    let e = c("views.run", json!({ "name": "broken" })).unwrap_err();
    assert!(e.contains("layout: one of"), "{e}");
    assert!(
        c("views.run", json!({ "name": "not a view" }))
            .unwrap_err()
            .contains("no view")
    );
    drop(ws);
    let _ = std::fs::remove_dir_all(dir);
}

/// tag sets: word types of one's own, learned from marked words; a checker; English only on English
#[test]
fn tag_sets_learn_marked_words() {
    let dir = temp("tagsets");
    let ws = Saves::new(&dir, "notes").with_models(Limits::NONE);
    save(&ws);
    let c = |op: &str, a: Value| ws.call(op, &a);
    let lists = c("lists.stats", json!({})).unwrap()["lists"].clone();
    let keys: Vec<Value> = lists.as_array().unwrap().iter().map(|l| l["list"].clone()).collect();
    c(
        "classify.create",
        json!({ "name": "sorter", "classes": keys, "preset": "tiny" }),
    )
    .unwrap();
    c(
        "classify.train",
        json!({ "name": "sorter", "epochs": 30, "wait": true }),
    )
    .unwrap();

    // checked
    let one = json!({ "name": "x", "types": [{ "name": "a", "colour": "#112233" }] });
    assert!(
        c("tagsets.save", json!({ "set": one }))
            .unwrap_err()
            .contains("two types")
    );
    let bad = json!({ "name": "english", "types": [{ "name": "a", "colour": "#112233" }, { "name": "b", "colour": "#112233" }] });
    assert!(c("tagsets.save", json!({ "set": bad })).is_err());
    let set = json!({
        "name": "taste",
        "types": [{ "name": "sweet", "colour": "#e0a03f" }, { "name": "tart", "colour": "#3fa0e0" }],
        "for_texts": { "classifier": "sorter", "class": "fruit" },
        "threshold": 0.0
    });
    let mut wrong_class = set.clone();
    wrong_class["for_texts"]["class"] = json!("pebbles");
    assert!(
        c("tagsets.save", json!({ "set": wrong_class }))
            .unwrap_err()
            .contains("no class")
    );
    c("tagsets.save", json!({ "set": set, "new": true })).unwrap();
    assert_eq!(c("tagsets.list", json!({})).unwrap().as_array().unwrap().len(), 2);

    // training waits for words of each type
    for w in ["banana", "mango", "peach", "melon"] {
        c("tagsets.mark", json!({ "name": "taste", "word": w, "type": "sweet" })).unwrap();
    }
    assert!(
        c("tagsets.train", json!({ "name": "taste" }))
            .unwrap_err()
            .contains("as tart")
    );
    for w in ["cherry", "plum", "grape", "apple"] {
        c("tagsets.mark", json!({ "name": "taste", "word": w, "type": "tart" })).unwrap();
    }
    // a word moves between types
    c(
        "tagsets.mark",
        json!({ "name": "taste", "word": "apple", "type": "sweet" }),
    )
    .unwrap();
    let sc = c("tagsets.score", json!({ "name": "taste" })).unwrap();
    assert_eq!(sc["marked"][0]["words"], 5, "{sc}");
    assert_eq!(sc["marked"][1]["words"], 3);
    let j = c("tagsets.train", json!({ "name": "taste", "wait": true })).unwrap();
    assert_eq!(j["state"], "done", "{j}");
    let sc = c("tagsets.score", json!({ "name": "taste" })).unwrap();
    assert_eq!(sc["trained"], true);
    assert_eq!(sc["types"].as_array().unwrap().len(), 2, "{sc}");
    // a mark on a trained set teaches it at once
    let m = c(
        "tagsets.mark",
        json!({ "name": "taste", "word": "plum", "type": "tart" }),
    )
    .unwrap();
    assert_eq!(m["taught"], true);

    // the set reads the texts it's for; English rules read English; other languages get none
    let t = c("tagsets.tag", json!({ "text": "banana plum cherry" })).unwrap();
    assert_eq!(t["set"], "taste", "{t}");
    let ws3 = t["words"].as_array().unwrap();
    assert_eq!(ws3.len(), 3);
    assert!(ws3.iter().all(|w| w["type"] == "sweet" || w["type"] == "tart"), "{t}");
    assert!(ws3[0]["colour"].as_str().unwrap().starts_with('#'));
    let e = c("tagsets.tag", json!({ "text": "the granite stands tall" })).unwrap();
    assert_eq!(e["set"], "english");
    assert!(
        e["words"].as_array().unwrap().iter().any(|w| w["type"] == "noun"),
        "{e}"
    );
    let af = c("tagsets.tag", json!({ "text": "die hond loop vinnig na die huis toe" })).unwrap();
    assert_eq!(af["english"], false);
    assert_eq!(af["words"], json!([]));

    // a checker: right and wrong, with the nearest right words for a wrong one
    c(
        "tagsets.save",
        json!({ "set": { "name": "elvish", "check": true, "types": [], "threshold": 0.0 }, "new": true }),
    )
    .unwrap();
    for w in ["mellon", "galadh", "lasse", "aear", "elen"] {
        c("tagsets.mark", json!({ "name": "elvish", "word": w, "type": "right" })).unwrap();
    }
    for w in ["melon", "galad", "lass", "ear", "eln"] {
        c("tagsets.mark", json!({ "name": "elvish", "word": w, "type": "wrong" })).unwrap();
    }
    c("tagsets.train", json!({ "name": "elvish", "wait": true })).unwrap();
    let k = c("tagsets.tag", json!({ "name": "elvish", "text": "mellon melon" })).unwrap();
    assert_eq!(k["check"], true);
    for w in k["words"].as_array().unwrap() {
        if w["type"] == "wrong" {
            assert!(w["suggest"].as_array().is_some(), "{k}");
        }
    }
    assert!(
        k["words"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["type"] == "wrong" && w["suggest"][0] == "mellon")
            || k["words"][1]["type"] == "right",
        "{k}"
    );

    c("tagsets.delete", json!({ "name": "taste" })).unwrap();
    assert!(c("classify.get", json!({ "name": "tags: taste" })).is_err());
    drop(ws);
    let _ = std::fs::remove_dir_all(dir);
}
