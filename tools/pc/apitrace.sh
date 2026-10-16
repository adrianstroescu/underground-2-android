#!/bin/bash
# Records the original game's Direct3D 9 calls with apitrace (installed in
# "D:/GAME install/apitrace"), to read the values it sets at run time: shader
# constants (fog, headlight, car lighting), render states, textures.
#
#   apitrace.sh on    put apitrace's d3d9.dll next to speed2.exe (the next launch records)
#   apitrace.sh off   remove it again (recording slows the game and writes big traces)
#   apitrace.sh dump TRACE [apitrace dump options...]   e.g. --calls=100-200
#
# The trace is written next to speed2.exe as speed2.trace (speed2.1.trace, ...).
APITRACE="${APITRACE:-/d/GAME install/apitrace}"
GAME="${UG2_GAME_DIR:-/d/GAME install/Need for Speed - Underground 2-NEW}"
case "$1" in
  on) cp "$APITRACE/lib/wrappers/d3d9.dll" "$GAME/d3d9.dll" && echo "tracing on" ;;
  off) rm -f "$GAME/d3d9.dll" && echo "tracing off" ;;
  dump) shift; t="$1"; shift; "$APITRACE/bin/apitrace.exe" dump "$@" "$t" ;;
  *) echo "usage: apitrace.sh on|off|dump TRACE [options]"; exit 1 ;;
esac
