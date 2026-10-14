//! Other data as bytes, so the same transformers (classifiers and generators) work on
//! it: numbers as fixed-width text, pictures as sequences of patch codes. Plus a
//! minimal PNG writer (no compression, no dependencies) to look at generated pictures.
//!
//! Pictures: an image is cut into square patches (8×8 pixels by default) and each
//! patch is replaced by the nearest entry of a codebook learned from the training
//! images (k-means, integer arithmetic, so the same images give the same codebook on
//! any machine). A code is one byte (0..=254), a row of patches ends with `ROW` (255).
//! A 64×64 picture with 8×8 patches is 64 codes + 8 row ends = 72 tokens instead of
//! 4096 pixels, which is what makes pictures of useful size affordable for attention,
//! whose cost grows with the square of the sequence length.

use serde::{Deserialize, Serialize};

use super::math::{Rng, key};

// ---------------------------------------------------------------- numbers

/// numbers as fixed-width ASCII: sign (when signed), `digits` integer digits, and
/// `decimals` after a point, e.g. `+0042.50`. Fixed width means every digit has a
/// fixed position, which a model learns more easily than free-form numerals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberCodec {
    pub digits: usize,
    pub decimals: usize,
    pub signed: bool,
}

impl NumberCodec {
    pub fn width(&self) -> usize {
        self.signed as usize + self.digits + if self.decimals > 0 { 1 + self.decimals } else { 0 }
    }

    /// the largest magnitude that fits
    pub fn max(&self) -> f64 {
        10f64.powi(self.digits as i32) - 10f64.powi(-(self.decimals as i32))
    }

    /// the encoding of x, clamped to what fits and rounded half away from zero
    pub fn encode(&self, x: f64) -> Vec<u8> {
        let neg = x < 0.0 && self.signed;
        let m = x.abs().min(self.max());
        let scaled = (m * 10f64.powi(self.decimals as i32)).round() as u128;
        let s = format!("{:0w$}", scaled, w = self.digits + self.decimals);
        let mut out = Vec::with_capacity(self.width());
        if self.signed {
            out.push(if neg { b'-' } else { b'+' });
        }
        out.extend_from_slice(&s.as_bytes()[..self.digits]);
        if self.decimals > 0 {
            out.push(b'.');
            out.extend_from_slice(&s.as_bytes()[self.digits..]);
        }
        out
    }

    /// the number in the first `width` bytes, if they are well formed
    pub fn decode(&self, b: &[u8]) -> Option<f64> {
        let b = b.get(..self.width())?;
        let (sign, rest) = if self.signed {
            match b[0] {
                b'+' => (1.0, &b[1..]),
                b'-' => (-1.0, &b[1..]),
                _ => return None,
            }
        } else {
            (1.0, b)
        };
        let (int, frac) = rest.split_at(self.digits);
        if self.decimals > 0 && frac.first() != Some(&b'.') {
            return None;
        }
        let frac = if self.decimals > 0 { &frac[1..] } else { frac };
        if !int.iter().chain(frac).all(u8::is_ascii_digit) {
            return None;
        }
        let digits = |d: &[u8]| d.iter().fold(0f64, |a, &c| a * 10.0 + (c - b'0') as f64);
        // one integer, one division: the nearest double to the decimal written
        Some(sign * (digits(&[int, frac].concat()) / 10f64.powi(self.decimals as i32)))
    }
}

// ---------------------------------------------------------------- pictures

/// an RGB picture, 8 bits a channel, rows top to bottom
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// width·height·3 bytes
    pub rgb: Vec<u8>,
}

impl Image {
    pub fn new(width: usize, height: usize, fill: [u8; 3]) -> Image {
        Image {
            width,
            height,
            rgb: fill.repeat(width * height),
        }
    }

    pub fn get(&self, x: usize, y: usize) -> [u8; 3] {
        let i = 3 * (y * self.width + x);
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    pub fn set(&mut self, x: usize, y: usize, c: [u8; 3]) {
        if x < self.width && y < self.height {
            let i = 3 * (y * self.width + x);
            self.rgb[i..i + 3].copy_from_slice(&c);
        }
    }

    /// the pixels of patch (px, py), row-major, p·p·3 bytes
    fn patch(&self, p: usize, px: usize, py: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(p * p * 3);
        for y in py * p..(py + 1) * p {
            let i = 3 * (y * self.width + px * p);
            v.extend_from_slice(&self.rgb[i..i + 3 * p]);
        }
        v
    }

    /// each pixel as an s×s block
    pub fn scaled(&self, s: usize) -> Image {
        let mut o = Image::new(self.width * s, self.height * s, [0; 3]);
        for y in 0..o.height {
            for x in 0..o.width {
                o.set(x, y, self.get(x / s, y / s));
            }
        }
        o
    }
}

/// the end of a row of patches
pub const ROW: u8 = 255;
/// codes a codebook may hold (bytes 0..=254)
pub const MAX_CODES: usize = 255;

/// patch prototypes: `codes` entries of patch·patch·3 bytes
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Codebook {
    pub patch: usize,
    pub entries: Vec<Vec<u8>>,
}

fn dist(a: &[u8], b: &[u8]) -> u64 {
    a.iter().zip(b).map(|(&x, &y)| (x as i64 - y as i64).pow(2) as u64).sum()
}

impl Codebook {
    /// k-means over every patch of `images` (all the same size, multiples of `patch`):
    /// `codes` starting entries drawn by a seeded shuffle of the distinct patches, then
    /// `iters` rounds of nearest-entry assignment (ties to the lower code) and integer
    /// means (rounded half up); an entry left without patches keeps its value
    pub fn learn(images: &[Image], patch: usize, codes: usize, iters: usize, seed: u64) -> Result<Codebook, String> {
        if patch == 0 || codes == 0 || codes > MAX_CODES {
            return Err(format!("a codebook has 1 to {MAX_CODES} codes of patches at least 1 pixel wide"));
        }
        let mut all: Vec<Vec<u8>> = Vec::new();
        for im in images {
            if im.width % patch != 0 || im.height % patch != 0 {
                return Err(format!("{}×{} is not a multiple of the {patch} pixel patch", im.width, im.height));
            }
            for py in 0..im.height / patch {
                for px in 0..im.width / patch {
                    all.push(im.patch(patch, px, py));
                }
            }
        }
        if all.is_empty() {
            return Err("no pictures to learn patches from".into());
        }
        let mut distinct = all.clone();
        distinct.sort();
        distinct.dedup();
        Rng::new(key(&[seed, 0xc0de])).shuffle(&mut distinct);
        distinct.truncate(codes);
        let mut cb = Codebook {
            patch,
            entries: distinct,
        };
        let len = patch * patch * 3;
        for _ in 0..iters {
            let mut sums = vec![vec![0u64; len]; cb.entries.len()];
            let mut counts = vec![0u64; cb.entries.len()];
            for p in &all {
                let c = cb.nearest(p) as usize;
                counts[c] += 1;
                sums[c].iter_mut().zip(p).for_each(|(s, &v)| *s += v as u64);
            }
            for ((e, s), &n) in cb.entries.iter_mut().zip(&sums).zip(&counts) {
                if n > 0 {
                    e.iter_mut().zip(s).for_each(|(v, &t)| *v = ((2 * t + n) / (2 * n)) as u8);
                }
            }
        }
        Ok(cb)
    }

    /// the code of the nearest entry (squared distance, ties to the lower code)
    pub fn nearest(&self, p: &[u8]) -> u8 {
        let mut best = (u64::MAX, 0usize);
        for (i, e) in self.entries.iter().enumerate() {
            let d = dist(e, p);
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1 as u8
    }

    /// the picture as bytes: per row of patches, the codes, then ROW
    pub fn encode(&self, im: &Image) -> Vec<u8> {
        let p = self.patch;
        let mut out = Vec::new();
        for py in 0..im.height / p {
            for px in 0..im.width / p {
                out.push(self.nearest(&im.patch(p, px, py)));
            }
            out.push(ROW);
        }
        out
    }

    /// bytes back to a width×height picture: rows restart at each ROW, a short row is
    /// padded with code 0, a long one is cut, codes past the codebook read as code 0
    pub fn decode(&self, b: &[u8], width: usize, height: usize) -> Image {
        let p = self.patch;
        let (cols, rows) = (width / p, height / p);
        let mut im = Image::new(width, height, [0; 3]);
        let mut grid: Vec<Vec<u8>> = b.split(|&c| c == ROW).map(|r| r.to_vec()).collect();
        grid.resize(rows, Vec::new());
        for (py, row) in grid.iter().take(rows).enumerate() {
            for px in 0..cols {
                let c = row.get(px).copied().unwrap_or(0) as usize;
                let e = self.entries.get(c).unwrap_or(&self.entries[0]);
                for y in 0..p {
                    for x in 0..p {
                        let i = 3 * (y * p + x);
                        im.set(px * p + x, py * p + y, [e[i], e[i + 1], e[i + 2]]);
                    }
                }
            }
        }
        im
    }

    /// the tokens a width×height picture takes (codes, row ends)
    pub fn tokens(&self, width: usize, height: usize) -> usize {
        (width / self.patch + 1) * (height / self.patch)
    }
}

// ---------------------------------------------------------------- PNG

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for (n, v) in t.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *v = c;
    }
    t
}

fn crc32(t: &[u32; 256], data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xffff_ffff
}

/// a PNG file (RGB, 8 bits) of the picture: zlib with stored (uncompressed) deflate
/// blocks, so a few lines replace an image library; files are about the raw size
pub fn png(im: &Image) -> Vec<u8> {
    let t = crc_table();
    let mut raw = Vec::with_capacity(im.height * (1 + 3 * im.width));
    for y in 0..im.height {
        raw.push(0); // filter: none
        raw.extend_from_slice(&im.rgb[3 * y * im.width..3 * (y + 1) * im.width]);
    }
    let mut z = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, c) in chunks.iter().enumerate() {
        z.push((i + 1 == chunks.len()) as u8);
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let c = crc32(&t, &out[start..]);
        out.extend_from_slice(&c.to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(im.width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(im.height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

// ---------------------------------------------------------------- toy pictures

/// a toy picture for demos and tests: one of four shapes (0 disc, 1 square, 2 cross,
/// 3 stripes) in a colour on a background, placed and sized by the seed
pub fn toy(size: usize, shape: usize, seed: u64) -> Image {
    let mut r = Rng::new(key(&[seed, 0x70e]));
    let palette = [[230, 60, 50], [40, 160, 80], [50, 90, 220], [240, 200, 40], [150, 60, 200]];
    let bgs = [[250, 248, 240], [30, 30, 40], [200, 220, 235]];
    let fg = palette[r.below(palette.len())];
    let bg = bgs[r.below(bgs.len())];
    let mut im = Image::new(size, size, bg);
    let s = size as i64;
    let rad = s / 6 + r.below((s / 6).max(1) as usize) as i64;
    let cx = rad + r.below((s - 2 * rad).max(1) as usize) as i64;
    let cy = rad + r.below((s - 2 * rad).max(1) as usize) as i64;
    for y in 0..s {
        for x in 0..s {
            let (dx, dy) = (x - cx, y - cy);
            let on = match shape % 4 {
                0 => dx * dx + dy * dy <= rad * rad,
                1 => dx.abs() <= rad && dy.abs() <= rad,
                2 => (dx.abs() <= rad / 3 && dy.abs() <= rad) || (dy.abs() <= rad / 3 && dx.abs() <= rad),
                _ => (y / (rad / 2).max(1)) % 2 == 0,
            };
            if on {
                im.set(x as usize, y as usize, fg);
            }
        }
    }
    im
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_round_trip() {
        let c = NumberCodec {
            digits: 4,
            decimals: 2,
            signed: true,
        };
        assert_eq!(c.encode(42.5), b"+0042.50");
        assert_eq!(c.encode(-2.71), b"-0002.71");
        assert_eq!(c.encode(1e9), b"+9999.99");
        assert_eq!(c.width(), 8);
        assert_eq!(c.decode(b"-0002.71"), Some(-2.71));
        assert_eq!(c.decode(b"+00x3.14"), None);
        assert_eq!(c.decode(b"+0003"), None);
        let u = NumberCodec {
            digits: 3,
            decimals: 0,
            signed: false,
        };
        assert_eq!(u.encode(7.0), b"007");
        assert_eq!(u.decode(b"123rest"), Some(123.0));
    }

    #[test]
    fn pictures_through_a_codebook() {
        let ims: Vec<Image> = (0..8).map(|i| toy(32, i % 4, i as u64)).collect();
        let cb = Codebook::learn(&ims, 8, 24, 4, 1).unwrap();
        assert_eq!(cb, Codebook::learn(&ims, 8, 24, 4, 1).unwrap());
        let b = cb.encode(&ims[0]);
        assert_eq!(b.len(), cb.tokens(32, 32));
        assert_eq!(b.iter().filter(|&&c| c == ROW).count(), 4);
        let back = cb.decode(&b, 32, 32);
        assert_eq!(cb.encode(&back), b);
        // uniform patches are reproduced exactly when the codebook holds them
        let flat = Image::new(32, 32, [250, 248, 240]);
        let cb2 = Codebook::learn(std::slice::from_ref(&flat), 8, 4, 2, 0).unwrap();
        assert_eq!(cb2.decode(&cb2.encode(&flat), 32, 32), flat);
        // ragged bytes still decode
        assert_eq!(cb.decode(&[1, 2, ROW, 3], 32, 32).width, 32);
        assert!(Codebook::learn(&ims, 7, 4, 1, 0).is_err());
    }

    #[test]
    fn png_is_well_formed() {
        let im = toy(16, 0, 3).scaled(2);
        let p = png(&im);
        assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&p[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(p[16..20].try_into().unwrap()), 32);
        // the IHDR's CRC as zlib's crc32 gives it
        let t = crc_table();
        assert_eq!(crc32(&t, b"123456789"), 0xcbf4_3926);
        assert!(p.ends_with(&[0xae, 0x42, 0x60, 0x82]));
    }
}
