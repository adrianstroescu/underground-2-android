//! World collision: triangles from road/terrain/building geometry in a 2D hash grid.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

pub struct Tri {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    /// Unit normal, flipped to point up for ground triangles.
    pub n: Vec3,
    pub wall: bool,
    /// Index into [`CollisionWorld::owners`] (debugging which object blocks the car).
    pub owner: u32,
    /// Scenery group set the triangle belongs to (0 = always there).
    pub group: u16,
}

pub struct CollisionWorld {
    cell: f32,
    cells: HashMap<(i32, i32), Vec<u32>>,
    pub tris: Vec<Tri>,
    pub owners: Vec<String>,
    group: u16,
    /// Which group sets are switched on (race barriers); index 0 is unused.
    groups_on: Vec<bool>,
}

impl Default for CollisionWorld {
    fn default() -> Self {
        Self::new(8.0)
    }
}

/// Surfaces steeper than this (|n.z| below) are walls.
const WALL_NZ: f32 = 0.6;

impl CollisionWorld {
    pub fn new(cell: f32) -> Self {
        Self { cell, cells: HashMap::new(), tris: Vec::new(), owners: Vec::new(), group: 0, groups_on: Vec::new() }
    }

    fn key(&self, x: f32, y: f32) -> (i32, i32) {
        ((x / self.cell).floor() as i32, (y / self.cell).floor() as i32)
    }

    /// `ground_ok` false keeps only the wall part (used for buildings, whose roofs
    /// should not catch a car that clips into them).
    /// Names the object whose triangles are added next.
    pub fn begin_owner(&mut self, name: &str, group: u16) {
        self.owners.push(name.to_string());
        self.group = group;
    }

    /// Switches scenery group sets on or off (`on[set]`); sets past the end are off.
    pub fn set_groups_on(&mut self, on: Vec<bool>) {
        self.groups_on = on;
    }

    fn active(&self, t: &Tri) -> bool {
        t.group == 0 || self.groups_on.get(t.group as usize).copied().unwrap_or(false)
    }

    pub fn add(&mut self, a: Vec3, b: Vec3, c: Vec3, ground_ok: bool) {
        let n = (b - a).cross(c - a);
        let len = n.length();
        if len < 1e-6 {
            return;
        }
        let mut n = n / len;
        let wall = n.z.abs() < WALL_NZ;
        if !wall && !ground_ok {
            return;
        }
        if !wall && n.z < 0.0 {
            n = -n;
        }
        let idx = self.tris.len() as u32;
        let owner = self.owners.len().saturating_sub(1) as u32;
        self.tris.push(Tri { a, b, c, n, wall, owner, group: self.group });
        let min = a.min(b).min(c);
        let max = a.max(b).max(c);
        let (k0, k1) = (self.key(min.x, min.y), self.key(max.x, max.y));
        // Huge triangles (skydome-like) are not worth indexing.
        if (k1.0 - k0.0) > 64 || (k1.1 - k0.1) > 64 {
            return;
        }
        for x in k0.0..=k1.0 {
            for y in k0.1..=k1.1 {
                self.cells.entry((x, y)).or_default().push(idx);
            }
        }
    }

    fn cell_tris(&self, x: f32, y: f32) -> &[u32] {
        self.cells.get(&self.key(x, y)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Highest ground surface under (p.x, p.y) between `p.z - below` and `p.z + above`.
    pub fn ground(&self, p: Vec3, above: f32, below: f32) -> Option<(f32, Vec3)> {
        let mut best: Option<(f32, Vec3)> = None;
        for &i in self.cell_tris(p.x, p.y) {
            let t = &self.tris[i as usize];
            if t.wall || !self.active(t) {
                continue;
            }
            let Some(z) = height_on(t, Vec2::new(p.x, p.y)) else { continue };
            if z <= p.z + above && z >= p.z - below && best.is_none_or(|(bz, _)| z > bz) {
                best = Some((z, t.n));
            }
        }
        best
    }

    /// Calls `f(push_normal, depth, triangle)` for every wall triangle within `radius` of `p`
    /// that reaches above `min_top`. Lower ones are curbs and the vertical seams between
    /// road pieces, which a car drives over.
    pub fn walls(&self, p: Vec3, radius: f32, min_top: f32, mut f: impl FnMut(Vec3, f32, &Tri)) {
        let mut seen: Vec<u32> = Vec::new();
        let r = radius;
        for (dx, dy) in [(-r, -r), (r, -r), (-r, r), (r, r)] {
            for &i in self.cell_tris(p.x + dx, p.y + dy) {
                if seen.contains(&i) {
                    continue;
                }
                seen.push(i);
                let t = &self.tris[i as usize];
                if !t.wall || t.a.z.max(t.b.z).max(t.c.z) < min_top || !self.active(t) {
                    continue;
                }
                let q = closest_point(p, t.a, t.b, t.c);
                let d = p - q;
                let dist = d.length();
                if dist < radius && dist > 1e-5 {
                    let mut n = d / dist;
                    n.z = 0.0;
                    let n = n.normalize_or_zero();
                    if n != Vec3::ZERO {
                        f(n, radius - dist, t);
                    }
                }
            }
        }
    }
}

fn height_on(t: &Tri, p: Vec2) -> Option<f32> {
    let (a, b, c) = (t.a, t.b, t.c);
    let v0 = Vec2::new(b.x - a.x, b.y - a.y);
    let v1 = Vec2::new(c.x - a.x, c.y - a.y);
    let v2 = p - Vec2::new(a.x, a.y);
    let den = v0.x * v1.y - v1.x * v0.y;
    if den.abs() < 1e-9 {
        return None;
    }
    let v = (v2.x * v1.y - v1.x * v2.y) / den;
    let w = (v0.x * v2.y - v2.x * v0.y) / den;
    let u = 1.0 - v - w;
    const E: f32 = -1e-4;
    if u < E || v < E || w < E {
        return None;
    }
    Some(u * a.z + v * b.z + w * c.z)
}

/// Closest point on triangle abc to p (Ericson, Real-Time Collision Detection 5.1.5).
pub fn closest_point(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    a + ab * v + ac * w
}
