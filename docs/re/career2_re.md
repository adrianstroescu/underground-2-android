# NFSU2 career, part 2 (speed2.exe RE). Corrects career_re.md where noted.

Career object 0x85ad40: cash +0x7134 (=0x861e74), stage +0x7af8 (=0x862838), stage records +0x72d8 (=0x862018, 8 x 0x104),
race-state table +0x7afc count / +0x7b00 entries 8 B {u32 hash, u8 +4 seen/engaged, u8 +5, u8 +6 status (1 won, 2 tried, 3 ?)}.
Available-race list object +0x7148: +0x80 64 race ptrs, +0x180 per-category counters.

## Race record 0x34A11 (136 B) – corrected/extended
+0 name, +2 pre movie, +4 post movie, +6 locator, +8 hash = bh(name) (h=-1; h=h*33+c).
+0xC unlock type: 0 = prerequisite hash in +0x10 (data only, not checked except S5_CIRCUIT_36), 1 = normal pool race,
     3 = URL: unlock when regular wins in stage >= u16 +0x12, 4 = stage-final URL: regular wins >= u8 +0x11,
     sponsor wins >= u8 +0x12, URL wins >= u8 +0x13 AND all required DVD covers of the stage done (0x513510, 0x513490).
+0xD 1 = SUV race (only offered when the current car is an SUV, car +0x88a; not counted in any win counter).
+0xF type 0 circuit,1 sprint,2 streetX,3 drive-to,4 drag,5 drift.
+0x18 heats[ +0x7E ] 4 B {u16 track, u8 flags (bit0 -> heat+4, reverse?), u8 laps}.
+0x28 hash (unused here). +0x30 u32 cash prize. +0x34 kind: 0 story, 1 sponsor, 2 URL, 3 regular. +0x36 1 normal / 2 drift-scored.
+0x37 stage. +0x40 opponents[+0x7C] 12 B: u16 driver name, u8 mode, u8 level, u32 car preset hash (0x7710f0e0 = bh("RANDOM")),
     u8 skill[3] indexed by AI-skill setting [0x89e7f4] (career copies it from [0x83ac30] in 0x5322e0), u8.
+0x7F 1 = hidden race (S?_H_*).

## 1. Chunk 0x34A1C = PART unlock table (not races/shops/SMS)
86 x 40 B: u32 id (1..86), then 3 levels x 12 B {i16 +0 value (0x501860), i16 +2 PRICE (0x514280, paid at 0x528a0b),
u8 +4 cond type, u8 +5 level 1..3, u32 +8 param}. Index = 0x501d70(car, part); level = part[+5]>>5.
Checker 0x515360(eax=id, edi=level): type 0/5/6 -> true (6 = always), 1 -> shop/marker hash found (0x4ffe20 -> 0x513110),
2 -> regular wins (0x5004d0) >= param&0xffff in stage param>>16, 3 -> URL wins (0x500520), 4 -> sponsor wins (0x5005a0).
id 0x44 special (0x579d60, promo code), cheat [0x838441] unlocks all. 0x34A15 (30 x 380 B) = the matching performance
packages {u32 category, u32 level, u32 n, n x 92 B items {id, hash, price ...}}, purchased bits at profile+0x590. NOT opponents.
Shop unlock (0x512870) uses shop +0x9C type (0 always,1 race hash +0x74 won,2 URL wins,3 regular wins,4 sponsor wins >= +0x74 in stage +0x9D).

## How races become available: 0x527d30 (rebuild list; called 0x5291e4, 0x53331b, 0x5417e0)
- stage 0 and !career+0x873f, or DDAY_EVENT_B current -> empty. Stage 5 final S5_CIRCUIT_36 (0x04CCE3E1): once its prereq
  S5_URL_9 is won and it is not, it is the ONLY race. If stage-5 final won -> empty (career done).
- 0x34A1B triggers for the stage (0x527970).
- keep previous list entries of this stage that are not won.
- per category 0..10 (filter 0x513600): 0-4 regular (kind 3, non-SUV) circuit/drag/drift/sprint/streetX, 5 SUV, 6-10 sponsor
  circuit/drag/drift/sprint/streetX. Cap 0x527aa0 from stage config: +0x30 circuit,+0x31 drag,+0x33 drift,+0x34 sprint,+0x32 streetX,
  +0x37 SUV; sponsor caps = count of chosen sponsor's types (sponsor codes 1 circuit,2 drift,3 drag,4 sprint,5 streetX) minus sponsor
  races of that category already won (0x5137c0). While count < cap: 0x527bb0 picks a RANDOM (0x43c1c0 shuffle) not-won race of
  the stage, kind != 2, +0x7F == 0, matching category. => no fixed order inside a category.
- hidden races (+0x7F) of the stage, not won, added while hidden-in-list < cfg +0x41 (0x500880); engaging one first time
  (0x512f70) sets state+4 and posts the "found" notice (0x500e60 / 0x4eccf0).
- URL races (kind 2) of the stage, not won, added when 0x513510 passes.
Win counters (0x513390 on win): kind 3 non-SUV -> 35 slots +0x00 (0x5004d0 "regular wins"); kind 1 -> 4 slots +0x8C (0x5005a0
"sponsor wins"); kind 2 -> 10 slots +0x9C (0x500520 "URL wins"). (career_re.md had 0x5005a0/0x500520 swapped.)

## Stage config 0x34A18 (80 B), extended
+0 stage, +1 nSponsors, +2 u16 outrun stake (win +, loss -, 0x541320: 100,100,100,100,250,400), +4 u16 cover multiplier,
+8 sponsor brand hashes[5], +0x1C i16 sponsor min stage wins[5], +0x28 final race hash, +0x2C u8[4] UI totals (races,sponsor,URL,DVD),
+0x30..0x37 category caps, +0x41 hidden cap (-1 none), +0x42 i16 wins for full AI ramp, +0x44/+0x48 f32 AI level ramp start/end.

## 2. Stage advance: 0x532c60 (per-stage record), called after a win (0x54150f) and after a cover (0x5410b4/0x5410c8)
cfg+0x28 != 0: final race won (0x500b80) AND all DVD covers (0x34A16 +0x35 == 1, +0x25 == stage) done (0x513490) ->
record +0xFC = 1, 0x528320(stage): [0x862838] = stage+1, timer event 0xC (60000) on 0x8635bc once ([0x863661]).
cfg+0x28 == 0 (stage 1): every race with +0x37 == 1 won (0x5130a0).
Finals: stage0 DDAY_EVENT_B, stage1 all 5 STAGE_1_*, stage2 S2_URL_3, stage3 S3_URL_8, stage4 S4_URL_7, stage5 S5_CIRCUIT_36.
Effects: free-roam barrier set = "BARRIERS_CAREER%d" with %d = stage (0x578270, 0x5f554d; stage 0 uses FREE_ROAM_STAGE0/0A);
post-race movie 0x513a50 (Scene07+UpperClass on 1->2; region movies are the finals' +4 post movies SUBURBAN/INDUSTEAST/INDUSTWEST).

## 3. Money
Win only (0x541420, from 0x54c9a9 when 0x5287a0(1) says player won): state=1; cash += 0x532e50 = (kind 1 and stage sponsor chosen)
? sponsor +2 : race +0x30. No per-place money; losing (0x528500) pays nothing. Sponsor signing: cash += sponsor +0xC (0x532cf0).
Covers (0x541210): cash += cover +0x26 (500) + round(cfg+4 * f(0x528410) + 0.005). Outrun: +/- cfg+2.
Reputation: 0x512b50(margin, race): margin from 0x513fe0 = (t2/t1 - 1)*100 (heats: points ratio); table 0x7f7ac8 (7 x 28 B
{cat, a,b,c, 100,200,300}): margin < a -> 0, < b -> 100, < c -> 200, else 300 (a,b,c = 1,2,5; drift cat 4 = 10,20,30).
Category 0x512ae0: circuit 1, drag 2, sprint 3, drift 4, streetX 5, URL 6. Summed in stage +0xE0/+0xE4 (0x513430), average vs
sponsor +0xE (0x4a4610).
NEW CAREER CASH: 0x532908 cash = 0x576090() = [0x86512c] + [0x865130]; both are 0 unless promo codes ran
0x5760a0 (type 2 -> 200, type 4 -> 1000). So the base starting cash is $0 (not 6000).

## 4. URL tournaments
Kind 2, 1-3 heats in +0x18 list (+0x7E), 5 opponents. Race instance 0x532ea0 (career+0xd24): <=4 heats of 0xd24 B, <=6 racers.
>1 heat: 0x5287a0 requires all heats finished, winner = 0x528710 = max points 0x513eb0, points per heat place from 0x7f7b88
[_,8,6,4,3,2,1] (1st..6th), ties go to the player. 1 heat: normal finish. Each won URL = one URL win.

## 5. Opponents / AI difficulty (0x5260b0)
For each opponent entry: car = random AI car (0x4feaf0) when hash == RANDOM, else preset 0x637270. mode 0: perf level = byte level;
1: car built by 0x5b7570 (match player); 2: stock; 3: level = 0x5132e0 = lerp(cfg+0x44, cfg+0x48, wins/cfg+0x42) where wins =
regular+sponsor+URL wins of the stage (stage2 0->3 over 16, 3: 3->4 over 28, 4: 4->8 over 40, 5: 8->10 over 47). Level clamped
<= 10, *0.1 -> 0x5b6ad0 upgrade amount. Driver skill = skill[ [0x89e7f4] ] byte (e.g. 0/2/5, 1/3/6, 1/5/8, 2/6/9).
