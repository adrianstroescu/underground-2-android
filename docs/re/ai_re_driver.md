# NFSU2 AI low-level driver (PsychoDriver) — speed2.exe static RE

Reference code: `ai_ref_driver.py` (same folder). Times are game ticks (1/4000 s, `[0x86518c]` = now; ×0.00025 → s).
Angles: u16 binary angles (65536 = 360°; ×9.58738e-5 → rad, ×0.00549316 → deg). Speeds m/s.
`car` = the car object (PsychoDriver +0x220). Car fields used: +0x60/64/68 position, +0x70/74 velocity xy,
+0x40c heading (u16), +0x42c signed forward speed, +0x430 |v|, +0x390+i wheel i on ground (byte),
+0x28 mover type, +0x24 controller kind, +0x1c race-progress object, +0x0c car type record (+0x88a = SUV flag),
+0x34 mover, +0x20 (mellow) steering scale at +0x270, +0x40 CarState.

## Class / object layout
- `CarDriver` (input object, ctor 0x423330, 0x220 B, vt 0x7849c0 / 0x7855d8). It is what `car+0x30` points to and
  what every mover reads. Defaults: +0x1e0 steer 0, +0x1e4 = 1, +0x1e8 = 1, pedals 0.
  - +0x10 DriveTarget (0x1cc B, copied by 0x401d40): byte n points (0..20), byte flag; points xyz at +0x10+16i
    (i<20); u16[20] at +0x150 and +0x178, u8[20] at +0x1a0, misc +0x1b4..+0x1c9.
  - **+0x1e0 u16 steering angle** (degrees×65536/360, wheel angle, ±max steer).
  - +0x1e4 int: direction/gear mode (vt slot 8 getter 0x405b20, setter 0x405b40). AI writes −1 (drive forward),
    0 (reverse), 1 (neutral, used in race state 3). The Mellow mover reads it via vt+0x20 (0x5a2a8e → mellow+0x134).
  - +0x1e8 int: manual gear request (vt slot 9 0x405b30), read by the full physics input stage (0x5abd3e, vt+0x24).
    PsychoDriver never writes it, it stays 1 (= "no request") → **AI uses the automatic gearbox** in full physics.
    (Writers 0x408077 / 0x408142 exist in controller code 0x408000.. — probably drag AI shifting; not decoded.)
  - **+0x20c throttle** (setter 0x5eb910), **+0x210** (setter 0x5eb900), **+0x214** (setter 0x5eb8f0).
    In the AI code +0x214 is the *brake* (gas−brake is computed as +0x20c − +0x214 at 0x41dc95; the speed brake
    output goes to 0x5eb8f0) and +0x210 is the *handbrake* (timed "stop" at 0x421ff3; hold mode). The Mellow mover
    reads +0x20c → +0x128 and +0x214 → +0x12c (0x5a2a7a). NOTE: physics1_re.md calls +0x210 "brake"; the
    other agent should reconcile — from the AI side +0x214 = foot brake, +0x210 = handbrake.
- `PsychoDriver` (ctor 0x425ab0, 0x460 B, vt 0x784eb0) : CarDriver. Normal AI racer and traffic driver
  (RaceCarController / TrafficController `+8`).
  - +0x220 car; +0x224/+0x228 steer target point x,y (slot 16 0x40ffd0 or 0x415f80).
  - +0x22c **throttle cap**: AI racers (racer kind 2) = `0x783a28[[0x89e804]]` = {0.93, 0.97, 1.0, 1.0}
    (index = GRaceParameters+0x64, probably the AI difficulty setting); others 1.0.
  - SetSpeed (slot 14, vt+0x38) **0x4054a0(speed, t, b, flag)** → +0x230 target speed (signed; <0 = reverse),
    +0x234 timer (only counted down to 0 each update, otherwise unused here), +0x238 b (unused here),
    +0x23c byte "pedals enabled" (0 → gas = brake = 0, coasting).
  - SetDriveTarget (slot 15, vt+0x3c) **0x405910(dt)**: if 0 ≤ n ≤ 20 copy DriveTarget into +0x10 (0x401d40).
  - +0x240 handbrake timer, +0x248 "mode 5" timer, +0x24c "hold" byte, +0x24e u16: only zeroed by ctor /
    snapshot in the AI code (no writer found) → effectively 0.
  - +0x244 byte "moving in wanted direction" (hysteresis), +0x250 reset request (see below), +0x258 tick,
    +0x25c nitrous latch, +0x260 previous target speed.
  - Trackers (class 0x4016b0, 0x84 B): +0x264 heading, +0x268 offset, +0x26c velocity-direction (window 5),
    +0x270 speed error (window `[0x7e8ea0]`=4), +0x274 wheel-slip-forward, +0x278 wheel-slip-lateral (window 4).
    (Snapshot names: Psycho Heading/Offset/Body/Velocity/WheelSlipForward/WheelSlipLateral Pid.)
  - +0x280 `AdaptivePIDControllerComplicated` (0x409b50, vt 0x78463c) = adaptive steering controller.
  - +0x334 `AdaptivePIDControllerSimple` (0x409f80, vt 0x784614) = adaptive speed controller.

## Tracker ("PID" history) 0x4016b0(nP, nID, k=30.0)
Three AverageWindows (0x579f20, sliding mean, add = 0x576970): W0 at +0 (n=nP), WI at +0x28 (n=nP),
WD at +0x50 (n=nID); +0x78 = k (30.0); +0x7c = current error e, +0x80 = previous error.
Window: +5 size, +6 count, +7 index, +8 sum, +0xc mean, +0x10 samples.
- update **0x576bb0(e, dt, noD, noI)**: prev = +0x7c; +0x80 = prev; +0x7c = e;
  `inc = noI ? 0 : 0.5·(e+prev)·dt`; `d = noD ? 0 : (e−prev)/dt`; W0.add(dt); WI.add(inc); WD.add(d).
- reset 0x57a010(e): +0x7c=0, +0x80=e, WI and WD reset (0x5769d0). 0x576c40(e): same but only WD.
- Derived terms used everywhere: **P = +0x7c**, **I = WI.sum·WI.count/(W0.sum·30)** (0 if count==0),
  **D = WD.mean (+0x5c)**.

## Update — PsychoDriver vt slot 1, 0x421030(dt)
1. `dt' = max(dt, 0.0001)`; read current steer angle (deg, wrapped ±180) from +0x1e0; signed speed
   `v = car+0x430`, negated if car+0x42c < 0.
2. **0x415f80 — steer target point (look-ahead).** L by mover type:
   - full physics (1), belt (4), mellow non-traffic (3, kind≠4): `L = G_LA(car+0x42c) · G_cls(class)` where
     G_LA = graph 0x7e9d94 over 0..160 (10 pts, step 17.78): 22.90 22.90 24.0 27.9 33.0 39.0 45.8 49.4 53.1 55.9 m;
     class = 0x5b5330(car record) (byte 0..3, car handling/perf class, computed by 0x5b4fc0) and G_cls =
     0x7e9df8 {1.3 → 1.1} over 0..3 for physics, 0x7e9de4 {0.6 → 1.1} otherwise (linear, 2 pts).
   - mellow traffic (kind 4): `L = max(0.9·v_fwd, 9)`; other movers (0, 2, 5): `L = max(0.55·v_fwd, 5)`
     (tables 0x7839b0 / 0x7839cc index 4 / 3).
   - If DriveTarget n < 1: point straight ahead from 0x403af0 (v_fwd ≥ 5) / 0x403ba0, and **+0x230 = +0x234 = 0**
     (stop). Else **0x415630(car, L)**: n==1 → point at distance L along car→p0; n>1 → project the car on the
     polyline (0x404ef0, uses car pos, velocity and forward speed), then walk segments accumulating distance
     (with a correction weight 1, or 7 when 0x402000() ≥ 5 — not fully decoded) until L, interpolate; past the
     last point extrapolate along the last segment. Result → +0x224/+0x228.
3. Wheels: for i in 0..3 with car+0x390+i set: `frontOnGround |= (i<2)`; `drivenOnGround |= 0x592870(i)`
   (0x592870 = wheel i is driven: bias carstate+0x1c0 < 0.2 → wheels 0,1; > 0.8 → 2,3; else all).
4. **Mode** m by mover type (car+0x28): 1 physics → m=1, nonPhysics=0; 3 mellow → m=0, nonPhysics=0;
   4 belt → m=3, nonPhysics=1; 2 traffic mover → m = (kind==4 ? 4 : 0), nonPhysics = (m==4); other → m=3, nonPhysics=1.
   Then `+0x248 -= dt; if +0x248 > 0: m=5`; `if +0x24c: m=6`; belt flag = (type==4).
5. **Errors 0x4054d0** (target point T):
   - `ang = atan2-angle(T − carPos)` (0x43e6d0, u16); **hErr = wrap(heading − ang)** (rad).
   - `w = G(0x81e7e4)(|v|)` = linear 0 at 5 m/s → 1 at 10 m/s; **vErr = (1−w)·hErr + w·wrap(velAngle − ang)**
     (velocity direction blended in above 5 m/s).
   - **oErr = wrap(angle(5.0, d))** where d = 0x404af0(DriveTarget, carPos) = distance from the car to the
     drive-target polyline (n==1: distance to the point) → atan(d/5) style offset angle.
6. Reset requests +0x250: 1 → vt+0x60 (0x405720: reset heading/offset/body trackers to current errors);
   2 → vt+0x68 (0x4056c0, derivative-only reset); then +0x250 = 0. (4 = set by AICarController 0x4208ca on the
   first frame of a new action: skip derivative this frame; 3 → reset speed tracker, see 13.)
7. If frontOnGround:
   - update heading tracker(hErr), offset tracker(oErr), body/vel-dir tracker(vErr) with noD = (+0x250==4);
     clear +0x250 if it was 4; +0x258 = 0.
   - Blend heading (b = +0x264.P) and vel-dir (a = +0x26c.P): if a·b < 0 → wa=0, wb=1; elif a²+b² < 1e-5 →
     wa=wb=0.5; else wa = a²/(a²+b²), wb = b²/(a²+b²).
     **eP = wa·a + wb·b; eI = wa·I(26c) + wb·I(264); eD = wa·D(26c) + wb·D(264)**.
     eI clamped to ±[0x81e5ac] = ±0.10472 (6°); eD clamped to ±1.
   - **Fixed-gain steering** (used when nonPhysics, belt, or m==5):
     `steer = −(K1[m]·eP + K2[m]·eD + K3[m]·eI)` (rad) with
     K1 0x78395c = {0.9, 1.0, 0.8, 0.7, 0.85, 0.5, 0}, K2 0x783994 = {0.3, 0.08, 0.18, 0, 0.12, 0, 0},
     K3 0x783978 = {0.2, 0, 0.1, 1.42, 0.79, 1.0, 0} indexed by m.
   - **Adaptive steering** (physics/mellow, m 0/1/6): inputs to the controller +0x280: e = (−eP, −eI, −eD)
     stored at +0x328/+0x32c/+0x330; adaptation period +0x2a0 = 0.2 s, thresholds +0x2bc..+0x2c4 = 0.01.
     Gain ranges (lo,hi per gain i at +0x2a4+8i/+0x2a8+8i) and adaptation rates (+0x2c8..+0x2d0):
     | case | rates | gP range | gI range | gD range |
     |---|---|---|---|---|
     | physics, SUV (rec+0x88a) | 5e-5 5e-5 5e-6 | 0.4–0.7 | 0–0.5 | 0–0.08 |
     | physics, other | 5e-5 5e-5 5e-6 | 0.15–0.5 | 0–0.5 | 0–0.08 |
     | not physics, Street X (0x401930) | 5e-6 5e-5 5e-6 | 0.275–0.3 | 0–0.3 | 0–0.06 |
     | not physics, other (also ctor default) | 5e-5 5e-5 5e-6 | 0.15–0.4 | 0–0.1 | 0.04–0.06 |
     (constants 0x78382c..0x7838dc.)
     Performance signal: `rate = sgn(eD·eP)·|eD|·57.2958` (deg/s, + when diverging);
     `perf = rate − G_ref(|eP|·57.2958)` with **G_ref = graph at 0x827a18** (bss, initialised by 0x77a750 from
     0x7e9c78, 10 pts): (0,0) (1,−0.5) (3,−1.5) (5,−2.5) (10,−5) (50,−5) (60,−5) (70,−5) (80,−5) (90,−5) —
     i.e. the wanted error closing rate is −0.5·error (deg/s), capped at −5 °/s. (A first evaluation at 0x42146e
     is discarded.) Gain adaptation **0x4182c0(perf, dt)**: `k = trunc(now_s/0.2) mod 3`; if |e_k| ≥ 0.01:
     `g_k = clamp(g_k + grad·dt, lo_k, hi_k)` with grad (0x411390, mode +0x1c = 0) =
     `−rate_k · perf · clamp(mean(dperf/dt)/mean(grad_k), ±1000)` (0x409ea0; windows +0x64+4k and +0x74,
     built by 0x579f80 with n = round(arg·60)). Then +0x70 = perf.
     **Gain schedule above 10 m/s** (car+0x42c ≥ 10, 0x42167d): the gains are overwritten every frame by speed
     graphs (0..160, 10 pts): gP = 0x7e9d44 {.328 .22 .148 .115 .09 .074 .057 .049 .043 .04},
     gI = 0x7e9d80 {.21 .244 .267 .29 .305 .321 .328 .336 .341 .344},
     gD = 0x7e9d6c (if 0x4018f0() and car+0x1c) else 0x7e9d58 — both {0 .0758 .0758 .0738 .0604 .047 .047 .0403 .0403 .0403}.
     So above 10 m/s the AI steering is effectively a speed-scheduled PID; adaptation only matters below 10 m/s.
     Output **0x409da0**: `steer = gP·e0 + gI·e1 + gD·e2` (errors clamped ±99999). If e2 > 999: steer ×= 999.
   - Then a "drift correction" loop over car+0x3c collision/contact list (byte count +0x18; 0x5a0820(i)):
     for a contact younger than 0.1 s and older than 0.6 s… (0x592c60/0x592c90/0x591fc0; other car slower than
     60 mph·0.447) adds `G(0x7e9e0c)(v)·deg2rad·min((age−0.6)/1, 1)` with sign — **not fully decoded**.
   - Final clamp of steer (rad) to ±0.785398 (45°).
   Else (no front wheel on the ground): if +0x258 unset → +0x258 = now; +0x250 = 0; if airborne ≥ 0.3 s:
   steer = −0x59fa10(carstate) (slip angle) and, for the Mellow mover, clamped to ±5°; else keep the current
   wheel angle. Gains are also clamped to [ (0.15|0.4 SUV), ≥0, ≤(0.05|0.1 SUV) ] (0x421a47.., physics only).
8. `speedErr = |+0x230| − |v|` (stored); if +0x250==3 → vt+0x70 (reset speed tracker to speedErr), +0x250=0.
9. Speed tracker +0x270 update(speedErr, dt, 0, 0); **0x4057c0**: slip trackers: sums over the 4 wheels of
   CarState+0x374+8i (longitudinal slip) and |+0x378+8i| (lateral), each clamped ±4 (0x7839f4/f8) and only
   added when inside the clamp; lateral sign flipped when CarState+0x3ec < 0; update +0x274 (forward), +0x278 (lateral).
10. **Pedals** (gas, brake, gear mode ebp=−1):
    - no driven wheel on the ground: gas = +0x22c (throttle cap), brake = 0, reset speed tracker (vt+0x6c).
    - |target| < 2: brake = 1, gas = 0.
    - target > 0: +0x244 = 0 when v < −7, = 1 when v > −2 (hysteresis). If 0 (rolling backwards): vt+0x6c,
      steer = −steer, brake = 0.9, gas = 0. Else → speed controller.
    - target < 0 (reverse): +0x244 = 0 when v > 7, 1 when v < 2. If 0: brake 0.9, gas 0. Else steer = −steer,
      **gear mode 0 (reverse)** → speed controller.
    - +0x24c (hold) → speed controller too.
11. **Speed controller 0x41d8d0(&gas, &brake, m, dt)**:
    - Not hold and |target| < 5: bang-bang: | |t| − v | ≤ 1 → 0/0; |t| > v → gas 1; else brake 1.
    - Adaptive path (not drag; not drift-with-progress; |t| < 30 m/s; racer kind ≠ 3): controller +0x334 with
      period 0.2, ranges gP [−0.3,−0.2], gI [0,0], gD [−0.6,0], thresholds 0.01, rates 5e-6, D clamp ±10;
      reference = graph 0x81e7f4(v − |t|) {(−20,100) (−10,10) (−5,7) (−2,3) (0,0) (2,0) (10,−3) (20,−10) (30,−100)};
      adapt step 0x4183f0 (flag = |t − previous t| > 1); **above 10 m/s the gains are fixed to (−1, 0, 0)**;
      u = 0x409da0. min = −1, max = 1.
    - Otherwise PID on the speed tracker: P = speedErr, I as above, D = clamp(mean D, ±10).
      If `|t| + 2 > v` (not too fast): gains acc (0x7e8fcc/0x7e8fe8/0x7e9004)[m]; range [0, +0x22c].
      Else: gains brk (0x7e9020/0x7e903c/0x7e9058)[m]; range [−1, 0].
      | m | 0 | 1 | 2 | 3 | 4 | 5 | 6 |
      |---|---|---|---|---|---|---|---|
      | accP | .1 | .1 | .1 | .1 | .005 | .1 | .1 |
      | accI | .02 | .02 | .02 | .02 | .005 | .02 | .02 |
      | accD | .02 | .02 | .01 | .01 | .01 | .01 | .01 |
      | brkP | .1 | .1 | .1 | .1 | .005 | .1 | .1 |
      | brkI | .02 | .02 | .02 | .02 | 0 | .02 | .02 |
      | brkD | .02 | .02 | .01 | .01 | .008 | .01 | .01 |
      `u = kP·P + kI·I + kD·D`.
    - **Rate limit**: `Δ = clamp(u, ±G(0x7e9d10)(v)·dt)` with graph 0..160: 30 15 7.5 6 6 6 6 6 6 6 (per s);
      `x = clamp((gas_prev − brake_prev) + Δ, min, max)` (pedal command is an integrator).
    - Traction limit `lim = 1`: if 0x410040() (|slip angle| > 5° or race clock < 4 s…, partly decoded) use the
      forward-slip tracker +0x274: `lim = 1 + (0.02·P + 0.02·I + 0.005·D)` when x ≥ 0, `1 + (−0.2·P −0.02·I −0.005·D)`
      when braking; lim ≥ 0. If |slip-lateral P| > 999 or |steer e2| > 999 → vt+0x44, lim = 0.
      In drift races with progress: `lim *= 0.5·(1 + f(2·slip + π/2))` (f = 0x43c350, sin-type table).
    - x < 0 → brake = min(|x|, lim), gas = 0; x ≥ 0 → gas = min(x, lim), brake = 0.
    - Final: gas clamped [0, +0x22c], brake [0, 1].
12. **Steering output and rate limit**: max wheel angle `S = 45°` in drift races, else `G(0x7e9e0c)(v)` =
    45 30 5.5 5 3 2 2 2 2 2 (deg, over 0..160 m/s), ≥0. `tgt = clamp(steer/(S·π/180), ±1)`, `cur = curDeg/S`.
    Physics (nonPhysics = 0): `rate = G(0x7e9e34)(|eP|deg) (=1) · G(0x7e9e20)(v) {4 3 2 1 .45 .14 .1 .1 .1 .1}`
    ×2 when moving toward centre; `Δ = clamp(tgt − cur, ±rate·dt)`. Non-physics: `Δ = clamp(tgt − cur, ±100·dt)`.
    `angleDeg = (cur + Δ)·S` (×mellow scale car+0x20→+0x270 when nonPhysics).
13. `+0x234 = max(+0x234 − dt, 0)`.
14. Pedal output: race state `[[0x890118]] == 3` (pre-start): brake(+0x214) = 1, gear mode 1, gas = 1,
    car 0x5ecd30(1) (rev at the line). Else if +0x23c: gas(+0x20c) = gas, +0x214 = brake; else both 0.
15. **Nitrous**: if CarState nitrous level (0x59fa30) > 0 → latch +0x25c. While latched and not drag:
    if target ≥ previous target (+0x260): gas = +0x22c, brake = 0; else unlatch. (The driver does not fire the
    nitrous; activation is in AICarController 0x42ad2d / 0x42b0ef via vt+0xa0/+0xb4/+0xa8, 0x592500 = stop.)
    +0x260 = +0x230.
16. Handbrake timer: `+0x240 -= dt`; if > 0 and |v| < 8.94 m/s: brake 0, handbrake(+0x210) = 1, gas 0;
    else handbrake = 0.
17. `+0x1e0 = (int)(angleDeg·65536)/360`; `+0x1e4 = gear mode` (−1 / 0 / 1).
18. Hold (+0x24c): handbrake 1, brake 1, gas 0, car+0x4d8 = 1.4 (0x5ebbe0, also mover+0x6c in physics).

## Drag / drift drivers (brief)
- **DragDriver** 0x427f20 (vt 0x785118, 0x7a0 B) = PsychoDriver + embedded `RealDriver` (0x425fc0, vt 0x784fc8)
  at +0x470. Update 0x4220a0: PsychoDriver update (lane steering), then +0x470 update (vt+4), then 0x406460
  at +0x690 (force-feedback only), then **gas/brake/handbrake copied from the embedded driver's
  +0x20c/+0x210/+0x214 (=+0x67c/+0x680/+0x684)**. Pedal and shift logic of that embedded driver not decoded.
- Drag launch delay (RaceCarController 0x42c0eb): `+0x75c = rand(row[0x28]−row[0x24]) + row[0x24] + 0.001`,
  row = AI skill row (GLOBALB 0x34b00 +0x524+skill·0x68): skill 0..9 row 0x24/0x28 = (−5,−4.3) (−4.6,−4) (−4.2,−3.5)
  (−3.5,−2.5) (−2.9,−2.2) (−2.9,−1.8) (−2.6,−1.3) (−2.2,−1.1) (−1.6,−0.8) (−1.5,−0.8) s; compared to the race
  clock (−x).
- Drag shift-rpm errors (PhysicsMover ctor 0x5b6204, kinds 2/3/4, drag only): drivetrain +0xe0/+0xdc/+0xd0/+0xcc =
  row 0x34/0x38/0x3c/0x40 (skill 0: −900,300,−900,300 … skill 9: −110,15,−110,15 rpm), +0xd4 = row 0x2c (0),
  +0xd8 = row 0x30 (0.01); drivetrain +0xe8 (shift speed) = 0x7a19f4[[0x89e7f4]] = {0.4, 0.8, 1.0} by difficulty
  (skipped when FE mode [0x850078] equals the compared value — split-screen case, not checked).
- **DriftDriver** 0x425e40 (vt 0x784f40, 0x250 B), update 0x416120: look-ahead `L = clamp(15 + 0.75·|d| + 0.04·v,
  15, 30)` (d = distance to the drive-target polyline), then a second distance `clamp(0.08·v + …, 8, 12)`;
  rest not decoded.

## Unresolved
- Exact 0x415630 walk weighting (the 1 / 7 factor via 0x402000) and 0x404ef0 projection.
- Contact-avoidance steering add-on (0x421775 loop) details.
- 0x410040 full condition; f = 0x43c350 (sin/cos table) argument meaning in drift.
- 0x4183f0 argument mapping for the speed adaptive controller (adaptation is only active below 10 m/s / |t| < 30).
- Who sets +0x240/+0x248/+0x24c (no writer found; probably never set in races).
- DragDriver embedded RealDriver pedal/shift logic; DriftDriver beyond look-ahead.
