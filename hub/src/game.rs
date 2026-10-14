//! The game in the hub: points for sharing saves people use, levels with titles from the
//! scribes and cipherers of old, and badges for milestones. Points come from what other
//! people do with your saves, so they can't be farmed alone: a download counts once per
//! person, a star once per account, and starring your own save earns nothing. Shared
//! classifiers earn the same way as saves, and have badges of their own.

use crate::store::{ClassifierRec, Save, Status, User};

/// an admin let one of your saves in
pub const APPROVED: i64 = 50;
/// someone starred one of your saves
pub const STAR: i64 = 5;
/// someone took one of your saves for the first time
pub const DOWNLOAD: i64 = 1;
/// you starred someone's save
pub const CURATE: i64 = 1;
/// an admin featured one of your saves
pub const FEATURED: i64 = 100;

const TITLES: [&str; 10] = [
    "seeker",
    "reader",
    "scribe",
    "counter",
    "cipherer",
    "numerologist",
    "gematrian",
    "archivist",
    "palace keeper",
    "sage",
];

/// level 1 at 0 points; each level costs a little more than the last
pub fn level(points: i64) -> u32 {
    let p = points.max(0) as f64;
    (p / 25.0).sqrt().floor() as u32 + 1
}

/// points needed to reach a level
pub fn points_for(level: u32) -> i64 {
    let l = level.saturating_sub(1) as i64;
    l * l * 25
}

pub fn title(level: u32) -> &'static str {
    TITLES[(level.saturating_sub(1) as usize).min(TITLES.len() - 1)]
}

/// how far through their level someone is, 0..100
pub fn progress(points: i64) -> u32 {
    let l = level(points);
    let (lo, hi) = (points_for(l), points_for(l + 1));
    (((points - lo) * 100) / (hi - lo).max(1)).clamp(0, 100) as u32
}

pub struct Badge {
    pub name: &'static str,
    pub about: &'static str,
    pub glyph: &'static str,
}

/// the badges a user has earned, from their saves, their classifiers and what they've done
pub fn badges(user: &User, saves: &[&Save], classifiers: &[&ClassifierRec], stars_given: u64) -> Vec<Badge> {
    let shared: Vec<&&ClassifierRec> = classifiers.iter().filter(|c| c.status == Status::Approved).collect();
    let approved: Vec<&&Save> = saves.iter().filter(|s| s.status == Status::Approved).collect();
    let stars: u64 = approved.iter().map(|s| s.stars).sum();
    let downloads: u64 = approved.iter().map(|s| s.downloads).sum();
    let memories: u64 = approved.iter().map(|s| s.memories).sum();
    let mut out = Vec::new();
    let mut give = |yes: bool, name, about, glyph| {
        if yes {
            out.push(Badge { name, about, glyph });
        }
    };
    give(user.number <= 10, "founder", "one of the hub's first ten", "א");
    give(user.is_admin(), "keeper", "looks after the hub", "ת");
    give(!approved.is_empty(), "first save", "shared a save", "1");
    give(approved.len() >= 5, "prolific", "shared five saves", "5");
    give(
        approved.iter().any(|s| s.featured),
        "featured",
        "had a save featured",
        "★",
    );
    give(
        approved.iter().any(|s| s.stars >= 10),
        "beloved",
        "a save with ten stars",
        "♥",
    );
    give(stars >= 100, "luminary", "a hundred stars in all", "☉");
    give(downloads >= 100, "well read", "saves taken a hundred times", "∞");
    give(memories >= 10_000, "memory palace", "ten thousand memories shared", "▦");
    give(
        (1_000..10_000).contains(&memories),
        "library",
        "a thousand memories shared",
        "▤",
    );
    give(stars_given >= 10, "curator", "starred ten saves", "✧");
    give(!shared.is_empty(), "first classifier", "shared a classifier", "◇");
    give(shared.len() >= 5, "trainer", "shared five classifiers", "◈");
    give(
        shared.iter().any(|c| c.stars >= 10),
        "oracle",
        "a classifier with ten stars",
        "◉",
    );
    give(
        shared.iter().map(|c| c.downloads).sum::<u64>() >= 50,
        "widely taught",
        "classifiers taken fifty times",
        "⟁",
    );
    give(
        shared
            .iter()
            .any(|c| c.examples >= 100 && c.accuracy.is_some_and(|a| a >= 0.9)),
        "sharp eye",
        "a classifier 90% right on a hundred examples",
        "◎",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(level(0), 1);
        assert_eq!(level(24), 1);
        assert_eq!(level(25), 2);
        assert_eq!(level(100), 3);
        assert_eq!(level(-10), 1);
        for l in 1..20 {
            assert_eq!(level(points_for(l)), l);
            assert_eq!(level(points_for(l + 1) - 1), l);
        }
        assert_eq!(title(1), "seeker");
        assert_eq!(title(99), "sage");
        assert_eq!(progress(0), 0);
        assert_eq!(progress(50), 33);
    }

    #[test]
    fn badges_follow_saves() {
        let u = User {
            number: 3,
            ..Default::default()
        };
        let s = Save {
            status: Status::Approved,
            stars: 12,
            memories: 2000,
            ..Default::default()
        };
        let names: Vec<_> = badges(&u, &[&s], &[], 0).iter().map(|b| b.name).collect();
        assert_eq!(names, ["founder", "first save", "beloved", "library"]);
        let c = ClassifierRec {
            status: Status::Approved,
            stars: 10,
            examples: 400,
            accuracy: Some(0.95),
            ..Default::default()
        };
        let pending = ClassifierRec::default();
        let names: Vec<_> = badges(&u, &[], &[&c, &pending], 0).iter().map(|b| b.name).collect();
        assert_eq!(names, ["founder", "first classifier", "oracle", "sharp eye"]);
        let pending = Save::default();
        assert!(
            badges(
                &User {
                    number: 40,
                    ..Default::default()
                },
                &[&pending],
                &[],
                0
            )
            .is_empty()
        );
    }
}
