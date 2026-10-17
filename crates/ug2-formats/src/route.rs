//! Road network, race layouts and race list.
//!
//! * `TRACKS/ROUTES<region>/Routes*.bin`: road routes (chunk 0x34121) and the links
//!   between them (0x34122). `RoutesFreeRoam.bin` is the whole city; `Routes<track>F.bin`
//!   / `...B.bin` hold the routes of one race (forward / backward), where every point
//!   also carries its distance along the race.
//! * `TRACKS/ROUTES<region>/TrackPosMarkersAll.bin` (0x34146): start grids, start and
//!   finish lines per track; `TrackPosMarkersFreeRoam.bin`: shops and garages.
//! * `GLOBAL/GLOBALB.BUN`: track table (0x34201) and career races (0x34A11 inside the
//!   first 0x80034A10).
//!
//! Coordinates are world space, Z up, like the streamed track geometry.

use crate::bytes::{cstr_at, f32_at, i16_at, slice, u16_at, u32_at, u8_at};
use crate::chunk::ChunkIter;
use crate::{bail, Result};

const ROUTES: u32 = 0x0003_4121;
const ROUTE_LINKS: u32 = 0x0003_4122;
const POS_MARKERS: u32 = 0x0003_4146;
const TRACK_INFOS: u32 = 0x0003_4201;
const GAMEPLAY: u32 = 0x8003_4A10;
const CAREER_RACES: u32 = 0x0003_4A11;

const ROUTE_PREFIX_LEN: usize = 0x30;
const ROUTE_HEADER_LEN: usize = 0x5C;
const POINT_LEN: usize = 56;
const LINK_LEN: usize = 32;
const MARKER_LEN: usize = 48;
const TRACK_INFO_LEN: usize = 0x128;
const CAREER_RACE_LEN: usize = 136;

#[derive(Clone, Copy, Debug)]
pub struct RoutePoint {
    pub pos: [f32; 3],
    /// Distance along the race in race route files; 0 in the free-roam network.
    pub race_distance: f32,
    /// Drivable corridor either side of `pos`, relative to the route direction.
    pub left_width: f32,
    pub right_width: f32,
    /// Unit direction to the next point (XY).
    pub dir: [f32; 2],
    /// Distance to the next point.
    pub segment_length: f32,
    /// Raw words +12 (route id in bits 0..10, flags above) and +14 (flags).
    pub flags: [u16; 2],
}

#[derive(Clone, Debug)]
pub struct Route {
    pub id: u16,
    /// Editor group, e.g. "TrackRoutesA32": the tens digit is the map area (1..4).
    pub group: String,
    pub flags: u32,
    pub length: f32,
    /// Race distance at the first / last point (race files only).
    pub race_start: f32,
    pub race_end: f32,
    /// min x, min y, max x, max y.
    pub bbox: [f32; 4],
    pub points: Vec<RoutePoint>,
}

impl Route {
    /// Short connector through a junction (turning lane between two streets).
    pub fn is_connector(&self) -> bool {
        self.flags & 0x400 != 0
    }
}

/// Point `point_a` of route `route_a` joins point `point_b` of route `route_b`
/// (junctions, merges, crossings; points are usually within 2 m of each other).
#[derive(Clone, Copy, Debug)]
pub struct RouteLink {
    pub route_a: u16,
    pub point_a: u16,
    pub route_b: u16,
    pub point_b: u16,
    /// 0x2222, 0x4444, 0x7272 or 0x7474; meaning not decoded.
    pub kind: u16,
    pub flags: u16,
}

#[derive(Clone, Debug, Default)]
pub struct RouteNetwork {
    pub routes: Vec<Route>,
    pub links: Vec<RouteLink>,
}

impl RouteNetwork {
    pub fn route(&self, id: u16) -> Option<&Route> {
        self.routes.iter().find(|r| r.id == id)
    }
}

/// Parses `Routes*.bin`.
pub fn parse_routes(buf: &[u8]) -> Result<RouteNetwork> {
    let mut net = RouteNetwork::default();
    let Some(chunk) = ChunkIter::new(buf).find(|c| c.id == ROUTES) else { bail!("no route chunk") };
    let b = chunk.data;
    let mut off = 0;
    while off + ROUTE_PREFIX_LEN + ROUTE_HEADER_LEN <= b.len() {
        let id = u16_at(b, off + 0x0A)?;
        let group = cstr_at(b, off + 0x10, 32)?;
        let h = off + ROUTE_PREFIX_LEN;
        let count = u16_at(b, h + 4)? as usize;
        let mut route = Route {
            id,
            group,
            flags: u32_at(b, h + 0x40)?,
            race_start: f32_at(b, h + 0x08)?,
            race_end: f32_at(b, h + 0x0C)?,
            length: f32_at(b, h + 0x10)?,
            bbox: [f32_at(b, h + 0x30)?, f32_at(b, h + 0x34)?, f32_at(b, h + 0x38)?, f32_at(b, h + 0x3C)?],
            points: Vec::with_capacity(count),
        };
        let first = h + ROUTE_HEADER_LEN;
        for i in 0..count {
            let p = slice(b, first + i * POINT_LEN, POINT_LEN)?;
            route.points.push(RoutePoint {
                pos: [f32_at(p, 0)?, f32_at(p, 4)?, i16_at(p, 18)? as f32 / 8.0],
                race_distance: f32_at(p, 8)?,
                left_width: u16_at(p, 20)? as f32 / 256.0,
                right_width: u16_at(p, 22)? as f32 / 256.0,
                dir: [i16_at(p, 24)? as f32 / 32767.0, i16_at(p, 26)? as f32 / 32767.0],
                segment_length: u16_at(p, 16)? as f32 / 256.0,
                flags: [u16_at(p, 12)?, u16_at(p, 14)?],
            });
        }
        net.routes.push(route);
        off = first + count * POINT_LEN;
    }
    if let Some(links) = ChunkIter::new(buf).find(|c| c.id == ROUTE_LINKS) {
        // 8-byte header, then fixed records.
        for rec in links.data.get(8..).unwrap_or(&[]).chunks_exact(LINK_LEN) {
            net.links.push(RouteLink {
                point_a: u16_at(rec, 0)?,
                point_b: u16_at(rec, 2)?,
                route_a: u16_at(rec, 4)?,
                route_b: u16_at(rec, 6)?,
                kind: u16_at(rec, 8)?,
                flags: u16_at(rec, 10)?,
            });
        }
    }
    Ok(net)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    ForwardGrid,
    BackwardGrid,
    ForwardStart,
    BackwardStart,
    ForwardFinish,
    BackwardFinish,
    /// Shops, garages and other named spots: `bin_hash` of the name, e.g.
    /// "CC_PERF_SHOP", "UC_CAR_LOT", "CRIB_1", "SH_BODYSHOP".
    Other(u32),
}

impl MarkerKind {
    fn from_hash(h: u32) -> Self {
        match h {
            0x8E55_C895 => Self::ForwardGrid,     // ForwardStartingPosition
            0x8F90_F7BF => Self::BackwardGrid,    // BackwardStartingPosition
            0x6A56_B90A => Self::ForwardStart,    // ForwardStartLine
            0xEEC7_0374 => Self::BackwardStart,   // BackwardStartLine
            0xC96C_D7FD => Self::ForwardFinish,   // ForwardFinishLine
            0xDBE6_6FA7 => Self::BackwardFinish,  // BackwardFinishLine
            h => Self::Other(h),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TrackMarker {
    /// Track number (4000 = free roam, 4001.. = races).
    pub track: u32,
    pub kind: MarkerKind,
    pub pos: [f32; 3],
}

/// Parses `TrackPosMarkers*.bin`. Grid slots come in file order (8 per direction); they
/// carry no heading: cars face from the grid towards the start line.
pub fn parse_markers(buf: &[u8]) -> Result<Vec<TrackMarker>> {
    let Some(chunk) = ChunkIter::new(buf).find(|c| c.id == POS_MARKERS) else { bail!("no marker chunk") };
    let mut out = Vec::new();
    for rec in chunk.payload().chunks_exact(MARKER_LEN) {
        out.push(TrackMarker {
            track: u32_at(rec, 0x24)?,
            kind: MarkerKind::from_hash(u32_at(rec, 0x08)?),
            pos: [f32_at(rec, 0x10)?, f32_at(rec, 0x14)?, f32_at(rec, 0x18)?],
        });
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct TrackInfo {
    pub number: u16,
    /// Developer name, e.g. "(4001) CC Outer Loop #1".
    pub name: String,
    /// World region, e.g. "L4RA" (TRACKS/L4RA.BUN, TRACKS/ROUTESL4RA).
    pub region: String,
    /// Lap (circuit) or race (sprint) length in metres.
    pub length: u32,
    /// Bayview district shown as the track's icon (speed2.exe 0x4984b0): 0 Upper Class,
    /// 1 City Core, 2 Suburban Hills, 3 Coal Harbor, 4 Airport, 5 by race type.
    pub location: u32,
    /// +0x80: for drift tracks, 0 = lap drift (laps and opponents can be set), 1 = the
    /// downhill kind (speed2.exe 0x4cd7a0).
    pub drift_kind: u8,
    /// +0x86: the track can be raced in reverse (Track Direction option).
    pub reversible: bool,
    /// Mini-map (TRACKS/TRACKMAP<n>.BIN) calibration: world position of the image's
    /// upper-left corner and the width it covers.
    pub map_upper_left: [f32; 2],
    pub map_width: f32,
    /// HUD minimap zoom: the map shows 0.3 / zoom of the image (speed2.exe 0x4ca840).
    pub map_zoom: f32,
}

/// Parses the track table from GLOBALB.
pub fn parse_track_infos(global_b: &[u8]) -> Result<Vec<TrackInfo>> {
    let Some(chunk) = ChunkIter::new(global_b).find(|c| c.id == TRACK_INFOS) else { bail!("no track table") };
    let mut out = Vec::new();
    for rec in chunk.data.chunks_exact(TRACK_INFO_LEN) {
        out.push(TrackInfo {
            number: u16_at(rec, 0x8A)?,
            name: cstr_at(rec, 0, 32)?,
            region: cstr_at(rec, 0x40, 8)?,
            length: u32_at(rec, 0x98)?,
            location: u32_at(rec, 0x7C)?,
            drift_kind: rec[0x80],
            reversible: rec[0x86] == 1,
            map_upper_left: [f32_at(rec, 0xAC)?, f32_at(rec, 0xB0)?],
            map_width: f32_at(rec, 0xB4)?,
            map_zoom: f32_at(rec, 0xC0)?,
        });
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaceType {
    Circuit,
    Sprint,
    StreetX,
    Drag,
    Drift,
    Other(u8),
}

#[derive(Clone, Debug)]
pub struct CareerRace {
    pub hash: u32,
    pub race_type: RaceType,
    pub track: u16,
    pub reverse: bool,
    /// Laps for circuits; unclear for other types.
    pub laps: u32,
    /// Cash prize.
    pub reward: u32,
    /// 1 for regular career races; URL tournament races use 3 and 4.
    pub group: u8,
    /// Name, e.g. "STAGE_1_DRAG_1" (string pool 0x34A1D).
    pub name: String,
    /// Story movies played before the race and after winning it (MOVIES/<name>.vp6).
    pub movie_before: Option<String>,
    pub movie_after: Option<String>,
    /// Career stage 0..5 (+0x37).
    pub stage: u8,
}

/// Parses the career race list (first gameplay container in GLOBALB).
pub fn parse_career_races(global_b: &[u8]) -> Result<Vec<CareerRace>> {
    let Some(gameplay) = ChunkIter::new(global_b).find(|c| c.id == GAMEPLAY) else { bail!("no gameplay data") };
    let Some(chunk) = gameplay.child(CAREER_RACES) else { bail!("no career races") };
    // Names are u16 offsets into the string pool (0x34A1D).
    let pool = gameplay.child(0x0003_4A1D).map(|c| c.data).unwrap_or(&[]);
    let text = |off: u16| -> Option<String> {
        let rest = pool.get(off as usize..)?;
        let end = rest.iter().position(|&b| b == 0)?;
        (end > 0).then(|| String::from_utf8_lossy(&rest[..end]).into_owned())
    };
    let mut out = Vec::new();
    for rec in chunk.data.chunks_exact(CAREER_RACE_LEN) {
        let off = |o: usize| u16::from_le_bytes([rec[o], rec[o + 1]]);
        let name = text(off(0)).unwrap_or_default();
        let movie_before = if off(2) != 0 { text(off(2)) } else { None };
        let movie_after = if off(4) != 0 { text(off(4)) } else { None };
        out.push(CareerRace {
            name,
            movie_before,
            movie_after,
            stage: u8_at(rec, 0x37)?,
            hash: u32_at(rec, 8)?,
            group: u8_at(rec, 12)?,
            reverse: u8_at(rec, 13)? != 0,
            race_type: match u8_at(rec, 15)? {
                0 => RaceType::Circuit,
                1 => RaceType::Sprint,
                2 => RaceType::StreetX,
                4 => RaceType::Drag,
                5 => RaceType::Drift,
                t => RaceType::Other(t),
            },
            laps: u32_at(rec, 16)?,
            track: u16_at(rec, 24)?,
            reward: u32_at(rec, 48)?,
        });
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug)]
pub struct RaceLinePoint {
    /// Metres from the start line.
    pub distance: f32,
    pub pos: [f32; 3],
    pub left_width: f32,
    pub right_width: f32,
}

#[derive(Clone, Copy)]
struct Sample {
    d: f32,
    pos: [f32; 3],
    lw: f32,
    rw: f32,
}

/// Centre line of a race from its route file (`Routes<track>F.bin` or `B`).
///
/// Every route segment whose race distance changes by about its own length lies on the
/// race (routes run either way). They are sampled every `step` metres, binned by
/// distance from the start line, and one sample per bin is chosen by dynamic
/// programming so that consecutive picks are as far apart as the race distance says;
/// side roads that merely share race distance would need a lateral jump and lose.
/// `finish` is `None` for circuits (`length` = lap length).
pub fn race_line(net: &RouteNetwork, start: [f32; 2], finish: Option<[f32; 2]>, length: f32, step: f32) -> Vec<RaceLinePoint> {
    let circuit = finish.is_none();
    let mut samples = Vec::new();
    for r in &net.routes {
        for w in r.points.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let mut delta = b.race_distance - a.race_distance;
            if circuit {
                if delta < -length / 2.0 {
                    delta += length;
                } else if delta > length / 2.0 {
                    delta -= length;
                }
            }
            let seg = dist2(a.pos, b.pos);
            if seg < 0.5 || (delta.abs() - seg).abs() > 0.3 * seg + 3.0 {
                continue;
            }
            let n = ((seg / step) as usize).max(1);
            for k in 0..n {
                let t = k as f32 / n as f32;
                let lerp = |i: usize| a.pos[i] + (b.pos[i] - a.pos[i]) * t;
                samples.push(Sample {
                    d: a.race_distance + delta * t,
                    pos: [lerp(0), lerp(1), lerp(2)],
                    lw: a.left_width,
                    rw: a.right_width,
                });
            }
        }
    }
    let nearest = |p: [f32; 2]| {
        samples.iter().min_by(|x, y| dist2(x.pos, [p[0], p[1], 0.0]).total_cmp(&dist2(y.pos, [p[0], p[1], 0.0]))).map(|s| s.d)
    };
    let Some(d0) = nearest(start) else { return Vec::new() };
    let total = match finish {
        Some(f) => nearest(f).map_or(length, |d| d - d0),
        None => length,
    };
    let nb = (total / step).max(1.0) as usize;
    let rel = |s: &Sample| if circuit { (s.d - d0).rem_euclid(length) } else { s.d - d0 };
    let mut bins: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for (i, s) in samples.iter().enumerate() {
        let r = rel(s);
        if r >= 0.0 && ((r / step) as usize) < nb {
            bins[(r / step) as usize].push(i);
        }
    }
    let start3 = [start[0], start[1], 0.0];
    let mut reach = 12;
    let picks = loop {
        // cost[i] / back[i] per sample index.
        let mut cost = vec![f32::INFINITY; samples.len()];
        let mut back = vec![usize::MAX; samples.len()];
        for bin in 0..nb {
            for &k in &bins[bin] {
                let s = &samples[k];
                if bin < reach {
                    cost[k] = dist2(s.pos, start3) + 4.0 * bin as f32;
                }
                for prev_bin in bin.saturating_sub(reach)..bin {
                    for &k2 in &bins[prev_bin] {
                        if !cost[k2].is_finite() {
                            continue;
                        }
                        let s2 = &samples[k2];
                        let c = cost[k2]
                            + (dist2(s.pos, s2.pos) - (rel(s) - rel(s2))).abs()
                            + 2.0 * (bin - prev_bin - 1) as f32;
                        if c < cost[k] {
                            cost[k] = c;
                            back[k] = k2;
                        }
                    }
                }
            }
        }
        let end = (nb.saturating_sub(reach)..nb)
            .flat_map(|b| bins[b].iter().map(move |&k| (k, b)))
            .filter(|&(k, _)| cost[k].is_finite())
            .min_by(|&(a, ba), &(b, bb)| {
                (cost[a] + 4.0 * (nb - 1 - ba) as f32).total_cmp(&(cost[b] + 4.0 * (nb - 1 - bb) as f32))
            });
        match end {
            Some((mut k, _)) => {
                let mut out = Vec::new();
                while k != usize::MAX {
                    out.push(k);
                    k = back[k];
                }
                out.reverse();
                break out;
            }
            None if reach < 96 => reach *= 2, // a long hole in the data: allow bigger skips
            None => return Vec::new(),
        }
    };
    let raw: Vec<RaceLinePoint> = picks
        .iter()
        .map(|&k| {
            let s = &samples[k];
            RaceLinePoint { distance: (rel(s) / step).floor() * step, pos: s.pos, left_width: s.lw, right_width: s.rw }
        })
        .collect();
    // Light smoothing of the position.
    (0..raw.len())
        .map(|i| {
            let w = &raw[i.saturating_sub(3)..(i + 4).min(raw.len())];
            let n = w.len() as f32;
            let mut p = raw[i];
            p.pos[0] = w.iter().map(|q| q.pos[0]).sum::<f32>() / n;
            p.pos[1] = w.iter().map(|q| q.pos[1]).sum::<f32>() / n;
            p
        })
        .collect()
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::GameDir;

    const GAME: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../..");

    fn game() -> Option<GameDir> {
        std::path::Path::new(GAME).join("TRACKS").exists().then(|| GameDir::new(GAME))
    }

    #[test]
    fn marker_hashes() {
        for (name, kind) in [
            ("ForwardStartingPosition", MarkerKind::ForwardGrid),
            ("BackwardStartingPosition", MarkerKind::BackwardGrid),
            ("ForwardStartLine", MarkerKind::ForwardStart),
            ("BackwardStartLine", MarkerKind::BackwardStart),
            ("ForwardFinishLine", MarkerKind::ForwardFinish),
            ("BackwardFinishLine", MarkerKind::BackwardFinish),
        ] {
            assert_eq!(MarkerKind::from_hash(crate::bin_hash(name)), kind, "{name}");
        }
    }

    #[test]
    fn free_roam_network() {
        let Some(g) = game() else { return };
        let net = parse_routes(&g.read("TRACKS/ROUTESL4RA/RoutesFreeRoam.bin").unwrap()).unwrap();
        assert_eq!(net.routes.len(), 855);
        assert_eq!(net.links.len(), 3288);
        for r in &net.routes {
            for (i, p) in r.points.iter().enumerate() {
                assert_eq!(p.flags[0] & 0x7FF, r.id);
                if let Some(q) = r.points.get(i + 1) {
                    assert!((dist2(p.pos, q.pos) - p.segment_length).abs() < 0.01);
                }
            }
        }
        for l in &net.links {
            let a = net.route(l.route_a).unwrap();
            let b = net.route(l.route_b).unwrap();
            assert!((l.point_a as usize) < a.points.len() && (l.point_b as usize) < b.points.len());
        }
        // The street the player spawns on runs past (-344, 90) at road height ~8.7.
        let p = net.route(655).unwrap().points[0];
        assert!((p.pos[0] + 344.15).abs() < 0.1 && (p.pos[2] - 8.75).abs() < 0.2);
    }

    #[test]
    fn tables() {
        let Some(g) = game() else { return };
        let gb = g.read("GLOBAL/GLOBALB.BUN").unwrap();
        let tracks = parse_track_infos(&gb).unwrap();
        assert_eq!(tracks.len(), 131);
        let t = tracks.iter().find(|t| t.number == 4001).unwrap();
        assert_eq!((t.length, t.region.as_str()), (6274, "L4RA"));
        let races = parse_career_races(&gb).unwrap();
        assert_eq!(races.len(), 224);
        assert!(races.iter().any(|r| r.track == 4001 && r.race_type == RaceType::Circuit && r.laps == 3));
        let markers = parse_markers(&g.read("TRACKS/ROUTESL4RA/TrackPosMarkersAll.bin").unwrap()).unwrap();
        assert_eq!(markers.len(), 1290);
        assert_eq!(markers.iter().filter(|m| m.track == 4001 && m.kind == MarkerKind::ForwardGrid).count(), 8);
    }

    fn check_race(track: u16, file: &str, length: f32) {
        let Some(g) = game() else { return };
        let net = parse_routes(&g.read(&format!("TRACKS/ROUTESL4RA/{file}")).unwrap()).unwrap();
        let markers = parse_markers(&g.read("TRACKS/ROUTESL4RA/TrackPosMarkersAll.bin").unwrap()).unwrap();
        let find = |k| markers.iter().find(|m| m.track == track as u32 && m.kind == k).map(|m| [m.pos[0], m.pos[1]]);
        let start = find(MarkerKind::ForwardStart).unwrap();
        let finish = find(MarkerKind::ForwardFinish);
        let line = race_line(&net, start, finish, length, 4.0);
        let coverage = line.len() as f32 * 4.0 / length;
        let max_gap = line.windows(2).map(|w| dist2(w[0].pos, w[1].pos)).fold(0.0, f32::max);
        let end = finish.unwrap_or(start);
        let last = line.last().unwrap().pos;
        let miss = dist2(last, [end[0], end[1], 0.0]);
        assert!(coverage > 0.85 && max_gap < 15.0 && miss < 30.0, "{file}: coverage {coverage} gap {max_gap} end {miss}");
    }

    #[test]
    fn circuit_line() {
        check_race(4001, "Routes4001F.bin", 6274.0);
    }

    #[test]
    fn sprint_line() {
        check_race(4101, "Routes4101F.bin", 4419.0);
    }
}

#[cfg(test)]
mod career_tests {
    #[test]
    fn career_movies() {
        let game = crate::fs::GameDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let Ok(gb) = game.read("GLOBAL/GLOBALB.BUN") else { return };
        let races = super::parse_career_races(&gb).unwrap();
        let drag = races.iter().find(|r| r.name == "STAGE_1_DRAG_1").expect("STAGE_1_DRAG_1");
        assert_eq!(drag.movie_before.as_deref().map(str::to_ascii_uppercase).as_deref(), Some("DRAGTUT"));
        for r in races.iter().filter(|r| r.movie_before.is_some() || r.movie_after.is_some()) {
            println!("{} stage {} before {:?} after {:?}", r.name, r.stage, r.movie_before, r.movie_after);
        }
    }
}
