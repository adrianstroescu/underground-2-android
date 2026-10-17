# NFSU2 traffic cars (speed2.exe, static analysis)

Conventions: ticks = 1/4000 s ([0x86518c] = now), speeds m/s, 24.5865 m/s = 55 mph (0x7845a4).
"road net" = global 0x88f298 (route/road segments from TRACKS/ROUTESL4Rx/Routes*.bin);
0x5d5cd0(segA, segB) = signed road distance between two segment positions; 0x5d9bf0(pos,0x10,0) = nearest segment.

## 1. How many traffic cars (count = fixed pool, not density)
- Traffic option [0x89e7a8] (GRaceParameters+8): 0 None, 1 Min, 2 Moderate, 3 Max (quick race option; career sets
  it in 0x5322e0 @0x53235c). 0 = no traffic at all (0x4fe9f0 and the manager update 0x42f910 both test it).
- Pool creation 0x4fe9f0 (end of quick-race setups 0x53f150 etc.), skipped if [0x89e7e4]:
  ```
  rec = track record (0x5d3e40: list [0x883da0], count [0x883da4], stride 0x128, id at +0x8a == [0x89e7a0])
  n = (int8) rec[0xec + reverse([0x89e7a4]) + level*2]      # level = [0x89e7a8]
  n = min(n, 16)
  0x81e810.reset() (0x41aaf0); for i<n: model = 0x81e810.next() (0x40bde0);
      RacerInfo(slot 0x32+i, kind 3, model, name "" (0x783fe5), skill -1, 0)  -> added to GRaceParameters (0x601600)
  ```
  Racer kind 3 -> car controller kind 4 = TrafficController (0x5f6d50), mover via 0x5f4400 -> 2 (TrafficMover).
- Track record bytes +0xec..+0xf3 (GLOBALB 0x34201) = [fwd,rev] for level 0..3. Shipped data: 12 for every level and
  direction on almost all tracks. Exceptions: DT1 Olympic Sq L1 6/6, L2 10/10, L3 12/12; template 4000 and drag 4201
  L1 10, L2 12, L3 16; Organic drags 4401/4402/4404 L1 10, L2 12, L3 14; SH drift 4174-4179 fwd L1 8/L2 12/L3 16,
  rev 0. So the car COUNT barely changes with the option; density comes from spacing (section 2).
- Track record +0x108 + level*4 floats (0.5 everywhere; 4401: .5 .6 .7 .8) are NOT a density: in the spawn
  0x42e9f0 `flag = rand01 < 1 - f[level]` picks which lane-direction bit of the segment (bit 25 vs bit 26 of seg+0xc)
  is tried first => f = probability of the other direction (0.5 = even split oncoming / same direction).

## 2. Spacing (the real density control): 0x418240(car)
```
seg = roadnet.nearest(car.pos)            # 0x5d9bf0
if !seg or 0x4112a0(car): return 99999    # 0x47c34f80
cls = (int) seg[+0x2c + level*4]          # per-segment class per traffic level (road data)
lo,hi = AI chunk (GLOBALB 0x34b00, [0x81cb60]) + 0xa8c + cls*8
return lo + rand(hi-lo)                   # 0x5ea500 = rand float [0,x)
```
Pairs (m): cls0 (240,300) 1 (110,145) 2 (80,95) 3 (90,110) 4 (70,100) 5 (50,65) 6 (50,70) 7 (35,45) 8 (20,25).
Computed per human player (and for 0x5f0e30(-1), e.g. the camera/extra target) each spawn tick, default 999999.
Used in 0x42e9f0 as the step length when walking the road graph from the player to find a free spawn slot (a slot
per `spacing` metres, at most 500 m (0x783ba8) and 100 iterations). Segment +0x2c class values: UNRESOLVED (need the
road segment record layout in Routes*.bin 0x34121 to dump them).

## 3. Manager (world [0x890080]+0x118, 0x1c bytes, built by 0x4179f0 in 0x5ff490)
Fields: +0 spawn timer (s), +4 round-robin index in car list, +8 enabled, +0x10 last streaming tick, +0x18 per-segment
cumulative lane weights (0x409900: sum of seg word +0x34 over [0x88d218] segments).
Update 0x42f910(dt) (called from 0x609ed7):
1. Only if level != 0, car list non-empty, race state [[0x890118]] == 4 (racing), humans [0x8900bc] > 0, enabled.
2. Every 5 s (0x783bbc): StreamingTrafficCarManager 0x81e810 model swap 0x412950.
3. timer -= dt; when <= 0: timer = 0.3 s in drag (0x783ba4) else 1.0 s (0x783ba0) (0x4098d0); then visit up to
   N cars round-robin (N = 16 if drag (0x4018f0) else 1). For each car with 0x417bb0(car) == "recyclable":
   0x42f800(car, spacing[]) tries to respawn it; on failure the controller is parked (0x4020d0, +0x77c = 1).
- Recyclable 0x417bb0: active traffic (controller goal vt+0x10) -> recyclable if (+0x798 and no slow player near:
  0x4094a0 = no human with speed < 20 m/s (0x783bc0) within 50 m road distance (0x783bc4)) or
  (!+0x77c and NOT within 250 m road distance (0x409800: 0x5f12a0 < 250, 0x78429c)) of any player.
  Parked cars: 0x410de0 -> 0x409890 / 0x409840: >= 250 m (0x7e9720 / 0x78429c) from players.
  => despawn/recycle distance = 250 m (road distance to the nearest human; 2nd players included in split screen).
- 0x42f800 respawn: p = 1.0, or 0.35 (0x783bd4) when the player speed >= 20 m/s (0x783bcc); if rand < p try
  cross-traffic emitter spawn 0x42e790 first. Else spacing = spacing[player idx] (non-human: [2]); if spacing==99999
  fail; drag -> 0x42e620; otherwise 0x42e9f0 (lane traffic near the player).
- 0x42e790 cross-traffic: needs emitter records [0x883dc4] (chunk 0x34124, 0x108 B, count [0x883dc8]); picks an
  emitter (0x409570) 60 m (0x783bd8) around the player's segment (or at it if no slow player near), speed =
  min(emitter[+0xb0+level*8] * 0.447027 (mph->m/s), lane speed byte), distance rand in [+0x70+L*8 .. +0x90+L*8].
  Chunk 0x34124 was not found in the shipped TRACKS bundles -> probably inactive (UNVERIFIED).
- 0x42e9f0 lane traffic: if the player's speed (car+0x430) >= 24.5865 m/s, or the player's car record +0x248 flag,
  the "ahead/behind" preference is flipped with 50 % (0x784260); walks roads (0x415130) from the player, distance
  limit 500 m, picks lane via 0x410ca0 using the direction bit, rejects occupied slots (0x418110), then
  0x42c950(car, seg, 0, lane, 24.5865, laneSpeed = seg byte +0x2c+lane (m/s), 0).
- Placement 0x42c950: lanes 2..5 -> forward (dir +1); others backward (dir -1, lane -= 4); controller +0x10 = forward,
  +0x79c = max speed (24.5865), +0x799 = "Nice/Jerk" value by difficulty [0x89e7f4]: easy 9, medium 4, hard 0, else -1
  (meaning UNRESOLVED), +0x7a4 = spawn tick; goal 0x42aa20(7 or 8); car placed 0x5fbed0 at the lane pose with
  the initial speed; mover 2.

## 4. Models: StreamingTrafficCarManager 0x81e810 (snapshot 0x412720, name 0x785080)
- Loaded-model list at +0x900; each entry: car type +0x10 (into car table [0x8a1ccc], 0x890 B, GLOBALB 0x34600),
  load tick +0x900, in-use count +0x90c, swap flag +0x914, +0x908.
- 0x412950 (every 5 s): region = player segment +0x3c when [0x8654a4]==6; weight = (int8) cartype[+0x854+region];
  a model loaded >= 6 s (0x783cd8) with more instances than max(weight,4) is flagged for swap; cartype[+0x859] != 0
  forbids swapping. Weights: TAXI, 4DR_SEDAN02, TAXI02, COUPE, PICKUP, 4DR_SEDAN, SUV, MINIVAN, HATCHBACK,
  HATCHBACK02 [5,5,5,5,2]; PANELVAN [3,3,3,5,1]; PARCELVAN [3,3,3,3,1]; racer cars [5,2,2,2,2].
  Which types are allowed as traffic (filter) = UNRESOLVED (presumably the traffic models 31..45).

## 5. TrafficController (car controller kind 4, ctor 0x426580, vt 0x785428, 0x7a8 B)
Base AICarController 0x420500, driver = PsychoDriver 0x425ab0 (+8, same as racers). Slots: 2 snapshot 0x429cf0,
3 update 0x429d90, 4 0x408c40 (restore default mover), 5 event 0x408ca0 (events 3/4 -> 0x5ed5c0(1), mover 2),
14 0x408ed0 (-> Mellow 3 if collision ground +0x5cc), 24 0x408c70 returns 4.0, 31 desired speed 0x41e7a0 =
min(+0x79c, 24.5865), 32 0x408c80 (catch-up mult = 1).
- Action ids (action +0x204): 0 Pass, 1 AvoidAvoidable, 2 AvoidAvoidableAsTraffic, 3 Draft, 4 Block, 6 JustDrive,
  7 DriveTrafficLane, 8 PullOver, 9 GetUnstuck, 11 IJustGotHit, 12 IJustGotHitDragRace, 13 AccidentRecovery,
  15 HitWallDragRace, 16 StopDriving, 17 ObeyTrafficIntersections, 18 PickRouteToAvoidAnimation, 19 DriveAtOffset,
  20 RollingRoadblock (ctor push before 0x4234a0).
- DriveTrafficLane execute 0x4242d0: lane path from +0x30c (0x420ed0 / 0x420d40 depending on goal), speed
  +0x1f0 = controller vt+0x7c (<= 55 mph), +0x1f8 = 2.0 (0x783724), +0x1fd = +0x1fe = 1.
- Mover switching (0x429d90 tail @0x42a004): when the chosen action is 1, 2 or 8 (avoid / avoid-as-traffic /
  pull over) and the car has collision ground (+0x5cc) and mover == 2 -> mover 3 (MellowMover, simplified physics).
  Being hit also goes through slot 14 -> mover 3. Back to the kinematic mover through slot 5 (events 3/4) or a
  respawn (0x42c950 / 0x409770 -> mover 2). Parked: mover 0 (0x409960, 0x4099b0).

## 6. TrafficMover (mover type 2, ctor 0x408740, vt 0x78456c, 0x24 B) - kinematic
State: +0xc,+0x10 = x,y (car+0x60/+0x64), +0x14 heading = car+0x40c (u16) * 2pi/65536, +0x18 speed (car+0x430).
Update 0x408820(dt):
- Steering angle = input steer (0x5ebf50, u16 -> rad, wrapped to +-pi). If |steer| < 0.01*L (L = wheelbase from
  physics data rb+0x10 - rb+0xa0): straight move pos += speed*dt*(cos,sin)(heading); else rotate about the turning
  centre (0x43d090) and heading += turn (bicycle model; exact radius formula UNRESOLVED).
- Speed: v += (throttle(input+0x20c) * A(v) - brake(input+0x214) * D(v)) * dt, clamp to [0, 24.5865].
  A = 4-point table 0x7ea03c over 0..24.5865 m/s: 3.576, 2.682, 2.012, 1.341 m/s^2 (8/6/4.5/3 mph/s);
  D = 0x7ea050: 6.705, 13.41, 20.12, 26.82 m/s^2 (15/30/45/60 mph/s). Interpolation 0x5764d0.
- Writes body pos (z kept), velocity, yaw (car+0x3cc u16), yaw rate +0xd8, steer +0x3ce; ground snap 0x5f5010.

## Unresolved
- Road segment layout (seg +0x2c per-level class, +0x34 lane weight, +0x3c region, lane speed bytes, direction bits).
- ObeyTrafficIntersections (id 17, intersections chunk 0x34123: 0x114-byte records, 38 per Routes file) not decoded.
- AvoidAvoidableAsTraffic distances / horn; meaning of +0x799 Nice/Jerk.
