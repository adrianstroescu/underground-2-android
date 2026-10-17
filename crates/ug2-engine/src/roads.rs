//! Road data for gameplay: the free-roam road network as a graph traffic can drive,
//! race definitions (track table, career list, grids, start/finish lines) and the
//! centre line of every race in the loaded region.

use std::collections::HashMap;

use glam::{Vec2, Vec3};
use ug2_formats::fs::GameDir;
use ug2_formats::route::{
    parse_career_races, parse_markers, parse_routes, parse_track_infos, race_line, CareerRace, MarkerKind, RaceType,
    Route, RouteNetwork, TrackMarker,
};

/// One end of a route at a junction node.
#[derive(Clone, Copy, Debug)]
pub struct RouteEnd {
    pub route: usize,
    /// True for the route's first point, false for its last.
    pub start: bool,
}

/// Free-roam roads: routes plus junction nodes formed by route ends that meet.
pub struct RoadGraph {
    pub routes: Vec<Route>,
    /// Node index of each route's first and last point.
    pub ends: Vec<[usize; 2]>,
    pub nodes: Vec<Vec<RouteEnd>>,
}

impl RoadGraph {
    fn build(net: RouteNetwork) -> Self {
        let routes: Vec<Route> = net.routes.into_iter().filter(|r| r.points.len() >= 2).collect();
        let mut node_pos: Vec<Vec2> = Vec::new();
        let mut nodes: Vec<Vec<RouteEnd>> = Vec::new();
        let mut ends = Vec::with_capacity(routes.len());
        for (i, r) in routes.iter().enumerate() {
            let mut pair = [0; 2];
            for (k, start) in [(0, true), (1, false)] {
                let p = if start { r.points[0].pos } else { r.points[r.points.len() - 1].pos };
                let p = Vec2::new(p[0], p[1]);
                // Route ends meeting at a junction lie within a few metres of each other.
                let n = match node_pos.iter().position(|q| q.distance(p) < 7.0) {
                    Some(n) => n,
                    None => {
                        node_pos.push(p);
                        nodes.push(Vec::new());
                        nodes.len() - 1
                    }
                };
                nodes[n].push(RouteEnd { route: i, start });
                pair[k] = n;
            }
            ends.push(pair);
        }
        Self { routes, ends, nodes }
    }

    /// Route whose polyline passes closest to `p` (XY), with the point index.
    pub fn nearest(&self, p: Vec3) -> Option<(usize, usize, f32)> {
        let mut best: Option<(usize, usize, f32)> = None;
        for (ri, r) in self.routes.iter().enumerate() {
            let b = r.bbox;
            if p.x < b[0] - 30.0 || p.x > b[2] + 30.0 || p.y < b[1] - 30.0 || p.y > b[3] + 30.0 {
                continue;
            }
            for (pi, q) in r.points.iter().enumerate() {
                let d = Vec2::new(q.pos[0] - p.x, q.pos[1] - p.y).length();
                if best.is_none_or(|b| d < b.2) {
                    best = Some((ri, pi, d));
                }
            }
        }
        best
    }
}

/// Centre line of one race, resampled evenly.
#[derive(Clone, Debug)]
pub struct RaceLine {
    pub points: Vec<Vec3>,
    /// Distance from the start line at each point.
    pub dist: Vec<f32>,
    pub half_width: Vec<f32>,
    pub length: f32,
    pub circuit: bool,
}

impl RaceLine {
    /// The route data cuts corners and gores a little; with the game's walls in place a
    /// car following it exactly would hit them. Around each stretch that runs through a
    /// wall, picks for every point a sideways shift (up to 9 m, kept on the ground) so
    /// that no stretch crosses a wall, staying as close to the original as possible.
    /// Returns how many points moved.
    pub fn avoid_walls(&mut self, world: &crate::collision::CollisionWorld) -> usize {
        let n = self.points.len();
        if n < 3 {
            return 0;
        }
        let circuit = self.circuit;
        let segments = if circuit { n } else { n - 1 };
        let at = |i: isize| -> Option<usize> {
            if circuit {
                Some(i.rem_euclid(n as isize) as usize)
            } else {
                (0..n as isize).contains(&i).then_some(i as usize)
            }
        };
        let normals: Vec<Vec3> = (0..n as isize)
            .map(|i| {
                let a = self.points[at(i - 1).unwrap_or(i as usize)];
                let b = self.points[at(i + 1).unwrap_or(i as usize)];
                let d = (b - a).truncate().normalize_or_zero();
                Vec3::new(-d.y, d.x, 0.0)
            })
            .collect();
        let on_ground = |p: Vec3| world.ground(p + Vec3::Z * 2.0, 1.0, 5.0).map(|(z, _)| Vec3::new(p.x, p.y, z));
        let shifts: Vec<f32> = (-12..=12).map(|k| k as f32 * 0.75).collect();
        let mut pts = self.points.clone();
        let mut moved = std::collections::HashSet::new();
        let mut i = 0;
        while i < segments {
            let j = at(i as isize + 1).unwrap();
            if !world.wall_between(pts[i], pts[j]) {
                i += 1;
                continue;
            }
            // Widen the window until a clear path exists (both ends stay put).
            let mut done = false;
            for reach in [6isize, 12, 24, 40] {
                let (lo, hi) = (i as isize - reach, i as isize + 1 + reach);
                let idx: Vec<usize> = (lo..=hi).filter_map(at).collect();
                if idx.len() != (hi - lo + 1) as usize || (circuit && idx.len() >= n) {
                    continue;
                }
                let m = idx.len();
                // Candidate positions per point; the ends only keep their own.
                let cand: Vec<Vec<(f32, Vec3)>> = idx
                    .iter()
                    .enumerate()
                    .map(|(k, &p)| {
                        if k == 0 || k == m - 1 {
                            vec![(0.0, pts[p])]
                        } else {
                            shifts.iter().filter_map(|&s| Some((s, on_ground(pts[p] + normals[p] * s)?))).collect()
                        }
                    })
                    .collect();
                let mut cost: Vec<Vec<f32>> = cand.iter().map(|c| vec![f32::INFINITY; c.len()]).collect();
                let mut from: Vec<Vec<usize>> = cand.iter().map(|c| vec![0; c.len()]).collect();
                cost[0][0] = 0.0;
                for k in 1..m {
                    for (b, &(sb, pb)) in cand[k].iter().enumerate() {
                        for (a, &(sa, pa)) in cand[k - 1].iter().enumerate() {
                            let c = cost[k - 1][a];
                            if !c.is_finite() {
                                continue;
                            }
                            let total = c + sb.abs() + (sb - sa).abs() * 0.5;
                            if total < cost[k][b] && !world.wall_between(pa, pb) {
                                cost[k][b] = total;
                                from[k][b] = a;
                            }
                        }
                    }
                }
                if !cost[m - 1][0].is_finite() {
                    continue;
                }
                let mut b = 0;
                for k in (1..m).rev() {
                    let (s, p) = cand[k][b];
                    if s != 0.0 {
                        pts[idx[k]] = p;
                        moved.insert(idx[k]);
                    }
                    b = from[k][b];
                }
                done = true;
                break;
            }
            // Past the fixed window, or nothing clear within reach: move on.
            i += if done { 1 } else { 1 };
        }
        self.points = pts;
        moved.len()
    }

    /// Index of the point nearest to `p`, searching around `hint` first.
    pub fn locate(&self, p: Vec3, hint: Option<usize>) -> usize {
        let n = self.points.len();
        let d2 = |i: usize| self.points[i].truncate().distance_squared(p.truncate()) + (self.points[i].z - p.z).powi(2) * 4.0;
        if let Some(h) = hint {
            let mut best = (h % n, d2(h % n));
            for k in 1..60 {
                for i in [(h + k) % n, (h + n - k % n) % n] {
                    let d = d2(i);
                    if d < best.1 {
                        best = (i, d);
                    }
                }
            }
            if best.1 < 30.0 * 30.0 {
                return best.0;
            }
        }
        (0..n).min_by(|&a, &b| d2(a).total_cmp(&d2(b))).unwrap_or(0)
    }
}

/// The original's race modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Circuit,
    Sprint,
    Drag,
    Drift,
    StreetX,
    /// Underground Racing League: circuits on closed tracks.
    Url,
}

impl Mode {
    pub const ALL: [Mode; 6] = [Mode::Circuit, Mode::Sprint, Mode::Drag, Mode::Drift, Mode::StreetX, Mode::Url];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Circuit => "CIRCUIT",
            Mode::Sprint => "SPRINT",
            Mode::Drag => "DRAG",
            Mode::Drift => "DRIFT",
            Mode::StreetX => "STREET X",
            Mode::Url => "URL",
        }
    }

    /// Raced over laps of a closed loop (sprints and drags go from A to B).
    pub fn laps(self) -> bool {
        !matches!(self, Mode::Sprint | Mode::Drag)
    }
}

/// A race from the game's career list, in any region; its line is built once the
/// region is loaded (see [`RaceDef`]).
#[derive(Clone, Debug, PartialEq)]
pub struct RaceInfo {
    pub track: u16,
    pub name: String,
    pub mode: Mode,
    /// World region it is held in, e.g. "L4RA" (Bayview) or "L4RC" (drift tracks).
    pub region: String,
    pub reverse: bool,
    pub laps: u32,
    pub reward: u32,
    /// Lap (circuit) or race (sprint) length in metres, from the track table.
    pub length: u32,
    /// District icon index (see `TrackInfo::location`).
    pub location: u32,
}

/// Every race of the career list whose region `available` says is installed, one per
/// track and direction.
pub fn race_catalog(global_b: &[u8], available: impl Fn(&str) -> bool) -> Vec<RaceInfo> {
    let tracks = parse_track_infos(global_b).unwrap_or_default();
    let career = parse_career_races(global_b).unwrap_or_default();
    // Regular career entries (group 1) and URL ones (3, 4) first; tutorial and
    // special entries only fill gaps.
    let mut ordered: Vec<&CareerRace> = career.iter().collect();
    ordered.sort_by_key(|c| !matches!(c.group, 1 | 3 | 4));
    let mut out: Vec<RaceInfo> = Vec::new();
    for c in ordered {
        if out.iter().any(|r| r.track == c.track && r.reverse == c.reverse) {
            continue;
        }
        let Some(info) = tracks.iter().find(|t| t.number == c.track) else { continue };
        let region = info.region.to_ascii_uppercase();
        if !available(&region) {
            continue;
        }
        let mode = match c.race_type {
            // Bayview's drift entries are unfinished (the line has gaps of 200 m).
            RaceType::Drift if region == "L4RA" => continue,
            RaceType::Circuit if c.group >= 3 || region == "L4RG" => Mode::Url,
            RaceType::Circuit => Mode::Circuit,
            RaceType::Sprint => Mode::Sprint,
            RaceType::Drag => Mode::Drag,
            RaceType::Drift => Mode::Drift,
            RaceType::StreetX => Mode::StreetX,
            RaceType::Other(_) => continue,
        };
        // Some entries pack more than the lap count into the field.
        let laps = match c.laps & 0xFF {
            _ if !mode.laps() => 1,
            l @ 1..=6 => l,
            _ => 2,
        };
        out.push(RaceInfo {
            track: c.track,
            name: clean_name(&info.name),
            mode,
            region,
            reverse: c.reverse,
            laps,
            reward: c.reward,
            length: info.length,
            location: info.location,
        });
    }
    out.sort_by_key(|r| (r.mode as u8, r.track, r.reverse));
    out
}

/// True if the files of world region `r` are present.
pub fn region_installed(game: &GameDir, r: &str) -> bool {
    [format!("TRACKS/{r}.BUN"), format!("TRACKS/STREAM{r}.BUN"), format!("TRACKS/ROUTES{r}/TrackPosMarkersAll.bin")]
        .iter()
        .all(|p| game.resolve(p).is_some())
}

/// The races that can be started with the installed files: the region is there and the
/// race has a grid and a start line in its direction.
pub fn installed_races(game: &GameDir) -> Vec<RaceInfo> {
    let Ok(global_b) = game.read("GLOBAL/GLOBALB.BUN").or_else(|_| game.read_unpacked("GLOBAL/GlobalB.lzc")) else {
        return Vec::new();
    };
    let mut markers: HashMap<String, Vec<TrackMarker>> = HashMap::new();
    let mut races = race_catalog(&global_b, |r| region_installed(game, r));
    races.retain(|r| {
        let m = markers.entry(r.region.clone()).or_insert_with(|| {
            let path = format!("TRACKS/ROUTES{}/TrackPosMarkersAll.bin", r.region);
            game.read(&path).ok().and_then(|b| parse_markers(&b).ok()).unwrap_or_default()
        });
        let (grid, start) = if r.reverse {
            (MarkerKind::BackwardGrid, MarkerKind::BackwardStart)
        } else {
            (MarkerKind::ForwardGrid, MarkerKind::ForwardStart)
        };
        let has = |k: MarkerKind| m.iter().any(|x| x.track == r.track as u32 && x.kind == k);
        has(grid) && has(start)
    });
    races
}

/// A race that can be started in the loaded region.
#[derive(Clone, Debug)]
pub struct RaceDef {
    pub info: RaceInfo,
    /// Grid slots, front first.
    pub grid: Vec<Vec3>,
    pub start_line: Vec3,
    pub finish_line: Option<Vec3>,
    pub line: RaceLine,
}

pub struct RoadData {
    pub region: String,
    pub graph: RoadGraph,
    pub races: Vec<RaceDef>,
    /// Named places in free roam (shops, garages), by name hash.
    pub places: Vec<TrackMarker>,
}

fn clean_name(n: &str) -> String {
    // "(4001) CC Outer Loop #1" -> "CC OUTER LOOP"
    let n = n.split_once(')').map_or(n, |(_, r)| r).trim();
    let n = n.split('#').next().unwrap_or(n).trim();
    n.to_ascii_uppercase()
}

impl RoadData {
    pub fn load(game: &GameDir, region: &str) -> anyhow::Result<Self> {
        let dir = format!("TRACKS/ROUTES{region}");
        // The race-only regions have no streets for traffic.
        let net = game.read(&format!("{dir}/RoutesFreeRoam.bin")).ok().and_then(|b| parse_routes(&b).ok()).unwrap_or_default();
        let markers = parse_markers(&game.read(&format!("{dir}/TrackPosMarkersAll.bin"))?)?;
        let places = game
            .read(&format!("{dir}/TrackPosMarkersFreeRoam.bin"))
            .ok()
            .and_then(|b| parse_markers(&b).ok())
            .unwrap_or_default();
        let global_b = game.read("GLOBAL/GLOBALB.BUN").or_else(|_| game.read_unpacked("GLOBAL/GlobalB.lzc"))?;
        let mut races = Vec::new();
        for info in race_catalog(&global_b, |r| r.eq_ignore_ascii_case(region)) {
            let grid = if info.reverse { MarkerKind::BackwardGrid } else { MarkerKind::ForwardGrid };
            if !markers.iter().any(|m| m.track == info.track as u32 && m.kind == grid) {
                continue; // that direction is not raced
            }
            match build_race(game, &dir, &markers, &info) {
                Some(r) => races.push(r),
                None => log::warn!("race {} ({}): incomplete data", info.track, info.name),
            }
        }
        let graph = RoadGraph::build(net);
        log::info!("roads {region}: {} routes, {} junctions, {} races", graph.routes.len(), graph.nodes.len(), races.len());
        Ok(Self { region: region.to_string(), graph, races, places })
    }

    /// The loaded race for catalogue entry `info`.
    pub fn race(&self, info: &RaceInfo) -> Option<&RaceDef> {
        self.races.iter().find(|r| r.info.track == info.track && r.info.reverse == info.reverse)
    }
}

fn build_race(game: &GameDir, dir: &str, markers: &[TrackMarker], info: &RaceInfo) -> Option<RaceDef> {
    let (grid_kind, start_kind, finish_kind, suffix) = if info.reverse {
        (MarkerKind::BackwardGrid, MarkerKind::BackwardStart, MarkerKind::BackwardFinish, "B")
    } else {
        (MarkerKind::ForwardGrid, MarkerKind::ForwardStart, MarkerKind::ForwardFinish, "F")
    };
    let of = |k: MarkerKind| markers.iter().filter(move |m| m.track == info.track as u32 && m.kind == k).map(|m| Vec3::from(m.pos));
    let grid: Vec<Vec3> = of(grid_kind).collect();
    let start = of(start_kind).next()?;
    let finish = of(finish_kind).next();
    let circuit = info.mode.laps();
    let net = parse_routes(&game.read(&format!("{dir}/Routes{}{suffix}.bin", info.track)).ok()?).ok()?;
    let raw = race_line(&net, [start.x, start.y], if circuit { None } else { finish.map(|f| [f.x, f.y]) }, info.length as f32, 4.0);
    if raw.len() < 20 || grid.is_empty() {
        return None;
    }
    let points: Vec<Vec3> = raw.iter().map(|p| Vec3::from(p.pos)).collect();
    let dist: Vec<f32> = raw.iter().map(|p| p.distance).collect();
    let half_width: Vec<f32> = raw.iter().map(|p| p.left_width.min(p.right_width).clamp(3.0, 12.0)).collect();
    let length = if circuit { info.length as f32 } else { *dist.last()? };
    Some(RaceDef {
        info: info.clone(),
        grid,
        start_line: start,
        finish_line: finish,
        line: RaceLine { points, dist, half_width, length, circuit },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Option<GameDir> {
        let g = GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        g.resolve("TRACKS/ROUTESL4RC").is_some().then_some(g)
    }

    #[test]
    fn races_in_every_region() {
        let Some(g) = game() else { return };
        let all = installed_races(&g);
        for mode in Mode::ALL {
            let n = all.iter().filter(|r| r.mode == mode).count();
            assert!(n >= 3, "{mode:?}: {n} races");
        }
        for region in ["L4RA", "L4RB", "L4RC", "L4RD", "L4RF", "L4RG"] {
            let roads = RoadData::load(&g, region).unwrap();
            let listed = all.iter().filter(|r| r.region == region).count();
            assert_eq!(roads.races.len(), listed, "{region}: races built");
            for r in &roads.races {
                let line = &r.line;
                // The line starts near the start line and covers the race.
                assert!(line.points[0].truncate().distance(r.start_line.truncate()) < 30.0, "{} {}: start", region, r.info.name);
                assert!(line.length > 200.0, "{} {}: {} m", region, r.info.name, line.length);
            }
        }
    }
}
