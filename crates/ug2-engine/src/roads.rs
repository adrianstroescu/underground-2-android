//! Road data for gameplay: the free-roam road network as a graph traffic can drive,
//! race definitions (track table, career list, grids, start/finish lines) and the
//! centre line of every race in the loaded region.

use std::collections::HashMap;

use glam::{Vec2, Vec3};
use ug2_formats::fs::GameDir;
use ug2_formats::route::{
    parse_career_races, parse_markers, parse_routes, parse_track_infos, race_line, CareerRace, MarkerKind, RaceType,
    Route, RouteNetwork, TrackInfo, TrackMarker,
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

/// A race that can be started in the loaded region.
#[derive(Clone, Debug)]
pub struct RaceDef {
    pub track: u16,
    pub name: String,
    pub kind: RaceType,
    pub reverse: bool,
    pub laps: u32,
    pub reward: u32,
    /// Grid slots, front first.
    pub grid: Vec<Vec3>,
    pub start_line: Vec3,
    pub finish_line: Option<Vec3>,
    pub line: RaceLine,
}

pub struct RoadData {
    pub graph: RoadGraph,
    pub tracks: Vec<TrackInfo>,
    pub career: Vec<CareerRace>,
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
        let net = parse_routes(&game.read(&format!("{dir}/RoutesFreeRoam.bin"))?)?;
        let markers = parse_markers(&game.read(&format!("{dir}/TrackPosMarkersAll.bin"))?)?;
        let places = game
            .read(&format!("{dir}/TrackPosMarkersFreeRoam.bin"))
            .ok()
            .and_then(|b| parse_markers(&b).ok())
            .unwrap_or_default();
        let global_b = game.read("GLOBAL/GLOBALB.BUN").or_else(|_| game.read_unpacked("GLOBAL/GlobalB.lzc"))?;
        let tracks = parse_track_infos(&global_b)?;
        let career = parse_career_races(&global_b).unwrap_or_default();

        // One race per (track, direction) used by circuits and sprints in this region.
        let mut seen = HashMap::new();
        let mut races = Vec::new();
        for c in &career {
            if !matches!(c.race_type, RaceType::Circuit | RaceType::Sprint) || seen.contains_key(&(c.track, c.reverse)) {
                continue;
            }
            let Some(info) = tracks.iter().find(|t| t.number == c.track && t.region.eq_ignore_ascii_case(region)) else {
                continue;
            };
            seen.insert((c.track, c.reverse), ());
            match build_race(game, &dir, &markers, info, c) {
                Some(r) => races.push(r),
                None => log::warn!("race {} ({}): incomplete data", c.track, info.name),
            }
        }
        races.sort_by_key(|r| (r.kind != RaceType::Circuit, r.track, r.reverse));
        let graph = RoadGraph::build(net);
        log::info!("roads: {} routes, {} junctions, {} races", graph.routes.len(), graph.nodes.len(), races.len());
        Ok(Self { graph, tracks, career, races, places })
    }
}

fn build_race(game: &GameDir, dir: &str, markers: &[TrackMarker], info: &TrackInfo, c: &CareerRace) -> Option<RaceDef> {
    let (grid_kind, start_kind, finish_kind, suffix) = if c.reverse {
        (MarkerKind::BackwardGrid, MarkerKind::BackwardStart, MarkerKind::BackwardFinish, "B")
    } else {
        (MarkerKind::ForwardGrid, MarkerKind::ForwardStart, MarkerKind::ForwardFinish, "F")
    };
    let of = |k: MarkerKind| markers.iter().filter(move |m| m.track == c.track as u32 && m.kind == k).map(|m| Vec3::from(m.pos));
    let grid: Vec<Vec3> = of(grid_kind).collect();
    let start = of(start_kind).next()?;
    let finish = of(finish_kind).next();
    let circuit = c.race_type == RaceType::Circuit;
    let net = parse_routes(&game.read(&format!("{dir}/Routes{}{suffix}.bin", c.track)).ok()?).ok()?;
    let raw = race_line(&net, [start.x, start.y], if circuit { None } else { finish.map(|f| [f.x, f.y]) }, info.length as f32, 4.0);
    if raw.len() < 20 || grid.is_empty() {
        return None;
    }
    let points: Vec<Vec3> = raw.iter().map(|p| Vec3::from(p.pos)).collect();
    let dist: Vec<f32> = raw.iter().map(|p| p.distance).collect();
    let half_width: Vec<f32> = raw.iter().map(|p| p.left_width.min(p.right_width).clamp(3.0, 12.0)).collect();
    let length = if circuit { info.length as f32 } else { *dist.last()? };
    Some(RaceDef {
        track: c.track,
        name: clean_name(&info.name),
        kind: c.race_type,
        reverse: c.reverse,
        laps: if circuit { c.laps.clamp(1, 5) } else { 1 },
        reward: c.reward,
        grid,
        start_line: start,
        finish_line: finish,
        line: RaceLine { points, dist, half_width, length, circuit },
    })
}
