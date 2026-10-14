//! The measures an entry is filed under: ordinal and reversed sums (from `simd`) and the
//! letter frequency key.

use crate::simd;

/// which number a text is filed under
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mode {
    /// a = 1 .. z = 26, other characters 0 (`simd::sums`)
    Ordinal = 0,
    /// a = 26 .. z = 1 (`simd::sums`)
    Reversed = 1,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Ordinal, Mode::Reversed];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Ordinal => "ordinal",
            Mode::Reversed => "reversed",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.name() == s.trim())
    }

    pub fn from_u8(b: u8) -> Option<Mode> {
        Mode::ALL.get(b as usize).copied()
    }
}

/// everything a text is filed under
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measure {
    pub ordinal: i64,
    pub reversed: i64,
    pub frequency: String,
}

impl Measure {
    pub fn of(text: &str) -> Measure {
        let s = simd::sums(text.as_bytes());
        Measure {
            ordinal: s.ordinal as i64,
            reversed: s.reversed as i64,
            frequency: simd::frequency_key(text.as_bytes()),
        }
    }

    pub fn get(&self, mode: Mode) -> i64 {
        match mode {
            Mode::Ordinal => self.ordinal,
            Mode::Reversed => self.reversed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes() {
        let m = Measure::of("hello");
        assert_eq!((m.get(Mode::Ordinal), m.get(Mode::Reversed)), (52, 83));
        assert_eq!(Mode::parse(" reversed "), Some(Mode::Reversed));
        assert_eq!(Mode::parse("atbash"), None);
        assert_eq!(Mode::from_u8(0), Some(Mode::Ordinal));
        assert_eq!(Mode::from_u8(1), Some(Mode::Reversed));
        assert_eq!(Mode::from_u8(2), None);
    }
}
