//! Settling placements that come in on top of others (docs/format.md, answer 5): when two
//! scenes become one, or a merge brings another save's blocks into the room, what was
//! there stays put ("keep" semantics) and each incoming body is pushed out of whatever it
//! overlaps, done once when the merge is made and written as the settled positions.
//!
//! Deterministic: the bodies are boxes (centre and half sides), the incoming ones are
//! settled one at a time in the order given, each pushed along the axis it overlaps
//! least, away from the other's centre (ties: x, then y, then z, and the positive way),
//! with plain f32 arithmetic in a fixed order, so the same inputs always give the same
//! positions on every machine.

/// a box: its centre and half its sides
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub at: [f32; 3],
    pub half: [f32; 3],
}

/// room left between two bodies once they're apart
pub const GAP: f32 = 0.05;

fn overlap(a: &Body, b: &Body) -> Option<[f32; 3]> {
    let o: [f32; 3] = std::array::from_fn(|i| a.half[i] + b.half[i] + GAP - (a.at[i] - b.at[i]).abs());
    o.iter().all(|v| *v > 0.0).then_some(o)
}

/// moves each of `incoming` (in order) out of `fixed` and out of the incoming ones
/// settled before it; how many moved
pub fn settle(fixed: &[Body], incoming: &mut [Body]) -> usize {
    let mut moved = 0;
    for k in 0..incoming.len() {
        let mut b = incoming[k];
        let start = b.at;
        // each push clears one overlap; a few hundred rounds settle any crowd met in practice
        for _ in 0..512 {
            let hit = fixed
                .iter()
                .chain(incoming[..k].iter())
                .find_map(|o| overlap(&b, o).map(|ov| (*o, ov)));
            let Some((o, ov)) = hit else { break };
            let mut axis = 0;
            for i in 1..3 {
                if ov[i] < ov[axis] {
                    axis = i;
                }
            }
            let dir = if b.at[axis] >= o.at[axis] { 1.0 } else { -1.0 };
            // a hair past, so rounding never leaves a sliver of overlap
            b.at[axis] += dir * (ov[axis] + 1e-4 * (1.0 + b.at[axis].abs() * 1e-3));
        }
        if b.at != start {
            moved += 1;
        }
        incoming[k] = b;
    }
    moved
}

/// a text placement's box, roughly as the space draws it: about a quarter unit a
/// character wide (up to 12 units), half a unit high, scaled by its size, and deep (a
/// label faces the viewer, so two at different depths still cover each other: they part
/// sideways or up and down, as the space's own overlap rule has them)
pub fn text_body(at: [f32; 3], chars: usize, size: f32) -> Body {
    let k = size / 0.5;
    Body {
        at,
        half: [((chars as f32 * 0.27 + 0.1).min(12.0)) * k, 0.25 * k, 0.75],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(x: f32, y: f32) -> Body {
        Body {
            at: [x, y, 0.0],
            half: [0.5; 3],
        }
    }

    #[test]
    fn incoming_bodies_leave_what_was_there_and_each_other() {
        let fixed = [cube(0.0, 0.0), cube(3.0, 0.0)];
        let mut came = [cube(0.2, 0.0), cube(0.2, 0.1), cube(10.0, 10.0)];
        let moved = settle(&fixed, &mut came);
        assert_eq!(moved, 2, "the far one stays");
        assert_eq!(came[2], cube(10.0, 10.0));
        let all: Vec<Body> = fixed.iter().chain(came.iter()).copied().collect();
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert!(overlap(&all[i], &all[j]).is_none(), "{i} and {j} still overlap");
            }
        }
        // the same inputs, the same bits
        let mut again = [cube(0.2, 0.0), cube(0.2, 0.1), cube(10.0, 10.0)];
        settle(&fixed, &mut again);
        assert_eq!(again, came);
        // a crowd of identical bodies spreads out without overlaps
        let mut crowd = vec![cube(0.0, 0.0); 40];
        settle(&[cube(0.0, 0.0)], &mut crowd);
        for i in 0..crowd.len() {
            for j in i + 1..crowd.len() {
                assert!(
                    overlap(&crowd[i], &crowd[j]).is_none(),
                    "{i} {:?} {j} {:?}",
                    crowd[i],
                    crowd[j]
                );
            }
        }
    }
}
