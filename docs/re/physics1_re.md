# NFSU2 player-car physics, part 1: update structure, engine, clutch, gearbox, drivetrain (speed2.exe)

Reference sim: `physics1_ref.py` (same folder). Units are consistent and SI-like: torque in kN·m,
mass in t, force in kN, inertia in t·m², speed in m/s, angular speed in rad/s (rpm = w·9.549304).

## Objects
- Car object (`car`, mover type at car+0x28/0x38). `0x5f46a0` picks the mover by type through jump
  table 0x5f493c: **type 1 = full PhysicsMover** (built by `0x5b5cc0`, used for the player).
  Type 2 = TrafficMover 0x408740; type 3 = MellowMover (`0x5b6d00`); 4 = Belt 0x5a9030; 5 = NIS (corrected by ai_re.md).
- PhysicsMover (0x130 bytes): vtable **0x7a25e0**, update = **slot 1, 0x5b72d0(dt)**.
  Embedded component list at +8 (vtable 0x7a25d4). Fields:
  - +4 = input
  - +0x20 = RigidBody (0x7a20c4)
  - +0x24 = steering
  - +0x28..0x34 = 4 suspensions
  - +0x38..0x44 = 4 Wheels (0x7a20e8)
  - +0x48 = Engine (0x7a235c)
  - +0x4c = DriveTrain (0x7a2508)
  - +0x50 = Aero (not a component)
  - +0x54 = ps
  - +0x58 = CarState (= car+0x40)
  - +0x5c = car
- ps = physics struct = p+0x30. **ps+X = record+(X+0x110)**.
  - Engine data = ps+0x1f0 (rec 0x300).
  - DriveTrain data = ps+0x1b0 (rec 0x2c0).
  - Wheel i geometry = ps+0x10+0x30i (rec 0x120+0x30i: +0x10 radius, +0x18 wheel index 0..3, rear ≥ 2).
  - Axle tyre data = ps+0xd0/0xf0 (rec 0x1e0/0x200): +0 max static slip angle, +4 brake, +8 handbrake,
    +0xc static grip, +0x10 dynamic grip.
- CarState (car+0x40) outputs, written at the end of the frame by `0x5a5540`:
  - +0x3f0 = |v|
  - +0x3f4 = gear
  - +0x3f8 = effective throttle
  - **+0x400 = displayed/shift rpm** (car+0x440)
  - +0x408 = clutch engaged
  - +0x40c = drive torque
  - +0x3e4 = number of wheels on the ground

## Time step and frame order
- The frame dt is the real frame time. The frame limiter `0x57eae0` has a minimum of 1/120 s
  ([0x865558]) and a maximum of [0x7fb714]/60 (3 → 0.05 s).
- Path: car update `0x6012b0(dt)` → mover vt+4.
- **Substeps: n = trunc(dt/0.0044)+1, h = dt/n** (0x7fc070). At 60 fps that is 4 × 1/240 s. The substep dt is
  also kept in [0x7fc800].

`0x5b72d0` order:
1. Clear wheel flags.
2. `0x5ab990`: static wheel loads (g = 9.8 from 0x7fbef0, weight split from rec 0x130/0x220). It sets
   **wheel inertia +0x2c = 0.01** (0x7fc6f0).
3. `0x5a49a0`: mode flags, gravity, aero +0x30/+0x34. It sets rb+0x204 = speed limiter (ps+0x290) and
   drivetrain bias +0xec (see "Drive to road").
4. **`0x5abbf0` = input stage:**
   - Throttle input (input+0x20c): ≤ 0.15 → 0.
   - If 0 < throttle < 0.5, in gear and rpm < 3000 → throttle = 0.5.
   - Writes engine+0x78 = throttle, aero+0x10 = throttle.
   - Gear request (input vt+0x24) goes to drivetrain+0x54.
   - Nitrous level `0x59fa30` → engine+0x84.
   - `0x5aa960(req, rpm=car+0x440)` = gearbox.
5. `0x5a4d80(dt)`: per-wheel grip multiplier +0x180. It includes nitrous level × {1.0, 1.4, 1.7} (0x7a17a4).
6. `0x5b6300` (assists).
7. Substep loop: `0x5aa480(h)` = explicit Euler over all components, then rigid-body finalize `0x5bb0d0`.
   - First, derivatives: vt+0x18 for each component, in list order.
   - Then `state += h·deriv`.
   - Then the container post `0x5b04d0`: each component's vt+0x1c post-step, then `0x5abb00`, then
     **force stage `0x5b0520`**, which runs, in order:
     1. aero `0x59e620`
     2. tyre loads `0x59d160` ×4
     3. **`0x5abdf0`**:
        - clutch torque `0x594cb0`
        - tyre forces `0x5a8280` ×4
        - 5 iterations of the static-wheel/body solve, each with the locked-clutch constraint `0x594da0`
        - friction circle `0x59d600` ×4
        - engine sync `0x5aa830`
     4. hydraulics `0x5a51b0`

     The forces it computes are used by the next substep's derivatives.
8. After the loop: `0x5a5540(dt)` publishes CarState and the smoothed rpm.

## Engine (0x5aa5a0 ctor, vtable 0x7a235c)
- State: +0x14 = w (rad/s).
- Curves (n, x0, x_end, inv, y*):
  - +0x24: torque, 9 points, idle..max, y = rec 0x310. Includes the stock turbo rec 0x334 added by 0x5a6e30.
  - +0x38: **engine braking, 3 points, idle..max, y = rec 0x358**.
  - +0x4c: turbo, 9 points (unused by the physics).
  - +0x60: NOS, 9 points, **idle..redline**, y = 0x7fbf38 = .3 .4 .45 .5 .55 .6 .7 1 .7.
- Throttle fields:
  - +0x78 = throttle.
  - +0x80 = throttle scale set by the clutch state: 0 / 0 / 0.7 / 1 for states 0/1/2/3, or 1 with drivetrain+0x74.
    Drag 2P input type 2 gives 1.5.
- Other fields:
  - +0x7c = external torque, reset each post-step.
  - +0x84 = NOS level.
  - +0x88 = limiter on.
  - +0x8c = heat (−1 = off; drag only).
  - +0x90 = torque multiplier (1).
  - +0x74 = car byte +0xd8 → ×1.05.
- **Torque `0x594300`:**
  ```
  rpm = w·9.549304;  th = limiter ? 0 : (+0x80)·(+0x78)
  T  = Tq(rpm)·(+0x90)·(1.05 if +0x74)·th
  if NOS level n>0:  T += NOS(rpm)·ps[0x2ac](rec 0x3bc)·{1.0,1.4,1.7}[n] (0x7a1798)·th
  if heat>0.6:  T *= 1 - 0.75·(min(heat,1)-0.6)/0.4
  T += -(1-th)·EB(rpm)·(2 if limiter)
  if T<0 and !limiter:  T *= (gear==N ? 0.65 : 0.85)
  ```
- **Derivative `0x5a1290`:** `dw/dt = (T + ext)/I_e`, with **I_e = rec 0x30c** (Skyline 0.00026).
- **Post `0x5aa760`:**
  - Clamp w to [idle, max]·0.1047198.
  - ext = 0.
  - **Rev limiter:** on when rpm > max − 25. Off when rpm < max·(1 − 0.0125).
  - Turbo gauge `0x5a0f30`: the needle value is +0x9c ∈ [−1, 1]. It is visual only; the torque never reads it.
    There is no spool lag: the stock turbo torque is static.
  - Heat `0x5a11e0`: +0.45/s above redline, −0.1/s otherwise. At ≥ 1.2 the engine blows (set to 2).
- **Nitrous** (CarState):
  - +0x414 = capacity (ticks of 1/4000 s).
  - +0x41c = charge.
  - +0x420 = activation tick (0 or 0x7fffffff = off).
  - Charge left while active (0x592480) = 41c − (now − 420).
  - Gauge (0x5925b0) v = charge/cap. It can exceed 1, up to 2.
  - Level (0x59fa30): 3 if v > 1.7, 2 if v > 1.4, 1 if v > 0; at 0 it deactivates (0x592500).
  - Refill is gameplay, outside the physics.

## Clutch / DriveTrain (0x5af9d0 ctor, vtable 0x7a2508)
Fields:
- +0x14 = clutch c (state variable)
- +0x18 = data (rec 0x2c0)
- +0x1c = drive-type table 0x7fc2a8 + 0x144·type (FWD bias < 0.2 → 0, otherwise 1; bias > 0.8 → 2)
- +0x20 = engine
- +0x24..0x30 = wheels FL FR RL RR
- +0x38 = target gear
- **+0x3c = gear** (0 R, 1 N, 2 = 1st …)
- +0x44 = clutch state: 0 disengaging, 1 disengaged, 2 engaging, 3 engaged
- +0x48 = shift in progress
- +0x4c = wheel-side engine speed
- +0x54 = requested gear
- +0x60 = transmission mode
- +0x68 = drag shift bonus
- +0x70 = drive torque
- +0x84[] = up rpm, +0xa8[] = down rpm
- +0xe8 = shift speed (1)
- +0xec = bias

- **Wheel-side speed `0x5945f0`:**
  - rear = ½(wRL+wRR)·fd_r if bias ≥ 0.001
  - front = ½(wFL+wFR)·fd_f if bias ≤ 0.999
  - AWD: sum × 0.5
  - Result × gear[g]
- **Clutch rate `0x5949b0`:** `dc/dt = rec0x2dc · (+0xe8) · {−3, 0, +1.5, 0}[state]` (table 0x7fc298).
  - If +0x68 > 0: × (1 + bonus). If +0x68 < 0 and gear ≥ 2nd: × 0.35.
  - If +0x60 == 3: 0.
  - Skyline rec 0x2dc = 4: disengage in 1/12 s, engage in 1/6 s.
- **State machine `0x5a1400`** (post-step):
  - 3 Engaged: throttle scale 1. If gear ≠ N and (throttle < 0.3 and wheel-side ≤ idle·0.10472) → 0.
    If gear ≠ target: +0x48 = 1, +0x6c = rpm, → 0.
  - 0 Disengaging: scale 0. When c ≤ 0: c = 0 → 1.
  - 1 Disengaged: scale 0. gear = target. Re-randomise the shift offsets (`0x594710`). → 2, unless still
    in the declutch condition.
  - 2 Engaging: scale 0.7. When c ≥ 1: c = 1, and → 3 if |w_e − wheel-side| < 50 rad/s (0x7fc028) or gear = N.
    Declutch condition → 0.
  - With +0x74 set, it always goes to 1 (scale 1).
  - Missed-shift timer +0x5c: throttle 0 for 0.45 s.
- **Clutch torque `0x594cb0`** (states 0/2):
  ```
  T = rec0x2d4(0.04)·c·clamp(w_e − x4c, ±300)
  ```
  The stiffness is halved when Δrpm changes sign with |Δrpm| > 2000 both times (memory +0x58).
  State 3: T = engine torque. State 1: 0.
- **Apply(T) `0x594a30`** (no-op in N):
  - ext_engine −= max(0.25, wheelsOnGround/4)·T.
  - Front axle gets (1−bias)·fd_f·gear·T. Rear axle gets bias·fd_r·gear·T.
  - Left/right split s = 0.5, or 0.35 when both wheels have +0x11c. car+0x42c forces 1−s or s.
  - Otherwise, with r = w_R/(w_L+w_R): r < s → the right wheel gets 0.825, r > 1−s → 0.175, else r.
    The slower wheel gets 82.5 % (LSD).
- **Locked clutch `0x594da0`** (state 3, run in each of the 5 iterations):
  ```
  T = (α_e,free − α_drive)/(1/I_e + Σ_axle share·(gear·fd)²·0.5/I_w)
  ```
  α_drive = `0x594840`. This is a rigid engine↔wheel constraint.
- `0x5aa830`:
  - In state 3 with throttle > 0.2 and average driven-wheel slip > 0.1 m/s, `0x5a12d0` resyncs the driven
    wheels to the engine speed.
  - Otherwise ext += clamp(0.04·(x4c − w_e), ±|T_e|).
  - 1st-gear anti-stall: table 0x7fc8c0.

## Automatic gearbox `0x5aa960(req, rpm)` (req −1 = auto; rpm = displayed car+0x440)
- Shift points `0x5ac7d0`:
  - up[1] = idle + (red − idle)·rec 0x2c0
  - **up[g] = red − 100** for g = 2..ng
  - up[top] = max + 500
  - **down[g+1] = gear[g+1]/gear[g]·up[g]**
- Upshift rpm `0x594670(g, low)`:
  - min(cap, up[g] + o), cap = redline (max rpm in drag)
  - o = +0x80 for g = 3, else +0x78; both U[−100, 0] per shift
  - low → o = −100
- Rules:
  - g ≤ N → target = 1st.
  - Only in state 3: **upshift** if rpm ≥ up(g), g < top, and every wheel is on the ground (+0x11c) with
    slip velocity (+0x118) < 4 m/s.
  - **Downshift** (g > 1st) if rpm ≤ down[g] + U[−100, 0] **and** rpm ≤ up(g−1, low)·gear[g]/gear[g−1] − 200.
- Manual: req = gear. req −2 = semi-auto with timer +0x50 (10 s, red − 300).
- Mode +0x60 == 3: drag manual, missed-shift penalty. Perfect-shift bonus `0x5a1700`.
- **Shift rpm smoothing `0x5a5540`:**
  - raw = w_e·9.549. A fall is limited to −k/I_e rpm/s, k = 2.5 (15 while upshifting) from 0x7a1a2c.
  - **rpm_disp = 0.45·prev + 0.55·raw.**

## Drive to road / resistances
- Wheel derivative `0x59d350`:
  ```
  dw = ( ½((T_drive − r·Fx) + prev) − 0.002·surf·w − 0.1·(+0x174)·w ) / 0.01 − s/(r·h)
  ```
  The −s/(r·h) term is applied when not sliding. s = r·w − v_contact.
- Tyre `0x5a8280`:
  - Brake torques: rec+4 × 4 × brake; rec+8 × 10 × handbrake.
  - **Static if |s| ≤ 0.5 m/s** and the slip angle is ≤ rec+0: Fx = T_drive/r, then corrected by the 5-pass
    solve: Fx += I(α − a/r)/r.
  - Otherwise sliding: |F| = μ_dyn·load·grip mods along the slip direction. Below 1 mph F is divided by μ.
- **Friction circle `0x59d600`:**
  ```
  Fmax = μ_static(rec 0x1ec/0x20c)·surf·mods·load
  ```
  Driven wheels test 1.5·Fx. Over the limit, F is clamped to Fmax and the wheel spins up.
  - Player (car+0x424 == 1), rear wheels: × (0.85 + 0.15·clamp((v − 13.41)·0.11185, 0, 1)) (0x59d490).
- RWD with bias > 0.8, below 60 mph and assist flag (car+0x3c)+0x120: drive bias becomes 0.65 (0x5a4d14).
- **Aero `0x59e620`:**
  ```
  F = −v̂·|v|²·k(rec 0x370)·(1 + (1 − throttle)·(2 − 1))·(+0x30)(+0x34)(+0xc)/(+8)
  ```
  So drag doubles off-throttle. Downforce uses rec 0x374 + 0x378 (part 2).
- **Speed limiter `0x59c1e0`** (rb+0x204 = rec 0x3a0 mph):
  - lim = mph·rb+0x208(1) + 1.9, × 1.03 with flag.
  - Slope factor: 1 − 0.333·min(pitch, 0.3) uphill, 1 + 0.333·min(−pitch, 0.3) downhill.
  - When the horizontal speed ·2.23602 > lim and there is a forward force, the forward force becomes
    min(−5·F, −0.5).

## Not resolved / approximations in the reference
- Real wheel loads (suspension 0x59d160/0x5ab990), downforce, and the exact 5-pass solve.
- Surface table 0x800f00 is all 1.0 in the exe.
- Meaning of car+0x424 (0 AI? / 1 local / 2 remote), drivetrain+0x74, and engine+0x90 (AI catch-up?).
