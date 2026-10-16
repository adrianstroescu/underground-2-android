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
- **Render targets**:
  - post-processing: 3× full-screen RGBA8 and 3× half-screen;
  - road reflection: 2× 320×240;
  - car reflection: a 128 render-target cube map, plus a static 256 cube map.
  - The output is LDR (A8R8G8B8).

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

### Gameplay data

- Race lines come from `Routes<track><F|B>.bin`, chained by race distance
  (`ug2-formats/route.rs::race_line`).
- The race line cuts corners. AI and autopilot follow it, so it must avoid walls.
- Drag: the manual gearbox and lanes come from the grid. Drift: rivals are target
  scores. Both are described in `PROGRESS.md`.
