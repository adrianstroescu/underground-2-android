# Progress and next steps

Where the port stands, so work can pick up from here. Last updated 2026-10-16.

## Working (tested on the S23 Ultra, 120 Hz)

- Bayview free roam with traffic, world lights, road reflections and bloom (about 120 FPS).
- Touch HUD after `docs/hud-target.png`: pedals, NOS, E-BRAKE, analog gauge, minimap and standings.
- Garage:
  - body kit, widebody, hood, lights, rims, spoiler, neon, window tint and paint;
  - two-column menu, with the car shown on the right.
- Career: race ladder, prizes, car shop and upgrades (`career.cfg`).
- Engine, effects and music audio.
- **All race modes**, chosen in QUICK RACE > MODE or in the career ladder:
  - Circuit and Sprint (Bayview, L4RA);
  - Drag (L4RB, L4RD);
  - Drift (L4RC);
  - Street X (L4RF);
  - URL (L4RG).
- The game loads a race's region when it starts (`LoadRequest::World`). Choosing FREE ROAM goes back to Bayview.
- Scenery groups (chunks `0x34107` and `0x34108`): each race switches on only its own `BARRIERS_<track>` and `PLAYER_BARRIERS_<track>`. Free roam switches on `FREE_ROAM` (the shop light beams). Instances with flag `0x10` are group-controlled.
  - Before this fix, the drift lot showed the barriers of all 11 layouts at once, so the race route was blocked.
- Drift: points for sliding, a multiplier up to x5, and a crash loses the current slide. Rivals are target scores (`drift.rs`). Grip is looser in drift events.
- Drag:
  - manual gearbox with SHIFT +/- buttons and a shift light on the gauge;
  - PERFECT SHIFT gives a short boost;
  - the engine blows after about 3 s on the limiter;
  - the car holds its lane, and pressing left or right changes lane.

## Tested in this last session

- The quick race menu with MODE, the switch to L4RC, and a drift race with the HUD (score, rival standings, laps) at 120 FPS.
- With the barrier groups on, the drift lap goes all the way round.

## Not tested on the phone yet

- A **drag** race: the SHIFT buttons, lane changes and the blown engine.
- **Street X** and **URL** races, and drag on L4RD.
- Bayview races with the new barriers (`XO_TrackBarrier*` now block cars during that race).
- Hand-driven drift tuning: how easy it is to score, and whether rival scores (`rival_scores`) need adjusting.
- Career moving between regions; the ladder now mixes all modes, ordered by prize.

## Known issues and to-do

1. A white patch on the floor of the drift lot, probably `TRN_SI_DriftMarker*` without its texture.
2. The aftermarket rims on a Peugeot 106 with a widebody may not be visible. Check a side view of the wheels.
3. Start/finish graphics (flags `0x4` and `0x8`, `43xx_StartFinGraphic`) are always hidden. They could be shown for their own race.
4. `add_car_model` never frees older models, so memory grows with every garage change.
5. Remaining roadmap:
   - vinyls and decals;
   - car cube-map reflections;
   - fog and `PAN_*` panoramas;
   - rear-view mirror;
   - AI engine sounds;
   - TRACKMAP minimap textures;
   - gamepad menus;
   - README.

## Testing aids

- `autopilot = 1` in `ug2.cfg` lets the computer drive the player's car in races, including shifting in drags. Remove it to play normally.
- Every second, `car pos ... | wall <object>` in logcat names the object the car is touching.
- `scripts/push-game-data.ps1` now also copies L4RB, L4RC, L4RD, L4RF and L4RG.
- Desktop and `ug2-engine` tests crash on this PC before `main` (MinGW + wgpu, `0xC0000005`). Test the data logic in a separate crate without wgpu, or on the phone.
