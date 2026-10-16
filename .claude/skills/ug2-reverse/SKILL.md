---
name: ug2-reverse
description: Work out how the original NFS Underground 2 does something (a file format, a chunk, a shader, a render rule) from the game's data files and speed2.exe, then re-implement it in Rust. Use before adding any heuristic to the engine, whenever a visual or gameplay detail must match the PC game 1:1.
---

# Finding out what the original does

The data almost always has the answer. Guessing has been wrong before:
- road alpha was a reflection mask, not transparency;
- blend type 1 meant alpha blending, not additive;
- every venue's panorama was drawn everywhere.

## Method

1. **Find the data.** List chunk ids and sizes:
   ```bash
   cargo run -q -p ug2-tool -- chunks "$GAME/TRACKS/L4RB.BUN" 2
   ```
   Hex-dump candidates with a short Python script, checking the first records
   against known values. Chunk headers are `u32 id, u32 size`. Ids with the top bit
   set are containers. Bodies may start with `0x11` padding.
2. **Decode.** Look for record strides, section numbers (101, 121...), floats
   (`00 00 80 3f` = 1.0) and names. Coordinates are metres, z up. Integer
   coordinates are often fixed point (collision: x and y at /8, z at /256 plus an
   i16 base).
3. **Validate against something independent.** Compare with race lines (their z is
   the road height), instance bounding boxes, or counts (a table's count field
   equals the number of polygons using it). A hypothesis that holds for 99% of
   points is right. Look at the outliers before trusting it.
4. **Find the links.** An id is usually an index into a named table nearby.
   - Collision barrier id → `0x3412F` names.
   - Drivable section → `0x34153` visible list.
   - Mesh group shader index → `Solid::shaders` hash → `bin_hash("ROAD")`.
   Match by name hash: `bin_hash(s) = fold(0xFFFFFFFF, h*33 + c)`. Hashing every
   string in `speed2.exe` (as-is and upper-case) names most hashes.
5. **Implement in `ug2-formats`** as a pure parser with a test that reads the real file
   and asserts known facts, skipping if the file is missing. Then wire it into
   `ug2-engine`.
6. **Write it down** in the "What we know about the original" section of `CLAUDE.md`.

## Tools (`tools/re`)

- `strings.py FILE REGEX MINLEN`: strings with offsets. Good for finding names in
  `speed2.exe` or GLOBALB.
- `exe_resources.py speed2.exe OUT`: extracts the PE resources. The effects are
  `IDI_*_FX` (D3DX fx_2_0 binaries, compiled vs_1_1 / ps_1_1).
- `d3dasm.py OUT/IDI_WORLD_FX.bin`: disassembles every shader in an effect, with
  constant-table names. Read the vertex and pixel shaders together:
  - `oD0` / `oD1` are the vertex colours `v0` / `v1` in the pixel shader;
  - `oTn` is `tn`;
  - `_x2`, or `add r, r, r`, is the 2× overbright.
- `visible_sections.py TRACKS/L4RB.BUN`: dumps the visible section manager.
- `collision_plot.py STREAM.BUN X Y R out.png [line.csv] [Z]`: draws the collision from
  above, with walls in red, infinite walls in yellow and the race line as dots.
  Export race lines with a scratch crate that includes `roads.rs`.

## The original game as a reference (owner approved)

The PC game is installed in the parent folder and may be launched for reference
shots and traces. The owner's career save is in it, so **never save, buy or
progress the career**. Use Quick Race. The game runs as a borderless window
(WidescreenFix `WindowedMode = 4`).

- `python -I tools/pc/ug2pc.py launch | keys enter right*3 wait:2 | shot NAME DIR | close`.
  Keys and shots only work while speed2.exe is the foreground window. If the owner
  is using the PC (another window in front), stop and ask them to click into the game.
- Menu path to a race:
  1. Enter at the title screens leads to Main Menu (Quick Race selected).
  2. Then Enter for the mode (Circuit), Enter for the track (City Hall first), Enter
     for the options, Enter for the car, then Enter for automatic.
  3. The race starts about 15 s later.
- **apitrace** (installed permanently in `D:/GAME install/apitrace`, owner's request):
  1. `bash tools/pc/apitrace.sh on`, then launch and play a few seconds;
  2. close the game, then `apitrace.sh off`. Recording is slow, and a 1 min trace is
     about 2 GB.
  3. The trace is written to `C:/Users/Adi/Desktop/speed2.trace`. Move it to
     `D:/GAME install/apitrace/traces/`.
  4. Find frames with `apitrace.exe dump --calls=frame T | grep Present`. Dump one
     frame with `--calls=A-B`.
- Shader constants are re-sent from register 0 for every draw (the whole constant
  table). Match them to the effect's constant table from `d3dasm.py` (for example
  24 vectors means the CAR vertex shader).

## A scratch crate for engine logic

The engine's test binaries crash on this PC, so make a crate in the scratchpad:

```toml
[package]
name = "roadtest"
edition = "2021"
[workspace]
[dependencies]
ug2-formats = { path = "<repo>/crates/ug2-formats" }
glam = "0.30"
anyhow = "1"
log = "0.4"
[profile.dev]
opt-level = 3
```

```rust
#[path = "<repo>/crates/ug2-engine/src/roads.rs"] #[allow(dead_code)] mod roads;
#[path = "<repo>/crates/ug2-engine/src/collision.rs"] #[allow(dead_code)] mod collision;
```

This makes it possible to run, say, `RaceLine::avoid_walls` on all 103 races in about
1 s and count what is left.

## Legal line

- Read EA files locally. Never commit them or their extracts: no dumped shaders,
  textures or tables.
- Re-implementing the maths and formats is fine.
- Mod packs (.tpf, textures) are not to be bundled.
