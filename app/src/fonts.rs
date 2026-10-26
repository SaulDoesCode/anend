//! The Bevy app's faces, bundled: Fira Sans (regular, bold, italic) and Fira Mono, subset
//! to Latin, Greek and the common symbols (assets/fonts, from bevy_feathers; OFL). They go
//! into the page as `@font-face` rules with data URLs, so the desktop and the web look the
//! same whatever the system has installed.

use std::sync::OnceLock;

const FACES: [(&str, &str, &str, &[u8]); 4] = [
    (
        "Fira Sans",
        "400",
        "normal",
        include_bytes!("../../assets/fonts/FiraSans-Regular.woff2"),
    ),
    (
        "Fira Sans",
        "700",
        "normal",
        include_bytes!("../../assets/fonts/FiraSans-Bold.woff2"),
    ),
    (
        "Fira Sans",
        "400",
        "italic",
        include_bytes!("../../assets/fonts/FiraSans-Italic.woff2"),
    ),
    (
        "Fira Mono",
        "400",
        "normal",
        include_bytes!("../../assets/fonts/FiraMono-Medium.woff2"),
    ),
];

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// base64 back to bytes (standard alphabet, padding optional); None when it doesn't read
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// the `@font-face` rules, made once
pub fn css() -> &'static str {
    static CSS: OnceLock<String> = OnceLock::new();
    CSS.get_or_init(|| {
        FACES
            .iter()
            .map(|(family, weight, style, data)| {
                format!(
                    "@font-face {{ font-family: \"{family}\"; font-weight: {weight}; font-style: {style}; font-display: block; src: url(data:font/woff2;base64,{}) format(\"woff2\"); }}\n",
                    base64(data)
                )
            })
            .collect()
    })
}

/// the families offered when the system's can't be listed (the web: a page can't see them)
pub const COMMON: [&str; 14] = [
    "Fira Sans",
    "Inter",
    "Noto Sans",
    "DejaVu Sans",
    "Liberation Sans",
    "Ubuntu",
    "Cantarell",
    "Georgia",
    "Times New Roman",
    "Liberation Serif",
    "DejaVu Serif",
    "Noto Serif",
    "Fira Mono",
    "DejaVu Sans Mono",
];

#[cfg(not(target_arch = "wasm32"))]
static SYSTEM: OnceLock<Vec<String>> = OnceLock::new();

/// the families installed on this machine, alphabetically, once `scan_system` has found
/// them (None until then, and always on the web)
pub fn system() -> Option<&'static [String]> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        SYSTEM.get().map(Vec::as_slice)
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

/// finds the installed families on a thread of its own (the desktop's font folders, each
/// file's family name read from its `name` table)
#[cfg(not(target_arch = "wasm32"))]
pub fn scan_system() {
    let _ = std::thread::Builder::new().name("anend fonts".into()).spawn(|| {
        let _ = SYSTEM.set(families(&font_dirs()));
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn font_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "windows") {
        if let Some(w) = std::env::var_os("WINDIR") {
            dirs.push(PathBuf::from(w).join("Fonts"));
        }
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        dirs.extend(home.iter().map(|h| h.join("Library/Fonts")));
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        dirs.extend(data.map(|d| d.join("fonts")));
        dirs.extend(home.iter().map(|h| h.join(".fonts")));
    }
    dirs
}

/// every family name in the font files under `dirs` (a few levels down), alphabetically,
/// one of each ignoring case
#[cfg(not(target_arch = "wasm32"))]
pub fn families(dirs: &[std::path::PathBuf]) -> Vec<String> {
    let mut found: std::collections::BTreeMap<String, String> = Default::default();
    let mut stack: Vec<(std::path::PathBuf, u32)> = dirs.iter().map(|d| (d.clone(), 0)).collect();
    let mut files = 0;
    while let Some((dir, depth)) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if depth < 6 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let ext = path
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc" | "pfb" | "pfa") || files > 20_000 {
                continue;
            }
            files += 1;
            let names = if ext.starts_with("pf") {
                type1_family(&path).into_iter().collect()
            } else {
                file_families(&path)
            };
            for name in names {
                let name = name.trim().to_string();
                if !name.is_empty() && !name.starts_with('.') {
                    found.entry(name.to_lowercase()).or_insert(name);
                }
            }
        }
    }
    found.into_values().collect()
}

/// the family names in one font file (each face of a collection), reading only the
/// headers and the `name` table
#[cfg(not(target_arch = "wasm32"))]
fn file_families(path: &std::path::Path) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return vec![];
    };
    let mut at = |off: u64, len: usize| -> Option<Vec<u8>> {
        f.seek(SeekFrom::Start(off)).ok()?;
        let mut b = vec![0; len];
        f.read_exact(&mut b).ok()?;
        Some(b)
    };
    let u16_at = |b: &[u8], i: usize| b.get(i..i + 2).map(|x| u16::from_be_bytes([x[0], x[1]]));
    let u32_at = |b: &[u8], i: usize| b.get(i..i + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]));
    let Some(head) = at(0, 12) else { return vec![] };
    let faces: Vec<u64> = if &head[0..4] == b"ttcf" {
        let n = u32_at(&head, 8).unwrap_or(0).min(64) as usize;
        let Some(offs) = at(12, n * 4) else { return vec![] };
        (0..n).filter_map(|i| u32_at(&offs, i * 4)).map(u64::from).collect()
    } else {
        vec![0]
    };
    let mut out = Vec::new();
    for face in faces {
        let Some(h) = at(face, 12) else { continue };
        let tables = u16_at(&h, 4).unwrap_or(0).min(200) as usize;
        let Some(dir) = at(face + 12, tables * 16) else {
            continue;
        };
        let Some((off, len)) = (0..tables)
            .find(|i| &dir[i * 16..i * 16 + 4] == b"name")
            .and_then(|i| Some((u32_at(&dir, i * 16 + 8)?, u32_at(&dir, i * 16 + 12)?)))
        else {
            continue;
        };
        let Some(t) = at(off as u64, (len as usize).min(1 << 20)) else {
            continue;
        };
        if let Some(name) = family_of(&t) {
            out.push(name);
        }
    }
    out
}

/// a PostScript Type 1 font's family, from `/FamilyName (…)` in its clear-text start
#[cfg(not(target_arch = "wasm32"))]
fn type1_family(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut b = vec![0; 8192];
    let n = std::fs::File::open(path).ok()?.read(&mut b).ok()?;
    let text = String::from_utf8_lossy(&b[..n]);
    let rest = &text[text.find("/FamilyName")? + 11..];
    let rest = &rest[rest.find('(')? + 1..];
    Some(rest[..rest.find(')')?].to_string())
}

/// a `name` table's family: the typographic family (16) when there is one, else the
/// family (1); Windows or Unicode English first, then the Mac's Roman
#[cfg(not(target_arch = "wasm32"))]
fn family_of(t: &[u8]) -> Option<String> {
    let u16_at = |i: usize| t.get(i..i + 2).map(|x| u16::from_be_bytes([x[0], x[1]]));
    let count = u16_at(2)? as usize;
    let strings = u16_at(4)? as usize;
    let mut best: Option<(u32, String)> = None;
    for r in 0..count {
        let b = 6 + r * 12;
        let (platform, encoding, language, id, len, off) = (
            u16_at(b)?,
            u16_at(b + 2)?,
            u16_at(b + 4)?,
            u16_at(b + 6)?,
            u16_at(b + 8)?,
            u16_at(b + 10)?,
        );
        if id != 1 && id != 16 {
            continue;
        }
        let Some(raw) = t.get(strings + off as usize..strings + off as usize + len as usize) else {
            continue;
        };
        let (text, rank) = match platform {
            0 | 3 => {
                let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                let english = platform == 0 || language == 0x409;
                (String::from_utf16_lossy(&units), if english { 0 } else { 2 })
            }
            1 if encoding == 0 => (raw.iter().map(|&c| c as char).collect(), 1),
            _ => continue,
        };
        // the typographic family before the family, then the language's rank
        let rank = rank + if id == 16 { 0 } else { 10 };
        if best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, text));
        }
    }
    best.map(|(_, s)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert!(css().matches("@font-face").count() == 4);
        for t in [&b""[..], b"M", b"Ma", b"Man", b"hello world \xff\x00"] {
            assert_eq!(unbase64(&base64(t)).unwrap(), t);
        }
        assert!(unbase64("a$b").is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn reads_font_families() {
        // the repo's own faces are woff2 (not read); a TrueType file made by hand: one
        // `name` table with a family (1) on the Mac and a typographic family (16) on Windows
        let mut name = Vec::new();
        let mac = b"Plain Family";
        let win: Vec<u8> = "Típo".encode_utf16().flat_map(u16::to_be_bytes).collect();
        name.extend([0u16, 2, 6 + 2 * 12].iter().flat_map(|v| v.to_be_bytes()));
        for (p, e, l, id, len, off) in [
            (1u16, 0u16, 0u16, 1u16, mac.len() as u16, 0u16),
            (3, 1, 0x409, 16, win.len() as u16, mac.len() as u16),
        ] {
            name.extend([p, e, l, id, len, off].iter().flat_map(|v| v.to_be_bytes()));
        }
        name.extend(mac);
        name.extend(&win);
        let mut file = Vec::new();
        file.extend(0x0001_0000u32.to_be_bytes());
        file.extend([1u16, 0, 0, 0].iter().flat_map(|v| v.to_be_bytes()));
        file.extend(b"name");
        file.extend([0u32, 12 + 16, name.len() as u32].iter().flat_map(|v| v.to_be_bytes()));
        file.extend(&name);
        let dir = std::env::temp_dir().join(format!("anend-fonts-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.ttf"), &file).unwrap();
        std::fs::write(dir.join("b.TTF"), &file).unwrap();
        std::fs::write(dir.join("c.txt"), b"no").unwrap();
        assert_eq!(families(std::slice::from_ref(&dir)), vec!["Típo".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
