"""Drives the original PC game (speed2.exe) to take reference screenshots.

usage:
  ug2pc.py launch                 start the game (from the game folder)
  ug2pc.py keys KEY[*N] ...       press keys: enter esc up down left right space
                                  a-z 0-9, f1-f12; KEY*N repeats; wait:SECONDS pauses;
                                  hold:KEY:SECONDS keeps a key down
  ug2pc.py shot NAME [OUT_DIR]    screenshot of the game window -> NAME.png + NAME.jpg
  ug2pc.py close                  end the game

Screenshots are taken only while speed2.exe owns the foreground window, so nothing
else on the desktop is captured.
"""
import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import sys
import time

GAME_DIR = os.environ.get("UG2_GAME_DIR", r"D:\GAME install\Need for Speed - Underground 2-NEW")
user32 = ctypes.WinDLL("user32", use_last_error=True)

SCAN = {
    "esc": 0x01, "enter": 0x1C, "space": 0x39, "tab": 0x0F, "backspace": 0x0E,
    "up": (0x48, True), "down": (0x50, True), "left": (0x4B, True), "right": (0x4D, True),
    "lshift": 0x2A, "lctrl": 0x1D,
}
for i, c in enumerate("1234567890"):
    SCAN[c] = 0x02 + i
for row, start in (("qwertyuiop", 0x10), ("asdfghjkl", 0x1E), ("zxcvbnm", 0x2C)):
    for i, c in enumerate(row):
        SCAN[c] = start + i
for i in range(10):
    SCAN[f"f{i + 1}"] = 0x3B + i
SCAN["f11"], SCAN["f12"] = 0x57, 0x58


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_byte * 32)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


def send_scan(code, up):
    ext = False
    if isinstance(code, tuple):
        code, ext = code
    flags = 0x0008 | (0x0002 if up else 0) | (0x0001 if ext else 0)  # SCANCODE, KEYUP, EXTENDED
    vk = user32.MapVirtualKeyW(code | (0xE000 if ext else 0), 3)  # MAPVK_VSC_TO_VK_EX
    inp = INPUT(type=1)
    inp.ki = KEYBDINPUT(vk, code, flags, 0, None)
    user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


def game_window():
    """Handle of the foreground window when it belongs to speed2.exe, else None."""
    hwnd = user32.GetForegroundWindow()
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    out = subprocess.run(["tasklist", "/FI", f"PID eq {pid.value}", "/FO", "CSV", "/NH"], capture_output=True, text=True).stdout
    return hwnd if "speed2.exe" in out.lower() else None


def press(name, hold=0.08):
    code = SCAN[name]
    send_scan(code, False)
    time.sleep(hold)
    send_scan(code, True)
    time.sleep(0.25)


def main():
    cmd = sys.argv[1]
    if cmd == "launch":
        subprocess.Popen([os.path.join(GAME_DIR, "speed2.exe")], cwd=GAME_DIR)
        print("launched")
    elif cmd == "close":
        subprocess.run(["taskkill", "/IM", "speed2.exe", "/F"], capture_output=True)
        print("closed")
    elif cmd == "keys":
        if not game_window():
            sys.exit("speed2.exe is not in the foreground")
        for k in sys.argv[2:]:
            if k.startswith("wait:"):
                time.sleep(float(k[5:]))
            elif k.startswith("hold:"):
                _, name, secs = k.split(":")
                press(name, float(secs))
            else:
                name, _, n = k.partition("*")
                for _ in range(int(n or 1)):
                    press(name)
        print("ok")
    elif cmd == "shot":
        from PIL import ImageGrab
        hwnd = game_window()
        if not hwnd:
            sys.exit("speed2.exe is not in the foreground; not capturing")
        r = wt.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(r))
        im = ImageGrab.grab(bbox=(r.left, r.top, r.right, r.bottom), all_screens=True).convert("RGB")
        out = sys.argv[3] if len(sys.argv) > 3 else "."
        im.save(os.path.join(out, sys.argv[2] + ".png"))
        im.thumbnail((1280, 720))
        im.save(os.path.join(out, sys.argv[2] + ".jpg"), quality=85)
        print(os.path.join(out, sys.argv[2] + ".jpg"), im.size)


if __name__ == "__main__":
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
    main()
