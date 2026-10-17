# underground-2-android: notes for Claude

A native Rust port of Need for Speed: Underground 2 (PC, 2004) for Android, with a
touch HUD. Nothing is emulated: the engine reads the original game files and draws
them with wgpu (Vulkan). Target phone: Galaxy S23 Ultra (120 Hz).

**The goal is a 1:1 copy of the PC game**: graphics, environment, handling,
everything. When the original does something, find out how, from the data files or the
executable, and do the same. Don't invent heuristics where the data has the answer.
`docs/PROGRESS.md` says where the work stands; read it when starting a session.

The owner writes in Romanian, so reply in Romanian. Code, comments and commits are in
English.

## Rules

- Never commit EA game data (`*.BUN *.BIN *.lzc *.abk *.gin *.mus`...), APKs or
  keystores. Never bundle textures, .tpf packs or code from mods. Facts learned from
  them may be re-implemented.
- End commit messages with the session's `Co-Authored-By` / `Claude-Session` lines.
  Pushing to `origin main` is allowed.
- Every milestone ends with the APK installed on the phone.
- The owner may be watching the phone. Don't leave the autopilot driving into a wall.
  `force-stop` the app after a test, and remove `autopilot = 1` from `ug2.cfg` when done.
- Keep the Android emulator closed unless it is needed. The PC has 16 GB of RAM and
  builds starve.

## Layout

- `crates/ug2-formats`: parsers for the game's files (bChunk trees). They are pure,
  and their tests read the real game files from `../../..` (skipped if missing).
- `crates/ug2-engine`: the game.
  - `assets.rs` holds the loader thread, world and collision building.
  - `render.rs` and `shaders/*.wgsl` do the drawing.
  - `lib.rs` is `Game`. `frontend.rs` has the menus and starting races.
  - `play.rs` (session, AI), `race.rs`, `roads.rs` (race lines and catalogue),
    `vehicle.rs`, `collision.rs`, `hud.rs`, `drift.rs`.
- `crates/ug2-android`: the `NativeActivity` entry point. `crates/ug2-desktop` is the
  desktop runner. `crates/ug2-tool` is a CLI: `chunks FILE DEPTH`, `car`, `track`,
  `textures`.
- `tools/re`: Python reverse-engineering helpers (see the `ug2-reverse` skill).
- `tools/phone/shot.sh`: phone screenshot.
- The game data lives in the parent folder: `../TRACKS`, `../CARS`, `../GLOBAL`,
  `../speed2.exe`.

## Build and run (Windows, Git Bash)

- Run builds from Bash, not the PowerShell tool: its `2>&1` breaks on cargo's
  stderr.
- `source <scratchpad>/env.sh` sets PATH, `GAME` and NDK and **changes the cwd to the
  repo**. Without the scratchpad copy you need:
  - `PATH += ~/.cargo/bin` and the rustc sysroot `lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained`;
  - `NDK=C:/Users/Adi/AppData/Local/Android/Sdk/ndk/27.1.12297006`;
  - `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C dlltool=$NDK/toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-dlltool.exe"`.
- Type-check with `cargo check -q -p ug2-engine`.
- Format tests: `cargo test -q -p ug2-formats`.
- APK: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-apk.ps1 [-Fast] [-Install] [-Run]`.
  `-Fast` takes about 50 s and is for test builds. Without it you get a release build
  with LTO for milestones.
- Desktop and `ug2-engine` test binaries crash before `main` (MinGW + wgpu,
  0xC0000005). To test engine logic, use a standalone scratch crate with its own
  `[workspace]` that `#[path]`-includes `roads.rs` / `collision.rs` / `drift.rs` and
  depends on `ug2-formats` and `glam = "0.30"`. Dev profile `opt-level = 3`.
- For multi-line source edits from Python, write the script to a file with the Write
  tool. Bash heredocs with quotes inside have broken before.

## Phone testing

See the `phone-test` skill. In short:
- Use `export ANDROID_SERIAL=R3CW9021WWL MSYS_NO_PATHCONV=1`.
- Data lives in `/sdcard/Android/data/com.adrianstroescu.ug2/files/` (`NFSU2/`,
  `ug2.cfg`, `career.cfg`).
- Read logs with `adb logcat -s ug2`. Every second a `car pos ... | wall <owner>` line
  names what the car touches.

## What we know about the original (keep this up to date)

### Files and sections

- Region `L4RA` is Bayview. `L4RB` is the drag venues, `L4RC` the drift lot, `L4RD`
  more drag, `L4RF` Street X and `L4RG` URL tracks.
  - `TRACKS/L4Rx.BUN` is the index.
  - `TRACKS/STREAML4Rx.BUN` holds the streamed sections.
  - `TRACKS/ROUTESL4Rx/` has the routes and markers.
- Stream section `A21` (number 121) contains scenery sections 121 *and* 101.
  - Drivable section N01 pairs with N21 (+20).
  - x41 and x42 are panoramas.
  - 24xx and 25xx are shared solids and textures.
  - 26xx is the sky, plus event sections.
- **Visible section manager** (`0x80034150` in the index), in `ug2-formats/visible.rs`:
  - `0x34151` is the list of drivable sections.
  - `0x34152` holds outlines: 8 bytes, u16 section, u16 n, bbox, centre, n×(x, y).
  - `0x34153` holds 164-byte records: u16 section, u32 count at +16, then u16 visible
    sections.
  - `0x34154` maps event id to 26xx section. `0x34155` holds zone names (CC1, CC10...).
  - The game draws only the list of the drivable section the camera is on. Regions
    hold several venues: drawing everything put one venue's panorama walls across
    another's road.
- **Track collision** (`0x80034130`, one per drivable section, in the stream), in
  `ug2-formats/collision.rs`:
  - `0x34131` header: u32 section (the +20 one), grid.
  - `0x34134` 32-byte polygons:
    - u8 barrier id, u8 shape, u8 surface, u8 flags, i16 base height, 0xFFFF;
    - i16 x[4] at 1/8 m, i16 y[4] at 1/8 m, i16 z[4] at 1/256 m above the base;
    - a triangle repeats its last corner.
  - Surface 0x14 is a wall. Flags 0x12 mark a finite barrier. 0x16 is an infinite
    (±127 m) wall: map edges, lane splits.
  - The barrier id indexes `0x3412F` in the index: 32-byte records of u32 hash,
    u32 count, u8 1 and name[23] (`NONE`, `BARRIERS_4221`, `PLAYER_BARRIERS_4105`...).
    Those walls exist only during that race.
  - The engine uses this collision, not render meshes. The race line is pushed off
    walls at race start (`RaceLine::avoid_walls`). About 14 Bayview spots remain where
    the route data jumps between roads through the air.
- Scenery groups: `0x34107` overrides, `0x34108` named groups (`BARRIERS_<track>`,
  `PLAYER_BARRIERS_<track>`, `FREE_ROAM`, `SMOKEABLE`). Instance flag 0x10 means
  group-controlled. Flags 0x4/0x8 are start/finish graphics.
- GLOBALB:
  - `0x34201` track infos: 0x128-byte records, number at 0x8A, region at 0x40.
  - Career races live in the gameplay container.
  - `0x135200` light materials: 160 bytes, name at 0x14, floats from 0x30. Car paints
    (REGPAINT*, METPAINT*, PEARL*), CHROME, windshield tints and so on. The float
    layout is not decoded yet; it holds the car shader's DiffuseMin/Range,
    Specular*, Envmap* and Power values.

### Textures and shaders

- TPK texture `alpha_blend`: **0 = not blended** (an alpha channel is a mask, e.g. road
  reflectivity), **1 = alpha blend** (glass, fences, shadows, decals), **2 = additive**
  (neon, glows, lamps). `alpha_usage` 1 = cutout.
- A solid's mesh groups name a shader by hash (`Solid::shaders`). `bin_hash("ROAD")`
  marks the street surfaces that use the reflective road effect. Unknown so far:
  `5f5fa298` and `2e65e067` on some signs.
- The original effects are D3DX `fx_2_0` blobs in `speed2.exe` resources (RCDATA,
  `IDI_*_FX`). Extract them with `tools/re/exe_resources.py` and disassemble with
  `tools/re/d3dasm.py`.
  - **WORLD**: `rgb = 2·tex·vertexColour`, `a = tex.a·vc.a`. Fog is linear on view
    depth, `(FogValue.x − z)·FogValue.y`, clamped to [FogValue.z, 1].
  - **WORLDREFLECT** (ROAD): `rgb = 2·tex·vc + reflection·tex.a·vc.a`, opaque. The
    reflection is sampled at the same screen position, with no fresnel or blur.
  - **CAR**:
    - diffuse is second-order spherical harmonics `HarmonicCoeff[10]` on the
      normal × (DiffuseMin + N·V·DiffuseRange);
    - specular is the same SH on the reflection vector, with alpha^SpecularPower ×
      (SpecularMin + (N·V)^SpecularPower·SpecularRange);
    - the envmap is looked up with the view-space reflection,
      × (EnvmapMin + (N·V)^EnvmapPower·EnvmapRange);
    - pixel: `2·tex·diffuse + 2·tex.a·(2·env·k + spec)`.
  - **WORLDHEADLIGHT**: `rgb = sat(2·tex·(vc + HEADLIGHTS(projector uv)·clip·HeadlightColor))`.
    `HEADLIGHTS` is a 128×64 beam pattern in GLOBALB. The clip texture, indexed by
    depth·0.005+0.5, only cuts what is behind the projector. The projector pose and
    colour are set at run time, so ours are chosen in `lib.rs::headlight_projector`.
    Only the player's car projects. The old glow "pool" on the road is gone.
  - Car light materials (GLOBALB `0x135200`), 28 floats from 0x30:
    - `[0]` DiffuseMinScale, `[1..3]` DiffuseMin RGB, `[4]` DiffuseMaxScale,
      `[5..7]` DiffuseMax RGB;
    - `[8]` DiffuseMinAlpha, `[9]` DiffuseMaxAlpha;
    - `[10]` SpecularPower, `[11]` SpecMinScale, `[12..14]` SpecMin RGB,
      `[15]` SpecMaxScale, `[16..18]` SpecMax RGB;
    - `[19]` EnvmapPower, `[20]` EnvMinScale, `[21..23]` EnvMin RGB,
      `[24]` EnvMaxScale, `[25..27]` EnvMax RGB.
    - The shader's Range = Max − Min, lerped by N·V. The diffuse alpha (1 → 0.05 on
      paints) weights env and specular: a fresnel term.
    - Car mesh groups name their material through the solid's shader hash:
      `CARSKIN` is the body paint (replaced by the chosen paint), then CHROME, RUBBER,
      WINDSHIELD, CALIPER, MAGSILVER... World signs use ALUMINUM and ALUMINUM_2.
    - The SH light coefficients (HarmonicCoeff) are not in the data; they are built at
      run time.
  - Also present: SKYMORPH, OVERBRIGHT (bloom), SCREENFILTER, DEPTH_OF_FIELD,
    RAIN_DROP, RUNWAYLIGHT, HORIZON_FOG and FE. Not yet re-implemented.

### Run-time values captured from the PC game (apitrace, City Hall circuit at night)

Captured with `tools/pc/apitrace.sh` and `tools/pc/ug2pc.py`; see the `ug2-reverse` skill.
The traces live in `D:/GAME install/apitrace/traces/`.

- **Fog**:
  - world VS c8 `FogValue = (250, 1/240, 0.6, …)`, so `f = clamp((250 − viewZ)/240, 0.6, 1)`;
  - fog starts at 10 m and is at most 40 % from 106 m on;
  - `D3DRS_FOGCOLOR = 0xFF5D5F5A`, RGB (93, 95, 90).
- **Car CAR-effect constants**:
  - VS c0–c9 `HarmonicCoeff` (grey light, rgb equal, alpha differs), for example:
    - c0 = (.2772, .2772, .2772, .3641) and c1 = (−.0446 …, −.0586);
    - c2 = (.0384, .0504) and c3 = (.1476, .1939);
    - c4 = (.2480, .3258) and c5 = (−.00464, −.0061);
    - c6 = (−.0461, −.0606) and c7 = (.0759, .0997);
    - c8 = (−.0263, −.0345) and c9 = (.4667, .6130).
  - c17 is LocalEyePos.
  - c18–c23 are DiffuseMin, DiffuseRange, SpecularMin, SpecularRange, SpecularPower
    and EnvmapPower. For a paint: (.662, .655, .672, .993), (.41, .417, .251, −.828),
    (1, 1.015, 1.038), (−.365, −.406, −.458), 5.72 and 0.262.
- **Glows**:
  - Lamp glows sit at the light flares (`0x135102`). Each is HEADLIGHTGLOW 6 m wide,
    moved 4 m towards the camera, colour RGB (110, 100, 75) at alpha 220, blended
    SRCALPHA/ONE.
  - In the road reflection the same sprite is stretched to 6 × 16 m.
  - Tail lights are about 1.6 m, from (76, 14, 14) at alpha 143 to (121, 23, 23) at
    alpha 227 when braking.
  - Every additive draw has FOGENABLE false.
- **Grid**: race grid slots sit side by side up to 7 m from the start marker, and
  they all face along the race line.
- **Render targets**:
  - post-processing: 3× full-screen RGBA8 and 3× half-screen;
  - road reflection: 2× 320×240;
  - car reflection: a 128 render-target cube map, plus a static 256 cube map.
  - The output is LDR (A8R8G8B8).

### Post-processing chain (apitrace frame + speed2.exe, `tools/pc/frame_summary.py`, `tools/re/x86.py`)

Targets: scene A/B/C full-size RGBA8, H1/H2/H3 half-size, M = MSAA "light" target that
shares the scene depth. Per frame, in this order:

1. World (sky, opaque, alpha, additive) into the back buffer.
2. **Speed blur** (`0x5cc460`), only if t > 1/255:
   - t = clamp((|camera velocity| − 25) / 100, 0, 1) + 1.1·g, at most 0.65.
     g moves +0.2 / −0.1 per frame towards 1 while a camera flag is set (probably NOS).
   - n = normalize(camera velocity in D3D view space: x right, y up, z forward).
   - K = 0.0075·t·(1 + 0.5·g). Per pixel, the uv offset is
     K·(n.z·(2u−1) − n.x, n.z·(2v−1) − n.y), with taps at 0, 1, 2 and 3 × offset, 0.25 each.
   - A = copy of the back buffer; B = taps(A, K); A = taps(B, 4K).
   - back = lerp(back, A, floor(t·255)/255).
3. Cars, car shadows and glass (not blurred).
4. **Light trails**:
   - C *= (120, 90, 170)/255 per frame (BLENDFACTOR 0xFF785AAA, constant per frame);
   - M = C;
   - lamp and car-light glow sprites go into M with depth test (not into the scene!);
   - C = resolve(M).
5. A = back. Half-res blur of A (unused by the output, skip).
6. **Colour grading** (ps_1_4 effects):
   - gainmap: `samp_gainmap` 32×32, at (lum, R) and (G, B);
   - tint: `SaturationScale` 0.4, `TintColor`;
   - rampmap: `samp_rampmap` 1×256.
   The LUT contents are still to be captured.
7. **Overbright bloom**:
   - H3 = half(B);
   - H1 = (sat(4·(H3 − off)) · dot(sat, 0.588·(1,1,1)))², alpha 0. `OverBrightOffset` =
     ftol(216 − 154·s)/510, which is 0.3627 in a race.
   - 2× (H, V) blur: taps at ±1 and ±3 texels with weights 1/3 and 1/6, no centre tap.
   - H1 += H3 · (0xB0 in race, 0xA0 in menus)/255.
   - C += H1 · bloom/255. bloom = 10 + 62·s in a race (22 at rest), 80 in menus.
     s ← 0.8·s + 0.2·(0.2 + 0.8·x) per frame, with x = 0 normally.
8. Output = B + C. The HUD goes on top.

### Environment parameter maps (`0x8003b600` in `L4RA.BUN`)

- Each `0x8003b601` holds:
  - `0x3b602` header: u32 name hash, bbox x0 y0 x1 y1, u32 nodes, u32 records,
    u32 record size, u32 fields;
  - `0x3b603` field types; `0x3b604` field offsets; `0x3b605` records;
  - `0x3b608` quadtree: nodes of 4 × u16. A first u16 of 0 marks a leaf, whose second
    u16 is the record. Otherwise the child is index 2·(x ≥ mid) + (y ≥ mid).
- Names (bin_hash):
  - Sunrise Screen Tint and Night Screen Tint: R, G, B, f3, f4, f5, f6;
  - Normal Fog, Night Normal Fog and Night Raining Fog: f0, start, R, G, B, max, f6, f7;
  - Fog Blend Distance (50), Horizon Fog Visibility, Rain, Wind.
- `0x5d77f0` samples both tint maps at the camera and blends them, Night·(1−b) +
  Sunrise·b, with b from `0x60fa80` (time of day).
- Checked at the City Hall capture (camera −1378.65, 214.29, 11.95): all maps give
  record 2.
  - TintColor = (156, 160, 145)/255 × 0.2, with w = 0.19 = f6, so b = 1.
  - Fog start is 10 and max 0.4, as captured.

### Frontend (FEng), movies, fonts

- The owner's install has mods (Dec 2024) in `FRONTEND/FrontB.lzc`, `GLOBAL/GlobalB.lzc`,
  `LANGUAGES/English.bin` and `*MemoryFile.bin`. The `.bak` files next to them are the
  2004 originals: read those first (`GameDir::read_original`).
- **FEng packages** (`ug2-formats/feng.rs`; spec from NFSTools/FEngLib, which has no
  licence, so it is used as documentation only):
  - `FrontB.lzc`: chunks 0x30210 of u32 hash + a JDLZ/HUFF-compressed package.
  - Package chunks: PkHd, Res (RsNm/RsRq), Obj (Butn, FObê with ObjD tags, Scrp,
    MsgR), PkgR, Targ.
  - Script tracks are word offsets into the object state: colour 0 (BGRA as i32),
    pivot 4, position 7, rotation 10, size 14, UV 17/19. Each has a base key
    (time −1) and delta keys, in ms.
  - Messages are `bin_hash(upper(name))`. Response commands: 0 SetScript,
    1 PostToFEng, 2 PostToGame, 0x100 SetActiveButton, and If/Else/EndIf.
  - The virtual screen is 640×480 with the origin at the centre. Larger z draws first.
    Send CALL PACKAGE and CALL FRAMES on open, or only the INIT objects show.
  - The FE shader is `2·tex·vc`, blended SRCALPHA/INVSRCALPHA. TPK blend type 2 is
    additive (icons) and 5 is opaque (the track maps have alpha 0).
  - Textures are looked up by name hash, because TPK names are cut at 24 characters.
    They come from FrontB and GLOBALB.
- Screen factory table at `0x7f8570` in speed2.exe, 7 dwords per entry (package,
  constructor, hash, flags...). Quick race flow (`0x7f445c`): ModeSelect →
  ModeOptions → TrackSelect → CarSelect.
  - Main menu options are built at `0x4aee40`, quick race modes at `0x4b2d80`, the
    IconScroller at `0x543a90`/`0x543c10` (spacing = width + 40).
  - Track select: the district icon comes from track record +0x7C (`0x4984b0`); names
    are TRACK_NAME_%d / REVERSE_TRACK_NAME_%d; the map is TRACKMAP%d from
    `TRACKS/TRACKMAP<n>.BIN`.
- Quick race Game Options (`UI_QRModeOptions`, ctor `0x4fa550`): generic option list
  `0x552780`. Rows are laid out from the TITLE_MASTER / DATA_MASTER rectangles (their
  own data, before scripts), one title height (36) apart, 7 visible
  (`0x51ff80`, `0x54ef30`, `0x52d800`). Rows per mode: `0x4ccf60` and the following
  builders. The settings record is one per mode at `0x83aac4 + 20·mode`, with defaults
  from `0x52a220`. On PC the button bar is ConsoleOnly (flag 0x40): a click on the
  highlighted row's name accepts.
- Quick race Car Select (`UI_QRCarSelect`, ctor `0x4f9d90`):
  - logo CARSELECT_MANUFACTURER_<record +0xC0>, badge SECONDARY_LOGO_<car type>
    (`0x511e60`);
  - locked cars are left out; the starter set is at `0x7f7c08`;
  - filters (`0x4eed10`): All Cars, Stock Cars, My Cars, Career, Sponsor Cars.
  - Stat bars: `ug2-formats/carstats.rs` (accel `0x5b53a0`, top speed `0x5b6a20`,
    handling `0x5b5490`, range table GLOBALB chunk 0x34a32). Bars are value × (backing
    − 92) × 0.95 (`0x540e90`).
- In-game screens (UI_Pause, HUD_*, dialogs) are FEng packages in `GLOBAL/InGame*.bun`.
  Resident ones (UI_PC_Help_Bar, GenericDialog_*) are in GLOBALB.
- Icon menus are built with `0x51f670(icon texture hash, label hash)`. Grep the calls
  to list every menu: main `0x4aee40`, career `0x4af110`, crib `0x4af865`, quick race
  modes `0x4b2d80`, options `0x4b6230`, post race `0x4bee6f`, pause `0x4c2902` (free
  run) / `0x4c2bd2` (race), pause options `0x4c30b1`, profiles `0x4c58f4`.
- The 4th word of a factory entry holds the help bar flags. `0x52d330` gives the
  labels: 1 Quit Game, 4 Continue, 0x400 Back, 0x1000 Customize... Buttons fill the
  slots in bit order (button1 bottom right, button2 top right, button3...).
- Options (`UI_OptionsMain` `0x4b6230`, rows `0x4b8470`): Audio `0x4b6e60`, Gameplay
  `0x4b7310`, Player `0x4b7cd0` (profile `0x83aa64 + player·0x28`), defaults in
  `0x5116e0`. Sliders checked against a PC capture: track = base − 92, fill =
  value × track × 0.95, knob 7 past it. UI_Options shows 8 rows, the quick race list 7.
- Dialogs: `0x558790(package, button labels..., message)`, GenericDialog_SMALL: text
  `message_blurb`, buttons button1 / button2 (two) or button3 (one).
- Post race (quick race): UI_PostRaceResults then the UI_PostRace menu (`0x4bee10`).
  Times print as "M:SS.hh", "SS.hh" under a minute (`0x576d30`). All these in-game
  packages are in `GLOBAL/InGame*.bun`.
- Race HUD (`fe_hud.rs`, notes in the HUD ctor `0x4f8bf0`): HUD_SingleRace for circuit,
  sprint and URL; HUD_Drag / HUD_Drift / HUD_Short_Track otherwise. The dials are
  `*_CUSTOM_00` from `GLOBAL/HUD_CustomTextures_00.bin` (the fitted HUD part picks NN).
  Needle = rpm / dial max × 235°. The minimap shows 0.3 / zoom of TRACKMAP round the
  car; track record +0xAC minX, +0xB0 maxY, +0xB4 size, +0xC0 zoom.
- HUD packages by race type: drag HUD_Drag (needle −43° + 86° × rpm / max, the
  DRAG_RPM_* textures), drift HUD_Drift (multiplier steps 300/1200/3600/9600 on Medium,
  drift comments from GLOBALB 0x34a07), Street X HUD_Short_Track. In a package every
  object is visible unless a widget list hides it (FNG flag 0x1 = invisible).
- Quick race AI: driver names are 53 ASCII strings at `0x7f6cd0`, drawn without
  repeats. Cars come from the 26-car pool `0x7f6da4` (SUV pool `0x7f6ddc`), capped by
  the player's performance rating (`0x4feaf0`). Skill comes from GLOBALB 0x34b00.
- Traffic level: a float per level at `[0x883dac] + 0x108 + level·4` (source not traced).
- Credits: `CREDITS/NA_ENGLISH.TXT` (UTF-16). Each line is a copy of CreditsArea running
  ROLLCREDIT; its event at 300 ms starts the next line.
- Career data (GLOBALB gameplay container 0x80034A10; names are u16 offsets into the
  string pool 0x34A1D):
  - races 0x34A11 (136 B): +0 name, +2 / +4 movies before / after a win, +0x37 stage;
  - shops 0x34A12, SMS 0x34A17, stage configs 0x34A18, sponsors 0x34A19, unlock
    conditions 0x34A1C.
  - Resume Career lands on the crib menu once DDAY_EVENT_B is done; a new career
    plays Scene01 and then the "drive to the car lot / crib" events (Scene05 / Scene06).
  - Rules (`career_orig.rs`, `ug2-formats/career.rs`): races open per category (circuit,
    drag, Street X, drift, sprint, SUV) up to the stage config's caps, picked at random
    (`0x527d30`); URL races open by win thresholds; a win pays the race's prize only
    (no money for places or losses); the stage final (stage 1: all its races) advances
    the stage. A new career starts with $0. 0x34A1C gates car parts, not races.
  - Career start and car lot (`docs/re/career_start_re.md`): no intro race. Reset
    `0x532830` → $0, slot 0 open but empty → scene01 → DDAY_EVENT_A drives Rachel's car
    (race +0x2C preset DDAY_PLAYER_CAR, a 350Z) to CC_CAR_LOT_1 (SCENE05 first visit) →
    a FREE stock car (NA starters: PEUGOT, FOCUS, COROLLA, 240SX, MIATA, CIVIC = player
    cars, CarTypeInfo +0xDA bit 1, minus region list 0x5022c0, minus locks 0x34A1F) →
    DDAY_EVENT_B drives it to CRIB_1 (SCENE06) → crib menu.
  - **Cars are never bought**: the lot never touches cash. Up to 5 car slots; slot 0
    opens at start, one more per sponsor signing; empty open slot = free car, otherwise
    trade (parts lost, no refund). Lot stock unlocks by URL races (chunk 0x34A1F).
  - Other money: hidden bonus zones (chunk 0x34A1E, 59 × $50–400, once each),
    ENGAGE_TIPS_* messages ($25 / $50 first time), sponsor signing, covers, outrun.
  - World Map: `fe_worldmap.rs` (rows 36 apart at MINI_MASTER, TRACKMAP zoom from track
    record +0x118, stage overview maps TRACKMAP4500..4504).
- EA Trax: 27 songs in an exe table at `0x7ecd80` (title / artist / album / FE|IG);
  playlist and modes (Off / Menu / Racing / All) in the profile at `0x83acb8`.
  Song id N is SCHl stream `music::SONG_STREAMS[N]` of `pfdata/MusicSFx.mus` (MusicSFx.mpf
  event N), played whole. Menus play the Menu|All set, races the Racing|All set; no song
  repeats until all enabled ones played; Sequential = next list position, Random = the
  r-th remaining (`0x479220`, `0x46b6f0`). The other streams are the background score.
- **Strings**: `LANGUAGES/English.bin`, chunk 0x39000 (count, table offset, strings
  offset; entries are (bin_hash(label), offset)).
- **Fonts**: FNTF, chunk 0x30201 in the language files and GLOBALA. Glyph images are
  in `LANGUAGES/LanguageTextures.bin` and GLOBALA (e.g. TT21I). Glyph record: x offset
  at byte 9, y offset at 10, advance at 14 (`ug2-formats/font.rs`).
- **Movies** (`MOVIES/*.vp6`, `ug2-formats/movie.rs`, `crates/ug2-movie`):
  - EA container: MVhd (width/height at 12/14, frames at 16, rate den/num at 24/28),
    then MV0K/MV0F frames. Pictures are stored bottom-up.
  - Sound: SCHl with a "GSTR" header (tags from +8), big-endian EA-XA v2 SCDl blocks
    with no history pair, 32 kHz stereo.
  - VP6 is decoded with nihav-vp6 (ruffle-rs fork, MIT). It takes 7.4 ms per frame on
    a Galaxy S9+.
  - Boot order: ealogo, THX_logo, FMVOpening. A tap skips.
- **Showroom** (`FRONTEND/PLATFORMS/Showroom.BIN`): 6 solids (DISC01–03,
  BACKDROP, BACKDROP_FLOOR, PLATFORMBASE), each placed by its own matrix.
  - Camera: eye (0, −4, 1.4), direction (0, 0.999922, −0.012462), vertical FOV 29.2°.
  - Fog: black, from 0 to 250 m, at most 0.4.
  - The car sits on disc 1 at (0, 5.44, 0.1697), yaw −54.2°, with no glows.

### Car shading in the port

- `ug2-formats/material.rs` parses the light materials. They are checked against the
  capture: every material the game sent matches, and specular power is data × 1.1.
- Each car mesh group gets its material from its shader hash (`DrawRange::material`,
  id = index + 1). CARSKIN becomes `PAINT_MATERIAL` and is resolved per car to the
  nearest REGPAINT* colour (`lib.rs::PAINT_MATERIALS`). Traffic cars use TRAFFIC.
- The id reaches the shader as the draw's instance index. `mesh.wgsl` evaluates the
  CAR effect per pixel in car space.
- The SH is still one captured set (`render.rs::CAR_SH`). The game builds one per car
  from nearby lights: 21 different sets in one frame.
- The envmap is a gradient; the game renders a 128 px cube map.

### Car physics (speed2.exe PhysicsMover, being re-implemented)

- The player uses the full PhysicsMover (vtable 0x7a25e0, update 0x5b72d0); AI and
  traffic may use the MellowMover. Frame dt is split into n = trunc(dt/0.0044)+1
  explicit-Euler substeps. Units: t, kN, kN·m, t·m², rad/s. A physics-struct offset X
  is CarTypeInfo record offset X + 0x110 (`ug2-formats/carphysics.rs`).
- Engine, clutch, gearbox and drive split: `ug2-engine/powertrain.rs` (addresses in its
  header). The clutch state machine sets the throttle scale per state (0 / 0 / 0.7 / 1);
  a new car starts in neutral, clutch 0, engaging. Upshift quality (0x594f20, windows
  0x7fbf60 by skill row +0xf0 = 3) gives the drag "perfect shift" throttle bonus
  (1 + 2·q fading over 0.5 s); a drag shift outside clutch state 1 is a missed shift
  (no throttle 0.45 s). Who puts the drag clutch in state 1 is not traced yet.

### Gameplay data

- Race lines come from `Routes<track><F|B>.bin`, chained by race distance
  (`ug2-formats/route.rs::race_line`).
- The race line cuts corners. AI and autopilot follow it, so it must avoid walls.
- Drag: the manual gearbox and lanes come from the grid. Drift: rivals are target
  scores. Both are described in `PROGRESS.md`.
