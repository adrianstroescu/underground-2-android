# Progress and next steps

Where the port stands, so work can pick up from here. Last updated 2026-10-16
(evening). What we know about the original game's data and shaders is in
`../CLAUDE.md`.

## Working (tested on the S23 Ultra, 120 Hz)

- Bayview free roam with traffic, world lights, road reflections and bloom (about 120 FPS).
- Touch HUD after `docs/hud-target.png`: pedals, NOS, E-BRAKE, analog gauge, minimap and standings.
- Garage:
  - body kit, widebody, hood, lights, rims, spoiler, neon, window tint and paint;
  - two-column menu, with the car shown on the right.
- Career: race ladder, prizes, car shop and upgrades (`career.cfg`).
- Engine, effects and music audio.
- **All race modes**, chosen in QUICK RACE > MODE or in the career ladder:
  - Circuit and Sprint (L4RA);
  - Drag (L4RB, L4RD);
  - Drift (L4RC);
  - Street X (L4RF);
  - URL (L4RG).
- **Visible sections as in the original** (`0x80034150`): only the sections listed for
  the camera's drivable section are drawn, which doubled the FPS on the drag map.
  Lights follow them, and collision keeps every section seen from any car.
- **Original road shader**: streets are opaque, and the texture alpha sets how much
  of the mirrored city they show.
- **Original texture blend types**:
  - type 0 is opaque (asphalt, concrete, bridge decks were drawn see-through before);
  - type 1 is alpha blended (glass, fences, shadows);
  - type 2 is additive (neon, lights, car light lenses).
- **Original track collision** (`0x80034130`): road, kerbs, barriers and map-edge
  walls. Each race's barriers (`BARRIERS_<track>`) switch on only during that race.
  Render meshes are no longer used for collision.
- Race lines are moved off walls when a race starts (`RaceLine::avoid_walls`), so the
  AI and autopilot don't hit gore walls.
- Drag race tested end to end: CITY DRAG, 1st in 0:38.
- Cars stop past the finish line instead of reversing.
- Drift race on L4RC: the lap completes and scores, and the lot no longer shows white
  patches.

- **Original post-processing** (2026-10-17), re-implemented from speed2.exe and an
  apitrace frame (see `CLAUDE.md`):
  - speed blur;
  - gainmap, tint and rampmap colour treatment, with the tint taken per city zone from the
    Sunrise Screen Tint map;
  - overbright bloom;
  - light trails: glows go to their own target, which decays per channel.
- **Movies**: the VP6 FMVs with sound. The EA logo, the THX logo and the opening play at
  start; a tap skips. Story scenes are not hooked to the career yet.
- Android 10 phones start (Galaxy S9+ tested); 120 Hz is requested only where the API exists.

## Not tested on the phone yet

- Street X and URL races, and drag on L4RD.
- Hand-driven drift tuning: the autopilot scores only ~10 points per lap against
  rivals at 5–9k, so check `rival_scores` against real driving.
- Career moving between regions.

## Known issues and to-do

0. Next big items, in order:
   - **Original frontend (FEng)**: the 137 `.fng` screens are in
     `FRONTEND/FrontB.lzc(.bak)`: 0x30210 = u32 hash + JDLZ package. Format notes come
     from NFSTools/FEngLib, which has no license, so it is read as documentation only.
     Strings are in `LANGUAGES/English.bin(.bak)` (chunk 0x39000: count, table offset,
     string offset; (label hash, offset) pairs).
   - The owner's PC install has mod files (Dec 2024: FrontB.lzc, GlobalB.lzc,
     English.bin, *MemoryFile.bin). The originals are the `.bak` files; prefer them.
   - Story scenes and tutorial / region movies at the right career moments. The FEng
     screens IG_PlayMovie and UI_Trailers drive them.
   - Fog per zone: the Normal Fog map is decoded (start, max and colour fields), but
     the fog end and the ×0.745 colour factor are not found yet.
   - Boot order of the movies to confirm on the PC (EA logo, THX, PSA?, opening).

1. About 14 places in Bayview where a race's route data jumps between two roads
   through the air (e.g. the V junction in a tunnel at (362, 268)). The AI can get
   stuck there.
   - The autopilot resets after 3 s.
   - Fix: build race lines with collision-aware route chaining, so a jump through a
     wall is not allowed (`route.rs::race_line`).
2. Car shading is still a guess. The original CAR effect uses:
   - spherical-harmonics lighting;
   - per-material DiffuseMin/Range, Specular and Envmap values from GLOBALB `0x135200`
     (layout not decoded yet);
   - a reflection envmap.
   See `CLAUDE.md`.
3. Other original effects not re-implemented yet:
   - SKYMORPH;
   - OVERBRIGHT bloom parameters;
   - SCREENFILTER;
   - WORLDHEADLIGHT (headlight projection; ours is a glow pool that looks too bright);
   - fog values;
   - HORIZON_FOG;
   - DEPTH_OF_FIELD.
4. Start/finish graphics (instance flags `0x4`/`0x8`) are always hidden. They could be
   shown for their own race.
5. `add_car_model` never frees older models.
6. The surface types in the collision (byte 2) are not used yet: grip, sound and
   sparks per surface.
7. Remaining roadmap:
   - vinyls and decals;
   - car cube-map reflections;
   - `PAN_*` panoramas;
   - rear-view mirror;
   - AI engine sounds;
   - TRACKMAP minimap textures;
   - gamepad menus.

## Testing aids

- `autopilot = 1` in `ug2.cfg` lets the computer drive the player's car in races. Remove
  it to play normally. See the `phone-test` skill.
- Every second, `car pos ... | wall <object>` in logcat names what the car touches.
  For track collision the name is `TRACK_<section>_SURFACE_<xx>` or
  `TRACK_<section>_<barrier set>`.
- Desktop and `ug2-engine` tests crash on this PC before `main` (MinGW + wgpu). Use a
  scratch crate (see the `ug2-reverse` skill) or the phone.
