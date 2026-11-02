//! Simple shapes for what's drawn over the room (the gizmo, light markers, outlines,
//! wires), as three.js's geometries made them, and a ray's hit on triangles.

use std::f32::consts::{PI, TAU};

use glam::{Mat4, Quat, Vec3};

use crate::Vert;

/// triangles as a flat list of corners
pub type Tris = Vec<Vec3>;

/// a cylinder along y, centred (three.js's CylinderGeometry, open ends closed)
pub fn cylinder(r_top: f32, r_bottom: f32, h: f32, segs: u32) -> Tris {
    let mut out = Vec::new();
    let at = |i: u32, y: f32, r: f32| {
        let a = TAU * i as f32 / segs as f32;
        Vec3::new(r * a.sin(), y, r * a.cos())
    };
    let (top, bot) = (h / 2.0, -h / 2.0);
    for i in 0..segs {
        let (a, b) = (at(i, top, r_top), at(i + 1, top, r_top));
        let (c, d) = (at(i, bot, r_bottom), at(i + 1, bot, r_bottom));
        out.extend_from_slice(&[a, c, b, b, c, d]);
        out.extend_from_slice(&[Vec3::new(0.0, top, 0.0), a, b]);
        out.extend_from_slice(&[Vec3::new(0.0, bot, 0.0), d, c]);
    }
    out
}

/// a cone along y, its point up, centred
pub fn cone(r: f32, h: f32, segs: u32) -> Tris {
    cylinder(0.0, r, h, segs)
}

/// a ring in the xy plane round z (three.js's TorusGeometry)
pub fn torus(radius: f32, tube: f32, radial: u32, tubular: u32) -> Tris {
    let at = |j: u32, i: u32| {
        let u = i as f32 / tubular as f32 * TAU;
        let v = j as f32 / radial as f32 * TAU;
        Vec3::new(
            (radius + tube * v.cos()) * u.cos(),
            (radius + tube * v.cos()) * u.sin(),
            tube * v.sin(),
        )
    };
    let mut out = Vec::new();
    for j in 1..=radial {
        for i in 1..=tubular {
            let (a, b, c, d) = (at(j, i - 1), at(j - 1, i - 1), at(j - 1, i), at(j, i));
            out.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }
    out
}

/// a ball, centred
pub fn ball(r: f32, segs: u32, rings: u32) -> Tris {
    let at = |i: u32, j: u32| {
        let th = PI * i as f32 / rings as f32;
        let ph = TAU * j as f32 / segs as f32;
        Vec3::new(-r * ph.cos() * th.sin(), r * th.cos(), r * ph.sin() * th.sin())
    };
    let mut out = Vec::new();
    for i in 0..rings {
        for j in 0..segs {
            let (a, b, c, d) = (at(i, j + 1), at(i, j), at(i + 1, j), at(i + 1, j + 1));
            out.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }
    out
}

/// a box, centred
pub fn cuboid(s: Vec3) -> Tris {
    crate::unit_shape(false, [1.0; 4])
        .iter()
        .map(|v| Vec3::from(v.pos) * s)
        .collect()
}

/// a box's twelve edges, as line segments (two corners each)
pub fn edges(s: Vec3) -> Vec<Vec3> {
    let h = s / 2.0;
    let c = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
    let mut out = Vec::new();
    for a in [-1.0, 1.0] {
        for b in [-1.0, 1.0] {
            out.extend_from_slice(&[c(-1.0, a, b), c(1.0, a, b)]);
            out.extend_from_slice(&[c(a, -1.0, b), c(a, 1.0, b)]);
            out.extend_from_slice(&[c(a, b, -1.0), c(a, b, 1.0)]);
        }
    }
    out
}

/// segments cut into dashes (so long, so far apart)
pub fn dashed(segs: &[Vec3], dash: f32, gap: f32) -> Vec<Vec3> {
    let mut out = Vec::new();
    for s in segs.as_chunks::<2>().0 {
        let (a, b) = (s[0], s[1]);
        let len = a.distance(b);
        let d = (b - a) / len.max(1e-6);
        let mut t = 0.0;
        while t < len {
            out.push(a + d * t);
            out.push(a + d * (t + dash).min(len));
            t += dash + gap;
        }
    }
    out
}

/// corners placed (turned, moved, scaled) and coloured
pub fn verts(tris: &[Vec3], m: Mat4, colour: [f32; 4]) -> Vec<Vert> {
    tris.iter()
        .map(|p| Vert {
            pos: m.transform_point3(*p).to_array(),
            uv: [0.0, 0.0],
            colour,
        })
        .collect()
}

/// a turn taking `from` onto `to` (both unit)
pub fn turn(from: Vec3, to: Vec3) -> Quat {
    Quat::from_rotation_arc(from.normalize_or(Vec3::Y), to.normalize_or(Vec3::Y))
}

/// how far along a ray it meets a triangle (Möller–Trumbore), either side
pub fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-5).then_some(t)
}

/// the nearest hit of a ray on triangles placed by `m`, as a distance along the ray
pub fn ray_tris(o: Vec3, d: Vec3, tris: &[Vec3], m: Mat4) -> Option<f32> {
    let inv = m.inverse();
    let (lo, ld) = (inv.transform_point3(o), inv.transform_vector3(d));
    let mut best: Option<f32> = None;
    for t in tris.as_chunks::<3>().0 {
        if let Some(s) = ray_tri(lo, ld, t[0], t[1], t[2])
            && best.is_none_or(|b| s < b)
        {
            best = Some(s);
        }
    }
    // the same parameter in the room (the transform is affine)
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_meet_shapes() {
        let b = cuboid(Vec3::ONE);
        let t = ray_tris(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, &b, Mat4::IDENTITY).unwrap();
        assert!((t - 4.5).abs() < 1e-4);
        let moved = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        assert!(ray_tris(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, &b, moved).is_none());
        let r = torus(1.25, 0.09, 6, 48);
        assert!(ray_tris(Vec3::new(1.25, 0.0, 5.0), Vec3::NEG_Z, &r, Mat4::IDENTITY).is_some());
        assert!(ray_tris(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, &r, Mat4::IDENTITY).is_none());
        assert_eq!(edges(Vec3::ONE).len(), 24);
        assert!(!cylinder(0.1, 0.1, 1.0, 8).is_empty() && !cone(0.1, 0.2, 8).is_empty() && !ball(0.1, 8, 6).is_empty());
    }
}
