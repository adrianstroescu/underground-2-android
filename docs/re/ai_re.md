# NFSU2 AI opponents and traffic (speed2.exe, static RE)

Reference code: `ai_ref.py` (skill table, rubber band, target speed). Detailed sub-reports
(merged below, full text kept in the files): `ai_re_driver.md` (PsychoDriver steering/throttle/
brake/PIDs, drag/drift drivers), `ai_re_plan.md` (planning, actions, race line following,
recovery, belt mover), `ai_re_traffic.md` (traffic).
Ticks are 1/4000 s (`[0x86518c]` = now). Every "s" below is ticks*0.00025 (const 0x784268).

## 1. Object model

### Car controller kind (car+0x24), set by `0x5eba30(kind)`
`car+0x2c` = controller, `car+0x30` = driver = the input object the mover reads (controller+8).
| kind | ctor (size) | class (snapshot name) | driver (car+0x30) |
|---|---|---|---|
| 0 | 0x5eb930 (0x54) | CarController (vt 0x7a5ca8) | CarDriver 0x423330 (vt 0x7855d8) |
| 1 | 0x4266d0 (0x820) | RealController : TrackedController (vt 0x785230) | RealDriver 0x425fc0 (vt 0x784fc8) = local human |
| 2 | 0x4299c0 (0x7d4) | DragController : RaceCarController (vt 0x785518) | DragDriver 0x427f20 (vt 0x785118) |
| 3 | 0x4260b0 (0x7b4) | RaceCarController : AICarController (vt 0x785370) | PsychoDriver 0x425ab0 (vt 0x784eb0); DriftDriver 0x425e40 (vt 0x784f40) in drift races |
| 4 | 0x426580 (0x7a8) | TrafficController : AICarController (vt 0x785428) | PsychoDriver 0x425ab0 |
| 5 | 0x426650 (0x68) | TrackedController (vt 0x784680) | CarDriver (remote/network) |

Car activation `0x5f6d50`: racer kind (RacerInfo+4) 1 = human -> controller 1 (5 if racer+0x900, remote);
2 = AI -> 3; 3 -> 4 (traffic). DragController (2) is set by `0x5f3023` for drag races.
RaceCarController picks DriftDriver when `![0x89e7d8] && ![0x890108] && ([0x890114]||[0x89e7d9]||[0x890104])`
(drift flags), else PsychoDriver (0x426141..0x4261bc).

CarDriver input fields: +0x1e0 u16 steering angle (65536 = 360 deg, x0.00549316 = 0x784248),
+0x20c gas (setter 0x5eb910), +0x214 foot brake (0x5eb8f0), +0x210 handbrake (0x5eb900) per the driver
fork (AI, TrafficMover and MellowMover all use gas - (+0x214)); **this conflicts with physics1_re.md,
which calls +0x210 the brake - to reconcile**. +0x1e4 direction mode (-1 fwd, 0 reverse, 1 neutral),
+0x1e8 manual gear request (never written by AI -> AI uses the automatic gearbox).

### AICarController / RaceCarController fields (this = controller)
+4 car, +8 driver, +0xc path, +0x54 planning context (FuturePoints at +0x54+0x31c, index +0x308),
+0x6e0/+0x6e4/+0x6e8 route cursors, +0x6f0 replan countdown, +0x6f8 current action, +0x6fc current goal,
+0x700/+0x704 braking-point noise (time, metres), +0x710 "stopped/finished" flag,
+0x72c last catch-up evaluation tick, +0x730 belt-exit timer, +0x734 physics timer,
+0x738/+0x73c stuck AverageWindow (10 samples), +0x744 catch-up enabled (1), +0x748 catch-up target
player, +0x750 speed-zone index (-1), +0x758 AverageWindow(5), +0x770 catch-up multiplier (raw),
+0x778 last replan tick, +0x790 human-autopilot speed scale (1), +0x7a8 nitrous start tick,
+0x7ac nitrous end tick, +0x7b0 nitrous straight timer, +0x798/+0x79c shortcut choice (route id, tick).
Skill = vt+0x78 = `0x402580` -> `0x610510(racer)` = RacerInfo+0x901 (online: 9 unless 0xfe/0xff).

## 2. Which mover AI racers use (`0x5f46a0(type)`, jump table 0x5f493c)

Correction to physics1_re.md: the table is 0 = none (0x7a5bb0), **1 = PhysicsMover** 0x5b5cc0,
**2 = TrafficMover** 0x408740 (0x24 B), **3 = MellowMover** 0x5b6d00, **4 = Belt** 0x5a9030 (0x4c B),
5 = NIS 0x5b6fc0 (names from the enum strings at 0x7e8e34: No/Physics/Traffic/Mellow/Belt/NIS movement).
If car+0x5c0 == 0 and the request is not 0/4, the type becomes 0 (car kind 1) or 4.

Default mover `0x5f4400(car)`:
- car kind 4 (traffic) -> 2.
- local human (racer kind 1, !+0x900, !(car+0x18)+0x9a8, ![0x89008c]) -> 1; remote (+0x900) -> 3.
- otherwise -> 1 if controller vt+0x50 (`0x41e510`: physics timer +0x734 running), or drag
  (`[0x89e7d8]`/`[0x890108]`), or drift (`0x401950`); else **3 (Mellow) for kinds 3/5**, 0 for kind 0.
- debug override `[0x8021ac]` (-1), online `[0x8650b0]` -> 1.

So in circuit / sprint / Street X / URL / Lap KO the AI racers drive with the **MellowMover**,
and switch to the full PhysicsMover only:
- for 5 s from the green light (`0x42c0c0`: before the start every frame `+0x734 = 5 s`),
- for 5 s after a collision with a vehicle (`0x402450`, controller vt+0x38 OnCollision:
  `other && other+0x18` or the flag arg -> `+0x734 = 5 s`, mover 1),
- while that timer runs; when it expires (`0x42c281..`) and car+0x424 (wheels on ground) < 4 it is
  re-armed for 1 s (0xfa0 ticks), else the car goes back to the default mover,
- after finishing (`0x5ee250(nav)`: nav+0x10 != 0 and race type != 0) -> physics.
Physics/mellow are only used while car+0x5cc (the collision-surface pointer from the world tracker
car+0x5c4) is set; without loaded collision the car is forced to Belt (4) (`0x4024cc`).
Drag and drift AI always use the PhysicsMover.

Belt (4) when far from the player (`0x402180`, checked at the end of AICarController::Update 0x42b261,
gated by tweak `[0x7e8d70]` = 1):
not drag/drift, not track 0x44b special case, driver+0x10 >= 3, belt-exit (+0x730) and physics
(+0x734) timers idle, car+0x42c (speed) >= 5, car+0x424 >= 1 wheel down, current action type not
8/9/0xa/0xb/0xd, vt+0x78 skill != -2, distance from the camera/player (`[0x890080]` 0x5f1160(car+0x60))
>= **120 m**, race clock >= **30 s**, racer not finished. Leaving belt (e.g. hit while on belt):
`+0x730 = 2 s`. Belt motion: see section 6 / ai_re_plan.md.

## 3. Creating AI for a race
- Quick race `0x53f150` (and siblings 0x53f370.. per mode): copies options from the per-mode struct
  `0x83aac4 + mode*20` (+0 humans -> [0x89e7c0], +1 opponents -> [0x89e7c4], +2 difficulty -> [0x89e7f4],
  +4 traffic level -> [0x89e7a8], +5 catch-up -> [0x89e7f8]); per-player catch-up scale
  `[0x89e7fc]/[0x89e800] = data+0x94c + (carClass + difficulty*4)*4` (all 1.0); then `0x53ee90(n)`
  creates n RacerInfo(slot 100+i, kind 2, car, name, skill) (see ainames_re.md: names, car pool
  capped by the player's rating + 0.06, stock parts, **no upgrades**), then `0x4fe9f0` (traffic).
- Skill `0x4fed00(difficulty)`: cumulative table data+0x9f4+difficulty*40:
  Easy {0.3,0.7,1} -> skill 0/1/2 at 30/40/30 %, Medium -> 3/4/5 at 30/40/30 %, Hard -> 6/7/8 at 20/40/40 %.
  Skill 9 = online only.
- Career `0x5260b0` (career2_re.md section 5): per opponent u8 skill[3] indexed by [0x89e7f4]
  (career copies it from [0x83ac30] in 0x5322e0), performance level -> 0x5b6ad0 upgrade package
  (closest precomputed package to the target rating); mode 1 = match the player (0x5b7570).
- Career race setup 0x5322e0 never writes [0x89e7f8] (only the quick race setups 0x53f1..0x53fb.. and
  the resets 0x4fef80..0x4ff160 (=0) do), so in career the Catch Up flag keeps its last value
  (0 after boot / after a reset) -> **no distance rubber band in career** (probable; verify in game).

## 4. Skill table (GLOBALB chunk 0x34b00, 0xae0 bytes, ptr [0x81cb60])

Layout (offsets in the chunk):
- 0x000..0x2c0: 8 graphs x 11 (x,y) points (0x58 B): rubber band, graph i = A-set (i<4) / B-set (i>=4).
  A (all four identical): (-320,.22) (-160,.15) (-120,.09) (-85,.06) (-60,.02) (0,0) (120,-.025)
  (170,-.06) (240,-.08) (320,-.15) (640,-.25). B: (-320,.37) (-160,.22) (-120,.12) (-85,.07) (-60,.03)
  (0,0) (120,-.01) (170,-.04) (240,-.06) (320,-.10) (640,-.15). x = gap in m (AI ahead > 0).
- 0x2c0 / 0x318: Street X graphs A/B (2 dummy (0,0) points then (-200,.15)(-150,.12)(-100,.08)(-50,.05)
  (0,0)(50,-.10)(150,-.17)(200,-.22)(250,-.25) and (-200,.20)(-150,.15)(-100,.12)(-50,.08)(0,0)(50,-.02)
  (150,-.10)(200,-.17)(250,-.22)).
- 0x370 / 0x3c8: two more graphs (0,0)x4 (-100,.5)(-50,.25)(0,0)(25,-.05)(50,-.2)(75,-.65)(100,-1)
  (not referenced by the code found so far; maybe type-7 outrun).
- 0x420..0x423 bytes: flags (all 0): +0x420 position term in Street X, +0x421 per-player scale in
  Street X, +0x422 perf-adjust in Street X, +0x423 race fade in Street X.
- 0x424/0x464 (+idx*4): position-difference table (y at skill 0 / skill 9): **all zero**.
- 0x49c+pos*8: player-faster table (pos 1..6): (.75,.97) (.6,.7) (.4,.5) (.2,.2) (.4,.6) (.3,.5).
- 0x4cc+pos*8: player-slower table: (1.02,.5) (1.07,.74) (1.17,.81) (1.27,1.0) (1.2,1.0) (1.25,1.0).
- 0x504: fade graph 4 pts (0,1) (.33,1) (.67,1) (1,0) over race fraction.
- 0x524 + skill*0x68: per-skill rows (skill 0..9):

| row off | abs | meaning (users) | skill 0..9 |
|---|---|---|---|
| 0x00/0x04/0x08 | 0x524.. | 3-way random decision thresholds (0x413cc0) | 0 / 0,0,.05,.2,.35,.5,.7,.7,.85,.9 / 1 |
| 0x0c | 0x530 | shortcut acceptance probability (0x408280) | 1 |
| 0x10 | 0x534 | ? (0.8 1 1.1 1.3 1.4 1.6 1.8 1.9 2 2.1) unresolved | |
| 0x14 | 0x538 | **race-line speed scale** (0x41c85c) | .74 .76 .78 .84 .86 .88 .93 .95 .97 1.0 |
| 0x18 | 0x53c | perf-adjust floor (0x4175c6) | .76 .77 .79 .80 .82 .84 .86 .88 .90 .92 |
| 0x1c | 0x540 | Street X line-speed scale (0x41c7db) | .75 .77 .79 .81 .83 .85 .87 .89 .91 .93 |
| 0x20 | 0x544 | Street X perf-adjust floor | .61 .63 .65 .67 .69 .71 .73 .75 .77 .79 |
| 0x24/0x28 | 0x548/0x54c | drag launch window, s rel. to green (0x42c10f) | -5..-4.3 ... -1.5..-0.8 |
| 0x2c/0x30 | 0x550/0x554 | drag drivetrain +0xd8/+0xd4 (0x5b6251) | 0 / 0.01 |
| 0x34..0x40 | 0x558.. | drag shift-rpm error lo/hi (drivetrain +0xe0,+0xdc,+0xd0,+0xcc) | -900/300 ... -110/15 |
| 0x44 / 0x48 | 0x568/0x56c | nitrous cooldown s (normal / last lap) | 10/5 10/5 8/4 6/3 6/3 6/3 6/3 5/2 5/2 10/5 |
| 0x4c | 0x570 | nitrous min burn s, and min charge to start | .5 .5 1 1 1 1.5 1.5 2 2 .5 |
| 0x50 | 0x574 | nitrous max burn s | 1.5 2 2.5 3 3 3.5 3.5 4 4 4 |
| 0x54/0x58 | 0x578/0x57c | ? (.025/.1 ... .95/1.1) unresolved | |
| 0x5c | 0x580 | catch-up start delay, race clock s | 3 4 5 6 8 10 13 15 17 19 |
| 0x60 | 0x584 | braking-point noise +- m | 40 35 30 30 25 20 20 20 20 20 |
| 0x64 byte | 0x588 | drag: min gear index-1 for nitrous (gear >= v+1, 2 = 1st) | 5 5 5 5 5 4 3 3 3 3 |
| 0x65 byte | 0x589 | "save nitrous" flag (skill >= 5) | 0 0 0 0 0 1 1 1 1 1 |

- 0x934: 3 x {i32 dist, i32 flag} = {200,1} {200,1} {400,1}, indexed by RacerInfo+0x902 (0x414130).
- 0x94c: 12 floats 1.0 (per-player catch-up scale, by car class + difficulty*4), then 1.0/.3/.8 ...
- 0x9f4: skill pick tables (3 x 10).
- 0xa6c: 2-player split-screen catch-up graph (-600,1.08) (-250,1.03) (-120,1.015) (-50,1.0) (0x5f6740).
- 0xa8c + c*8: traffic spacing ranges (see traffic).
- 0xad4 = 5 s (braking-noise re-roll period), 0xad8 = 0.3 s (catch-up period), 0xadc = 1e5 m (gap limit).

## 5. Difficulty and rubber band

### What skill changes
Nothing in the tyre/engine model reads the skill (only drag shift tuning in the PhysicsMover ctor
0x5b61b0..0x5b625a for car kinds 2/3/4 in drag: drivetrain+0xe8 = 0x7a19f4[difficulty] = .4/.8/1.0
(not in FE mode [0x850078]==1), rows 0x2c..0x40 -> drivetrain +0xcc..+0xe0).
Skill changes: the target speed scale (row 0x14), braking-point noise (row 0x60), rubber-band
strength (A/B blend), catch-up start delay (row 0x5c), nitrous habits, drag launch / shifting,
avoidance decisions (row 0x04).

### Catch-up multiplier `RaceCarController vt+0x80 = 0x4172d0`, called each frame from 0x420828
Result m: **car+0x468 = max(m, 1)** and **controller+0x770 = m**.
- car+0x468 goes through `0x5a4c27 -> 0x599170(m)` into **engine+0x90 (torque multiplier)**,
  **rb+0x208 (speed-limiter scale)** and aero+8 (if [0x7fc0a0]; x0.4 when car+0x95c && (car+0x3c)+0x18;
  car+0x4e4 overrides when non-zero and !car+0x95c). The MellowMover reads it too (0x5a3398 -> +0x28 obj +8).
- +0x770 multiplies the AI target speed (0x41c8a8 / 0x41cb76).

Order (this = controller, nav = car+0x1c):
1. ok = this+0x744 && car+0x18 == 0 (AI) && nav; drag or drift -> ok = 0.
2. race type [0x89e7b0] == 0 (free roam) -> m = 0.75, done. Type 7 (probably outrun) and nav+0xa == 1 -> m = 1.
   !ok -> m = 1.
3. Re-evaluate only if (now - this+0x72c) > data+0xad8 (0.3 s); otherwise m = this+0x770.
   this+0x72c = now, this+0x748 = 0.
4. act = current action +0x1fe (allows catch-up). started = race clock >= row 0x5c.
   If ![0x81cb98] (debug) && humans [0x89e7c0] > 0 && act && started:
   - target = the human ([0x8900b4]) if [0x8900bc] == 1; split screen ([0x8900bc] == 2): AIs placed
     worse than half the field (+1 per human in the top half) chase the worse human, others the better.
   - (a, b) = `0x4084a0(myNav, targetNav)`; scale = [0x89e7fc + target+0x10*4] (1.0) unless Street X;
     **m = scale*b + 1 + a**.
5. If act && target && (!Street X || data[0x422]) && [0x89e7f8]:
   r = target player's AverageWindow(10) (player+0xa0, fed by 0x602838 with car |v| / line speed of
   the current route node), pos = target nav+0xa (1-based),
   tbl = r >= 1 ? data+0x49c+pos*8 : data+0x4cc+pos*8, v = lerp(tbl[0], tbl[1], skill/9),
   **m = max(1 + (r-1)*v, row 0x18 (Street X: row 0x20)) * m**.
6. If act && target && (!Street X || data[0x423]): x = myNav+0x14 / max(laps,1),
   **m = (m - 1) * graph0x504(x) + 1** (full until 2/3 of the race, fades to none at the finish).
7. car+0x468 = max(m,1), this+0x770 = m.

`0x4084a0(myNav, tgtNav, &a, &b)` (needs [0x89e7f8] = Catch Up option, else a = b = 0):
- gap = (myNav+0x14 - tgtNav+0x14) * [0x88f2b0] (lap/route length in m; laps-progress difference),
  race type 7: gap = tgtNav+0x24 - myNav+0x24. gap > 1e5 -> return.
- graphs: Street X -> 0x2c0/0x318; else iA = (traffic level [0x89e7a8] in {2,3}) + 2*(circuit && laps >= 4
  (table 0x7e95c8 = {0,4})), graphA = iA*0x58, graphB = (iA+4)*0x58 (identical in the data).
- **a = lerp(graphA(gap), graphB(gap), skill/9)**; type 7 && my pos != 1 -> a *= 1.5.
- speed matching: **a += clamp(-0.005*(v_me - v_tgt) / (1 + 0.01*gap^2), -0.05, 0.05)**
  (v_me = car+0x42c when on belt, else car+0x430; constants 0x7834f8, 0x7834fc, 0x783500).
- b: (unless Street X && !data[0x420]) d = tgtPos - myPos, idx = d<0 ? d+8 : d+7,
  b = lerp(data[0x424+idx*4], data[0x464+idx*4], skill/9) = 0 (table zero).
Example a (gap 50 m/s both): skill 0: -400 m -> +0.22, -100 -> +0.073, +300 -> -0.13, +700 -> -0.25;
skill 8: -400 -> +0.35, +700 -> -0.16.

Human split-screen catch-up (0x5f6740): for the local human when [0x89e7f8] && [0x8900bc] == 2:
car+0x468 = graph0xa6c((otherNav+0x14 - myNav+0x14)*route length) (up to x1.08 when 600 m behind).

## 6. Target speed (`0x41c5e0`, racing case; speed planners in ai_re_plan.md)
```
skill = vt+0x78
if this+0x750 != -1: speed zone [0x883db0]+idx*32 ...         (zone speed)
elif 0x401fa0() (timer this+0x754 running) or (drag 0x4018f0() and nav): speed = 99999
else:
  if 0 <= skill < 10 and (now - this+0x700) (0x4026e0) >= data+0xad4 (5 s):
      +0x704 = rand(2*row0x60) - row0x60 ; +0x700 = now          (braking-point error, m)
  d = current FuturePoint distance (+0x18 of point[+0x308]) or 0x419bc0(...)
  scale: human autopilot (racer kind 1 && (car+0x18)+0x9a8) -> this+0x790
         Street X -> row 0x1c
         type 7   -> 0x78357c[k] (0.84) if my pos == 1 else 0x783598[k] (1.0), k = clamp([0x862838],0,6)
         online   -> 1.0
         else     -> row 0x14, and d += this+0x704
  v = 0x411dd0(path, d, car)   (race-line speed = route node byte +0x2c, m/s); |v| < 0.001 -> 30
  speed = v * scale
speed *= this+0x770
if action+0x1fc: limit = speed, b = 0 else limit = action+0x1f0, b = action+0x1f4
limit = min(limit, vt+0x4c(ctx), speed)
driver->vt+0x38(limit, b, action+0x1f8, action+0x1fd)       (PsychoDriver SetSpeed 0x4054a0)
```
Debug overrides: [0x81cb74] -> 15 m/s; [0x81cb88] -> raw node speed.

## 7. Nitrous (RaceCarController)
- Stop check in AICarController::Update (0x42ad24 / 0x42b0e6): AI && active && !vt+0xa0 && vt+0xb4 -> stop
  (0x592500) and vt+0xa8.
- vt+0xb4 `0x408430` CanStop: drag -> 1; else (now - +0x7a8) > row 0x4c (min burn).
- vt+0xa0 `0x416f70` WantNitrous: false on belt / without nav. Drag -> `0x40d850`.
  pct = lap position (0x5d5d10(car+0x8f8*100, track value), +100 if < 0); last lap = nav+9 == laps-1;
  final = last lap && pct > 75 %. Skill >= 5 (row byte 0x65) with no charge left (0x592480 <= 1) keeps it
  unless final (or type 7). Active and burned >= max (row 0x50, type 7: 2 s) -> false.
  Cooldown = last lap ? row 0x48 : row 0x44 (final: 0); inactive and (now - +0x7ac) <= cooldown -> false.
  Then want = route lane type of the point ahead in 2..5 (0x403d80) or car |v| < 0.8 x line speed
  (type 7: 1.0) (0x4024f0), plus a 1 s window on 0x412220 (+0x7b0); want -> `0x40d850`.
- `0x40d850` CanUse: (charge 0x592480 >= row 0x4c or active) && 4 wheels down && wheel checks &&
  gear (CarState+0x3f4) >= (drag ? row byte 0x64 + 1 : [0x7e8d90] = 2) && !0x402590.

## 8. Low-level driver (PsychoDriver) - summary of ai_re_driver.md / ai_ref_driver.py
- SetSpeed vt+0x38 `0x4054a0`: +0x230 target speed (signed), +0x234 time constant, +0x238, +0x23c pedals
  enabled. Throttle cap +0x22c = 0x783a28[[0x89e804]] = .93/.97/1/1 for AI racers, 1 otherwise.
- Look-ahead `0x415f80/0x415630`: physics/belt/racer-mellow L = graph 0x7e9d94(v) (22.9..55.9 m over
  0..160 m/s) x car-class factor; traffic max(0.9v, 9); other max(0.55v, 5). Target point L m along the
  action's DriveTarget polyline (<= 20 points).
- Errors `0x4054d0`: heading error to the target point, blended with velocity-direction error (weight 0 at
  5 m/s -> 1 at 10 m/s), offset error = atan-like(5 m, distance to polyline); windowed P/I/D history 0x4016b0/0x576bb0.
- Steering: fixed gains for non-physics movers (0x78395c/0x783994/0x783978); physics uses the adaptive
  PID 0x409b50 toward reference curve 0x827a18, gains overwritten above 10 m/s by speed graphs
  0x7e9d44/0x7e9d80/0x7e9d58/0x7e9d6c; clamp +-45 deg; max wheel angle graph 0x7e9e0c (45 deg at rest
  -> 2 deg), rate graph 0x7e9e20 (x2 toward centre), 100/s for non-physics movers.
- Speed control `0x41d8d0`: bang-bang under 5 m/s target, else PID on (target - v) with separate
  accel/brake gains (0x7e8fcc..0x7e9058, P .1 I .02 D .01-.02), pedal integrated with rate limit graph
  0x7e9d10 (30 -> 6 /s), traction cap from wheel-slip history. Pre-start revs in neutral; rolling the
  wrong way -> brake .9 (-7/-2 m/s hysteresis); reverse mirrors steering.
- Drag: DragDriver = PsychoDriver steering + embedded RealDriver pedals (+0x470, not decoded). Drift:
  look-ahead clamp(15 + .75|offset| + .04v, 15, 30), rest not decoded.

## 9. Planning layer - summary of ai_re_plan.md
- Goals (controller+0x6fc, factory 0x42aa20): racer "Win road race" (planner 0x41c5e0, chooser 0x42d0f0,
  default JustDrive, branch choice 0x413cc0); drag "Win drag race"; traffic "Be (ambient) traffic car" (0x41c380).
- Replanning 0x42abd0: on own frame slot (frame % 8, drag % 4) and >= 0.2 s since last.
- Actions (base 0x4234a0(type, priority, timeout)); ids: 0 Pass, 1 AvoidAvoidable, 2 AvoidAsTraffic,
  3 Draft, 4 Block, 6 JustDrive, 7 DriveTrafficLane, 8 PullOver, 9 GetUnstuck, 11 IJustGotHit,
  12 HitDrag, 13 AccidentRecovery, 15 HitWallDrag, 16 StopDriving, 17 ObeyIntersections, 18 PickRoute,
  19 DriveAtOffset, 20 RollingRoadblock. Replace only if priority strictly higher (0x402360).
- Racer chooser 0x42d0f0 order: GetUnstuck (500, <= once / 6 s: >= 20 m off route, or slow wrong way, or
  >= 0.4 s against a wall < 5 m/s) > IJustGotHit (400) > AvoidAvoidable (300, 3 m clearance) >
  AccidentRecovery (500: < 2 wheels down and up.z < 0.707) > Block (200/200/400 by RacerInfo+0x902,
  follower within 50 m arriving within 30 s human / 2 s AI, never Street X) > Pass (100: gap < 10 m and
  closing or predicted contact, 3 m extra clearance) > JustDrive (0). Draft / NudgeAvoidable never chosen.
- Drive target: lateral-offset struct ctx+0x30c {mode, offset m, b (.05; Street X .2; 999 none), tick},
  0x41d530 builds <= 20 route points. JustDrive changes lane when time-to-collision with a car ahead < 0.75 s.
  Line rebuilt when > 1.5 m off it (drag 3, drift 4.5, traffic 10); else look-ahead min(0.5v, 30 m) (drag 0.25v).
- Branches (0x413cc0): non-main segment taken with probability row 0x04 (0 .. 0.9 by skill); type-4
  segments 0.5 (0.333 with flag 0x8000000).
- Recovery: stuck window sampled every 1.5 s (10 samples); mean < 0.2 m/s, >= 50 m from the player and
  past the post-reset grace -> car+0x96c reset request. AccidentRecovery resets after 5 s. GetUnstuck
  reverses at -15 m/s (racers) / -6 m/s up to 5 s. Reset 0x6012b0 -> 0x5fbed0: back on the current route
  segment, speed max(0, v_fwd x road alignment) capped at 8 m/s.
- Belt mover: spline through the drive-target points; a = (0.899 x planned speed - v)/T (T = driver+0x234
  or 0.5 s), capped by the car's own full-throttle acceleration at v/catch-up (0x4038d0) and >= -20 m/s^2.

## 10. Traffic - summary of ai_re_traffic.md
- Traffic option [0x89e7a8] 0..3 (0 = none). Count 0x4fe9f0 = signed byte trackrec[+0xec + reverse +
  level*2] (cap 16; mostly 12), racers of kind 3 (slot 0x32+i) -> TrafficController + TrafficMover.
- Track +0x108 floats = probability of the first lane direction tried (0.5), not density.
- Density = spawn spacing 0x418240: class = segment[+0x2c + level*4], random in data+0xa8c+class*8
  ((240,300) (110,145) (80,95) (90,110) (70,100) (50,65) (50,70) (35,45) (20,25) m), walking roads up to
  500 m / 100 steps.
- Manager update 0x42f910 (racing only): timer 1 s (drag 0.3 s), recycles 1 car per tick (drag 16); a car is
  recycled at 250 m road distance from every human or when no slow (< 20 m/s) human within 50 m.
- Lanes 2..5 forward, others oncoming; max 24.5865 m/s (55 mph), initial speed = lane speed byte.
- TrafficMover 0x408820: kinematic bicycle, v += (gas*A(v) - brake*D(v))dt, A 3.58/2.68/2.01/1.34,
  D 6.71/13.41/20.12/26.82 m/s^2 over 0..24.59 m/s; switches to mover 3 (Mellow) on avoid / pull over / hit.
- Models: TAXI, TAXI02, 4DR_SEDAN(02), COUPE, PICKUP, SUV, MINIVAN, HATCHBACK(02), PANELVAN, PARCELVAN.

## Unresolved (consolidated)
- Input +0x210/+0x214 brake vs handbrake (conflict with physics1_re.md).
- Career Catch Up flag (probably 0 -> no rubber band in career).
- Race type 7 identity (probably free-roam outrun); speed-zone table [0x883db0].
- Skill row 0x10 and 0x54/0x58 meaning; graphs at chunk 0x370/0x3c8.
- Goal vt+0x4c extra limit; Execute of Avoid/Block/IJustGotHit; polyline spacing 0x41d530.
- DragDriver pedal/shift logic, DriftDriver beyond look-ahead, contact-avoidance steering add-on 0x421775.
- Traffic: road segment layout, intersections, cross traffic (chunk 0x34124 absent?).
