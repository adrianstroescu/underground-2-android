# NFSU2 player-car physics, part 2: rigid body, suspension, steering, tyres, aero, assists (speed2.exe)

Static analysis only. Reference sim: `physics2_ref.py` (same folder). Lifter used: `lift.py START [END]`
(x87 symbolic annotator; also `scanw.py`, `grepw.py`, `curve.py`).
Units as in part 1: t, kN, kN·m, m, s, rad. Coordinates: body x forward, y left, z up; world z up.
Matrices are row-major, **row-vector convention** (`v_world = v_body · R`, rows of `R` = body axes in world):
- `0x441000 / 0x43c9c0(out, M, v)` = `v·M` (adds row 3),
- `0x43c990(out, A, B)` = **B·A** (it calls `0x4013f0(out, B, A)` = B·A),
- `0x43c9d0` = transpose, `0x43d1e0(out, a, b)` = a × b, `0x43d0c0(out, a, b, s)` = a + b·s,
- `0x43c780` = identity, `0x43cf10` = normalise, `0x43c7c0` = 4×4 inverse,
- `0x43c570(x, y)` = **fixed-point atan2(y, x)**, 16-bit (65536 = 2π), byte table 0x7eacb0 (257 entries,
  index `trunc(256·min/max)`, `<<5`, octant logic). The tyre code converts with 9.58738e-5 and wraps to (−π, π].
- `0x43c350(x)` = sin via a 16-bit table (`ftol(x·10430.4)`); `+0x4000` = cos.
- `0x5764d0` = Curve (n, x0, x_end, inv, y*), `i = trunc((x−x0)·inv)`, clamp to ends, lerp.
- `0x576790` = XY curve {pairs*, n}, clamped, linear.
- `0x75bed4` = ftol (truncation).

## 1. Component map (PhysicsMover 0x7a25e0, built by 0x5b5cc0)
Ctor args: `(ps, carcfg = car+0xc, input = car+0x30, car)` (from factory 0x5f46a0, type 1).
Component list (mover+8), insertion order = derivative/post order:
`rb, then for i = 0..3: Wheel_i, Suspension_i, then engine, drivetrain`.

| mover | object | vtable | deriv (vt+0x18) | post (vt+0x1c) | state (size) |
|---|---|---|---|---|---|
| +0x20 | RigidBody (0x5a7f30) | 0x7a20c4 | 0x59c1e0 | 0x59c5f0 | rb+0x20, 32 floats |
| +0x24 | Steering (not a component) | 0x7a2084 | – | – | update 0x59def0 |
| +0x28+4i | Suspension_i (0x5a86c0) | 0x7a210c | 0x59e130 (returns 0,0) | 0x5a8790 | +0x1c, 2 floats (x, rate) |
| +0x38+4i | Wheel_i (0x5a8160) | 0x7a20e8 | 0x59d350 | 0x59d030 | +0x24, 2 floats (angle, ω) |
| +0x48 / +0x4c | Engine / DriveTrain | part 1 | | | |
| +0x50 | Aero (plain struct, 0x34 bytes) | – | – | – | – |

Record layout used here (`ps+X = record+(X+0x110)`), all values Skyline:
- rec 0x110 (ps+0) **CG offset** in model space (0.22, 0, 0.22) → rb+0x1f0.
- rec 0x120+0x30i wheel geometry: +0 pos (x,y,z) (FL 1.341, 0.74, 0.146; FR y<0; [2] = RR, [3] = RL),
  +0x10 radius 0.3266, +0x14 0.245 (width, unused here), **+0x18 int wheel index 0..3** (≥2 = rear),
  +0x1c/+0x20 (0.878/0.963, sign of y) not used by the physics paths read.
- rec 0x1e0 (front) / 0x200 (rear) axle tyre data: +0 **max slip angle in degrees** (12), +4 lateral-curve load
  divisor (1.0), +8 lateral-force scale (1.0), +0xc static μ (1.85/1.9), +0x10 dynamic μ (1.7/1.75).
  **Correction to part 1:** +4/+8 are NOT brake/handbrake.
- rec 0x220 mass (1.5), 0x224..0x22c length/width/height, **rec 0x230 4×4 inertia** (diag 0.624, 2.816, 2.984, 1),
  **rec 0x270 yaw-inertia scale** (2.0).
- rec 0x280 (front) / 0x2a0 (rear) suspension: +0 progressive factor (5.7), +4 spring k (40 kN/m),
  +8 bump damping (6.0 kN·s/m), +0xc rebound damping (5.4), +0x10 anti-roll k (38 kN/m),
  +0x14 max compression (0.14 m), +0x18 max extension (−0.13 m), +0x1c (0.06, not referenced in code read).
- rec 0x370 aero: +0 drag k (0.00043), +4 front downforce coeff (0.16), +8 rear (0.17), +0xc 0.01 (preload term).
- **rec 0x380** (wheel+0x1c): +0 **steering rate scale** (1.1, used by the keyboard steering 0x416bb0),
  +4 **brake** (0.485), +8 **handbrake** (0.51), +0xc **brake bias to the rear** (0.52).
- rec 0x390 / 0x394 rear "drift-recovery grip" (0, 2.5); 0x398 yaw-torque clamp for car-state+0x424 == 0 (17);
  0x39c yaw-torque clamp for human (0 = none); 0x3a0 speed limiter mph.
- Surface table 0x800f00 stride 0x98: +4 static-μ mult, +8 dynamic-μ mult, +0xc rolling loss, +0x10 aero
  drag mult (all 1.0 in the exe; surface id = wheel+0x114).

## 2. Frame and substep order (adds to part 1)
Per frame `0x5b72d0(dt)`: `0x5ab990` static preload → `0x5a49a0` modes → `0x5abbf0` input (steering
`0x59def0`, aero+0x10 = throttle, **aero+0x14 = wheelsOnGround·0.25**) → `0x5a4d80(dt)` brake/grip →
`0x5b6300` assists → n substeps of `{0x5aa480(h): derivatives, Euler, posts (incl. 0x5abb00), force stage
0x5b0520}` + orthonormalise `0x5bb0d0`.
Force stage 0x5b0520 (this = mover+8):
1. if drag (mover+0x7c) and track type 2: engine+0x80 = 1.5
2. rb vt+0x14 (no-op), **aero 0x59e620**
3. **tyre loads**: `wheel.0x59d160(susp.onGround, susp.F)` ×4
4. **0x5abdf0**: clutch torque (adds drive torque to wheel+0x108) → tyres 0x5a8280 ×4 → 5-pass solve
   → friction circle 0x59d600 ×4 → engine sync
5. for each susp with onGround: **AddForce(wheel+0x130, wheel+0x140 − rb.pos)** (unless [0x86b36c])
6. **0x5a5b60** angular damping; car vt (0x5ecf70); drift helper **0x5ac260** if drift-type race;
   hydraulics 0x5a51b0; drag lane spring **0x5a5c20** if mover+0x7c.

## 3. Rigid body (rb = mover+0x20)
Fields: +0x14 data ptr (= rec 0x220: mass, +0x10 inertia, +0x50 yaw scale); state block rb+0x20:
`+0x20 pos, +0x30..0x6f R (rows fwd, left, up, row3 = 0001), +0x70 P (momentum), +0x80 L (angular
momentum), +0x90 rest-damping level`; derived: +0xa0 v, +0xb0 accel in g (F·invm/9.8, previous force),
+0xc0 ω, +0xd0..0x10f world transform (R + pos); +0x110 F accum, +0x120 T accum, +0x130/+0x140 last F/T;
+0x150 rest-damping on, +0x154 rot-damping on, +0x158 use scaled yaw inertia, +0x15c car flag (×1.03 limiter),
+0x160 gravity, +0x164 mass, +0x168 1/m, +0x170 I⁻¹, +0x1b0 I⁻¹ with Izz·rec0x270, +0x1f0 CG offset,
+0x200 yaw-torque clamp, +0x204 limiter mph, +0x208 limiter scale (1).

- **Mass/inertia 0x59b940** (ctor and every frame from 0x5ab990): m = rec 0x220, I⁻¹ = inv(rec 0x230),
  I'⁻¹ = inv(rec 0x230 with Izz·rec 0x270). rb+0x158 = (car+0x424 != 0) (set in 0x5a49a0; when it drops to 0,
  L.z /= scale). So **the player's yaw inertia is doubled** (Skyline 2.98→5.97 t·m²).
- **Post 0x59c5f0**: `v = P/m`; if |v| > 200 (0x7fc6ec) and m > 0.7 → P rescaled; `acc_g = F·(1/m)/9.8`;
  `I_w⁻¹ = Rᵀ·I⁻¹·R` (via the B·A convention), `ω = L·I_w⁻¹`, |ω| ≤ 250; rest level ≥ 0; copy world transform;
  save F/T, clear; **gravity: AddForce((0,0,−m·g), 0)** with g = rb+0x160.
- **Derivative 0x59c1e0**: `dpos = v; d(row_i) = ω × row_i (0x59c120); dP = F; dL = T; d(rest) = ±`, plus:
  - speed limiter (part 1);
  - rb+0x154: `T −= c(|ω|)·L`, c = Curve 0x7fc92c {0.05,0.05,0.25,0.7,2,5} over |ω| 0..10;
  - rest level: `d/dt = +0.1 if C(max(|v|/4, |ω|/2)) > level else −10`, C = 0x7fc940 {1.5,1,.6,.22,.06,.03,.015,0}
    over 0..1; if rb+0x150: `T −= 2·level·L; F −= 8·level·P` (only matters near standstill);
  - if rb+0x200 ≠ 0: clamp T.z (yaw torque) to ±rb+0x200.
- **Integrate**: explicit Euler on all 32 floats, then **0x5bb0d0**: `r0 = norm(r0); r2 = norm(r0 × r1); r1 = r2 × r0`.
- **AddForce 0x59bb00(F, r)**: F clamped to |F| ≤ 1200 (0x7a1ad8); torque `r × F_unclamped` clamped to ≤ 100
  (0x7a1adc). r = world offset from the CG.
- **ApplyImpulse 0x59bc60(J, r)**: |J| ≤ 1000; P += J; L += r × J (collision/scripted impulses).
- Gravity rb+0x160 = 9.8 normally, **11.0 (0x7fc084)** in car state +0x38 == 1 (0x5a49a0, also 0x5a5e20).

## 4. Suspension (S = mover+0x28+4i, 0x5a86c0)
Fields: +0x14 data (rec 0x280/0x2a0), +0x18 partner (FL↔FR, RL↔RR), +0x1c x (compression, + = compressed),
+0x20 rate, +0x24 force out, +0x30 wheel centre (body), +0x80 onGround, +0x88 preload, +0x90 hardpoint (body),
+0xa0 wheel, +0xa4 rb, +0xa8 steering (front only), +0xac index.
- **Static preload 0x5ab990 → 0x59e0c0(load, rideAdj)** (every frame):
  `S.90 = wheelpos − CG(rec 0x110); S.98 −= rideAdj (CarState+0x3b8+4i, tuning)`;
  `f = (x_FL − cg_x)/(x_FL − x_RR) · 0.5` (cg_x = rb+0x1f0 if [0x7fc06c] (=1) else 0);
  `W = m·9.8 + (Cf+Cr)·aero+0x30·max(car+0x42c, 0)·rec 0x37c`; front wheels `S.88 = W·(0.5−f)`, rear `W·f`.
  → the spring is preloaded with the static weight, so x ≈ 0 at rest (S.90 = wheel centre at rest).
  Also sets wheel inertia +0x2c = 0.01.
- **Post 0x5a8790** (per substep, after the rb post):
  1. `p = S.90·R + pos`; `t = wheel.0x59d250(&n, &surface, p, R)` = signed distance from p to the ground
     along body up: `t = dot(G − p, n) / dot(up, n)`; t = −100 if dot(up,n) < 0.001.
     **Ground query** `0x5e7e70(world, p, &surface, &normal, &found)` returns the ground height under p
     (+ wheel+0x18c offset; −10000 if not found); `0x5d3ca0(xy, surface)` adds a bump/height noise
     (stored wheel+0x168); point G = (p.x, p.y, h).
  2. `x = t + radius`; `x > maxC → x = maxC, onGround`; `x < maxExt → x = maxExt, NOT onGround`; else onGround.
  3. `rate = (x − x_prev)/h` (h = [0x7fc800], if h > 0.0001).
  4. wheel local centre `S.30 = S.90 + (0,0,x)`; wheel local matrix = steer_i (front) with row3 = S.30
     (camber rotation only if wheel+0x174 ≠ 0); `wheel+0xb0 = local`, `wheel+0x70 = local·rbWorld`;
     wheel point velocity `wheel+0xf0 = v + ω × (S.30·R)`.
  5. **Spring force** if onGround:
     `F = k·x·(1 + prog·max(x,0)) + (rate > 0 ? bump : rebound)·rate + S.88 + (x − x_partner)·arb`, F < 0 → 0;
     not on ground → 0. The partner's x is the value current at that moment (FR sees the new FL x, FL the
     previous FR x).
- **Tyre load 0x59d160(onGround, F)**: wheel+0x11c = onGround; contact point `+0x140 = centre − r·n`;
  if F > 0: `load (+0x164) = max(1 − 4·(lat·n)², 0.3)·F` (lat = wheel lateral axis +0x80; computed as
  `4·((lat×n)×lat)·n − 3`), else 0.
- 0x5abb00 (container post): bottomed-out suspensions (x == maxC) add `0x59e1a0()` (hardpoint vertical
  speed) to CarState+0x330+4i (effects only); then 0x5a5e20 reset/respawn handler.
- There is no separate anti-roll or weight-transfer code: load transfer is purely the rigid body's response to
  the spring forces, which act through the contact point inside the tyre force vector (§6.6).

## 5. Steering
- **Steering object 0x59def0(angle16)** (mover+0x24, data = ps): θ = angle16·2π/65536, wrapped; Ackermann
  with L = x_FL − x_RR, W = y_FL − y_FR: `θ_out = L·|θ| / (|θ|·W + L)`; θ > 0 (left): FL = θ, FR = θ_out;
  θ < 0: FL = −θ_out, FR = θ. Matrices rows (cos a, sin a, 0), (−sin a, cos a, 0) at +0x10 (FL) / +0x50 (FR).
- **input+0x1e0** (16-bit) is written by the driver objects: `0x405990(target, step)` clamps the change to
  ±2π·step per call and the result to **±π/4 (45°)**. The human driver (vt 0x784fc8, update 0x41e2c0) uses
  `0x41dfc0(stick, dt, car)` → keyboard/pad model **0x416bb0**, wheel model 0x4061c0 (when 0x5c1000(car)),
  debug 0x406a40; then 0x406460 (force feedback) and stores the angle.
  0x416bb0 (partly resolved): stick shaped by Curve 0x7e9e60 (normal) / 0x7e9e9c (drag/drift races),
  21 points on [−1,1] (e.g. 0.5 → 0.148); current angle in degrees; slew rate ∝ rec 0x380 (1.1);
  target from **0x405dc0**: max lock vs **car+0x42c (speed, mph, 0..160)** = Curve 0x7e9ed8
  {40, 20, 10, 5, 3.5, 3.1, 2.9, 2.9, 2.9, 2.9}° (≈4.4° at 60 mph) × Curve 0x7e9f14 {1 … 0.32} blended with
  throttle/brake/handbrake, × Curve 0x7e9f00 of |stick| {1,1,1.1,1.2,1.25,1.35}; counter-steer allowance from the
  body slip angle (0x59fa10) with Curves 0x7e9eb0 / 0x7e9ec4 (drift) up to 45°; final clamp ±45°.
  The exact blend in 0x405dc0/0x4060e0 is **not fully resolved**.
- No speed-sensitive reduction exists in the physics steering itself; it is all in the controller layer.

## 6. Tyres (Wheel W = mover+0x38+4i)
Fields: +0x14 geometry, +0x18 axle data, +0x1c rec 0x380 block, +0x20 CarState, +0x24 angle, +0x28 ω,
+0x2c I (0.01), +0x70 world matrix (rows fwd/lat/up, +0xa0 = centre), +0xb0 local matrix, +0xf0 point velocity,
+0x100 last s, +0x104 last vy, +0x108 **net torque** (drive + brakes), +0x10c **locked**, +0x110 rolling-direction
state, +0x114 surface, +0x118 slip s, +0x11c onGround, +0x120 smoothed torque, +0x124 **Fx**, +0x128 **Fy**,
+0x130 world force, +0x140 contact point, +0x150 ground normal, +0x160 **sliding**, +0x164 **load**,
+0x170 grip usage, +0x174 flat-tyre flag (car+0x95c bit i), +0x178 brake input, +0x17c handbrake input,
+0x180 grip mult, +0x184 per-car grip (0x599170), +0x188 drift rear grip, +0x190/+0x194 long/lat mults (1).

### 6.1 Brake/grip inputs 0x5a4d80(dt) (per frame)
- `g0 = mover+0x6c` (car+0x4d8, per-car) × 0.85 if CarState+0x424 == 0 (except drift-type races);
  `g0 ×= 1 − 0.025·0x5ed5d0(car)` (car damage/wear value, unresolved).
- Handbrake ramp mover+0x70: input+0x210 ≥ 0.5 → −20·dt down to 0.5; else +dt up to 1.
- Brake +0x178: rear wheels with throttle > 0.8, brake > 0.5 and < 10 mph: `mph·0.05` (burnout);
  else `2·(rear ? rec38c : 1 − rec38c)·brake`.
- Handbrake +0x17c: `h = {0,0,1,1}[i]·input+0x210`; human: `h > 0.2 and |bodySlip| > 0.3 and mph < 80 → h + 0.5`;
  CarState+0x424 == 0: `h > 0.2 → h + 0.5`.
- Grip +0x180: front `g0`; rear in normal races `g0·(1 + (Y − 1)·hbRamp)`,
  **Y = 0x5990c0** = `1 + |fwd.z| + min(|bodySlip|·c·|v|/30, 0.35)` (but just `1 + |fwd.z|` when handbrake > 0.5
  and |bodySlip| < 20°); `c = rec390 + rec394` (2.5) for CarState+0x424 == 0, else `rec390 + rec394·E(t)`,
  E = XY curve 0x89cce4 {(0,0),(0.4,1),(1,1),(2,0)} of the seconds since timestamp (car+0x3c)+0x140 (< 4 s).
  Drift-type races and [0x89e7e2]/[0x890100]/[0x890110]: rear = g0.
  Then × NOS {1.0, 1.4, 1.7}[level] (0x7a17a4); staging (mover+0x84): driven wheels with throttle == 1 → 0.
- Body slip angle 0x591d90 = heading16 (CarState+0x3cc) − atan2(v.y, v.x), wrapped; 0 below 1 m/s (0x59fa10).

### 6.2 Tyre force 0x5a8280(0, absOff = (car+0x3c)+0x18)
```
Tb = rec384·brake·4 ; Th = rec388·hb·10 ; both · (ω > 0 ? −1 : +1); if !locked: T(+0x108) += both
(vx, vy) = wheel velocity in wheel axes (rows of +0x70)
β = atan16(vx, vy) (+π if vx < 0, wrapped)           s = r·ω − vx
ABS: if s < −0.5 and T < 0 and Th == 0 and absOff == 0:  T *= 0.5/(−s)
if hypot(s, vy) == 0 or (vx == 0 and vy == 0): STATIC
μ = surf[+8]·W184·W180·rec(+0x10 dyn μ); player (car+0x424==1): μ·= rearFactor (0x59d490)
k = load·μ/hypot(s, vy)
lock test 0x59d400(arg = load/hypot(vx,vy)·μ·|vx|):
    Tbk = (rec388·hb·10 + rec384·brake·4)·1.2
    if Tbk > 0.1|ω| + r·arg:  ω = 0; locked = (Tbk > 1)   else locked = 0
if |s| > 0.5 or |β| > rec(+0, DEGREES vs radians: never true for 12) or locked:   SLIDING
    sliding = 1; Fx = s·W190·k; Fy = −k·W194·vy
    if CarState+0x3ec·2.237 < 1 mph and μ > 0.1: Fx /= μ; Fy /= μ
else STATIC:
    sliding = 0; Fx = W190·T/r
    Fy = latCurve(load, |β|)·W194·(β > 0 ? −1 : +1)·(vx > 0 ? +1 : −1)
```
rearFactor 0x59d490 (only rear, not locked, ω ≥ 0): `0.85 + 0.15·clamp((speed − 13.41)·0.11185, 0, 1)`.

**Lateral curve 0x59d510** ([0x7fc7f0] = 1, the active path):
```
x = |β|·57.2958 / maxSlipDeg · 6         (6 at the max slip angle)
Lr = 0.8·load / axle(+4)
k = trunc(x);  F = k ≥ 6 ? C6(Lr) : Ck(Lr) + (x − k)(Ck+1(Lr) − Ck(Lr))
Fy_mag = F · axle(+8) · 2.5
```
Ck = Curves 0x7fc968 + 0x14k, x0 = 0, inv 0.5 (load 0..10 kN, 6 points): C0 = 0, C1 {0,1.2,2.3,3,3,2.8},
C2 {0,1.7,3.2,4.3,5.1,5.2}, C3 {0,1.8,3.5,4.9,5.8,6.1}, C4 {0,1.83,3.6,5,5.96,6.4},
C5 {0,1.86,3.7,5.1,6.13,6.7}, C6 {0,1.9,3.8,5.2,6.3,7.1}. (The other path, Curve 0x7fc954 · load, is dead.)
Skyline front at 3.7 kN: 4.3 kN at 2°, saturates against the friction circle (6.8 kN) at ≈3°.

### 6.3 Five-pass static solve (0x5abdf0)
```
Fb = Rᵀ·rb.F      (gravity + aero so far, body frame)
repeat 5:
  for each wheel: f_i = (row1.x·Fy + row0.x·Fx,  row1.y·Fy + row0.y·Fx)   (row = wheel LOCAL rows, +0xb0/+0xc0)
  a = (Fb.xy + Σf_i)/m ;  α = Σ(f_i.y·hp_i.x − f_i.x·hp_i.y) · I'⁻¹zz      (hp = suspension hardpoint S.90)
  for each wheel with !sliding and !locked:
     a_long = (a.x − α·hp.y)·row0.x + (a.y + α·hp.x)·row1.x       ← exe uses row1.x (−sin δ) where row0.y (+sin δ)
     Fx += ((T − r·Fx)/I_w · r − a_long) / r · I_w                  i.e. Fx += (T − r·Fx) − I_w·a_long/r
  0x594da0 locked-clutch constraint (part 1)
```
Converges to `Fx = (T − I_w·a_long/r)/r` (wheel acceleration matches the contact-point acceleration).
The row1.x / row0.y mix-up only affects steered front wheels in the static state (sign of the lateral term).

### 6.4 Friction circle 0x59d600(i)
```
boost = T > 0 and !locked:  Fx *= 1.5
Fmax = surf[+4]·axle(+0xc static μ)·W188·W184·W180·load ; player rear: · rearFactor
if |F| > Fmax and |F| > 0.001:  (Fx, Fy) *= Fmax/|F|      (boosted Fx is kept)
elif boost: Fx /= 1.5
W170 = min(|F|/Fmax, 1) (1 if sliding/locked, 0 if airborne)  — grip usage for effects
β' = slip angle again; if |vx| > 1: Fx −= 0.15·sin(β')·Fy                (induced drag)
world force (+0x130) = Fx·row0 + Fy·row1 + load·row2     ([0x7fc7fc] = 1: wheel world rows)
```
### 6.5 Wheel spin (0x59d350 / 0x59d030)
`dω = (½(T − r·Fx + T120_prev) − 0.002·surf[+0xc]·ω − 0.1·W174·ω)/I_w` (T120 = that average, stored);
locked → 0; if !sliding and h > 0.001: `dω −= s/(r·h)` (snaps r·ω to vx when gripping).
Post: clear Fx, Fy, T, s; angle wrapped ±628; direction hysteresis +0x110 (0 fwd, 1 stopped, 2 back):
a wheel rolling forwards cannot flip to negative ω in one step (clamped to 0) and vice versa.
### 6.6 Force application
Each on-ground wheel: AddForce(+0x130, contact − rb.pos). The vertical part `load·up` is the suspension force
(with the camber/tilt factor); the horizontal parts are the tyre forces. Airborne wheels: load = 0 → Fmax = 0.

## 7. Aero 0x59e620 (corrects part 1)
aero: +0 data (rec 0x370), +4 rb, +8 = +0xc = 1, +0x10 throttle, **+0x14 = wheels on ground / 4**, +0x30 on/off,
+0x34 = mean surface drag (+0x10) of the on-ground wheels (0x5a49a0; 1 if none).
```
sp = |v|
drag  F = −v·sp·k·(1 + (1 − thr)) · A.34·A.30           at r = (0,0, A.14 ≥ 0.5 ? −0.1·(1−thr) : 0)·R
u = A.14 ≥ 0.5 ? 1 : max(up.z, 0) ;  c = max(v̂·fwd, 0)
f = max(c², 0.4)    (drift-type races: c⁴)
if A.14 == 0:  Ff = 0.4·(Cf+Cr)·u·f·sp ; Fr = 0.41·(Cf+Cr)·u·f·sp
else:          Ff = Cf·u·f·sp ;          Fr = Cr·u·f·sp
AddForce((0,0,−Ff·A.30)·R, (+1,0,0)·R) ; AddForce((0,0,−Fr·A.30)·R, (−1,0,0)·R)
```
**Downforce is linear in speed** (Skyline: 0.33 kN per m/s → ≈ its own weight at 100 mph), applied 1 m ahead
of / behind the CG. Car state +0x38 == 1: A.30 = 0 (no aero), gravity 11. Staging (mover+0x84): A.30 = 0.

## 8. Modes, assists and stability helpers
- **0x5a49a0** (per frame): surface feedback to the input (0x5eb910); car+0x38 ∈ {0, 2}: rb.150 = rb.154 = 1,
  g = 9.8, aero on; car+0x38 == 1: no damping, g = 11, aero off; drag-type ([[0x890118]] == 4): no rest damping when
  throttle > 0.3 or (4 wheels down and |CarState+0x58| ≤ 0.15); `0x599170` per-car grip → wheel+0x184;
  yaw clamp rb.200 = 1e-14 (staging) / rec 0x398 (CarState+0x424 == 0) / rec 0x39c (human; if ≠ 0 and
  bodySlip·L.z < 0 with |bodySlip| > 0.1 → 20); limiter; RWD (bias > 0.8) below 60 mph with (car+0x3c)+0x120 > 0:
  drive bias 0.65.
- **0x5a5b60** angular damping (car+0x424 < 2, car+0x38 == 0): only while |ω| < 0.5 rad/s:
  `T −= k(|ω|)·m·ω`, k = XY curve 0x86b4a0 {(0,0),(0.4,0),(2,20)} → acts only between 0.4 and 0.5 rad/s.
- **0x5b6300** assists: drag shift bonus (drivetrain+0x68 > 0): all W180 ·= 1 + min(bonus/T, 1) (T from the
  drive-type table 0x7fc2a8+0x144·type); "wheel hop" for car+0x24 ∈ {1,2,3} with car+0x42c > 0 (drag-type): when a
  wheel's slip·min(1, load/3) > 0.5 above 15 mph, CarState+0x42c = counter, every 0.5 s alternately
  W180 ·= 1 − 0.35·(1 − r) or 1 − 0.35·0.65·(1 − r) (r from XY curve 0x7fc924 {(0,1),(5,.8),(9,.9),(12.6,.833),
  (17.1,.72),(25,.65)}); cancelled by |bodySlip| > 0.3 or all slips < 2.5 for 1 s.
- **Drift-type races** (`!drag && ([0x890114] || drift [0x89e7d9]/[0x890104])`): aero f = c⁴, no 0.85 grip
  penalty, rear W180 = g0, steering curve set 4, and **0x5ac260**: rear W188 =
  Curve 0x7fc910 {1.1,.93,.86,.82,.8,.792,.78,.77,.765,.76}(x)·T14, x = |bodySlip|·(1 − T13) + |CarState+0xd8|·T13
  (T13 = 0.35/0.55/0.5, T14 = 1/1/1.125 for FWD/AWD/RWD); for car+0x424 ≥ 2 an extra yaw torque
  ∝ bodySlip about car up (curves 0x86b570/0x86b3c8).
- **Drag lane spring 0x5a5c20** (mover+0x7c): force 150·(target − pos) − 15·vel on the body; the drag clutch is a
  pedal (§10).
- Staging (mover+0x84): no aero, no limiter, yaw clamp ≈ 0, driven wheels zero grip at full throttle (burnout).

## 9. Collisions, MellowMover
- **Collision response is not in the PhysicsMover update.** The mover exposes ApplyImpulse 0x59bc60 (thunks
  0x5a9db0 / 0x5b1ce0, |J| ≤ 1000) and AddForce; the world/car collision system calls these (not traced).
  0x5a5e20 (from 0x5abb00) handles resets/respawns: zeroes P, L and wheel ω, or damps them ×0.95 per substep,
  and sets gravity 11/9.8.
- **MellowMover** (type 2, ctor in 0x5b6d00, vt 0x7a2538, update 0x5aad90): same rigid body (0x5a7f30) and
  4 Wheels (visual), plus the "Mellovator" 0x5b0030 (vt 0x7a2560): its own engine/gear model (record torque
  curve, shift points 0x5ac7d0, NOS curve 0x7fbf38), derivative 0x598ac0 (probes the ground with 0x5e7e70),
  post 0x5ab190 (wheel-speed rpm), force 0x5a38e0: one combined tyre force at the body from the heading vs.
  velocity angle (sin/cos, constants 0.05, −0.6, 1.25, 2.5), applied with AddForce; 0x5a4410 applies yaw/heading
  impulses (ApplyImpulse) toward a target heading. Substepping and integration are the same (0x5aa480 + 0x5bb0d0).
  A second variant (vt 0x7a2590, update 0x5ab3c0) is the traffic mover.

## 10. Input object, reverse, drag clutch (coordinator questions)
Input = car+0x30, human class vtable 0x784fc8 (base 0x784eb0; update 0x41e2c0). Slots used by 0x5abbf0:
- **vt+0x20 = 0x405b20 → input+0x1e4** (no args; the pushed rpm is reused as the 2nd argument of 0x5aa960).
  Values: −1 automatic, −2 semi-auto (timer), ≥ 0 = requested gear index (0 R, 1 N, 2 = 1st …).
- vt+0x24 = 0x405b30 → input+0x1e8 (target gear for semi-auto; copied to drivetrain+0x54 when 1 < g ≤ top+1).
- vt+0x28 = 0x405b40(x): input+0x1e4 = 0x5aa960's return (the request is consumed/acknowledged).
- vt+0x2c = 0x405b50(x): input+0x1e8 = drivetrain+0x54.
- Transmission mode input+0x2c0 (copied to drivetrain+0x60 by 0x407280): 0 auto, 1 manual, 2 semi (−2), 3 drag
  manual, 4 forced back to auto. Shift-up 0x407fd0 / shift-down 0x4080b0: manual: 1e4 ±1 within [N, top]
  (no R from shift-down); drag: 1e4 = gear ± 1; semi: 1e4 = −2, 1e8 = gear ± 1; auto: no gear change.
- **Reverse** is set by the human pedal logic 0x4077d0: a reverse-intent flag (+0x2f0) is set when the raw brake
  (+0x2dc) > 0.6 and raw gas (+0x2d4) < 0.5 (cleared when moving forward faster than −2 m/s); while set and the
  car moves backwards (−30 < speed < −2) the brake pedal is fed to the throttle and the brake is 0; when
  `gas + 0.2 < brake` it calls setThrottle(brake) and writes **input+0x1e4 = 0 (reverse)**. 0x407310 (vt+0x30)
  puts 1e4 back to −1 (auto) or 2 (1st, manual/drag). Exact local variables partly unresolved.
- **Drag clutch**: the human update calls **0x594910(clutchPedal = input+0x2f4)** when mode == 3; it sets
  drivetrain+0x60 = 3 and `pedal > 0.95 → state 3 (c = 1); pedal ≤ 0.05 → state 1 (c = 0); else state 2 if
  pedal > c else 0, c = pedal`. input+0x2f4 comes from the clutch action 0x408180: analog
  `1 − min(raw·1.07, 255)/255`, digital pressed → 0, released → 1. So **holding the clutch button gives state 1**;
  a shift requested in any other state counts as missed (0x5aac49 sets the +0x5c timer). 0x5a1400 in mode 3
  only stores the shift rpm/time and performs the gear change in state 1.
- **drivetrain+0x74** is only written by the ctor (0x5afb04, = 0) in the whole exe → always 0 (dead feature).
- The 3000 rpm launch check in 0x5abbf0 uses engine+0x14 (engine ω·9.549), as noted by the coordinator.

## 11. Reference sim (physics2_ref.py) results, Skyline (240 Hz substeps, 60 fps)
- At rest: wheel loads sum 15.1 kN (m·g = 14.7), x ≈ 1 cm.
- Steady state at 60 mph (road-wheel input θ, Ackermann): 1° → 0.16 rad/s, 0.4 g; 2° → 0.35 rad/s, 0.95 g;
  3° → 0.47 rad/s, 1.27 g; 4° → 0.58 rad/s, 1.55 g; slip angles 0.5–2.6°, slight understeer; downforce adds
  ≈8.8 kN (loads 22.9 kN). The exe's controller would cap 60 mph steering near 4.4°.
- Full braking from 60 mph: 1.2–1.46 g, wheels not locking (load transfer + linear downforce keep the lock test
  below the brake torque).
- Handbrake 1 s at 40 mph with 15° steer: rear wheels lock, yaw rate reaches 1.85 rad/s, rear slip angle −26°,
  car rotates and recovers when released.

## 12. Faithful vs approximated, unresolved
Faithful in the reference: rigid body (all damping/clamps), Euler + orthonormalise, suspension, loads,
tyre model (incl. exe atan table, ABS, lock test, curves, 5-pass solve with its row1.x quirk, friction circle,
induced drag), aero, steering geometry, brake/handbrake inputs and grip multipliers for a human player.
Approximated: drivetrain coupling (part 1 has the exact one), sin/cos tables, controller layer, flat ground.
Unresolved / not traced:
- 0x405dc0/0x4060e0 exact blend (max-lock vs speed, throttle/brake terms, counter-steer), wheel-steering path
  0x4061c0, force feedback 0x406460.
- car+0x42c (float; looks like speed in mph — steering curves span 0..160; also used in the preload term),
  CarState+0x424 vs car+0x424 meaning (0 = AI/remote?), car+0x38 states (1 = airborne/special?),
  [0x890114] and [0x890110] race kinds, 0x5ed5d0 (grip wear/damage), 0x599170 (per-car grip +0x184),
  event timestamp (car+0x3c)+0x140 used by the rear-grip pulse.
- 0x5a38e0/0x598ac0 (MellowMover tyre/engine) only surveyed; world collision detection/response not located
  beyond the impulse entry points.
- Stationary chatter: with the brake held at a standstill the locked-wheel low-speed branch (F/μ) gives ±1 g
  per substep in the reference; the game normally selects reverse there (§10), so this was not pursued.
