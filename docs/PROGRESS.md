# Progress and next steps

Where the port stands, so work can pick up from here. Last updated 2026-10-17
(paused at the owner's request). What we know about the original game's data and shaders is in
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
- **Original frontend screens** (FEng packages driven like speed2.exe does, `fe.rs`),
  over the Showroom turntable:
  - Main menu (Career, Quick Race, Customize, Options; the last three still open the
    port's own screens);
  - Quick Race: Mode Select → Track Select → Game Options → Car Select → race. The
    options rows per mode, defaults and limits are the game's. Car Select has the
    manufacturer logo, name badge, filters, race mode and the stat bars (game formulas),
    then the Transmission dialog;
  - Options (Audio, Gameplay, Player) with the game's rows and defaults, saved in
    ug2.cfg as `opt.*`; volumes, speedometer units, camera and transmission act;
  - Career menu (Resume / New / Load), the pause menu and pause options, the PC help
    bar and generic dialogs;
  - post race results table and menu (quick races);
  - the race HUDs (SingleRace, Short_Track, Drag, Drift, and Bayview free roam) at
    the PC positions; side groups follow the screen edges. The touch layout of
    docs/hud-target.png comes at the end (owner's call);
  - quick race AI driver names and car pools; Lap KO races.

## Not tested on the phone yet

- Street X and URL races, and drag on L4RD.
- Hand-driven drift tuning: the autopilot scores only ~10 points per lap against
  rivals at 5–9k, so check `rival_scores` against real driving.
- Career moving between regions.

## Session 2026-10-17 (paused here)

Done and pushed:
- EA Trax drives the music (menu vs racing sets, sequential / random, no repeats);
  tested on the S23.
- `ug2-formats/carphysics.rs` and `ug2-engine/powertrain.rs`: the original engine,
  clutch state machine, automatic gearbox, drive split and drag shift quality from
  speed2.exe. Checked against `docs/re/physics1_ref.py`; NOT wired into the car yet
  (vehicle.rs is still the invented arcade model).
- Reverse-engineering notes, all in `docs/re/`: physics part 1 (`physics1_re.md`) and
  part 2 (`physics2_re.md`: rigid body, suspension, tyres, 5-pass solve, steering,
  aero, drift / drag modes, MellowMover), AI racers and traffic (`ai_re*.md`), career
  start and car lot (`career_start_re.md`). Reference sims `physics*_ref.py`,
  `ai_ref*.py` (not validated against the game).

Resume with, in order:
1. Port physics part 2 into a new `physics.rs` mover (rigid body + suspension + tyres +
   powertrain), replacing vehicle.rs for the player; then compare against the PC game.
   Open: steering controller blend 0x405dc0 / 0x4060e0, collision impulse callers,
   car+0x42c / +0x424, damage 0x5ed5d0, per-car grip 0x599170.
2. Career rewrite to the original: $0 start, scene01 → DDAY_EVENT_A in Rachel's 350Z →
   free starter car at the car lot → DDAY_EVENT_B to the crib; no car prices (5 slots,
   one per sponsor), bonus zones and tip money. Remove career.rs's invented prices.
3. AI: MellowMover / PhysicsMover switching, planner and PsychoDriver, rubber band;
   traffic spawn spacing per level.
4. Then the rest of the to-do list below; mobile HUD redesign last.

## Known issues and to-do

0. Next big items, in order:
   - **Original frontend (FEng)**: the 137 `.fng` screens are in
     `FRONTEND/FrontB.lzc(.bak)`: 0x30210 = u32 hash + JDLZ package. Format notes come
     from NFSTools/FEngLib, which has no license, so it is read as documentation only.
     Strings are in `LANGUAGES/English.bin(.bak)` (chunk 0x39000: count, table offset,
     string offset; (label hash, offset) pairs).
   - The owner's PC install has mod files (Dec 2024: FrontB.lzc, GlobalB.lzc,
     English.bin, *MemoryFile.bin). The originals are the `.bak` files; prefer them.
   - Remaining original screens: career results, Customize, the career screens
     (Crib, World Map, SMS), Profile Manager, EA Trax, Credits, Display options.
     Several new screens (options, pause, dialogs) still need a run on the phone.
     Quick race traffic density per level (None turns traffic off; the other
     levels are not decoded) and the AI performance cap are not done.
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
