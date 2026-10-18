//! The network: an embedder (bytes, optional NUM tokens, optional feature channels),
//! pre-norm blocks (RMSNorm, multi-head self-attention with rotary positions and
//! optional QK-norm, SwiGLU feed-forward, no biases), a final RMSNorm, a pooler, any
//! number of heads reading the pooled vector (with the side vector and inputs from
//! other models when configured), and optionally a generative head reading every
//! position for the next byte.
//!
//! A causal model (`Config::causal`) masks attention so each position sees only itself
//! and earlier ones; that is what lets the generative head be trained on every position
//! at once and decoded one byte at a time with a cache (`Decoder`), bit for bit equal to
//! the full pass.
//!
//! Weights are row-major `[in × out]`, so every layer is `y = x · W` (see `kernels` for
//! the summation order). Forward passes keep a `Trace`; `backward` walks it in reverse
//! and adds this window's gradients into a gradient `Net` of the same shape.

use serde::{Deserialize, Serialize};

use super::kernels::{
    self as k, Norm, matmul, norm_back, norm_fwd, rows, softmax_back_rows, softmax_rows, transpose, transposed,
};
use super::math::{self, Rng, key, mix, unit};
use super::pool::Pool;
use super::text::{self, LETTERS, NUM, ROOT_BUCKETS, SIDE, SUM_BUCKETS, VOCAB, Window};

/// the generative head's outputs: the 256 byte values, then END
pub const GEN_OUT: usize = 257;
/// the generative head's "the data ends here" (SEP in the input)
pub const END: usize = 256;

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

/// how the encoder's token vectors become one vector
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pooling {
    /// the vector at the CLS token
    Cls,
    /// the mean over all tokens
    Mean,
    /// CLS and mean side by side (twice as wide)
    ClsMean,
    /// a learned query's softmax-weighted sum over the tokens
    Attention,
}

/// how strongly each input channel is mixed in; 1 is as trained
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gains {
    /// the letter value embedding added to each token
    pub letter: f32,
    /// the word sum embeddings added to each token
    pub word: f32,
    /// the text level side vector at the heads
    pub side: f32,
    /// inputs from other classifiers at the heads
    pub extra: f32,
}

impl Default for Gains {
    fn default() -> Gains {
        Gains {
            letter: 1.0,
            word: 1.0,
            side: 1.0,
            extra: 1.0,
        }
    }
}

impl Gains {
    /// channel by channel product
    pub fn times(&self, o: &Gains) -> Gains {
        Gains {
            letter: self.letter * o.letter,
            word: self.word * o.word,
            side: self.side * o.side,
            extra: self.extra * o.extra,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub d_model: usize,
    /// width of one attention head (even, for the rotary pairs)
    pub head_dim: usize,
    /// heads per block at creation; blocks may grow or lose heads later
    pub heads: usize,
    pub layers: usize,
    /// SwiGLU hidden units per block at creation
    pub d_ff: usize,
    /// tokens per window, CLS and SEP included
    pub max_len: usize,
    /// bytes between window starts on long texts
    pub stride: usize,
    pub dropout: f32,
    pub pooling: Pooling,
    /// RMSNorm on each head's queries and keys before the rotation
    pub qk_norm: bool,
    pub rope_base: f32,
    pub norm_eps: f32,
    /// whether the heads see the side vector
    pub side: bool,
    /// width of the inputs from other classifiers
    pub extra: usize,
    pub gains: Gains,
    /// residual output projections start at zero (each block starts as the identity)
    pub zero_out: bool,
    /// the letter value channel (a = 1 .. z = 26) added to each token; off for new
    /// models, on in files saved before it was optional
    #[serde(default = "yes")]
    pub letters: bool,
    /// the word sum channels (ordinal, reversed, Atbash, digit roots); same defaults
    #[serde(default = "yes")]
    pub words: bool,
    /// causal attention: each position sees only itself and what came before
    /// (needed by a generative head)
    #[serde(default)]
    pub causal: bool,
    /// NUM tokens: a token row plus value × a learned direction
    #[serde(default)]
    pub numbers: bool,
}

impl Config {
    /// for tests and CI only: a few thousand weights, so the suite runs in seconds in a
    /// debug build. Not a size to offer anyone; it is chosen for test speed, not for
    /// what the hardware can do (see `bench::suggest` for that)
    pub fn tiny() -> Config {
        Config {
            d_model: 16,
            head_dim: 8,
            heads: 2,
            layers: 2,
            d_ff: 24,
            max_len: 32,
            stride: 15,
            dropout: 0.0,
            pooling: Pooling::ClsMean,
            qk_norm: true,
            rope_base: 10000.0,
            norm_eps: 1e-5,
            side: true,
            extra: 0,
            gains: Gains::default(),
            zero_out: false,
            letters: false,
            words: false,
            causal: false,
            numbers: false,
        }
        .gematria(true)
    }

    /// the gematria features (letter channel, word sum channels, side vector) on or off
    pub fn gematria(mut self, on: bool) -> Config {
        self.letters = on;
        self.words = on;
        self.side = on;
        self
    }

    /// tokens in the embedding table
    pub fn vocab(&self) -> usize {
        VOCAB + self.numbers as usize
    }

    /// about 0.6M weights; trains on a laptop in minutes
    pub fn small() -> Config {
        Config {
            d_model: 128,
            head_dim: 32,
            heads: 4,
            layers: 4,
            d_ff: 352,
            max_len: 256,
            stride: 127,
            dropout: 0.1,
            ..Config::tiny()
        }
    }

    /// about 4.5M weights
    pub fn base() -> Config {
        Config {
            d_model: 256,
            head_dim: 32,
            heads: 8,
            layers: 6,
            d_ff: 704,
            max_len: 512,
            stride: 255,
            dropout: 0.1,
            ..Config::tiny()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.d_model == 0 || self.layers == 0 || self.heads == 0 || self.d_ff == 0 {
            return Err("sizes must be positive".into());
        }
        if self.head_dim == 0 || !self.head_dim.is_multiple_of(2) {
            return Err("head_dim must be even".into());
        }
        if self.max_len < 3 {
            return Err("max_len must leave room for a byte".into());
        }
        if !(0.0..1.0).contains(&self.dropout) {
            return Err("dropout must be in [0, 1)".into());
        }
        if self.causal && matches!(self.pooling, Pooling::Cls | Pooling::ClsMean) {
            return Err("a causal model pools by mean or attention (CLS sees only itself)".into());
        }
        Ok(())
    }

    /// bytes of text per window
    pub fn room(&self) -> usize {
        self.max_len - 2
    }

    /// width of the pooled vector
    pub fn pooled(&self) -> usize {
        match self.pooling {
            Pooling::ClsMean => 2 * self.d_model,
            _ => self.d_model,
        }
    }

    /// width of the heads' input: pooled, side vector, extra inputs
    pub fn head_in(&self) -> usize {
        self.pooled() + if self.side { SIDE } else { 0 } + self.extra
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeadKind {
    /// one class of K, softmax
    Single,
    /// any of K labels, a sigmoid each
    Multi,
    /// K real values, squared error
    Regress,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadSpec {
    pub name: String,
    pub kind: HeadKind,
    /// class, label or value names; their count is the head's width
    pub classes: Vec<String>,
    /// how much this head's loss counts when heads train together
    #[serde(default = "one")]
    pub weight: f32,
}

/// the generative head: next byte (or END) at every position of a causal model
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenSpec {
    pub name: String,
    /// how much its loss counts when heads train together
    #[serde(default = "one")]
    pub weight: f32,
}

impl GenSpec {
    pub fn new(name: &str) -> GenSpec {
        GenSpec {
            name: name.into(),
            weight: 1.0,
        }
    }
}

impl HeadSpec {
    pub fn single(name: &str, classes: &[&str]) -> HeadSpec {
        HeadSpec::new(name, HeadKind::Single, classes)
    }

    pub fn new(name: &str, kind: HeadKind, classes: &[&str]) -> HeadSpec {
        HeadSpec {
            name: name.into(),
            kind,
            classes: classes.iter().map(|c| c.to_string()).collect(),
            weight: 1.0,
        }
    }

    pub fn weighted(mut self, w: f32) -> HeadSpec {
        self.weight = w;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub heads: usize,
    pub ff: usize,
    /// RMSNorm gains before attention
    pub n1: Vec<f32>,
    /// [d × 3·heads·head_dim]: queries, keys, values, each head after head
    pub wqkv: Vec<f32>,
    /// QK-norm gains, [head_dim] each (empty without QK-norm)
    pub qn: Vec<f32>,
    pub kn: Vec<f32>,
    /// [heads·head_dim × d]
    pub wo: Vec<f32>,
    pub n2: Vec<f32>,
    /// SwiGLU gate and up projections [d × ff], down [ff × d]
    pub wg: Vec<f32>,
    pub wu: Vec<f32>,
    pub wd: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    pub spec: HeadSpec,
    /// [head_in × K]
    pub w: Vec<f32>,
    pub b: Vec<f32>,
}

impl Head {
    pub fn width(&self) -> usize {
        self.spec.classes.len()
    }
}

/// what a weight tensor is, which decides weight decay
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Embedding,
    Matrix,
    Gain,
    Bias,
}

impl Kind {
    pub fn decays(self) -> bool {
        self == Kind::Matrix
    }
}

/// rotary angles per position and pair: cos and sin, [max_len × head_dim/2]
#[derive(Clone, Debug, PartialEq)]
pub struct Rope {
    half: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
}

impl Rope {
    /// angle(t, i) = t · base^(−2i/head_dim), in f64 with `math`'s functions, then rounded
    pub fn new(max_len: usize, head_dim: usize, base: f32) -> Rope {
        let half = head_dim / 2;
        let lb = math::ln64(base as f64);
        let freq: Vec<f64> = (0..half)
            .map(|i| math::exp64(-((2 * i) as f64) / head_dim as f64 * lb))
            .collect();
        let (mut cos, mut sin) = (Vec::with_capacity(max_len * half), Vec::with_capacity(max_len * half));
        for t in 0..max_len {
            for f in &freq {
                let (s, c) = math::sincos64(t as f64 * f);
                cos.push(c as f32);
                sin.push(s as f32);
            }
        }
        Rope { half, cos, sin }
    }

    /// rotates rows [n × 2·half] in place: (a, b) → (a·c − b·s, a·s + b·c), pairs (i, i+half)
    fn apply(&self, x: &mut [f32], n: usize) {
        let d = 2 * self.half;
        for t in 0..n {
            self.apply_at(&mut x[t * d..(t + 1) * d], t);
        }
    }

    /// rotates one row as position t
    fn apply_at(&self, row: &mut [f32], t: usize) {
        let h = self.half;
        let (cs, sn) = (&self.cos[t * h..(t + 1) * h], &self.sin[t * h..(t + 1) * h]);
        for i in 0..h {
            let (a, b) = (row[i], row[i + h]);
            row[i] = a * cs[i] - b * sn[i];
            row[i + h] = a * sn[i] + b * cs[i];
        }
    }

    /// the transpose: (da, db) → (da·c + db·s, db·c − da·s)
    fn back(&self, dx: &mut [f32], n: usize) {
        let (h, d) = (self.half, 2 * self.half);
        for t in 0..n {
            let row = &mut dx[t * d..(t + 1) * d];
            let (cs, sn) = (&self.cos[t * h..(t + 1) * h], &self.sin[t * h..(t + 1) * h]);
            for i in 0..h {
                let (a, b) = (row[i], row[i + h]);
                row[i] = a * cs[i] + b * sn[i];
                row[i + h] = b * cs[i] - a * sn[i];
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Net {
    pub cfg: Config,
    /// byte and special token embeddings [vocab × d]
    pub tok: Vec<f32>,
    /// letter value [27 × d] (empty without the letter channel)
    pub letter: Vec<f32>,
    /// word sum buckets [97 × d] each, and digit roots [101 × d] (empty without the
    /// word channels)
    pub word_ord: Vec<f32>,
    pub word_rev: Vec<f32>,
    pub word_atb: Vec<f32>,
    pub word_root: Vec<f32>,
    pub blocks: Vec<Block>,
    /// final RMSNorm gains
    pub nf: Vec<f32>,
    /// attention pooling query [d] (empty for other poolings)
    pub pool_q: Vec<f32>,
    pub heads: Vec<Head>,
    /// the NUM value direction [d] (empty without numbers)
    pub num: Vec<f32>,
    /// the generative head, if any, and its weights [d × GEN_OUT] and [GEN_OUT]
    /// (empty without one)
    pub generator: Option<GenSpec>,
    pub gen_w: Vec<f32>,
    pub gen_b: Vec<f32>,
    pub rope: Rope,
}

/// uniform in [−a, a) with a = std·√3, so the variance is std²
fn init(r: &mut Rng, n: usize, std: f32) -> Vec<f32> {
    let a = std * 3f32.sqrt();
    (0..n).map(|_| r.symmetric(a)).collect()
}

impl Block {
    pub fn new(cfg: &Config, r: &mut Rng, zero_out: bool) -> Block {
        let (d, a, f) = (cfg.d_model, cfg.heads * cfg.head_dim, cfg.d_ff);
        let din = 1.0 / (d as f32).sqrt();
        let out = |fan: usize| 1.0 / (fan as f32).sqrt() / (2.0 * cfg.layers as f32).sqrt();
        let qk = if cfg.qk_norm {
            vec![1.0; cfg.head_dim]
        } else {
            Vec::new()
        };
        let wqkv = init(r, d * 3 * a, din);
        let wo = if zero_out {
            vec![0.0; a * d]
        } else {
            init(r, a * d, out(a))
        };
        let wg = init(r, d * f, din);
        let wu = init(r, d * f, din);
        let wd = if zero_out {
            vec![0.0; f * d]
        } else {
            init(r, f * d, out(f))
        };
        Block {
            heads: cfg.heads,
            ff: f,
            n1: vec![1.0; d],
            wqkv,
            qn: qk.clone(),
            kn: qk,
            wo,
            n2: vec![1.0; d],
            wg,
            wu,
            wd,
        }
    }

    fn zeros_like(&self) -> Block {
        let z = |v: &Vec<f32>| vec![0.0; v.len()];
        Block {
            heads: self.heads,
            ff: self.ff,
            n1: z(&self.n1),
            wqkv: z(&self.wqkv),
            qn: z(&self.qn),
            kn: z(&self.kn),
            wo: z(&self.wo),
            n2: z(&self.n2),
            wg: z(&self.wg),
            wu: z(&self.wu),
            wd: z(&self.wd),
        }
    }
}

impl Net {
    pub fn new(cfg: Config, heads: &[HeadSpec], seed: u64) -> Result<Net, String> {
        Net::with_gen(cfg, heads, None, seed)
    }

    /// a net with pooled heads and optionally a generative head (which needs `causal`)
    pub fn with_gen(cfg: Config, heads: &[HeadSpec], generator: Option<GenSpec>, seed: u64) -> Result<Net, String> {
        cfg.validate()?;
        if (heads.is_empty() && generator.is_none()) || heads.iter().any(|h| h.classes.is_empty()) {
            return Err("a model needs at least one head, each of at least one class".into());
        }
        if generator.is_some() && !cfg.causal {
            return Err("a generative head needs a causal model".into());
        }
        let fine = |w: f32| w >= 0.0 && w.is_finite();
        if !heads.iter().all(|h| fine(h.weight)) || generator.as_ref().is_some_and(|g| !fine(g.weight)) {
            return Err("head loss weights must be finite and not negative".into());
        }
        let mut r = Rng::new(key(&[seed, 0x1417]));
        let d = cfg.d_model;
        let tok = init(&mut r, VOCAB * d, 0.5);
        let mut table = |on: bool, rows: usize| if on { init(&mut r, rows * d, 0.25) } else { Vec::new() };
        let letter = table(cfg.letters, LETTERS);
        let word_ord = table(cfg.words, SUM_BUCKETS);
        let word_rev = table(cfg.words, SUM_BUCKETS);
        let word_atb = table(cfg.words, SUM_BUCKETS);
        let word_root = table(cfg.words, ROOT_BUCKETS);
        let blocks = (0..cfg.layers)
            .map(|_| Block::new(&cfg, &mut r, cfg.zero_out))
            .collect();
        let pool_q = if cfg.pooling == Pooling::Attention {
            init(&mut r, d, 0.02)
        } else {
            Vec::new()
        };
        let z = cfg.head_in();
        let heads = heads
            .iter()
            .map(|s| Head {
                spec: s.clone(),
                w: init(&mut r, z * s.classes.len(), 0.02),
                b: vec![0.0; s.classes.len()],
            })
            .collect();
        // tensors new since the first file version draw last, so older shapes start as before
        let mut tok = tok;
        let num = if cfg.numbers {
            tok.extend(init(&mut r, d, 0.5));
            init(&mut r, d, 0.5)
        } else {
            Vec::new()
        };
        let (gen_w, gen_b) = if generator.is_some() {
            (init(&mut r, d * GEN_OUT, 1.0 / (d as f32).sqrt()), vec![0.0; GEN_OUT])
        } else {
            (Vec::new(), Vec::new())
        };
        let rope = Rope::new(cfg.max_len, cfg.head_dim, cfg.rope_base);
        Ok(Net {
            num,
            generator,
            gen_w,
            gen_b,
            tok,
            letter,
            word_ord,
            word_rev,
            word_atb,
            word_root,
            blocks,
            nf: vec![1.0; d],
            pool_q,
            heads,
            rope,
            cfg,
        })
    }

    /// a net of the same shape, all zeros: a gradient or optimizer moment
    pub fn zeros_like(&self) -> Net {
        let z = |v: &Vec<f32>| vec![0.0; v.len()];
        Net {
            cfg: self.cfg.clone(),
            tok: z(&self.tok),
            letter: z(&self.letter),
            word_ord: z(&self.word_ord),
            word_rev: z(&self.word_rev),
            word_atb: z(&self.word_atb),
            word_root: z(&self.word_root),
            blocks: self.blocks.iter().map(Block::zeros_like).collect(),
            nf: z(&self.nf),
            pool_q: z(&self.pool_q),
            heads: self
                .heads
                .iter()
                .map(|h| Head {
                    spec: h.spec.clone(),
                    w: z(&h.w),
                    b: z(&h.b),
                })
                .collect(),
            num: z(&self.num),
            generator: self.generator.clone(),
            gen_w: z(&self.gen_w),
            gen_b: z(&self.gen_b),
            rope: self.rope.clone(),
        }
    }

    /// what each output is called: the pooled heads, then the generative head
    pub fn head_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.heads.iter().map(|h| h.spec.name.as_str()).collect();
        v.extend(self.generator.iter().map(|g| g.name.as_str()));
        v
    }

    /// every weight tensor in the fixed order the optimizer and the file format use
    pub fn params(&self) -> Vec<(Kind, &Vec<f32>)> {
        use Kind::*;
        let mut v = vec![
            (Embedding, &self.tok),
            (Embedding, &self.letter),
            (Embedding, &self.word_ord),
            (Embedding, &self.word_rev),
            (Embedding, &self.word_atb),
            (Embedding, &self.word_root),
        ];
        for b in &self.blocks {
            v.extend([
                (Gain, &b.n1),
                (Matrix, &b.wqkv),
                (Gain, &b.qn),
                (Gain, &b.kn),
                (Matrix, &b.wo),
                (Gain, &b.n2),
                (Matrix, &b.wg),
                (Matrix, &b.wu),
                (Matrix, &b.wd),
            ]);
        }
        v.extend([(Gain, &self.nf), (Bias, &self.pool_q)]);
        for h in &self.heads {
            v.extend([(Matrix, &h.w), (Bias, &h.b)]);
        }
        // added in file version 2; empty when unused, so version 1 files read unchanged
        v.extend([(Embedding, &self.num), (Matrix, &self.gen_w), (Bias, &self.gen_b)]);
        v
    }

    pub fn params_mut(&mut self) -> Vec<(Kind, &mut Vec<f32>)> {
        use Kind::*;
        let mut v = vec![
            (Embedding, &mut self.tok),
            (Embedding, &mut self.letter),
            (Embedding, &mut self.word_ord),
            (Embedding, &mut self.word_rev),
            (Embedding, &mut self.word_atb),
            (Embedding, &mut self.word_root),
        ];
        for b in &mut self.blocks {
            v.extend([
                (Gain, &mut b.n1),
                (Matrix, &mut b.wqkv),
                (Gain, &mut b.qn),
                (Gain, &mut b.kn),
                (Matrix, &mut b.wo),
                (Gain, &mut b.n2),
                (Matrix, &mut b.wg),
                (Matrix, &mut b.wu),
                (Matrix, &mut b.wd),
            ]);
        }
        v.extend([(Gain, &mut self.nf), (Bias, &mut self.pool_q)]);
        for h in &mut self.heads {
            v.extend([(Matrix, &mut h.w), (Bias, &mut h.b)]);
        }
        v.extend([
            (Embedding, &mut self.num),
            (Matrix, &mut self.gen_w),
            (Bias, &mut self.gen_b),
        ]);
        v
    }

    pub fn weight_count(&self) -> usize {
        self.params().iter().map(|(_, t)| t.len()).sum()
    }

    /// self += o, tensor by tensor (shapes must match)
    pub fn add(&mut self, o: &Net) {
        for ((_, a), (_, b)) in self.params_mut().into_iter().zip(o.params()) {
            for (x, y) in a.iter_mut().zip(b) {
                *x += y;
            }
        }
    }

    pub fn fill(&mut self, v: f32) {
        for (_, t) in self.params_mut() {
            t.iter_mut().for_each(|x| *x = v);
        }
    }

    /// the transposed weights backward needs, computed once per optimizer step
    pub fn transposes(&self) -> Vec<BlockT> {
        let d = self.cfg.d_model;
        self.blocks
            .iter()
            .map(|b| {
                let a = b.heads * self.cfg.head_dim;
                BlockT {
                    wqkv: transpose(&b.wqkv, d, 3 * a),
                    wo: transpose(&b.wo, a, d),
                    wg: transpose(&b.wg, d, b.ff),
                    wu: transpose(&b.wu, d, b.ff),
                    wd: transpose(&b.wd, b.ff, d),
                }
            })
            .collect()
    }
}

/// a block's weights transposed, `[out × in]`
pub struct BlockT {
    pub wqkv: Vec<f32>,
    pub wo: Vec<f32>,
    pub wg: Vec<f32>,
    pub wu: Vec<f32>,
    pub wd: Vec<f32>,
}

/// causal attention for position t: scores q·k_s·scale for s ≤ t (8-lane dots),
/// softmax over them into `p` (t + 1 long), out = Σ_s p_s·v_s in ascending s.
/// `ks` and `vs` hold at least t + 1 rows of `dh`.
#[allow(clippy::too_many_arguments)]
pub fn attend_row(q: &[f32], ks: &[f32], vs: &[f32], t: usize, dh: usize, scale: f32, p: &mut [f32], out: &mut [f32]) {
    for (s, ps) in p.iter_mut().enumerate().take(t + 1) {
        *ps = k::dot(q, &ks[s * dh..(s + 1) * dh]) * scale;
    }
    softmax_rows(&mut p[..t + 1], t + 1);
    out.iter_mut().for_each(|v| *v = 0.0);
    for (s, &ps) in p.iter().enumerate().take(t + 1) {
        k::axpy(out, ps, &vs[s * dh..(s + 1) * dh]);
    }
}

/// a dropout mask: 0 or 1/(1 − p) per element, drawn from `mix(key ^ index)`
pub fn dropout_mask(key: u64, n: usize, p: f32) -> Vec<f32> {
    let keep = 1.0 / (1.0 - p);
    let mut m = k::zeros(n);
    for (i, v) in m.iter_mut().enumerate() {
        *v = if unit(mix(key ^ i as u64)) < p { 0.0 } else { keep };
    }
    m
}

fn mul_into(x: &mut [f32], m: &Option<Vec<f32>>) {
    if let Some(m) = m {
        x.iter_mut().zip(m).for_each(|(a, b)| *a *= b);
    }
}

// ---------------------------------------------------------------- forward

/// how a forward pass runs: dropout on with its key, or inference
#[derive(Clone, Copy, Debug)]
pub struct Mode {
    pub dropout: f32,
    pub key: u64,
}

impl Mode {
    pub const EVAL: Mode = Mode { dropout: 0.0, key: 0 };

    fn mask(&self, site: u64, n: usize) -> Option<Vec<f32>> {
        (self.dropout > 0.0).then(|| dropout_mask(key(&[self.key, site]), n, self.dropout))
    }
}

pub struct BlockTrace {
    n1: Norm,
    a: Vec<f32>,
    /// per head [n × head_dim]: rotated queries and keys, values
    q: Vec<Vec<f32>>,
    kk: Vec<Vec<f32>>,
    v: Vec<Vec<f32>>,
    qn: Vec<Norm>,
    kn: Vec<Norm>,
    probs: Vec<Vec<f32>>,
    ctx: Vec<f32>,
    m1: Option<Vec<f32>>,
    n2: Norm,
    b: Vec<f32>,
    g: Vec<f32>,
    s: Vec<f32>,
    u: Vec<f32>,
    act: Vec<f32>,
    m2: Option<Vec<f32>>,
}

/// everything one window's forward pass leaves for backward
pub struct Trace {
    pub n: usize,
    tokens: Vec<u16>,
    values: Vec<f32>,
    chan: [Vec<u8>; 5],
    gains: Gains,
    m0: Option<Vec<f32>>,
    blocks: Vec<BlockTrace>,
    nf: Norm,
    xf: Vec<f32>,
    pool_p: Vec<f32>,
    /// pooled vector (after the pooling dropout)
    pub pooled: Vec<f32>,
    mp: Option<Vec<f32>>,
    /// the heads' input
    pub z: Vec<f32>,
}

impl Drop for BlockTrace {
    fn drop(&mut self) {
        let heads = [&mut self.q, &mut self.kk, &mut self.v, &mut self.probs];
        for list in heads {
            list.drain(..).for_each(k::recycle);
        }
        for nm in self
            .qn
            .iter_mut()
            .chain(self.kn.iter_mut())
            .chain([&mut self.n1, &mut self.n2])
        {
            k::recycle(std::mem::take(&mut nm.xhat));
        }
        for v in [
            &mut self.a,
            &mut self.ctx,
            &mut self.b,
            &mut self.g,
            &mut self.s,
            &mut self.u,
            &mut self.act,
        ] {
            k::recycle(std::mem::take(v));
        }
        for m in [&mut self.m1, &mut self.m2].into_iter().flatten() {
            k::recycle(std::mem::take(m));
        }
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        k::recycle(std::mem::take(&mut self.xf));
        k::recycle(std::mem::take(&mut self.nf.xhat));
        if let Some(m) = self.m0.as_mut() {
            k::recycle(std::mem::take(m));
        }
    }
}

impl Trace {
    /// the window's tokens
    pub fn tokens(&self) -> &[u16] {
        &self.tokens
    }

    /// block l's SwiGLU activations [n × ff]
    pub fn act(&self, l: usize) -> &[f32] {
        &self.blocks[l].act
    }

    /// block l's attention outputs before the projection [n × heads·head_dim]
    pub fn ctx(&self, l: usize) -> &[f32] {
        &self.blocks[l].ctx
    }
}

impl Net {
    /// the sum of the embeddings, per element `tok + gl·letter + gw·(((ord + rev) + atb) + root)`
    fn embed(&self, w: &Window, gains: &Gains) -> Vec<f32> {
        let d = self.cfg.d_model;
        let n = w.len();
        let mut x = vec![0f32; n * d];
        for t in 0..n {
            fn row(tab: &[f32], i: u8, d: usize) -> &[f32] {
                &tab[i as usize * d..(i as usize + 1) * d]
            }
            let tk = &self.tok[w.tokens[t] as usize * d..(w.tokens[t] as usize + 1) * d];
            let xr = &mut x[t * d..(t + 1) * d];
            xr.copy_from_slice(tk);
            if self.cfg.letters {
                let le = row(&self.letter, w.letter[t], d);
                for i in 0..d {
                    xr[i] += gains.letter * le[i];
                }
            }
            if self.cfg.words {
                let wo = row(&self.word_ord, w.ordinal[t], d);
                let wr = row(&self.word_rev, w.reversed[t], d);
                let wa = row(&self.word_atb, w.atbash[t], d);
                let wq = row(&self.word_root, w.root[t], d);
                for i in 0..d {
                    xr[i] += gains.word * (((wo[i] + wr[i]) + wa[i]) + wq[i]);
                }
            }
            if w.tokens[t] == NUM && !self.num.is_empty() {
                let v = w.value(t);
                k::axpy(xr, v, &self.num);
            }
        }
        x
    }

    /// one window through the trunk and every head; `extra` is this text's inputs from
    /// other classifiers (`cfg.extra` long). The pool parallelises big products only.
    pub fn forward(&self, w: &Window, extra: &[f32], gains: &Gains, mode: Mode, pool: Option<&Pool>) -> Trace {
        let cfg = &self.cfg;
        let (d, dh, n) = (cfg.d_model, cfg.head_dim, w.len());
        assert!(n <= cfg.max_len, "window longer than max_len");
        let mut x = self.embed(w, gains);
        let m0 = mode.mask(1, n * d);
        mul_into(&mut x, &m0);
        let scale = 1.0 / (dh as f32).sqrt();
        let mut blocks = Vec::with_capacity(self.blocks.len());
        for (l, b) in self.blocks.iter().enumerate() {
            let l = l as u64 + 1;
            let a_w = b.heads * dh;
            let (a, n1) = norm_fwd(&x, &b.n1, d, cfg.norm_eps);
            let qkv = matmul(rows(&a, d), &b.wqkv, n, d, 3 * a_w, pool);
            let mut ctx = k::zeros(n * a_w);
            let (mut qs, mut ks, mut vs, mut qns, mut kns, mut ps) = (vec![], vec![], vec![], vec![], vec![], vec![]);
            for h in 0..b.heads {
                let take = |off: usize| -> Vec<f32> {
                    let mut o = k::zeros(n * dh);
                    for t in 0..n {
                        let s = t * 3 * a_w + off + h * dh;
                        o[t * dh..(t + 1) * dh].copy_from_slice(&qkv[s..s + dh]);
                    }
                    o
                };
                let (mut q, mut kh, v) = (take(0), take(a_w), take(2 * a_w));
                if cfg.qk_norm {
                    let (qq, qn) = norm_fwd(&q, &b.qn, dh, cfg.norm_eps);
                    let (kq, kn) = norm_fwd(&kh, &b.kn, dh, cfg.norm_eps);
                    k::recycle(std::mem::replace(&mut q, qq));
                    k::recycle(std::mem::replace(&mut kh, kq));
                    qns.push(qn);
                    kns.push(kn);
                }
                self.rope.apply(&mut q, n);
                self.rope.apply(&mut kh, n);
                let s = if cfg.causal {
                    // row by row over the positions so far, exactly as `Decoder` does;
                    // the probabilities after t stay 0, which backward relies on
                    let mut s = k::zeros(n * n);
                    for t in 0..n {
                        let c = &mut ctx[t * a_w + h * dh..t * a_w + (h + 1) * dh];
                        attend_row(
                            &q[t * dh..(t + 1) * dh],
                            &kh,
                            &v,
                            t,
                            dh,
                            scale,
                            &mut s[t * n..t * n + t + 1],
                            c,
                        );
                    }
                    s
                } else {
                    let kt = transpose(&kh, n, dh);
                    let mut s = matmul(rows(&q, dh), &kt, n, dh, n, None);
                    s.iter_mut().for_each(|v| *v *= scale);
                    softmax_rows(&mut s, n);
                    let o = matmul(rows(&s, n), &v, n, n, dh, None);
                    for t in 0..n {
                        ctx[t * a_w + h * dh..t * a_w + (h + 1) * dh].copy_from_slice(&o[t * dh..(t + 1) * dh]);
                    }
                    k::recycle(o);
                    k::recycle(kt);
                    s
                };
                qs.push(q);
                ks.push(kh);
                vs.push(v);
                ps.push(s);
            }
            let mut o = matmul(rows(&ctx, a_w), &b.wo, n, a_w, d, pool);
            let m1 = mode.mask(l << 8 | 2, n * d);
            mul_into(&mut o, &m1);
            let mut hres = k::zeros(n * d);
            for ((h, a), b) in hres.iter_mut().zip(&x).zip(&o) {
                *h = a + b;
            }
            k::recycle(o);
            let (bn, n2) = norm_fwd(&hres, &b.n2, d, cfg.norm_eps);
            let g = matmul(rows(&bn, d), &b.wg, n, d, b.ff, pool);
            let u = matmul(rows(&bn, d), &b.wu, n, d, b.ff, pool);
            let mut s = k::zeros(g.len());
            k::sigmoid_slice(&g, &mut s);
            let mut act = k::zeros(g.len());
            for (((a, g), s), u) in act.iter_mut().zip(&g).zip(&s).zip(&u) {
                *a = (g * s) * u;
            }
            let mut dn = matmul(rows(&act, b.ff), &b.wd, n, b.ff, d, pool);
            let m2 = mode.mask(l << 8 | 3, n * d);
            mul_into(&mut dn, &m2);
            let mut xn = k::zeros(n * d);
            for ((o, a), b) in xn.iter_mut().zip(&hres).zip(&dn) {
                *o = a + b;
            }
            k::recycle(dn);
            k::recycle(qkv);
            k::recycle(std::mem::replace(&mut x, xn));
            blocks.push(BlockTrace {
                n1,
                a,
                q: qs,
                kk: ks,
                v: vs,
                qn: qns,
                kn: kns,
                probs: ps,
                ctx,
                m1,
                n2,
                b: bn,
                g,
                s,
                u,
                act,
                m2,
            });
        }
        let (xf, nf) = norm_fwd(&x, &self.nf, d, cfg.norm_eps);
        k::recycle(x);
        let mean = || {
            let mut m = vec![0f32; d];
            for row in xf.chunks_exact(d) {
                m.iter_mut().zip(row).for_each(|(a, b)| *a += b);
            }
            m.iter_mut().for_each(|a| *a /= n as f32);
            m
        };
        let mut pool_p = Vec::new();
        let mut pooled = match cfg.pooling {
            Pooling::Cls => xf[..d].to_vec(),
            Pooling::Mean => mean(),
            Pooling::ClsMean => [&xf[..d], &mean()[..]].concat(),
            Pooling::Attention => {
                pool_p = xf.chunks_exact(d).map(|r| k::dot(r, &self.pool_q)).collect();
                softmax_rows(&mut pool_p, n);
                let mut m = vec![0f32; d];
                for (row, &p) in xf.chunks_exact(d).zip(&pool_p) {
                    k::axpy(&mut m, p, row);
                }
                m
            }
        };
        let mp = mode.mask(0xff01, pooled.len());
        mul_into(&mut pooled, &mp);
        let mut z = pooled.clone();
        if cfg.side {
            z.extend(w.side.iter().map(|v| v * gains.side));
        }
        assert_eq!(extra.len(), cfg.extra, "extra inputs");
        z.extend(extra.iter().map(|v| v * gains.extra));
        Trace {
            n,
            tokens: w.tokens.clone(),
            values: w.values.clone(),
            chan: [
                w.letter.clone(),
                w.ordinal.clone(),
                w.reversed.clone(),
                w.atbash.clone(),
                w.root.clone(),
            ],
            gains: *gains,
            m0,
            blocks,
            nf,
            xf,
            pool_p,
            pooled,
            mp,
            z,
        }
    }

    /// the logits of head `h` for a trace: z·W, then + b
    pub fn logits(&self, tr: &Trace, h: usize) -> Vec<f32> {
        let hd = &self.heads[h];
        let kw = hd.width();
        let mut y = matmul(rows(&tr.z, tr.z.len()), &hd.w, 1, tr.z.len(), kw, None);
        y.iter_mut().zip(&hd.b).for_each(|(a, b)| *a += b);
        y
    }

    /// the generative head's logits at every position, [n × GEN_OUT]: row t predicts
    /// the token after position t
    pub fn gen_logits(&self, tr: &Trace) -> Vec<f32> {
        assert!(self.generator.is_some(), "no generative head");
        let d = self.cfg.d_model;
        let mut y = matmul(rows(&tr.xf, d), &self.gen_w, tr.n, d, GEN_OUT, None);
        for row in y.as_chunks_mut::<GEN_OUT>().0 {
            row.iter_mut().zip(&self.gen_b).for_each(|(a, b)| *a += b);
        }
        y
    }

    /// adds one window's gradients into `g`, given d(loss)/d(logits) for each head
    /// (empty for heads without a loss). Returns d(loss)/d(extra inputs), for stacks.
    pub fn backward(&self, tr: &Trace, dlogits: &[Vec<f32>], tp: &[BlockT], g: &mut Net) -> Vec<f32> {
        self.backward_gen(tr, dlogits, &[], tp, g)
    }

    /// `backward` with d(loss)/d(generative logits) too ([n × GEN_OUT], or empty)
    pub fn backward_gen(&self, tr: &Trace, dlogits: &[Vec<f32>], dgen: &[f32], tp: &[BlockT], g: &mut Net) -> Vec<f32> {
        let cfg = &self.cfg;
        let (d, dh, n) = (cfg.d_model, cfg.head_dim, tr.n);
        let zl = tr.z.len();
        let p = cfg.pooled();
        let mut dz = vec![0f32; zl];
        for (h, dl) in dlogits.iter().enumerate() {
            if dl.is_empty() {
                continue;
            }
            let (hd, gh) = (&self.heads[h], &mut g.heads[h]);
            let kw = hd.width();
            k::gemm(&mut gh.w, transposed(&tr.z, 1), dl, zl, 1, kw, None);
            gh.b.iter_mut().zip(dl).for_each(|(a, b)| *a += b);
            for (j, v) in dz.iter_mut().enumerate() {
                *v += k::dot(&hd.w[j * kw..(j + 1) * kw], dl);
            }
        }
        let dextra: Vec<f32> = dz[zl - cfg.extra..].iter().map(|v| v * tr.gains.extra).collect();
        let mut dpool = dz[..p].to_vec();
        mul_into(&mut dpool, &tr.mp);
        let mut dxf = k::zeros(n * d);
        let add_mean = |dxf: &mut [f32], dm: &[f32]| {
            let dmn: Vec<f32> = dm.iter().map(|v| v / n as f32).collect();
            for row in dxf.chunks_exact_mut(d) {
                row.iter_mut().zip(&dmn).for_each(|(a, b)| *a += b);
            }
        };
        match cfg.pooling {
            Pooling::Cls => dxf[..d].copy_from_slice(&dpool),
            Pooling::Mean => add_mean(&mut dxf, &dpool),
            Pooling::ClsMean => {
                dxf[..d].copy_from_slice(&dpool[..d]);
                add_mean(&mut dxf, &dpool[d..]);
            }
            Pooling::Attention => {
                let mut dp: Vec<f32> = tr.xf.chunks_exact(d).map(|r| k::dot(r, &dpool)).collect();
                for (row, &pp) in dxf.chunks_exact_mut(d).zip(&tr.pool_p) {
                    k::axpy(row, pp, &dpool);
                }
                softmax_back_rows(&tr.pool_p, &mut dp, n);
                for (t, &de) in dp.iter().enumerate() {
                    k::axpy(&mut g.pool_q, de, &tr.xf[t * d..(t + 1) * d]);
                    k::axpy(&mut dxf[t * d..(t + 1) * d], de, &self.pool_q);
                }
            }
        }
        if !dgen.is_empty() {
            k::gemm(&mut g.gen_w, transposed(&tr.xf, d), dgen, d, n, GEN_OUT, None);
            for row in dgen.as_chunks::<GEN_OUT>().0 {
                g.gen_b.iter_mut().zip(row).for_each(|(a, b)| *a += b);
            }
            let wt = transpose(&self.gen_w, d, GEN_OUT);
            k::gemm(&mut dxf, rows(dgen, GEN_OUT), &wt, n, GEN_OUT, d, None);
        }
        let mut dx = k::zeros(n * d);
        norm_back(&dxf, &tr.nf, &self.nf, d, &mut dx, &mut g.nf);
        let scale = 1.0 / (dh as f32).sqrt();
        for (l, b) in self.blocks.iter().enumerate().rev() {
            let (bt, t, gb) = (&tr.blocks[l], &tp[l], &mut g.blocks[l]);
            let a_w = b.heads * dh;
            // feed-forward
            let mut dd = dx.clone();
            mul_into(&mut dd, &bt.m2);
            k::gemm(&mut gb.wd, transposed(&bt.act, b.ff), &dd, b.ff, n, d, None);
            let dact = matmul(rows(&dd, d), &t.wd, n, d, b.ff, None);
            let mut dg = k::zeros(n * b.ff);
            let mut du = k::zeros(n * b.ff);
            for i in 0..n * b.ff {
                let (gv, s) = (bt.g[i], bt.s[i]);
                du[i] = dact[i] * (gv * s);
                dg[i] = (dact[i] * bt.u[i]) * (s * (1.0 + gv * (1.0 - s)));
            }
            k::gemm(&mut gb.wg, transposed(&bt.b, d), &dg, d, n, b.ff, None);
            k::gemm(&mut gb.wu, transposed(&bt.b, d), &du, d, n, b.ff, None);
            let mut dbn = matmul(rows(&dg, b.ff), &t.wg, n, b.ff, d, None);
            k::gemm(&mut dbn, rows(&du, b.ff), &t.wu, n, b.ff, d, None);
            let mut dhres = dx;
            norm_back(&dbn, &bt.n2, &b.n2, d, &mut dhres, &mut gb.n2);
            for v in [dd, dact, dg, du, dbn] {
                k::recycle(v);
            }
            // attention
            let mut dout = dhres.clone();
            mul_into(&mut dout, &bt.m1);
            k::gemm(&mut gb.wo, transposed(&bt.ctx, a_w), &dout, a_w, n, d, None);
            let dctx = matmul(rows(&dout, d), &t.wo, n, d, a_w, None);
            let mut dqkv = k::zeros(n * 3 * a_w);
            for h in 0..b.heads {
                let mut dco = k::zeros(n * dh);
                for r in 0..n {
                    dco[r * dh..(r + 1) * dh].copy_from_slice(&dctx[r * a_w + h * dh..r * a_w + (h + 1) * dh]);
                }
                let (q, kh, v, pr) = (&bt.q[h], &bt.kk[h], &bt.v[h], &bt.probs[h]);
                let vt = transpose(v, n, dh);
                let mut ds = matmul(rows(&dco, dh), &vt, n, dh, n, None);
                let dv = matmul(transposed(pr, n), &dco, n, n, dh, None);
                softmax_back_rows(pr, &mut ds, n);
                ds.iter_mut().for_each(|v| *v *= scale);
                let mut dq = matmul(rows(&ds, n), kh, n, n, dh, None);
                let mut dk = matmul(transposed(&ds, n), q, n, n, dh, None);
                self.rope.back(&mut dq, n);
                self.rope.back(&mut dk, n);
                if cfg.qk_norm {
                    let mut dq0 = k::zeros(n * dh);
                    norm_back(&dq, &bt.qn[h], &b.qn, dh, &mut dq0, &mut gb.qn);
                    let mut dk0 = k::zeros(n * dh);
                    norm_back(&dk, &bt.kn[h], &b.kn, dh, &mut dk0, &mut gb.kn);
                    k::recycle(std::mem::replace(&mut dq, dq0));
                    k::recycle(std::mem::replace(&mut dk, dk0));
                }
                for r in 0..n {
                    let row = &mut dqkv[r * 3 * a_w..(r + 1) * 3 * a_w];
                    let src = r * dh..(r + 1) * dh;
                    row[h * dh..(h + 1) * dh].copy_from_slice(&dq[src.clone()]);
                    row[a_w + h * dh..a_w + (h + 1) * dh].copy_from_slice(&dk[src.clone()]);
                    row[2 * a_w + h * dh..2 * a_w + (h + 1) * dh].copy_from_slice(&dv[src]);
                }
                for v in [dco, vt, ds, dv, dq, dk] {
                    k::recycle(v);
                }
            }
            k::gemm(&mut gb.wqkv, transposed(&bt.a, d), &dqkv, d, n, 3 * a_w, None);
            let da = matmul(rows(&dqkv, 3 * a_w), &t.wqkv, n, 3 * a_w, d, None);
            let mut dxin = dhres;
            norm_back(&da, &bt.n1, &b.n1, d, &mut dxin, &mut gb.n1);
            for v in [dout, dctx, dqkv, da] {
                k::recycle(v);
            }
            dx = dxin;
        }
        mul_into(&mut dx, &tr.m0);
        // embeddings, token by token in order
        let gn = &tr.gains;
        for t in 0..n {
            let r = &dx[t * d..(t + 1) * d];
            let rows_of = |i: u8| i as usize * d..(i as usize + 1) * d;
            let tk = tr.tokens[t] as usize;
            g.tok[tk * d..(tk + 1) * d].iter_mut().zip(r).for_each(|(a, b)| *a += b);
            if tr.tokens[t] == NUM && !g.num.is_empty() {
                let v = tr.values.get(t).copied().unwrap_or(0.0);
                g.num.iter_mut().zip(r).for_each(|(a, b)| *a += v * b);
            }
            let dl: Vec<f32> = r.iter().map(|v| gn.letter * v).collect();
            let dw: Vec<f32> = r.iter().map(|v| gn.word * v).collect();
            let [cl, co, cr, ca, cq] = &tr.chan;
            let add = |tab: &mut Vec<f32>, idx: u8, v: &[f32]| {
                tab[rows_of(idx)].iter_mut().zip(v).for_each(|(a, b)| *a += b);
            };
            if self.cfg.letters {
                add(&mut g.letter, cl[t], &dl);
            }
            if self.cfg.words {
                add(&mut g.word_ord, co[t], &dw);
                add(&mut g.word_rev, cr[t], &dw);
                add(&mut g.word_atb, ca[t], &dw);
                add(&mut g.word_root, cq[t], &dw);
            }
        }
        dextra
    }
}

/// One byte at a time through a causal model with a key/value cache, giving the
/// generative head's logits for the next byte. Each step computes exactly what the full
/// forward pass computes for that position (the same row operations in the same
/// order), so decoding equals `gen_logits` bit for bit. When the window is full the
/// decoder starts over from CLS with the last half window of bytes.
pub struct Decoder<'a> {
    net: &'a Net,
    gains: Gains,
    /// the bytes in the current window (after its CLS)
    bytes: Vec<u8>,
    /// per block, per head: rotated keys and values, rows of head_dim
    keys: Vec<Vec<Vec<f32>>>,
    vals: Vec<Vec<Vec<f32>>>,
    logits: Vec<f32>,
}

impl<'a> Decoder<'a> {
    /// a decoder that has read CLS and then `prompt`
    pub fn new(net: &'a Net, gains: Gains, prompt: &[u8]) -> Decoder<'a> {
        assert!(
            net.cfg.causal && net.generator.is_some(),
            "decoding needs a causal model with a generative head"
        );
        let mut dec = Decoder {
            net,
            gains,
            bytes: Vec::new(),
            keys: Vec::new(),
            vals: Vec::new(),
            logits: Vec::new(),
        };
        dec.restart(&[]);
        for &b in prompt {
            dec.push(b);
        }
        dec
    }

    /// the logits for the next byte (GEN_OUT: bytes, then END)
    pub fn logits(&self) -> &[f32] {
        &self.logits
    }

    /// the bytes the current window holds
    pub fn window(&self) -> &[u8] {
        &self.bytes
    }

    fn restart(&mut self, keep: &[u8]) {
        let heads = |b: &Block| vec![Vec::new(); b.heads];
        self.keys = self.net.blocks.iter().map(heads).collect();
        self.vals = self.net.blocks.iter().map(heads).collect();
        self.bytes.clear();
        self.step(text::CLS);
        for &b in keep {
            self.bytes.push(b);
            self.step(b as u16);
        }
    }

    /// reads one more byte
    pub fn push(&mut self, b: u8) {
        if self.bytes.len() + 1 >= self.net.cfg.max_len {
            let keep = self.bytes[self.bytes.len() - self.net.cfg.room() / 2..].to_vec();
            self.restart(&keep);
        }
        self.bytes.push(b);
        self.step(b as u16);
    }

    fn step(&mut self, token: u16) {
        let net = self.net;
        let cfg = &net.cfg;
        let (d, dh) = (cfg.d_model, cfg.head_dim);
        let pos = self.bytes.len();
        // the channels of this position, as a causal window computes them
        let mut w1 = Window {
            tokens: vec![token],
            letter: vec![0],
            ordinal: vec![0],
            reversed: vec![0],
            atbash: vec![0],
            root: vec![0],
            ..Window::default()
        };
        if pos > 0 && (cfg.letters || cfg.words) {
            let w = text::window_with(&self.bytes, true, false);
            w1.letter[0] = w.letter[pos];
            w1.ordinal[0] = w.ordinal[pos];
            w1.reversed[0] = w.reversed[pos];
            w1.atbash[0] = w.atbash[pos];
            w1.root[0] = w.root[pos];
        }
        let mut x = net.embed(&w1, &self.gains);
        let scale = 1.0 / (dh as f32).sqrt();
        for (l, b) in net.blocks.iter().enumerate() {
            let a_w = b.heads * dh;
            let (a, _) = norm_fwd(&x, &b.n1, d, cfg.norm_eps);
            let qkv = matmul(rows(&a, d), &b.wqkv, 1, d, 3 * a_w, None);
            let mut ctx = vec![0f32; a_w];
            let mut p = vec![0f32; pos + 1];
            for h in 0..b.heads {
                let mut q = qkv[h * dh..(h + 1) * dh].to_vec();
                let mut kh = qkv[a_w + h * dh..a_w + (h + 1) * dh].to_vec();
                let v = &qkv[2 * a_w + h * dh..2 * a_w + (h + 1) * dh];
                if cfg.qk_norm {
                    q = norm_fwd(&q, &b.qn, dh, cfg.norm_eps).0;
                    kh = norm_fwd(&kh, &b.kn, dh, cfg.norm_eps).0;
                }
                net.rope.apply_at(&mut q, pos);
                net.rope.apply_at(&mut kh, pos);
                self.keys[l][h].extend_from_slice(&kh);
                self.vals[l][h].extend_from_slice(v);
                attend_row(
                    &q,
                    &self.keys[l][h],
                    &self.vals[l][h],
                    pos,
                    dh,
                    scale,
                    &mut p,
                    &mut ctx[h * dh..(h + 1) * dh],
                );
            }
            let o = matmul(rows(&ctx, a_w), &b.wo, 1, a_w, d, None);
            let hres: Vec<f32> = x.iter().zip(&o).map(|(a, b)| a + b).collect();
            let (bn, _) = norm_fwd(&hres, &b.n2, d, cfg.norm_eps);
            let g = matmul(rows(&bn, d), &b.wg, 1, d, b.ff, None);
            let u = matmul(rows(&bn, d), &b.wu, 1, d, b.ff, None);
            let mut s = vec![0f32; g.len()];
            k::sigmoid_slice(&g, &mut s);
            let act: Vec<f32> = g.iter().zip(&s).zip(&u).map(|((g, s), u)| (g * s) * u).collect();
            let dn = matmul(rows(&act, b.ff), &b.wd, 1, b.ff, d, None);
            x = hres.iter().zip(&dn).map(|(a, b)| a + b).collect();
        }
        let (xf, _) = norm_fwd(&x, &net.nf, d, cfg.norm_eps);
        let mut y = matmul(rows(&xf, d), &net.gen_w, 1, d, GEN_OUT, None);
        y.iter_mut().zip(&net.gen_b).for_each(|(a, b)| *a += b);
        self.logits = y;
    }
}

/// a window for this net's sequence length
pub fn window_of(cfg: &Config, bytes: &[u8]) -> Window {
    text::window(&bytes[..bytes.len().min(cfg.room())])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loss(net: &Net, w: &Window, extra: &[f32], c: &[Vec<f32>]) -> f32 {
        let tr = net.forward(w, extra, &Gains::default(), Mode::EVAL, None);
        let mut l = 0.0;
        for (h, ch) in c.iter().enumerate() {
            l += k::dot(&net.logits(&tr, h), ch);
        }
        l
    }

    /// central differences against backward for a sample of every tensor's weights
    fn check(cfg: Config) {
        let heads = [
            HeadSpec::single("a", &["x", "y", "z"]),
            HeadSpec::new("b", HeadKind::Regress, &["v", "w"]),
        ];
        let mut net = Net::new(cfg.clone(), &heads, 3).unwrap();
        // nonzero gains everywhere, and a little noise on the zero-initialised ones
        let mut r = Rng::new(9);
        for (_, t) in net.params_mut() {
            for v in t.iter_mut() {
                *v += r.symmetric(0.05);
            }
        }
        let w = window_of(&cfg, b"The Owl and the Pussy-cat went to sea!");
        let extra: Vec<f32> = (0..cfg.extra).map(|i| 0.3 * i as f32 - 0.2).collect();
        let c: Vec<Vec<f32>> = net
            .heads
            .iter()
            .map(|h| (0..h.width()).map(|_| r.symmetric(1.0)).collect())
            .collect();
        let tr = net.forward(&w, &extra, &Gains::default(), Mode::EVAL, None);
        let mut g = net.zeros_like();
        let dextra = net.backward(&tr, &c, &net.transposes(), &mut g);
        let eps = 4e-3f32;
        let grads: Vec<Vec<f32>> = g.params().into_iter().map(|(_, t)| t.clone()).collect();
        let mut checked = 0;
        for (ti, gt) in grads.iter().enumerate() {
            let len = gt.len();
            if len == 0 {
                continue;
            }
            for s in 0..4 {
                let i = (mix(ti as u64 * 31 + s) as usize) % len;
                let ana = gt[i];
                if ana == 0.0 && ti < 6 {
                    continue; // an embedding row this window never used
                }
                let orig = net.params()[ti].1[i];
                net.params_mut()[ti].1[i] = orig + eps;
                let up = loss(&net, &w, &extra, &c);
                net.params_mut()[ti].1[i] = orig - eps;
                let dn = loss(&net, &w, &extra, &c);
                net.params_mut()[ti].1[i] = orig;
                let num = (up - dn) / (2.0 * eps);
                let tol = 3e-3 + 3e-2 * num.abs().max(ana.abs());
                assert!(
                    (num - ana).abs() <= tol,
                    "tensor {ti} [{i}]: numeric {num} analytic {ana} ({cfg:?})"
                );
                checked += 1;
            }
        }
        assert!(checked > 40);
        let mut e = extra.clone();
        for i in 0..e.len() {
            let o = e[i];
            e[i] = o + eps;
            let up = loss(&net, &w, &e, &c);
            e[i] = o - eps;
            let dn = loss(&net, &w, &e, &c);
            e[i] = o;
            let num = (up - dn) / (2.0 * eps);
            assert!(
                (num - dextra[i]).abs() <= 3e-3 + 3e-2 * num.abs(),
                "extra {i}: {num} {}",
                dextra[i]
            );
        }
    }

    #[test]
    fn gradients_match_finite_differences() {
        for pooling in [Pooling::Cls, Pooling::Mean, Pooling::ClsMean, Pooling::Attention] {
            for qk_norm in [true, false] {
                check(Config {
                    pooling,
                    qk_norm,
                    extra: 2,
                    ..Config::tiny()
                });
            }
        }
    }

    /// a causal model with a generative head, numbers and no gematria: finite
    /// differences on every tensor through the generative logits and a pooled head
    #[test]
    fn generative_gradients_match_finite_differences() {
        let cfg = Config {
            causal: true,
            numbers: true,
            pooling: Pooling::Mean,
            ..Config::tiny().gematria(false)
        };
        let heads = [HeadSpec::single("a", &["x", "y"])];
        let mut net = Net::with_gen(cfg.clone(), &heads, Some(GenSpec::new("next")), 4).unwrap();
        let mut r = Rng::new(10);
        for (_, t) in net.params_mut() {
            for v in t.iter_mut() {
                *v += r.symmetric(0.05);
            }
        }
        let mut w = text::window_with(b"owl and cat", true, true);
        w.push_numbers(&[0.5, -1.25]);
        let n = w.len();
        let ch: Vec<f32> = (0..2).map(|_| r.symmetric(1.0)).collect();
        let cg: Vec<f32> = (0..n * GEN_OUT).map(|_| r.symmetric(1.0)).collect();
        let loss = |net: &Net| {
            let tr = net.forward(&w, &[], &Gains::default(), Mode::EVAL, None);
            k::dot(&net.logits(&tr, 0), &ch) + k::dot(&net.gen_logits(&tr), &cg)
        };
        let tr = net.forward(&w, &[], &Gains::default(), Mode::EVAL, None);
        let mut g = net.zeros_like();
        net.backward_gen(&tr, std::slice::from_ref(&ch), &cg, &net.transposes(), &mut g);
        drop(tr);
        let grads: Vec<Vec<f32>> = g.params().into_iter().map(|(_, t)| t.clone()).collect();
        let eps = 4e-3f32;
        let mut checked = 0;
        for (ti, gt) in grads.iter().enumerate() {
            for s in 0..4 {
                if gt.is_empty() {
                    break;
                }
                let i = (mix(ti as u64 * 31 + s) as usize) % gt.len();
                if gt[i] == 0.0 && ti == 0 {
                    continue;
                }
                let orig = net.params()[ti].1[i];
                net.params_mut()[ti].1[i] = orig + eps;
                let up = loss(&net);
                net.params_mut()[ti].1[i] = orig - eps;
                let dn = loss(&net);
                net.params_mut()[ti].1[i] = orig;
                let num = (up - dn) / (2.0 * eps);
                let tol = 5e-3 + 3e-2 * num.abs().max(gt[i].abs());
                assert!(
                    (num - gt[i]).abs() <= tol,
                    "tensor {ti} [{i}]: numeric {num} analytic {}",
                    gt[i]
                );
                checked += 1;
            }
        }
        assert!(checked > 40);
    }

    /// decoding with the cache equals the full causal pass, bit for bit, including
    /// across a window restart
    #[test]
    fn decoder_equals_the_full_pass() {
        for gematria in [false, true] {
            let cfg = Config {
                causal: true,
                pooling: Pooling::Mean,
                max_len: 12,
                stride: 5,
                ..Config::tiny().gematria(gematria)
            };
            let net = Net::with_gen(cfg, &[], Some(GenSpec::new("next")), 7).unwrap();
            let text = b"Ab cd, efg";
            let mut dec = Decoder::new(&net, Gains::default(), &[]);
            let w = text::window_with(text, true, false);
            let tr = net.forward(&w, &[], &Gains::default(), Mode::EVAL, None);
            let full = net.gen_logits(&tr);
            assert_eq!(dec.logits(), &full[..GEN_OUT]);
            for (i, &b) in text.iter().enumerate() {
                dec.push(b);
                assert_eq!(
                    dec.logits(),
                    &full[(i + 1) * GEN_OUT..(i + 2) * GEN_OUT],
                    "position {}",
                    i + 1
                );
            }
            // past max_len the decoder keeps the last half window
            dec.push(b'!');
            dec.push(b'?');
            let kept = dec.window().to_vec();
            assert_eq!(kept, b" efg!?");
            let w = text::window_with(&kept, true, false);
            let tr = net.forward(&w, &[], &Gains::default(), Mode::EVAL, None);
            let full = net.gen_logits(&tr);
            assert_eq!(dec.logits(), &full[kept.len() * GEN_OUT..]);
        }
    }

    #[test]
    fn rope_back_is_the_transpose() {
        let rope = Rope::new(8, 4, 10000.0);
        let mut r = Rng::new(1);
        let x: Vec<f32> = (0..32).map(|_| r.symmetric(1.0)).collect();
        let y: Vec<f32> = (0..32).map(|_| r.symmetric(1.0)).collect();
        let (mut rx, mut ry) = (x.clone(), y.clone());
        rope.apply(&mut rx, 8);
        rope.back(&mut ry, 8);
        assert!((k::dot(&rx, &y) - k::dot(&x, &ry)).abs() < 1e-5);
        // position 0 is no rotation
        assert_eq!(&rx[..4], &x[..4]);
    }

    #[test]
    fn dropout_masks_are_keyed() {
        let a = dropout_mask(5, 1000, 0.25);
        assert_eq!(a, dropout_mask(5, 1000, 0.25));
        assert_ne!(a, dropout_mask(6, 1000, 0.25));
        let kept = a.iter().filter(|&&v| v > 0.0).count();
        assert!((700..800).contains(&kept));
    }
}
