# NFSU2 career start, car lot and money (speed2.exe RE, static only)

Builds on career_re.md / career2_re.md. Dumpers: `lot_dump.py` (CarTypeInfo flags, car unlock table 0x34A1F,
bonus zones 0x34A1E). Hash = bh(): h=-1; h=h*33+c (0x43db50; 0x505450 is the same with a-z uppercased).

## 0. Key globals (career object 0x85ad40)
| addr | career+ | meaning |
|---|---|---|
| 0x861e74 | 0x7134 | cash |
| 0x86347c | 0x873c | intro done: set to 1 when the Scene01 movie ends (0x54afca) |
| 0x86347d | 0x873d | **player owns a car** (set 1 in 0x541540 / 0x500bd0, cleared by new career) |
| 0x863480 | 0x8740 | hash of the current career car slot ("CAREER_SLOT_n"); new career sets bh("CAREER_0") = no slot |
| | 0x8734 | current shop record, 0x8738 current event (race record) |
| | 0x8768[64] | bonus-zone "collected" flags (0x34A1E) |
| 0x83ad90 (+0xab4c per player) | | garage. Stock-car table +0x00 (48 x 0x1c, count +0x540), custom +0x544 (20 x 0x434), **career slots +0x5954 (5 x 0x7f8, count +0x812c)**, sponsor +0x8130 (6 x 0x438), pointer table +0x9bd8 (91 ptrs: 48 stock, 20 custom, 5 career, ...) |

Career slot record (0x7f8 B, absolute slot0 = 0x8406e4): +0 vtable 0x79b12c (vfunc+4 = car type index from +0x20),
+8 name hash "CAREER_SLOT_%d" (0x797e74), +0x10 valid=1, +0x14 category flags = 4 (career), +0x18 car config
(+0x20 = CarTypeInfo hash), +0x434 installed parts (cleared by 0x503e70), **+0x7f0 money spent on parts for this car**
(added by 0x521b00 / 0x547570 / 0x54fe50, = parts +0x3bc), **+0x7f4 slot LOCKED**, **+0x7f5 slot HAS A CAR**.
Lookup by hash 0x503680; iterate by category 0x5162d0(flags, prev).

## 1. From "Start New Career" to the crib (certain unless marked)
1. UI_StartCareer "Start New Career" -> profile flow -> 0x532830 (career reset, arg=1):
   cash = 0x576090() = promo bonus only ([0x86512c] 200 + [0x865130] 1000, both 0 unless promo codes) -> **$0**;
   has-car = 0; current slot = bh("CAREER_0"); 0x516480 resets the 5 career slots (locked=1, has-car=0, parts cleared);
   then **opens the first locked slot** (0x5329cc: first slot with +0x7f4 != 0 -> 0). So: 1 usable slot, empty.
2. 0x495a30: DDAY_EVENT_B not done and intro not done -> 0x53d710 queues movie "scene01" (mode 6, IG_PlayMovie.fng).
   **No intro race / no prologue race** exists in code or data: the only race records with a forced car are below.
3. Scene01 ends (0x54af97): start DDAY_EVENT_A (B if A done) via 0x5410e0 + 0x5404a0, intro-done=1.
4. **DDAY_EVENT_A** (race 0x34A11: type 3 drive-to, track 4000, target shop hash +0x3c = CC_CAR_LOT_1, reward $0):
   race **+0x2c = player car preset hash = bh("DDAY_PLAYER_CAR") = 0xf7e19102**. 0x528600 (career player car):
   if the current race (+0x8730) or current event (+0x8738) has +0x2c != 0 -> car = preset 0x637270(hash);
   else the car in slot [0x863480]. Preset DDAY_PLAYER_CAR (GLOBALB chunk 0x30220, 32 presets x 0x338 B, model at +8,
   name at +0x28) = **Nissan 350Z ("Rachel's car")**. DDAY_EVENT_A is the only race with +0x2c set.
   (Also: UI_Main shows DDAY_PLAYER_CAR as the background car while has-car == 0, 0x4df1d7.)
   Free roam in stage 0 loads FREE_ROAM_STAGE0A while has-car==0, FREE_ROAM_STAGE0 after (0x578226).
   In stage 0 with no car only shops of type 4 (car lots) are available (0x533134).
   Unused presets: DDAY_PLAYER_CAR_OLD (Lancer Evo 8), DDAY_PLAYER_CAR_OLD_RX8.
5. Entering a shop 0x499640 -> 0x499470 -> 0x54caf0: if the active event targets this shop (0x4ffde0) it is won
   (win counter, 0x541420 pays race +0x30 = $0); DDAY_A -> immediately starts DDAY_EVENT_B; DDAY_B -> event cleared.
   Shop movie (shop +0x20) plays only on the first visit (0x500460 visited check): CC_CAR_LOT_1 -> **SCENE05**.
6. UI_CareerCarLot (mode 2 below): pick a stock car -> "Do you want this stock %s?" -> Yes = **free** first car.
   Back without a car -> "Rachel needs her car back. You have to select a new car before you leave." (OK only;
   checked on CAREER_SLOT_0 +0x7f5, 0x4ede54). After buying, autosave (if enabled) then exit to free roam.
7. **DDAY_EVENT_B** (prereq A, target CRIB_1, no preset -> your new car). Entering CRIB_1 plays **SCENE06** (first visit),
   completes B -> crib menu; CC_* shops unlock (shop +0x74 = DDAY_EVENT_B). Movies in order: scene01, SCENE05, SCENE06
   (FMVOpening is the boot movie, not part of the career).
   SMS around it (strings): "Hey man, I want my car back NOW!... Get to the car lot" / "Now that you've got a set of
   wheels, head over to my garage..." (SMS table 0x34A17; exact trigger rows not re-dumped here).

## 2. Car lot UI_CareerCarLot (factory 0x7f80f4 -> 0x4fc0b0 -> 0x4f9930, vtable 0x795cf0, handler 0x4ed9e0)
### Inventory (0x4df9c0(filter=1), 0x513c50, 0x4aff40) - all car lots identical
Iterate garage stock-car entries (category 1, built by 0x516500 = one "STOCK_<name>" entry per CarTypeInfo record with a
model (+0x40), +0xDA bit1, and +0x844 == 0 (traffic +0x844==2 only with debug flag [0x838470])). A car is listed iff:
- CarTypeInfo +0xDA bit1 set; model file exists (0x610180);
- type index (+0x840) not in the region exclusion list 0x5022c0: NA ([0x864f24]==0) excludes {7 CORSA, 6 PEUGOT106},
  ROW excludes {5 CIVIC, 0x12 RSX} (table 0x7f7c48 / 0x7f7c50; all allowed with cheat [0x838464]);
- not locked by **chunk 0x34A1F** (GLOBALB 0x80034A10 index 14, 24 x 12 B {u32 car hash (= CarTypeInfo +0xD0),
  u32 race hash NA, u32 race hash ROW}): locked while that race is not won (hash 0 / not found = never locked).
- per-lot check 0x500bf0 is a stub returning true -> CC_CAR_LOT_1 (stage 0), UC_CAR_LOT (stage 2), IND_CAR_LOT (stage 4)
  all show the same list. Order = CarTypeInfo index order (sort 0x43e650 with comparator 0x40cb20 = always true = no-op).

Car unlocks (0x34A1F, NA column; ROW in brackets where different):
| car | won race | car | won race |
|---|---|---|---|
| ESCALADE, NAVIGATOR | S2_URL_1 | HUMMER | S2_URL_2 |
| TIBURON | S2_URL_2 [S2_URL_3] | SENTRA | S2_URL_3 |
| CELICA | S2_URL_3 [S3_URL_4] | MIATA | none [S2_URL_2] |
| IS300 | S3_URL_4 [S3_URL_5] | SUPRA | S3_URL_5 [S3_URL_6] |
| GOLF | S3_URL_6 [S3_URL_7] | A3 | S3_URL_7 |
| RSX | S3_URL_7 [none, but ROW-excluded] | ECLIPSE, TT | S3_URL_8 |
| RX8 | S4_URL_1 | 350Z, RX7 | S4_URL_2 |
| G35 | S4_URL_3 | 3000GT | S4_URL_4 |
| GTO | S4_URL_5 | MUSTANGGT | S4_URL_6 |
| LANCEREVO8 | S4_URL_7 | SKYLINE | S5_URL_1 |
| IMPREZAWRX | S5_URL_2 | | |
**Starter choice (new career, NA): PEUGOT (206), FOCUS, COROLLA, 240SX, MIATA, CIVIC** (in this order).
ROW: PEUGOT, FOCUS, COROLLA, 240SX, PEUGOT106, CORSA.
[0x864f24] (NA=0 / ROW!=0, also picks GLOBALTEXTURESNA/ROW.BIN at 0x57fc4e) has no direct writer in the exe -> 0 = NA on
this PC build (UNRESOLVED whether something writes it indirectly).

### Modes ([this+0x4c]; ctor sets 1, then 0x4ed8a0(0x495f00(1)))
0x495f70 counts career slots: U = unlocked (+0x7f4==0), E = unlocked and empty (+0x7f5==0).
0x495f00(1): E>0 -> 2; else U==1 -> 3; U>1 -> 4; else 1. Next mode after an action: 2->6, 3->6, 4->5, 5->6.
| mode | accept on a car (msg 0x0c407210/0x406415e3) | Yes |
|---|---|---|
| 2 free car | "Do you want this stock %s?" Yes/No | 0x541540: has-car=1, current slot = CAREER_SLOT_<first unlocked empty slot> (0x495fd0), 0x5348e0 builds stock car there (+0x7f5=1). Mode 6. |
| 3 one slot | "Do you want to trade-in your %s ^for this stock %s?" Yes/No | 0x534950 on the CURRENT slot: parts cleared (0x503e70, incl. +0x7f0), car type replaced. Mode 6. |
| 4 several slots | "...for this stock %s?^Or do you want to choose one of your other cars?" buttons "Trade this car?" (0x526adefc) / "Browse My Other Cars" (0x75d1e35e) | 0x526adefc: trade the current car as mode 3, then mode 6. Browse: remember the stock car, mode 5 = list own career cars (filter 4, FILTERTYPE "Career Cars"). |
| 5 browsing own | "Do you want to trade-in your <own car> for this stock <remembered car>?" | 0x534950 on that slot; mode 6. |
| 6 | - | autosave 0x4a3100(0x141) if enabled (0x52a4c0) else post 0x9a3b571d -> 0x5404a0 back to free roam. |
Back (0x911ab364): CAREER_SLOT_0 has a car -> "Are you sure you wish to exit back to Explore Mode?" (Yes -> 0x51123592
-> exit), else the "Rachel needs her car back" OK box.
**Money: none.** No code path in the lot reads or writes cash; no price, no trade-in value, no cash check.
The strings "You'll lose all the mods, and you'll only get 80% of its cost" (0xb18c1000, 0xe1417aca), "You want to trade
for a car?" etc. have no code reference (leftovers). The FNG shows only player name and cash ($999999999 placeholder),
stat bars ACCELERATION/TOP SPEED/HANDLING, help "Select Car" (help bar 0xc3cabafd: Switch Cars / Rotate Car / Select Car /
Exit to Explore Mode).
Trading loses all installed parts of that car (and its +0x7f0 parts value) with no refund.

### Number of cars (career slots)
Max **5** (garage +0x5954 x5). Unlock one more locked slot (first with +0x7f4 != 0) at: new career (slot 0); each sponsor
signing (0x532cf0 and 0x541680 when the stage has no sponsor yet; together with the signing bonus) -> stages 2..5 give
4 more. An unlocked empty slot is filled for FREE at any car lot (mode 2). Crib Car Select (0x4dfc50/0x4e0180) shows all
5 slots: locked -> "You'll have to win some more races!", empty -> "Pick up your FREE sponsor car^ at the local car lot.",
current -> "You're already driving your %s!", other -> "Do you want to drive your %s instead of your %s?".

## 3. Car prices
**There are no car prices in UG2 PC career.** Cars are never bought with cash (first car and sponsor-slot cars free,
otherwise trades). CarTypeInfo (GLOBALB 0x34600, 46 x 0x890) fields used here: +0x00 name, +0x40 geometry path,
+0xC0 manufacturer, +0xD0 hash, +0xDA flags (bit1 = player car), +0x840 type index, +0x844 0 player / 2 traffic,
+0x88A SUV. No price field is read by any career/lot code found. The only per-car money value is the slot's +0x7f0
(sum of part prices paid for that car), shown on a status screen (0x4e1520) and lost on trade.

## 4. Other money sources/sinks (not in career2_re.md)
- **Hidden bonus zones (0x528370 / 0x5283c0)**: free-roam world triggers of type 0x13 at the player position (0x5eb040,
  career only). Trigger +0x30 hash matched against **chunk 0x34A1E** (GLOBALB index 13, [0x838430]/[0x838434], 59 x 12 B
  {u16 cash, u8 cond, u8 id, u32 param, u32 trigger hash}). Pay if cond 0 (always) / 1 (current stage > param) /
  2 (race param won) (0x513890) and not collected (career+0x8768[id]); then cash += u16, flag set, 0x5141b0 rebuilds the
  zone list, HUD popup 0x4ac1b0 (value). Table: id 0-1 $50 always; id 2-14 $50 stage>0; 15-28 $50 stage>1;
  29-36 $100 stage>2; 37-48 $200 stage>3; 49-58 $400 stage>4. Total $8650. Trigger names not found in the data
  (hashes 0x215a1b6e..0x215a1c1b look like a common prefix + 2 digits); the HUD label is probably "SHORTCUT" or
  "BONUS ZONE" (UNRESOLVED).
- **Engage-tip SMS cash (0x532d40)**: world triggers of type 0x12 -> SMS with trigger byte 15 whose param = trigger
  hash, first time only (0x514cb0) -> delivered and cash += SMS +0xC. ENGAGE_TIPS_* rows 125..162 $25 each (38),
  163..166 (FILTER, HIDDEN, TRLBRK, ORGANIC) $50 each. Total $1150.
- Parts: 0x521b00 / 0x54fe50 (packages, price from 0x5415e0) / 0x547570 (visual) / 0x528a0b: cash -= price, clamp 0,
  and the car's +0x7f0 += price. Purchase checks "You do not have enough bank to purchase this item." (0x5592bc etc.).
- Cheat [0x838440] (0x551320): cash = 1,000,000. Promo codes 0x5760a0: type 2 -> $200, type 4 -> $1000 (added at once
  only when the intro is not done and cash == 0; otherwise remembered for the next new career).
- Sponsor signing (0x532cf0 / 0x541680): cash += sponsor +0xC AND one career slot unlocked.

## Unresolved
- Whether [0x864f24] (NA/ROW) is ever set (no direct writer; assume NA on PC).
- Names / HUD text of the 0x34A1E bonus zones; where their triggers live (not in TRACKS/*.BUN as plain hashes).
- SMS rows that announce new car lot inventory (e.g. "New Inventory in the Car Lot") were not mapped to triggers.
