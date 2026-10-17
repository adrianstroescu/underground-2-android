# NFSU2 AI planning layer (speed2.exe, static RE) — goals, actions, drive target, recovery, belt

Companion to ai_re.md (parent: driver PsychoDriver, catch-up, skill table). Ticks: [0x86518c] = now in 1/4000 s,
`*0.00025` (0x784268) converts to seconds. "row" = AI tuning chunk GLOBALB 0x34b00 ([0x81cb60]) + 0x524 + skill*0x68,
skill = racer+0x901 (controller vt+0x78 = 0x402580, clamped >= 0).

## Object map
- Car: +0x14 RacerInfo (+4 kind 1 human / 2 AI / 3 traffic; +0x901 skill; +0x902 "personality" 0..2),
  +0x1c nav/progress (+0xa race position byte, +0x14 progress in laps, +0x18 progress rate),
  +0x24 controller kind, +0x28 mover type, +0x2c controller, +0x30 driver (= CarDriver input object),
  +0x34 mover, +0x424 wheels on ground, +0x42c forward speed, +0x430 |v|, +0x468 catch-up power mult,
  +0x5cc collision surface under car (0 = world not loaded), +0x96c "reset requested", +0x9c0 last reset/spawn tick.
- AICarController (ctor 0x420500, vt 0x7852b0; RaceCarController 0x4260b0 vt 0x785370; Traffic 0x426580 vt 0x785428;
  DragController 0x4299c0 vt 0x785518):
  - +4 car, +8 driver, +0x54 planning context "ctx" (built by 0x41c290/0x428720),
  - +0x6e0 → +0x360, +0x6e4 → ctx, +0x6e8 → +0x370 (route/path list; +4 count, +8 segment ptrs, seg+0x54 = end distance),
  - +0x6f0 replan countdown (init −0.0454545·0.1·carIndex: start stagger), +0x6f4 pending action type (0x16 = "let goal decide"),
  - +0x6f8 current action, +0x6fc current goal, +0x700/+0x704 braking-point-noise tick/value, +0x708 last GetUnstuck tick,
  - +0x710 "stopped/finished" flag, +0x72c/+0x748/+0x770 catch-up (see ai_re.md), +0x730/+0x734 physics-mover timers,
  - +0x738 stuck-sample timer (1.5 s), +0x73c AverageWindow(10) of progress speed, +0x744 catch-up enable (1),
  - +0x750 speed-zone index (−1), +0x758 AverageWindow(5) (not for traffic), +0x75c drag launch time,
  - +0x760 lateral-offset struct (init −1), +0x774/+0x77c finished flags, +0x778 last replan tick,
  - +0x790 autopilot speed scale (1.0), +0x798/+0x79c rejected car + tick (vt+0x9c), +0x7a0/+0x7a4 1.5 s timer +
    AverageWindow(5) of progress speed (RaceCarController only).
- ctx (= controller+0x54): +0 car, +4 count, +8 array of 0x30-byte "neighbour" entries sorted by route distance
  (cars ahead first). Entry E: +4 closing/relative speed (negative = closing), +8 half-width, +0xc half-length,
  +0x10 distance along route (m), +0x28 route segment index, +0x2c i16 vehicle id (car = [0x89e970 + id*4]).
  ctx+0x308 (i8) = index of the own car's entry (−1 = none). 0x60bff0(E) = E.dist − self.dist (gap ahead, m).
  0x60bfa0(E, S) = lateral position of E relative to the offset struct S. 0x60c120(car) = entry of a car.
  ctx+0x30c..0x318 = lateral offset struct S {i32 mode/side (−1 none), f32 lateral offset (m), f32 b (0.05 default
  [0x7e8e64], 999 = "no offset"), i32 tick}. ctx+0x31c = FuturePoints list (+4 count, +8 segment ptrs).

## Goals (controller+0x6fc), factory 0x42aa20(kind)
| kind | ctor | vt | name (vt slot 2) | used for |
|---|---|---|---|---|
| 1 | 0x40ca20 | 0x784768 | Win road race | AI racer (circuit/sprint/URL/Street X/drift/Lap KO) |
| 2 | 0x40ca70 | 0x7847d0 | Win drag race | AI in drag |
| 3 | 0x40cab0 | 0x784830 | Steer player in drag race | human (kind 1) in drag |
| 7 | 0x40caf0 | 0x7848a0 | Be traffic car | |
| 8 | 0x40cb30 | 0x784900 | Be ambient traffic car | fallback (also created inline, type 8) |
| 11 | 0x40cb80 | 0x784968 | Park | |
| 5 | 0x40daf0 | 0x784b58 | Chase car (cop leftovers) | |
RaceCarController vt+0x88 = 0x42c060: drag ([0x89e7d8]/[0x890108], or track 0x44b special) → kind 3 if human
else 2; otherwise kind 1.
Goal vtable slots: +0x28 (10) road-segment/shortcut choice (racer: 0x413cc0), +0x30 (12) road type filter
(0x41bd20: reject segment types (flags&0xf) 5, 7, 9, 10, 2), +0x34 (13) speed planner (racer & drag 0x41c5e0,
traffic 0x41c380, park 0x41c990), +0x38 (14) ChooseAction (racer 0x42d0f0 via 0x42efe0, drag 0x42d470,
traffic 0x426c20/0x426d70), +0x3c (15) default action (racer 0x4269e0 = JustDrive prio 0; traffic 0x426ec0 =
DriveTrafficLane; drag 0x427060 = StopDriving 600).

## Actions (base ctor 0x4234a0(type, priority, timeout))
Fields: +0x10 status (0 running, 1/2/3 done, set by 0x403120 via vt+0x18/+0x1c/+0x20), +0x20 DriveTarget
(0x1d0 bytes; count byte +0, 16-byte points from +0x10, max 20 = 0x14; +0x1c9 valid; +0x1b8..+0x1c4 timing),
+0x1f0 action speed (m/s, negative = reverse), +0x1f4, +0x1f8, +0x1fc "use planner speed" (1) / use +0x1f0 (0),
+0x1fd flag passed to driver vt+0x38 (4th arg), +0x1fe "catch-up allowed", +0x1ff "wants nitrous",
+0x200 priority, +0x204 type, +0x210.. per-action (start tick, target car +0x218, +0x21c tick).
Vtable: 0 dtor, 1 class name, 2 Execute(ctx), 3 OnExit, 5 GetName, 6/7 done checks.

| type | class | ctor | vt | Execute |
|---|---|---|---|---|
| 0 | Pass | 0x423610 | 0x7856d8 | 0x42b940 |
| 1 | AvoidAvoidable | 0x4243b0 | 0x785858 | 0x42f330 |
| 2 | AvoidAvoidableAsTraffic | 0x424b40 | 0x785950 | 0x424cb0 |
| 3 | Draft | 0x4235e0 | 0x7856b4 | (none, 0x4031c0) |
| 4 | Block | 0x4238c0 | 0x785728 | 0x423a10 |
| 5 | NudgeAvoidable | 0x423780 | 0x785704 | (none) |
| 6 | JustDrive | 0x4235a0 | 0x785684 | 0x42b570 |
| 7 | DriveTrafficLane | 0x424290 | 0x785820 | 0x4242d0 |
| 8 | PullOver | 0x423b60 | 0x785754 | 0x42eff0 |
| 9 | GetUnstuck | 0x423c30 | 0x785784 | 0x420940 |
| 0xb | IJustGotHit | 0x423cc0 | 0x7857b4 | 0x423e20 |
| 0xc | IJustGotHitDragRace | 0x424560 | 0x78588c | 0x4246b0 |
| 0xd | AccidentRecovery | 0x4241c0 | 0x7857e8 | 0x424210 |
| 0xf | HitWallDragRace | 0x424980 | 0x7858d4 | 0x4249c0 |
| 0x10 | StopDriving | 0x424b00 | 0x78591c | 0x42f6e0 |
| 0x12 | PickRouteToAvoidAnimation | 0x425150 | 0x7859d4 | 0x42dcb0 |
| 0x13 | DriveAtOffset | 0x4255d0 | 0x785a18 | 0x42bc50 |
| 0x14 | RollingRoadblock | 0x425630 | 0x785a4c | 0x425680 |
| 0x15 | RollingStart | 0x426b90 | 0x785a84 | 0x425870 |
| — | ObeyTrafficIntersections | 0x425100 | 0x785990 | 0x425150 |
Draft and NudgeAvoidable have no Execute (stubs) — Draft is never chosen by the racer goal.

Forced action by type (RaceCarController vt+0xac = 0x426230): 0x16 → goal->ChooseAction(ctx, curType, curPrio)
(no goal → DriveTrafficLane); 3 Draft prio 100; 6 JustDrive 0; 8 PullOver 600; 9 GetUnstuck 500 (5 s);
13 AccidentRecovery 500; 16 StopDriving 600; anything else → PullOver 600.
Replacement rule 0x402360: with pending type 0x16, the new action replaces the current one only if
new.priority > current.priority (else deleted). A non-0x16 pending type is forced, then pending = 0x16.

## Racer ChooseAction 0x42d0f0(ctx, curType, curPrio) — first match wins
1. GetUnstuck (9): curType≠9, curPrio ≤ 500, vt+0x94 (0x4083a0: > 6 s since last GetUnstuck, +0x708) and
   0x40e7c0(ctx) = 500 → GetUnstuck(500, 5.0, vt+0x98) (vt+0x98 0x4083e0 = 1.3, or 1.6 if the 1.5 s
   progress window +0x7a4 is full and its mean < 0.2 m/s); +0x708 = now.
   0x40e7c0 is stuck when: race not finished for the car (0x5eeff0), > 2 s since car+0x9c0, own route segment
   valid; and either (not drift/Street X) [lateral distance from route ≥ 20 m] or [AI racer speed ≤ 25 m/s
   (traffic ≤ 20) and 0x40df50(car,1)==1 (facing wrong way — unresolved)], or any contact in car+0x3c
   classified 0/2/3 by 0x402a40 that has lasted ≥ 0.4 s, was touched < 0.25 s ago, while speed < 5 m/s.
2. IJustGotHit (0xb): curType≠0xb, curPrio ≤ 400, 0x403320 finds the car that hit us (0x402ae0(ctx,−1,1)) →
   prio 400, target = hitter (0x40d5c0 registers it).
3. AvoidAvoidable (1): curType≠1, curPrio ≤ 300, 0x42ba20 → 0x428ce0(ctx, 3.0 m) finds an obstacle and a free
   offset → prio 300; vt+0x9c (0x408280) must accept → AvoidAvoidable(300, −1, obstacle, 2.0, offset).
4. AccidentRecovery (0xd): curType≠0xd, curPrio ≤ 500, wheels on ground < 2 and not upright
   (0x403380: up-vector z < 0.707) → prio 500.
5. Block (4): not Street X, curType≠4, curPrio ≤ P = int [data+0x934+pers*8] (pers = racer+0x902;
   P = 200, 200, 400), 0x414130 → Block(P, −1, target). 0x414130: own speed / 0x5ec070(car) > 0.8;
   scan cars BEHIND (entries after own index) moving ≥ 10 m/s; gap (minus both half-lengths if byte
   [data+0x938+pers*8] = 1 — it is 1) in (−50, 0) m, closing speed E+4 > −5; time to reach |gap/rel| < 30 s
   if the follower is a human (0x614880) else < 2 s.
6. Pass (0): curType≠0, curPrio ≤ 100, 0x42b5b0 → prio 100, vt+0x9c accepts → ctx S = new offset;
   Pass(100, 5.0, target). 0x42b5b0: race started > 5 s ago; for each car ahead: bumper gap
   = (E.dist − self.dist) − E.halfLen − self.halfLen > 0 and it is a vehicle; and either 0x403a40 predicts a
   collision with own speed/stopping data (0x4038d0) or (gap < 10 m and closing speed E+4 < 1); then
   0x429890 must find a lateral slot (3.0 m clearance).
7. else JustDrive (prio 0).
vt+0x9c 0x408280: forgets a rejected car after 1 s; accepts with probability row+0x0c (= 1.0 for all skills →
always accepts in shipped data).

## Drive target (lateral offset over the race line)
- JustDrive Execute 0x42b570 → 0x42b340 then 0x40e390:
  - default offset fraction b = 0.05 ([0x7e8e64]) (Street X 0.2 [0x783680]).
  - min time-to-collision over cars ahead: t = −gap/E.relSpeed (|relSpeed| > 0.1, t > 0).
    If t < 0.75 s ([0x78367c]) and S is the plain default: 0x428df0(…, 3.0 m) searches a free lateral slot and
    builds the target there; else 0x420d40 builds it from the current S.
  - 0x40e390: action+0x1ff (want nitrous) = AI racer && nitrous not active && controller vt+0xa0 (0x416f70).
  - flags +0x1fc = 1, +0x1f8 = 0, +0x1fd = 1, +0x1fe = 1.
- Pass Execute 0x42b940: if target still ahead (gap > 0) re-solve the slot with 0x429890(target, 3.0) else
  0x420d40; if |target.relSpeed| ≥ 5 run the nitrous check 0x40e390; flags 1/1/1. Done check 0x42b7f0:
  every 3 s ([0x783688]) re-validate; finished when target is > 10 m ([0x783684]) behind.
- Lane slot 0x429890: required separation = myHalfWidth + otherHalfWidth + 3.0 m ([0x783818]); if the other
  car is laterally closer than that, side = 0x41d2d0(): 2 → offset = otherLat − sep, 3 → otherLat + sep,
  1 → whichever side we already are; S.offset = that, S.b = 0.05; validated/clamped by 0x428df0 against the road.
- 0x420d40(S, target, ctx): weight = 1.0 ([0x7837ec]) or 0 when S.b == 999; 5.0 in drift-type races with a nav.
  If 0x41bfe0 says the car is off its line by more than the threshold (traffic 10 m, drift 4.5, drag 3,
  else [0x7e8d94] = 1.5 m) or +0x774: rebuild from the route (0x41d160 + 0x41d530). Otherwise continue the
  driver's previous points: look-ahead = min(0.5·|v|, 30 m) (0x7837e0, 0x7837e8); drag: 0.25·|v| (no cap).
  0x41d530 walks the FuturePoints segments (ctx+0x31c) from the own segment, placing up to 20 points at the
  lateral offset (0x40fc80). Exact point spacing: unresolved (0x4198c0 uses 0.5 [0x783808]).
- Driver receives it via driver vt+0x3c (0x405910) in 0x420810.

## Speed planners (goal vt+0x34) → driver vt+0x38(speed, a, b, flag)
- Racer 0x41c5e0 (decoded by parent): lineSpeed(dist + noise)·scale·catchup(+0x770); noise rerolled every
  [data+0xad4] = 5 s as rand(2·row+0x60) − row+0x60 (±40..±20 m); scale row+0x14 (Street X row+0x1c; Lap KO
  type 7: 0x78357c/0x783598 tables; online 1.0); line speed 0x411dd0 (≈0 → 30 m/s); 99999 when 0x401fa0 or
  0x4018f0 (no limit). Then if action+0x1fc: limit = that speed, else action+0x1f0; final =
  min(limit, goal speed, goal vt+0x4c(ctx)). Speed zone +0x750 overrides from [0x883db0]+idx*32.
- Traffic 0x41c380: look-ahead d1 = max(1.3·|v|, 20 m) (0x783564, 0x783568), d2 = max(1.3·|v|, 8 m)
  (0x78355c, 0x783560); speed = min(goal speed, line speed at d1 (0x411dd0), at d2); a slowdown factor via
  0x411fa0/0x40c840 (10 + (s−10)(1−k)).
- vt+0x4c extra limit: unresolved (RaceCarController slot 19).

## AICarController::Update 0x42abd0 (order)
1. tick timers +0x730 (and if expired with mover 4 on loaded ground → default mover), +0x754; +0x6f0 −= dt;
   clear ctx (+4 = 0, +0x308 = −1).
2. Replan decision: AI racer: N = 8 (4 in drag unless belt); this car's slot when
   [0x890080]+0x1c list[[0x890080]+0x14 + frame%N] == this car; replan when (now−+0x778) ≥ 0.2 s AND it is
   its slot, or there is no action. Human autopilot: slot = frame&7 == car+0xa. Others: when +0x6f0 ≤ 0.
3. No replan: nitrous stop check (vt+0xb4), goal speed planner, action Execute via vt+0xb0 (0x41bf10: refresh
   path then action vt+8), 0x420810 output.
4. Replan: +0x6f0 += 0.1, +0x778 = now, locate road segment (0x5ded60), create goal if none (vt+0x88), default
   action if none, rebuild ctx (0x428720). ctx fail: goal done → reset car (0x5ebf60) unless race type 7;
   else stop. Finish check (own distance ≥ path end or 0x40abb0) → StopDriving input (driver vt+0x38(0,0,0,1)),
   +0x710 = 1. Then action status (0x403120); done → exit + SetAction(0x16); else 0x402360 (priority rule).
5. 0x420810: driver vt+0x3c(target), catch-up vt+0x80 (0x4172d0), goal speed planner, nitrous start (vt+0xa4)
   when action+0x1ff; in some race modes driver+0x250 = 4.
6. Mover: if car on loaded collision: belt (4) when [0x7e8d70] and 0x402180 (≥120 m from player, ≥30 s race
   clock, speed ≥ 5, wheels ≥ 1, action not 8/9/10/11/13, not drag/drift); otherwise leave belt → default mover.
   If not on loaded collision and mover is belt: 0x5918a0 rebuild the belt spline.

## Recovery / reset
- Stuck reset (RaceCarController 0x42c0c0, race running, type ≠ 0/7): every 1.5 s ([0x7834a4], +0x738) push
  |nav+0x18|·routeLength into AverageWindow(10) +0x73c; if action ≠ PullOver, not finished, window full,
  mean < 0.2 m/s ([0x7834a0]), ≥ 50 m from the player ([0x783514], 0x5f1160) and not in the post-reset grace
  (0x5ebfa0: now − car+0x974 < car+0x978) → 0x5ebf60 (car+0x96c = 1).
- AccidentRecovery Execute 0x424210: keep the line target, speed 0 (flags 0); after 5 s ([0x783720]) since the
  action started (+0x210) request reset once (+0x214).
- GetUnstuck Execute 0x420940: target built backwards along the route (0x4122c0, weight 1.0, 0.15), action
  speed −15 m/s for AI racers ([0x7836d8]) / −6 m/s others ([0x7836dc]); no planner speed, no catch-up.
  Timeout 5 s (ctor). Drift/Street X flag target+1 = 1.
- Reset execution in car update 0x6012b0 when car+0x96c: AI racer → 0x5fbed0(car, segment of the controller's
  route position, speed, 1, dir, 0, 0, dist): car put back on the route at its current route segment; speed =
  max(0, forward speed · cos(heading vs road)) capped at [0x8021cc] = 8 m/s; dir −1 for reversed traffic.
  car+0x96c = 0.

## Belt mover (type 4, ctor 0x5a9030, vt 0x7a2144, update slot 1 0x59ec80, rebuild 0x5918a0 → 0x591690)
- Kinematic follower of the AI's own DriveTarget points (driver+0x10): rebuilds a spline (0x57a300, 2..20
  pts) through them. Param s advances s += v·dt + ½·a·dt² (fields +0xc time, +0x20 speed·L, +0x24 accel·L,
  +0x10 spline length).
- On rebuild: v0 = car+0x42c (forward speed); a = (0.899·desired − v0)/T with desired = driver vt+0x74
  (PsychoDriver +0x230, the planner speed) and T = driver vt+0x78 (+0x234) or 0.5 s; a ≤ 0x4038d0(car, v0)
  (the car's tabulated full-throttle acceleration at v0/catch-up, minus aero drag), a ≥ −20 m/s² ([0x7a17d0]).
  So a far-away AI cruises at ≈ 0.899 × its planned speed (which already includes skill scale and catch-up).
- 0x591950: lateral-grip check (0.5·(front+rear static grip)·g·mass terms; curves at 0x86b590/0x86b3c0 with
  points 0.9, 1.1, 1.5 × limit) switches spring damping 1.0/0.3 (0x7a1810/0x7a180c). Detail unresolved.
- Mellow mover (3) is the default for AI racers (0x5f4400 → 3 for car kinds 3/5 outside drag/drift);
  RaceCarController forces physics (1) while timer +0x734 runs (5 s after the start, extended 1 s while
  < 4 wheels down; parent: 5 s after vehicle hits via 0x402450) and only when car+0x5cc (collision loaded).

## Road/shortcut choice (goal vt+0x28 = 0x413cc0, racer)
Segment flags seg+0xc: type = &0xf. Rejected outright: types 5, 0xa, 7, flag bit 20, or road +0x70 bit 10.
Type 4: if 0x4018f0 → 0x40da10; else take with probability 0.5 (0.333 if flag 0x8000000).
Other types: r = rand(1): r ≤ row+0 (0) → allowed only if type 1; r ≤ row+4 → allowed if type ≠ 0;
r > row+8 (1) → allowed if type ≠ 0; else not allowed. With shipped data: a non-type-0 branch (shortcut/
alternate) is taken with probability row+4 = {0, 0, .05, .2, .35, .5, .7, .7, .85, .9} for skill 0..9.
(Interpretation of types 0/1 = main/alternate: likely, not verified against route data.)
Online ([0x8650b0]): fixed allowed list [0x81cb7c] (count [0x81cb78]).

## Unresolved
- 0x41d530 / 0x4198c0 point spacing and how PsychoDriver turns the points into steering (parent's area).
- 0x40df50 (wrong-way test), 0x402a40 contact classes, 0x5ec070 (Block speed reference), goal vt+0x4c limit.
- AvoidAvoidable / Block / IJustGotHit Execute details (offset sizes, durations).
- Exact belt lateral-grip logic.
