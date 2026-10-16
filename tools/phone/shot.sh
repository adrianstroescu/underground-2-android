#!/bin/bash
# Phone screenshot: OUT_DIR/name.png (2316x1080) and a half-size name.jpg to view.
# usage: shot.sh name [OUT_DIR]   (needs ANDROID_SERIAL, adb, python with Pillow)
export ANDROID_SERIAL=${ANDROID_SERIAL:-R3CW9021WWL} MSYS_NO_PATHCONV=1
S="${2:-${SHOT_DIR:-.}}"
adb exec-out screencap -p > "$S/$1.png"
python -I -c "
from PIL import Image; im=Image.open(r'$S/$1.png').convert('RGB'); im.thumbnail((1158,540)); im.save(r'$S/$1.jpg',quality=80)"
echo "$S/$1.jpg"
