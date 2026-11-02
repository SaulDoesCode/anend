//! s's values (docs/s.md §3): text, sometimes held as a number. A number's text is its
//! canonical form (shortest round-trip decimal, no exponent, no trailing `.0`, `-0` as `0`,
//! `NaN`, `inf`, `-inf`); text read as a number is trimmed and parsed, and what doesn't
//! parse reads as 0. Which one a value is held as can't be seen.

use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum V {
    N(f64),
    T(Rc<str>),
}

/// a number's canonical text
pub fn num_text(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "inf".into() } else { "-inf".into() }
    } else if x == 0.0 {
        "0".into()
    } else {
        // Rust writes the shortest round-trip decimal and never an exponent
        format!("{x}")
    }
}

/// text as a number: trimmed and parsed as a decimal float, 0 when it isn't one
pub fn text_num(t: &str) -> f64 {
    let t = t.trim();
    match t {
        "inf" | "+inf" => f64::INFINITY,
        "-inf" => f64::NEG_INFINITY,
        "NaN" => f64::NAN,
        _ => t
            .parse::<f64>()
            .ok()
            .filter(|_| {
                t.chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
            })
            .unwrap_or(0.0),
    }
}

impl V {
    pub fn text(&self) -> Rc<str> {
        match self {
            V::T(t) => t.clone(),
            V::N(x) => num_text(*x).into(),
        }
    }

    pub fn num(&self) -> f64 {
        match self {
            V::N(x) => *x,
            V::T(t) => text_num(t),
        }
    }

    pub fn str(s: &str) -> V {
        V::T(s.into())
    }

    /// whether two values have the same text
    pub fn same(&self, o: &V) -> bool {
        match (self, o) {
            (V::N(a), V::N(b)) => num_text(*a) == num_text(*b),
            _ => self.text() == o.text(),
        }
    }
}

impl From<String> for V {
    fn from(s: String) -> V {
        V::T(s.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_numbers() {
        assert_eq!(num_text(3.0), "3");
        assert_eq!(num_text(-0.0), "0");
        assert_eq!(num_text(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(num_text(1e21), "1000000000000000000000");
        assert_eq!(num_text(f64::NAN), "NaN");
        assert_eq!(num_text(f64::NEG_INFINITY), "-inf");
        assert_eq!(text_num(" 2.5 "), 2.5);
        assert_eq!(text_num("abc"), 0.0);
        assert_eq!(text_num("1e3"), 1000.0);
        assert!(V::N(3.0).same(&V::str("3")));
        assert!(!V::str("3.0").same(&V::N(3.0)));
    }
}
