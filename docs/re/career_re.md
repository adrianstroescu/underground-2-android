# NFSU2 career flow (speed2.exe RE notes)

## Career data: GLOBALB.BUN container 0x80034A10 (loader 0x512c00, jump table 0x512ddc)
- 0x34A1D string pool -> [0x838428]. All u16 "name" fields below are offsets into it.
- 0x34A11 races, 136 B -> [0x8383e0]/count [0x8383e4] (224).
  +0 u16 name, +2 u16 PRE-race movie, +4 u16 POST-race movie, +6 u16 locator (EV_xx / EVENT_LOCATOR_xx),
  +8 hash, +0xF type (0 circ,1 sprint,2 streetX,3 "drive to"(DDAY),4 drag,5 drift), +0x10 laps / prereq hash,
  +0x18 track, +0x30 reward, +0x34 kind (2 = stage boss?), +0x37 stage (0..5), +0x40.. opponents (u16 names).
- 0x34A12 shops, 160 B -> [0x8383f0] (24): name[32], movie[24] @0x20, hash @0x38, model BIN @0x40,
  type u8 @0x50 (0 crib,1 graphics,2 body,3 perf,4 car lot,5 specialty,6 uniques), unlock-after hash @0x74, stage @0x9c.
  CC_CAR_LOT_1 movie SCENE05; CRIB_1 movie SCENE06 (unlock after DDAY_EVENT_A); other CC shops after DDAY_EVENT_B.
- 0x34A14 brands 68 B (35). 0x34A15 opponent groups 0x17C (0x500000). 0x34A16 DVD/magazine covers 64 B (30).
- 0x34A17 SMS 20 B -> [0x838454]/[0x838450] (167): u16 name, u8 trigger, u8 category, u16 flags, u16, u32 param, u32 ?, u32 sender hash (RACHEL 0x770af86e).
- 0x34A18 stage configs 80 B (6, stage 0..5): u8 stage, u8 nSponsors, ..., +8 sponsor hashes[5], +0x1c i16 thresholds[5].
- 0x34A19 sponsors 16 B (20): u16 name, u16 URL-victory bonus, u8[3] required race types, u8, u32 brand hash, u16 signing bonus, i16 requirement.
- 0x34A1B magazine/DVD triggers 24 B; 0x34A1C unlock conditions 40 B -> [0x83845c] (checker 0x515360, types 0 race done,1 URL wins,2 counterB,3 races won).
- Per-stage runtime record 0x104 B at 0x862018 + stage*0x104 (current stage [0x862838]):
  +0..0x88 won-race slots (count 0x5004d0), +0x8c..0x98 URL slots (0x5005a0), +0x9c.. (0x500520),
  +0xe0 float sum / +0xe4 count (0x5005e0 average), +0xe8 chosen sponsor, +0x100 -> stage config.
- Career object 0x85ad40: 0x500b80(hash) = race completed. SMS inbox at +0x1400 (count), 0x14 per entry.

## New / resume career
Career menu 0x4af110 (UI_StartCareer): if a career exists (0x83a9e0.0x502fe0):
  [0x86347c] ? "Resume Career"(0xe072db21) : "Start New Career"(0x97b1dcda) -> 0x495a30; then "Start New Career" -> 0x4a3100(0x21961) (profile flow) when started;
  else only "Start New Career" -> 0x4a3100(0x21961). Always "Load Game"(0xaa3f09d8) -> 0x4a3100(0xc11).
0x495a30: [0x850078]=1 (career mode).
  - DDAY_EVENT_B (0xdd60e403) done -> UI_StartCareer -> UI_CareerCrib, [0x836324]=1 (crib menu).
  - else if ![0x86347c] -> 0x53d710: movie queue "scene01" (mode 6) -> IG_PlayMovie.fng; at end (0x54af97)
    start DDAY_EVENT_A (or B if A done) via 0x5410e0 + 0x5404a0, [0x86347c]=1.
  - else start DDAY A/B directly.
DDAY_EVENT_A: type 3, track 4000, target CC_CAR_LOT_1 (hash 0xec17e618) -> drive to car lot.
  Entering CC_CAR_LOT_1 plays SCENE05 (0x4995a0 -> 0x557df0 mode 2) then the car lot (buy first car).
DDAY_EVENT_B: prereq A, target CRIB_1 (0x55446b8f) -> SCENE06 on entering the crib.
Starting cash: not found in code (port uses 6000, unverified).

## Movie queue 0x8382a0 (4 x 64 chars), mode [0x838568], end handler 0x54af80 (table 0x54afe8)
- mode 1 pre-race 0x53d630 (caller 0x499b8f when engaging a race whose +2 is set): race +2 movie; if "SCENE20" also "@DriftTut". End -> 0x499b10 start race.
- mode 2 shop 0x557df0: shop movie @0x20. End -> 0x499560.
- mode 4 post-race 0x53d7a0 (from 0x4a3e30 after 0x54c9d0/0x513a50): name from 0x513a50:
  race +4 if set and race won; else if stage advanced and new stage==2 -> "Scene07" (+ "UpperClass");
  else S5_CIRCUIT_36 (0x4cce3e1): won -> "Scene15", lost -> "Scene16". End -> game state 0xb.
- mode 6 new career "scene01".
Race movie data: STAGE_1_DRIFT_1 pre SCENE20(+DriftTut); STAGE_1_DRAG_1 pre DRAGTUT; STAGE_1_SHORT_1 pre STREETXTUT;
S2_URL_1 pre SCENE17 post SCENE08; S2_URL_3 post SUBURBAN; S3_URL_4 pre SCENE18; S3_URL_6 pre SCENE09;
S3_DRIFT_7 pre ORGANICSTUT; S3_URL_8 post INDUSTEAST; S4_URL_4 pre SCENE10; S4_URL_7 post INDUSTWEST;
S5_URL_4 pre SCENE11 post SCENE12; S5_URL_8 pre SCENE13; S5_CIRCUIT_36 pre SCENE14 (post Scene15/16 hardcoded).

## Crib menu 0x4af820 (UI_CareerCrib), handler 0x4af3a0
Return to Explore Mode (0x194efa7a) dialog "Return to Explore Mode?" -> free roam (0x4af360/0x4af607, autosave 0x4a3100(0x151)).
World Map (0x9c07ca52) only if stage [0x862838] > 1 -> UI_CareerWorldMap.
Car Select (0x451879e2) -> UI_CareerCarSelect. Change Parts (0xabb24bbc) -> 0x552cc0 customize.
Performance Tuning / Dyno (0x8513f2c0) dialog 0xb10ae56f -> 0x495af0. View Rewards (0xce62c0bf) -> UI_CribRewardOptionsMain.
Save (0x3ec7395) -> 0x4a3100(0xc41). Back to Main Menu (0x46e7d84) dialog "Quit to Main Menu?" -> UI_Main.
Back key: [0x836324] ? quit dialog : explore dialog.

## World map (ctor 0x4ee690, vtable 0x795d10, handler 0x4e0f60, populate 0x4cbf80, list 0x4b1360)
Top scroller "Stage %d" (0x0a96b270) for each unlocked stage; event list = races with +0x37 == stage (stage 1 also stage-0,
minus DDAY A/B), max 70; panel: Race Status (completed), Race Type, Distance (track +0x98), Previous Time (best from 0x844adc).
Accept -> "Would you like to race this event?" (0xec9488bb) Yes -> 0x51123592: [0x837b6d]=1, jump straight into the race.

## Sponsors
Post-race chain 0x4f3f20: ... UI_PostRaceReward, then if 0x4a4610 -> UI_SponsorPopup ("SELECT A SPONSOR!") + UI_Sponsorship_new, then UI_PostRace.
0x4a4610: career, stage>1, no sponsor for stage yet (+0xe8==0), and >=1 sponsor of the stage qualifies
(won races in stage >= threshold[i] and sponsor requirement <= previous stage average (+0xe0/+0xe4)).

## SMS
Trigger byte: 1 race done (param hash), 2 arrival at shop (param = shop hash), 3 timer in free roam (param s),
4/5/6 outrun intro/victory/defeat, 9 URL wins in stage>=n, 10 counterB>=n, 12 races won>=n (param = stage<<16|n),
13 magazine/unique (from 0x34A1B), 14 DVD cover n, 15 engage-dialog tips.
Delivered in free roam (0x529750 timer, 0x529860 shop arrival, 0x529920 race end), popup 0x514c70; read in UI_SMS_Mailbox.
