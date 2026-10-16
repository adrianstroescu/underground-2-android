---
name: phone-test
description: Build the APK, install it on the owner's Galaxy S23 Ultra, drive the game through its menus with adb taps, run races with the autopilot, take screenshots and read the logs. Use whenever a change has to be checked on the phone or the owner asks for screenshots.
---

# Testing on the phone

The phone is a Galaxy S23 Ultra (SM-S918B), serial `R3CW9021WWL`, screen 2316x1080.

## Setup

```bash
export ANDROID_SERIAL=R3CW9021WWL MSYS_NO_PATHCONV=1
adb devices                                   # "device", not "unauthorized"
adb shell dumpsys window | grep mDreamingLockscreen   # true = locked
```

- The phone has a PIN, so only the owner can unlock it. Ask, then wait with a
  background loop until `mDreamingLockscreen=false`. **Never tap while it is
  locked**: taps would land on the PIN pad.
- With the owner's OK, keep the screen on while on USB with `adb shell svc power
  stayon usb`. Undo it with `svc power stayon false`.

## Build and install

```bash
source <scratchpad>/env.sh          # changes cwd to the repo
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-apk.ps1 -Fast -Install
```

- Use `-Fast` (about 50 s) while iterating. Do a full release build (LTO) for
  milestones.
- The game data must already be on the phone. `scripts/push-game-data.ps1` copies the
  regions L4RA, B, C, D, F and G.

## Launch and navigate

```bash
adb shell am force-stop com.adrianstroescu.ug2; adb logcat -c
adb shell monkey -p com.adrianstroescu.ug2 -c android.intent.category.LAUNCHER 1
sleep 8
```

Tap coordinates are full resolution, which is 2× the coordinates in the half-size
screenshots:

- **Title:**
  - CAREER (1158, 260)
  - QUICK RACE (1158, 404)
  - FREE ROAM (1158, 546)
  - CAR arrows (1054 / 1826, 686)
  - GARAGE (1158, 828)
- **Quick Race:**
  - MODE right arrow (1826, 244). Modes cycle Circuit → Sprint → Drag → Drift →
    Street X → URL.
  - START RACE is at (1156, 864) for Circuit/Sprint (rows: Mode, Track, Laps,
    Opponents, Car). It is at (1156, 828) for Drag and Drift, which have one row fewer.
- **In game:** the pause button is at (94, 94). In the pause menu, CAR left (1054, 332)
  and QUIT TO MENU (1156, 982). The BACK key also pauses.
- Rows move when menus change, so take a screenshot and check before tapping
  blind.

## Autopilot (testing aid)

```bash
F=/sdcard/Android/data/com.adrianstroescu.ug2/files/ug2.cfg
adb shell "echo 'autopilot = 1' >> $F"                    # before launching
adb shell "sed -i -e '/^autopilot/d' -e 's/^car = .*/car = 3000GT/' $F"   # after (app stopped)
```

- The computer drives the player's car along the race line, shifting in drags. It
  resets itself after 3 s stuck.
- Watch the log: if `speed` stays near 0 or `wall <owner>` repeats, **force-stop at
  once**. The owner may be watching.
- The owner's car is the 3000GT. Put it back if a test changed it.

## Screenshots and logs

```bash
SHOT_DIR=<scratchpad> bash tools/phone/shot.sh name   # name.png + half-size name.jpg
adb logcat -d -s ug2 | grep "car pos" | tail           # once a second: pos, speed, race, wall owner
adb logcat -d -s ug2 | grep -iE "loaded|race|visible|collision|panic"
```

- Look at the `.jpg` with Read. Send the useful ones to the owner with SendUserFile
  and a one-line caption in Romanian.
- The FPS and draw-call counter is at the top of the screen. Bayview runs at about
  120 FPS.

## When done

- `adb shell am force-stop com.adrianstroescu.ug2`.
- Remove `autopilot`.
- Tell the owner when the phone is no longer needed.
