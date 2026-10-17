"""Read the running PC game's camera from memory (Windows, read-only).

    python -I tools/pc/ug2mem.py camera      # eye position, forward, up (world, z up)
    python -I tools/pc/ug2mem.py watch       # print the camera once a second

speed2.exe has a fixed image base. Views are 0x70-byte records at 0x832de0 (view 1 is
the player's); view + 0x40 points to the camera, whose first 64 bytes are the D3D
view matrix (row vectors, world to view) and + 0x1e0 its velocity (speed2.exe 0x5cc460).
Only ReadProcessMemory is used: nothing in the game is changed.
"""
import ctypes
import ctypes.wintypes as wt
import struct
import sys
import time

PROCESS_VM_READ = 0x0010
PROCESS_QUERY_INFORMATION = 0x0400
VIEWS = 0x832DE0
VIEW_SIZE = 0x70

k32 = ctypes.WinDLL('kernel32', use_last_error=True)
psapi = ctypes.WinDLL('psapi', use_last_error=True)


def find_pid(name='speed2.exe'):
    arr = (wt.DWORD * 4096)()
    needed = wt.DWORD()
    psapi.EnumProcesses(arr, ctypes.sizeof(arr), ctypes.byref(needed))
    for pid in arr[:needed.value // 4]:
        h = k32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
        if not h:
            continue
        buf = ctypes.create_unicode_buffer(260)
        if psapi.GetModuleBaseNameW(h, None, buf, 260) and buf.value.lower() == name:
            return pid, h
        k32.CloseHandle(h)
    return None, None


def read(h, addr, n):
    buf = ctypes.create_string_buffer(n)
    got = ctypes.c_size_t()
    if not k32.ReadProcessMemory(h, ctypes.c_void_p(addr), buf, n, ctypes.byref(got)):
        raise OSError(f'read {addr:#x} failed ({ctypes.get_last_error()})')
    return buf.raw


def camera(h, view=1):
    cam = struct.unpack('<I', read(h, VIEWS + view * VIEW_SIZE + 0x40, 4))[0]
    m = struct.unpack('<16f', read(h, cam, 64))
    vel = struct.unpack('<3f', read(h, cam + 0x1E0, 12))
    # Row-vector view matrix: rows 0..2 of the 3x3 part are the columns of the camera
    # basis; eye = -t · R^T.
    r = [[m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]]
    t = [m[12], m[13], m[14]]
    eye = [-(t[0] * r[i][0] + t[1] * r[i][1] + t[2] * r[i][2]) for i in range(3)]
    right = [r[i][0] for i in range(3)]
    up = [r[i][1] for i in range(3)]
    fwd = [r[i][2] for i in range(3)]
    return {'eye': eye, 'right': right, 'up': up, 'forward': fwd, 'velocity': vel, 'matrix': m}


def main():
    pid, h = find_pid()
    if not h:
        sys.exit('speed2.exe is not running')
    cmd = sys.argv[1] if len(sys.argv) > 1 else 'camera'
    while True:
        c = camera(h)
        f = lambda v: '(' + ', '.join(f'{x:.3f}' for x in v) + ')'
        print(f"eye {f(c['eye'])} forward {f(c['forward'])} up {f(c['up'])} velocity {f(c['velocity'])}")
        if cmd != 'watch':
            break
        time.sleep(1)


if __name__ == '__main__':
    main()
