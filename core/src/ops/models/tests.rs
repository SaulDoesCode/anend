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
