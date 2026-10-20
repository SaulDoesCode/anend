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
}
