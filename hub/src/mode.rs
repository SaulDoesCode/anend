//! What a server runs, set with `--mode` (and `--local`):
//!
//! - `hub`: the sharing site (saves, classifiers, people, the leaderboard) and its API;
//! - `app`: the web app (each member's saves in a workspace of their own) and the app's API;
//! - `both` (the default): all of it, over one account system;
//! - `--local`: the web app alone for one person on their own machine, with no login.
//!
//! A route that isn't in the mode doesn't exist (a 404), and the pages only offer what's
//! there. A build may leave either half out (cargo features `hub` and `app`); a mode the
//! build doesn't carry is refused when the server starts.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    /// the sharing site and its API
    pub hub: bool,
    /// the web app and its API
    pub app: bool,
    /// one workspace and no login (implies the app alone)
    pub local: bool,
}

impl Mode {
    pub const BOTH: Mode = Mode {
        hub: true,
        app: true,
        local: false,
    };
    pub const HUB: Mode = Mode {
        hub: true,
        app: false,
        local: false,
    };
    pub const APP: Mode = Mode {
        hub: false,
        app: true,
        local: false,
    };
    pub const LOCAL: Mode = Mode {
        hub: false,
        app: true,
        local: true,
    };

    /// what this build carries, which is also the default mode
    pub const fn built() -> Mode {
        Mode {
            hub: cfg!(feature = "hub"),
            app: cfg!(feature = "app"),
            local: false,
        }
    }

    /// `hub`, `app` or `both`; refused when the build left that half out
    pub fn parse(s: &str) -> Result<Mode, String> {
        let m = match s {
            "hub" => Mode::HUB,
            "app" => Mode::APP,
            "both" => Mode::BOTH,
            _ => return Err(format!("--mode is hub, app or both (not {s})")),
        };
        m.check()?;
        Ok(m)
    }

    /// whether this build can run the mode
    pub fn check(self) -> Result<(), String> {
        let built = Mode::built();
        if self.hub && !built.hub {
            return Err("this build leaves the hub out (it was built without the `hub` feature)".into());
        }
        if self.app && !built.app {
            return Err("this build leaves the web app out (it was built without the `app` feature)".into());
        }
        Ok(())
    }

    pub fn name(self) -> &'static str {
        match (self.local, self.hub, self.app) {
            (true, _, _) => "local",
            (_, true, true) => "both",
            (_, true, false) => "hub",
            _ => "app",
        }
    }
}

static MODE: OnceLock<Mode> = OnceLock::new();

/// the mode the server started in (set once, by main)
pub fn set(m: Mode) {
    MODE.set(m).ok();
}

/// the server's mode, for code that has no request at hand (a request's own mode is
/// `web::mode`, which the router sets, so tests can build a router for each mode)
pub fn get() -> Mode {
    MODE.get().copied().unwrap_or(Mode::built())
}

/// one workspace for everyone and no login: a daemon on your own machine
#[cfg_attr(not(feature = "app"), allow(dead_code))]
pub fn local() -> bool {
    get().local
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for (s, m) in [("hub", Mode::HUB), ("app", Mode::APP), ("both", Mode::BOTH)] {
            if m.check().is_ok() {
                assert_eq!(Mode::parse(s).unwrap(), m);
                assert_eq!(m.name(), s);
            } else {
                assert!(Mode::parse(s).is_err());
            }
        }
        assert_eq!(Mode::LOCAL.name(), "local");
        assert!(Mode::parse("all").is_err());
    }
}
