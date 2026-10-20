//! The app's picture operations (`pictures.*`, listed in docs/app-api.md): pictures kept
//! in the save's own tables (`graph::assets`), each known by the hash of its bytes, with
//! a name, its size in pixels and when it was added, and in any number of lists (by
//! name, beside the texts of the list whose head has that name). A PNG or JPEG is
//! decoded on the way in (the `image` crate), so only pictures that read are kept.
//! Spec models train on them (`{"from": "pictures", "list": ..}` data, `ops::models`).

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

use super::classify::Limits;
use super::{Res, s};
use crate::classifiers::{base64, unbase64};
use crate::classify::codec::Image;
use crate::graph::Graph;
use crate::graph::assets::{Picture, picture_id};

/// a picture's bytes at most, on a shared hub and on one's own machine
pub const MAX_BYTES_SHARED: usize = 4 << 20;
pub const MAX_BYTES: usize = 64 << 20;
/// pictures a save holds at most on a shared hub (one's own machine: no limit)
pub const MAX_PICTURES_SHARED: u64 = 2000;
/// a side in pixels at most
pub const MAX_SIDE_SHARED: u32 = 8192;
pub const MAX_SIDE: u32 = 32768;
/// a thumbnail's side at most
pub const MAX_THUMB: u32 = 512;

/// whether `op` is one of these
pub fn routed(op: &str) -> bool {
    op.starts_with("pictures.")
}

/// what a picture file is: its mime and size, decoded to check it reads
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<(String, image::DynamicImage), String> {
    let format = image::guess_format(bytes).map_err(|_| "that isn't a picture (a PNG or JPEG)".to_string())?;
    let mime = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        f => return Err(format!("{f:?} pictures aren't kept; a PNG or JPEG, please")),
    };
    let side = if limits.shared { MAX_SIDE_SHARED } else { MAX_SIDE };
    let mut r = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut lim = image::Limits::default();
    lim.max_image_width = Some(side);
    lim.max_image_height = Some(side);
    lim.max_alloc = Some(if limits.shared { 512 << 20 } else { 4 << 30 });
    r.limits(lim);
    let im = r.decode().map_err(|e| format!("that picture doesn't read: {e}"))?;
    Ok((mime.to_string(), im))
}

/// a stored picture as RGB, scaled to `width` × `height` (stretched to fill), for a
/// picture model
pub fn as_image(g: &Graph, id: &str, width: usize, height: usize) -> Result<Image, String> {
    let b = g.picture_bytes(id)?.ok_or_else(|| format!("no picture {id:?}"))?;
    let im = image::load_from_memory(&b).map_err(|e| format!("picture {id}: {e}"))?;
    let im = if im.width() as usize == width && im.height() as usize == height {
        im
    } else {
        im.resize_exact(width as u32, height as u32, image::imageops::FilterType::Triangle)
    };
    Ok(Image {
        width,
        height,
        rgb: im.to_rgb8().into_raw(),
    })
}

fn thumbs() -> &'static Mutex<HashMap<(String, u32), String>> {
    static T: OnceLock<Mutex<HashMap<(String, u32), String>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// a PNG thumbnail fitting `side` × `side`, base64 (kept for later calls)
fn thumb(g: &Graph, id: &str, side: u32) -> Result<String, String> {
    let key = (id.to_string(), side);
    if let Some(t) = thumbs().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(t.clone());
    }
    let b = g.picture_bytes(id)?.ok_or_else(|| format!("no picture {id:?}"))?;
    let im = image::load_from_memory(&b).map_err(|e| format!("picture {id}: {e}"))?;
    let im = if im.width() <= side && im.height() <= side {
        im
    } else {
        im.thumbnail(side, side)
    };
    let mut out = Vec::new();
    im.to_rgba8()
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let t = base64(&out);
    let mut all = thumbs().lock().unwrap_or_else(|e| e.into_inner());
    if all.len() > 2000 {
        all.clear();
    }
    all.insert(key, t.clone());
    Ok(t)
}

fn details(g: &Graph, p: &Picture) -> Value {
    let mut v = json!(p);
    v["lists"] = json!(g.lists_of_picture(&p.id).unwrap_or_default());
    v
}

fn id_arg(args: &Value) -> Result<String, String> {
    match s(args, "id").trim() {
        "" => Err("which picture? (id)".into()),
        id => Ok(id.to_string()),
    }
}

fn thumb_arg(args: &Value) -> Option<u32> {
    args.get("thumb")
        .and_then(Value::as_u64)
        .map(|t| (t as u32).clamp(16, MAX_THUMB))
}

/// adds a picture's bytes (decoded and checked first); `(details, new)`
pub fn add(g: &Graph, bytes: &[u8], name: &str, limits: &Limits) -> Result<(Picture, bool), String> {
    let most = if limits.shared { MAX_BYTES_SHARED } else { MAX_BYTES };
    if bytes.len() > most {
        return Err(format!(
            "that picture is {} KB; pictures here are at most {} KB",
            bytes.len() / 1024,
            most / 1024
        ));
    }
    let id = picture_id(bytes);
    if let Some(p) = g.picture(&id)? {
        return Ok((p, false));
    }
    if limits.shared && g.picture_count()? >= MAX_PICTURES_SHARED {
        return Err(format!(
            "a save on this hub holds at most {MAX_PICTURES_SHARED} pictures: delete some first"
        ));
    }
    let (mime, im) = decode(bytes, limits)?;
    let p = Picture {
        id,
        mime,
        width: im.width(),
        height: im.height(),
        name: name.trim().chars().take(200).collect(),
        added: g.now(),
        size: bytes.len() as u64,
    };
    let new = g.put_picture(&p, bytes)?;
    Ok((p, new))
}

pub fn call(g: &Graph, op: &str, args: &Value, limits: &Limits) -> Res {
    match op {
        "pictures.add" => {
            let (bytes, from) = match (args.get("data").and_then(Value::as_str), s(args, "path")) {
                (Some(d), _) => {
                    // a data URL's prefix is fine too
                    let d = d.split_once("base64,").map_or(d, |x| x.1);
                    (
                        unbase64(d.trim()).ok_or("the picture's data isn't base64")?,
                        String::new(),
                    )
                }
                (None, p) if !p.trim().is_empty() => {
                    if limits.shared {
                        return Err("this hub takes pictures as data, not paths".into());
                    }
                    let path = std::path::Path::new(p.trim());
                    let b = std::fs::read(path).map_err(|e| format!("{p}: {e}"))?;
                    let n = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    (b, n)
                }
                _ => return Err("a picture, please: data (base64) or path".into()),
            };
            let name = match s(args, "name").trim() {
                "" => from,
                n => n.to_string(),
            };
            let (p, new) = add(g, &bytes, &name, limits)?;
            if let Some(l) = args
                .get("list")
                .and_then(Value::as_str)
                .filter(|l| !l.trim().is_empty())
            {
                g.tag_picture(&p.id, l, true)?;
            }
            let mut v = details(g, &p);
            v["new"] = json!(new);
            if let Some(t) = thumb_arg(args) {
                v["thumb"] = json!(thumb(g, &p.id, t)?);
            }
            Ok(v)
        }
        "pictures.list" => {
            let want = s(args, "list").trim();
            let ids: Option<std::collections::HashSet<String>> =
                (!want.is_empty()).then(|| g.list_pictures(want).unwrap_or_default().into_iter().collect());
            let t = thumb_arg(args);
            let mut out = Vec::new();
            for p in g.pictures()? {
                if ids.as_ref().is_some_and(|i| !i.contains(&p.id)) {
                    continue;
                }
                let mut v = details(g, &p);
                if let Some(t) = t {
                    v["thumb"] = json!(thumb(g, &p.id, t).unwrap_or_default());
                }
                out.push(v);
            }
            Ok(Value::Array(out))
        }
        "pictures.lists" => {
            let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
            for (l, _) in g.picture_list_rows()? {
                *counts.entry(l).or_default() += 1;
            }
            Ok(Value::Array(
                counts
                    .into_iter()
                    .map(|(list, count)| json!({ "list": list, "count": count }))
                    .collect(),
            ))
        }
        "pictures.get" => {
            let id = id_arg(args)?;
            let p = g.picture(&id)?.ok_or_else(|| format!("no picture {id:?}"))?;
            let mut v = details(g, &p);
            match thumb_arg(args) {
                Some(t) => v["thumb"] = json!(thumb(g, &id, t)?),
                None => v["data"] = json!(base64(&g.picture_bytes(&id)?.unwrap_or_default())),
            }
            Ok(v)
        }
        "pictures.name" => {
            let id = id_arg(args)?;
            g.name_picture(&id, s(args, "name").trim())?;
            Ok(details(g, &g.picture(&id)?.ok_or("gone")?))
        }
        "pictures.delete" => {
            let id = id_arg(args)?;
            if !g.delete_picture(&id)? {
                return Err(format!("no picture {id:?}"));
            }
            Ok(Value::Null)
        }
        "pictures.tag" => {
            let id = id_arg(args)?;
            let list = s(args, "list");
            let on = !args.get("remove").and_then(Value::as_bool).unwrap_or(false);
            let changed = g.tag_picture(&id, list, on)?;
            Ok(json!({ "id": id, "lists": g.lists_of_picture(&id)?, "changed": changed }))
        }
        _ => Err(format!("unknown op {op:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a small PNG of one colour
    pub fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
        let im = image::RgbImage::from_pixel(w, h, image::Rgb(rgb));
        let mut out = Vec::new();
        im.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let im = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 99]));
        let mut out = Vec::new();
        im.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
            .unwrap();
        out
    }

    #[test]
    fn pictures_in_a_save() {
        let g = Graph::in_memory().unwrap();
        let c = |op: &str, a: Value| call(&g, op, &a, &Limits::NONE);
        let red = png(20, 10, [200, 0, 0]);
        let a = c(
            "pictures.add",
            json!({ "data": base64(&red), "name": "red.png", "list": "warm" }),
        )
        .unwrap();
        assert_eq!((a["width"].as_u64(), a["height"].as_u64()), (Some(20), Some(10)));
        assert_eq!(a["mime"], "image/png");
        assert_eq!(a["new"], true);
        assert_eq!(a["lists"], json!(["warm"]));
        let id = a["id"].as_str().unwrap().to_string();
        assert_eq!(id, picture_id(&red));
        // the same bytes again are the same picture
        let again = c(
            "pictures.add",
            json!({ "data": format!("data:image/png;base64,{}", base64(&red)) }),
        )
        .unwrap();
        assert_eq!(
            (again["id"].as_str(), again["new"].as_bool()),
            (Some(id.as_str()), Some(false))
        );
        let j = c("pictures.add", json!({ "data": base64(&jpeg(16, 16)), "name": "j" })).unwrap();
        assert_eq!(j["mime"], "image/jpeg");
        // not pictures
        assert!(
            c("pictures.add", json!({ "data": base64(b"hello there") }))
                .unwrap_err()
                .contains("PNG or JPEG")
        );
        let mut broken = red.clone();
        broken.truncate(40);
        assert!(c("pictures.add", json!({ "data": base64(&broken) })).is_err());
        assert_eq!(g.picture_count().unwrap(), 2);

        // listed, by list, with thumbnails
        assert_eq!(c("pictures.list", json!({})).unwrap().as_array().unwrap().len(), 2);
        let warm = c("pictures.list", json!({ "list": "warm", "thumb": 8 })).unwrap();
        assert_eq!(warm.as_array().unwrap().len(), 1);
        let t = unbase64(warm[0]["thumb"].as_str().unwrap()).unwrap();
        let ti = image::load_from_memory(&t).unwrap();
        assert!(ti.width() <= 16 && ti.height() <= 16, "clamped to 16 at least");
        let got = c("pictures.get", json!({ "id": id })).unwrap();
        assert_eq!(unbase64(got["data"].as_str().unwrap()).unwrap(), red);

        // lists
        let jid = j["id"].as_str().unwrap();
        c("pictures.tag", json!({ "id": jid, "list": "warm" })).unwrap();
        c("pictures.tag", json!({ "id": jid, "list": "cool" })).unwrap();
        assert_eq!(
            c("pictures.lists", json!({})).unwrap(),
            json!([{"list": "cool", "count": 1}, {"list": "warm", "count": 2}])
        );
        let t = c("pictures.tag", json!({ "id": jid, "list": "warm", "remove": true })).unwrap();
        assert_eq!(t["lists"], json!(["cool"]));
        assert!(c("pictures.tag", json!({ "id": "nope", "list": "warm" })).is_err());

        // as a model's input: scaled to its size
        let im = as_image(&g, &id, 8, 8).unwrap();
        assert_eq!(im.rgb.len(), 8 * 8 * 3);
        assert_eq!(&im.rgb[..3], &[200, 0, 0]);

        // a shared hub's limits; a path only on one's own machine
        let shared = Limits::shared();
        let big = vec![0u8; MAX_BYTES_SHARED + 1];
        assert!(
            call(&g, "pictures.add", &json!({ "data": base64(&big) }), &shared)
                .unwrap_err()
                .contains("at most")
        );
        assert!(
            call(&g, "pictures.add", &json!({ "path": "/etc/hostname" }), &shared)
                .unwrap_err()
                .contains("not paths")
        );
        let dir = std::env::temp_dir().join(format!("anend-pics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("green.png");
        std::fs::write(&f, png(4, 4, [0, 200, 0])).unwrap();
        let p = c("pictures.add", json!({ "path": f.to_string_lossy() })).unwrap();
        assert_eq!(p["name"], "green.png");
        std::fs::remove_dir_all(dir).ok();

        // deleted, out of its lists too
        c("pictures.delete", json!({ "id": jid })).unwrap();
        assert!(c("pictures.delete", json!({ "id": jid })).is_err());
        assert_eq!(
            c("pictures.lists", json!({})).unwrap(),
            json!([{"list": "warm", "count": 1}])
        );

        // a backup carries them
        let (texts, other) = g.backup().unwrap();
        let back = Graph::in_memory().unwrap();
        back.restore(&texts, &other).unwrap();
        assert_eq!(back.pictures().unwrap(), g.pictures().unwrap());
        assert_eq!(back.picture_list_rows().unwrap(), g.picture_list_rows().unwrap());
        assert_eq!(back.picture_bytes(&id).unwrap().unwrap(), red);
        // and a merge (the same picture stays one)
        let other_save = Graph::in_memory().unwrap();
        call(
            &other_save,
            "pictures.add",
            &json!({ "data": base64(&red), "list": "reds" }),
            &Limits::NONE,
        )
        .unwrap();
        call(
            &other_save,
            "pictures.add",
            &json!({ "data": base64(&png(3, 3, [1, 2, 3])) }),
            &Limits::NONE,
        )
        .unwrap();
        let w = super::super::merge::assets(&g, &other_save).unwrap();
        assert_eq!(w.pictures, 1);
        assert_eq!(g.lists_of_picture(&id).unwrap(), ["reds", "warm"]);
    }
}
