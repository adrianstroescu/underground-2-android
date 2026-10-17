"""NFSU2 PC (speed2.exe) player-car physics, part 2 reference: full 3-D chassis on a flat plane.
Rigid body 0x59c1e0/0x59c5f0/0x5bb0d0, suspension 0x5a8790, tyre loads 0x59d160, tyres 0x5a8280,
5-pass static solve 0x5abdf0, friction circle 0x59d600, aero 0x59e620, steering 0x59def0,
grip/brake inputs 0x5a4d80, angular damping 0x5a5b60.  See physics2_re.md.

Usage:  python physics2_ref.py [CAR] [test]      test = all | launch | corner | brake | handbrake
        CAR default SKYLINE (also MUSTANGGT, CIVIC, ...)

FAITHFUL (from the exe): everything in the chassis/tyre path listed above, with the exe constants;
  order of operations per substep (derivative -> Euler -> posts -> force stage).
APPROXIMATED (marked APPROX below):
  - drivetrain: engine rigidly coupled to the driven wheels, instant gear changes on part-1 shift
    points, launch-clutch = engine torque at max(rpm, 3500) (part 1 has the exact clutch model);
    the locked-clutch constraint 0x594da0 inside the 5 passes is not run;
  - sin/cos use math.sin instead of the exe's 16-bit sine table (0x43c2b0); atan2 IS the exe's
    table version (0x43c570, table read from speed2.exe);
  - player-controller layer (stick shaping, max-lock vs speed 0x405dc0, slew 0x405990) is replaced by
    a scripted road-wheel steering angle; the throttle in the cornering test is a test-harness
    speed-holding P controller (not game logic);
  - flat ground z=0, surface table = 1.0 (exe defaults), per-car grip W.184 and mover.6c = 1,
    no NOS, no damage, car+0x42c (speed term in the static preload) taken as 0.
"""
import math, struct, sys

ARGS = sys.argv[1:]
NAME = ARGS[0] if ARGS else "SKYLINE"
TEST = ARGS[1] if len(ARGS) > 1 else "all"
GAME = "D:/GAME install/Need for Speed - Underground 2-NEW"
BUN = GAME + "/GLOBAL/GLOBALB.BUN"
EXE = GAME + "/speed2.exe"

# ------------------------------------------------------------------ data
d = open(BUN, 'rb').read()
def walk(off, end, out):
    while off + 8 <= end:
        cid, size = struct.unpack_from('<II', d, off)
        if size > end - off - 8: break
        out.append((cid, off + 8, size))
        if cid & 0x80000000: walk(off + 8, off + 8 + size, out)
        off += 8 + size
    return out
REC = 0x890
for cid, o, s in walk(0, len(d), []):
    if cid == 0x34600:
        b = d[o:o + s]; st = len(b) % REC
        recs = [b[st + i * REC: st + (i + 1) * REC] for i in range(len(b) // REC)]
        break
r = next(x for x in recs if x[:32].split(b'\0')[0].decode().upper() == NAME.upper())
F = lambda o: struct.unpack_from('<f', r, o)[0]
I = lambda o: struct.unpack_from('<I', r, o)[0]

# exe atan table (0x43c570 uses bytes at 0x7eacb0 .. +256)
_e = open(EXE, 'rb').read()
_pe = struct.unpack_from('<I', _e, 0x3c)[0]; _ns = struct.unpack_from('<H', _e, _pe + 6)[0]
_oh = struct.unpack_from('<H', _e, _pe + 20)[0]
def _off(va):
    for i in range(_ns):
        so = _pe + 24 + _oh + i * 40
        vs, vva, rs, ro = struct.unpack_from('<IIII', _e, so + 8)
        if 0x400000 + vva <= va < 0x400000 + vva + rs: return va - 0x400000 - vva + ro
ATAN_TAB = _e[_off(0x7eacb0):_off(0x7eacb0) + 257]

def ftol(x): return int(x)                        # 0x75bed4: truncation
def ang16(x, y):                                  # 0x43c570: fixed-point atan2(y, x), 65536 = 2*pi
    q = 0
    if x < 0: x = -x; q = 1
    if y < 0: q ^= 3; y = -y
    if x > y:   a = ATAN_TAB[ftol(y / x * 256.0)] << 5
    elif y > x: a = 0x4000 - (ATAN_TAB[ftol(x / y * 256.0)] << 5)
    else:       a = 0 if x == 0 else 0x2000      # equal (0,0 -> 0)
    if q == 0: v = a
    elif q == 3: v = -a
    elif q == 1: v = 0x8000 - a
    else: v = 0x8000 + a
    return v & 0xffff
A16 = 9.58738e-05                                 # 0x784240
PI = 3.14159265
def wrap_pi(a):                                   # "if a > pi: a -= 2pi"
    return a - 2 * PI if a > PI else a
def slip_angle(vx, vy):                           # 0x5a8280 / 0x59d600
    b = wrap_pi(ang16(vx, vy) * A16)
    if vx < 0:
        b = PI + b
        if b > PI: b -= 2 * PI
    return b

class Curve:                                      # 0x5764d0 (n, x0, x_end, inv, y*)
    def __init__(s, y, x0, inv): s.y, s.x0, s.inv = y, x0, inv
    def __call__(s, x):
        t = (x - s.x0) * s.inv; i = ftol(t)
        if i < 0 or t < 0: return s.y[0]
        if i >= len(s.y) - 1: return s.y[-1]
        f = t - i; return s.y[i] * (1 - f) + s.y[i + 1] * f
class XYCurve:                                    # 0x576790 (pairs, clamped)
    def __init__(s, pts): s.p = pts
    def __call__(s, x):
        p = s.p
        if x <= p[0][0]: return p[0][1]
        if x >= p[-1][0]: return p[-1][1]
        for (x0, y0), (x1, y1) in zip(p, p[1:]):
            if x0 <= x <= x1: return y0 + (y1 - y0) * (x - x0) / (x1 - x0)
        return p[0][1]

# ------------------------------------------------------------------ vector helpers
def add(a, b): return (a[0] + b[0], a[1] + b[1], a[2] + b[2])
def sub(a, b): return (a[0] - b[0], a[1] - b[1], a[2] - b[2])
def mul(a, s): return (a[0] * s, a[1] * s, a[2] * s)
def dot(a, b): return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
def cross(a, b): return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])
def norm(a): return math.sqrt(dot(a, a))
def normalize(a):
    n = norm(a); return mul(a, 1.0 / n) if n > 0 else a
def vrow(v, R):        # row-vector times matrix (0x441000): sum v_i * R[i]
    return (v[0] * R[0][0] + v[1] * R[1][0] + v[2] * R[2][0],
            v[0] * R[0][1] + v[1] * R[1][1] + v[2] * R[2][1],
            v[0] * R[0][2] + v[1] * R[1][2] + v[2] * R[2][2])
def tolocal(v, R): return (dot(v, R[0]), dot(v, R[1]), dot(v, R[2]))

# ------------------------------------------------------------------ constants (exe)
G = 9.8                     # 0x7fbef0 (rb+0x160 while driving; 11.0 = 0x7fc084 in car state 1)
MAX_SUB = 0.0044            # 0x7fc070
I_W = 0.01                  # 0x7fc6f0
ROT_DAMP = Curve([0.05, 0.05, 0.25, 0.7, 2.0, 5.0], 0.0, 0.5)                 # 0x7fc92c (|w|)
REST_DAMP = Curve([1.5, 1.0, 0.6, 0.22, 0.06, 0.03, 0.015, 0.0], 0.0, 7.0)   # 0x7fc940
LAT = [None] + [Curve(y, 0.0, 0.5) for y in (                                  # 0x7fc968.. (k = 0..6)
    [0, 1.2, 2.3, 3.0, 3.0, 2.8], [0, 1.7, 3.2, 4.3, 5.1, 5.2], [0, 1.8, 3.5, 4.9, 5.8, 6.1],
    [0, 1.83, 3.6, 5.0, 5.96, 6.4], [0, 1.86, 3.7, 5.1, 6.13, 6.7], [0, 1.9, 3.8, 5.2, 6.3, 7.1])]
LAT[0] = lambda L: 0.0                                                         # data in .bss = 0
YAW_DAMP = XYCurve([(0.0, 0.0), (0.4, 0.0), (2.0, 20.0)])                    # 0x86b4a0 (0x5a5b60)

# ------------------------------------------------------------------ car record
mass = F(0x220)
cg = (F(0x110), F(0x114), F(0x118))
Idiag = (F(0x230), F(0x244), F(0x258))
yaw_scale = F(0x270)                                       # player: Izz * rec 0x270 (rb+0x158 = 1)
invI = (1 / Idiag[0], 1 / Idiag[1], 1 / (Idiag[2] * yaw_scale))
WG = [dict(pos=(F(0x120 + 0x30 * i), F(0x124 + 0x30 * i), F(0x128 + 0x30 * i)), rad=F(0x130 + 0x30 * i),
           idx=I(0x138 + 0x30 * i)) for i in range(4)]
AX = [dict(maxslip=F(0x1e0 + 0x20 * a), latk=F(0x1e4 + 0x20 * a), lats=F(0x1e8 + 0x20 * a),
           mus=F(0x1ec + 0x20 * a), mud=F(0x1f0 + 0x20 * a)) for a in range(2)]
SU = [[F(0x280 + 0x20 * a + 4 * k) for k in range(8)] for a in range(2)]  # prog, k, bump, reb, arb, maxc, minc, ?
steer_rate_k, brake_k, hbrake_k, brake_bias = F(0x380), F(0x384), F(0x388), F(0x38c)
drag_k, dfF, dfR = F(0x370), F(0x374), F(0x378)
lim_mph = F(0x3a0)
# drivetrain / engine (part 1)
idle, red, mx = F(0x300), F(0x304), F(0x308)
TQ = Curve([F(0x310 + 4 * k) + F(0x334 + 4 * k) for k in range(9)], idle, 8.0 / (mx - idle))
EB = Curve([F(0x358 + 4 * k) for k in range(3)], idle, 2.0 / (mx - idle))
fd_f, fd_r, bias = F(0x2c8), F(0x2cc), F(0x2d0)
ng = I(0x2d8); gear = [F(0x2e0 + 4 * i) for i in range(ng + 2)]
up = {g: red - 100.0 for g in range(2, ng + 1)}; up[ng + 1] = mx + 500.0
down = {g + 1: gear[g + 1] / gear[g] * up[g] for g in range(2, ng + 1)}
RPM_K, RAD_K = 9.549304, 0.10471967

# ------------------------------------------------------------------ simulation state
class Wheel:
    def __init__(s, i):
        g = WG[i]; s.i = i; s.r = g['rad']; s.rear = g['idx'] >= 2; ax = AX[1 if s.rear else 0]
        s.ax = ax; s.w = 0.0; s.ang = 0.0
        s.T = 0.0; s.T120 = 0.0; s.Fx = 0.0; s.Fy = 0.0; s.s = 0.0
        s.lock = 0; s.slide = False; s.dirstate = 1; s.load = 0.0; s.ground = False
        s.brk = 0.0; s.hb = 0.0; s.g180 = 1.0; s.g188 = 1.0
        s.rows = None; s.lrows = None; s.vel = (0, 0, 0); s.cpt = (0, 0, 0); s.center = (0, 0, 0)
        s.Fw = (0, 0, 0); s.beta = 0.0
class Susp:
    def __init__(s, i):
        s.D = SU[1 if WG[i]['idx'] >= 2 else 0]
        s.hp = sub(WG[i]['pos'], cg)              # 0x59e0c0: S.90 = pos - CG (minus ride adjust = 0)
        s.x = 0.0; s.rate = 0.0; s.ground = False; s.F = 0.0; s.pre = 0.0; s.loc = s.hp

class Car:
    def __init__(s, v0=0.0):
        s.wh = [Wheel(i) for i in range(4)]; s.su = [Susp(i) for i in range(4)]
        for i in range(4): s.su[i].partner = s.su[i ^ 1]
        z0 = s.wh[0].r - s.su[0].hp[2]
        s.pos = (0.0, 0.0, 0.5 * (z0 + s.wh[2].r - s.su[2].hp[2]))
        s.R = [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)]
        s.P = (mass * v0, 0.0, 0.0); s.L = (0.0, 0.0, 0.0); s.x90 = 0.0
        s.v = (v0, 0, 0); s.om = (0, 0, 0); s.Fsum = (0, 0, -mass * G); s.Tsum = (0, 0, 0)
        s.Flast = (0, 0, 0)
        for w in s.wh: w.w = v0 / w.r
        s.g = 2; s.steer = 0.0; s.thr = 0.0; s.brake = 0.0; s.hbin = 0.0; s.hbramp = 1.0
        s.we = idle * RAD_K

    # ---- AddForce 0x59bb00: |F| <= 1200 kN, |r x F| <= 100 kN*m
    def add_force(s, Fv, rv):
        n = norm(Fv); Fc = mul(Fv, 1200.0 / n) if n > 1200.0 else Fv
        s.Fsum = add(s.Fsum, Fc)
        t = cross(rv, Fv); n = norm(t)
        if n > 100.0: t = mul(t, 100.0 / n)
        s.Tsum = add(s.Tsum, t)

    def invIw(s, Lv):                     # omega = L * R^T I^-1 R  (0x59c5f0)
        lb = tolocal(Lv, s.R)
        return vrow((lb[0] * invI[0], lb[1] * invI[1], lb[2] * invI[2]), s.R)

    # ================================================================ per frame
    def frame(s, dt):
        # 0x5ab990: static preload per wheel (W = m*g; the 0.01*(Cf+Cr)*speed term needs car+0x42c -> 0)
        xF, xR = WG[0]['pos'][0], WG[3]['pos'][0]
        rear_share = (xF - cg[0]) / (xF - xR) * 0.5
        W = mass * G
        for i, su in enumerate(s.su):
            su.pre = W * (rear_share if WG[i]['idx'] >= 2 else 0.5 - rear_share)
        # 0x5a4d80: brake/handbrake inputs, handbrake ramp, grip multipliers
        if s.hbin >= 0.5: s.hbramp = max(0.5, s.hbramp - 20.0 * dt)
        else: s.hbramp = min(1.0, s.hbramp + dt)
        mph = norm(s.v) * 2.237
        bcar = s.body_slip()
        for w in s.wh:
            if s.thr > 0.8 and s.brake > 0.5 and mph < 10 and w.rear: w.brk = mph * 0.05
            else: w.brk = 2.0 * (brake_bias if w.rear else 1.0 - brake_bias) * s.brake
            h = (1.0 if w.rear else 0.0) * s.hbin
            if h > 0.2 and abs(bcar) > 0.3 and mph < 80: h += 0.5
            w.hb = h
            g0 = 1.0
            if w.rear:   # normal race: g0 * (1 + (Y - 1) * hbramp), Y = 0x5990c0 (player, no recent event)
                Y = 1.0 + abs(s.R[0][2])
                if not (w.hb > 0.5 and abs(bcar) < 20 * PI / 180):
                    Y += min(abs(bcar) * (F(0x390) + F(0x394) * 0.0) * norm(s.v) / 30.0, 0.35)
                g0 = g0 * (1.0 + (Y - 1.0) * s.hbramp)
            w.g180 = g0
        # auto gearbox (APPROX: part-1 thresholds, instant shift)
        rpm = s.we * RPM_K
        if s.g <= 1: s.g = 2
        elif rpm >= min(red, up[s.g] - 50) and s.g < ng + 1 and not any(w.slide for w in s.wh): s.g += 1
        elif s.g > 2 and rpm <= down.get(s.g, 0) - 50: s.g -= 1
        n = int(dt / MAX_SUB) + 1; h = dt / n
        for _ in range(n): s.substep(h)

    def body_slip(s):                     # 0x591d90: heading - velocity direction
        if norm(s.v) < 1.0: return 0.0
        a = (ang16(s.R[0][0], s.R[0][1]) - ang16(s.v[0], s.v[1])) & 0xffff
        return wrap_pi(a * A16)

    # ================================================================ substep
    def substep(s, h):
        # ---- 1. derivatives (0x59c1e0 rigid body, 0x59d350 wheels), forces from last force stage
        F_, T_ = s.Fsum, s.Tsum
        # speed limiter (0x59c1e0)
        if lim_mph > 0:
            lim = lim_mph * 1.0 + 1.9
            pitch = s.R[0][2]
            lim *= (1 - (1 - 0.9) / 0.3 * min(pitch, 0.3)) if pitch > 0 else (1 + (1.1 - 1) / 0.3 * min(-pitch, 0.3))
            if math.hypot(s.v[0], s.v[1]) * 2.23602 > lim and dot(s.R[0], F_) > 0:
                fl = tolocal(F_, s.R); fx = max(-5.0 * fl[0], -0.5)
                F_ = vrow((fx, fl[1], fl[2]), s.R)
        womag = norm(s.om)
        T_ = add(T_, mul(s.L, -ROT_DAMP(womag)))                       # rb+0x154
        dx90 = 0.1 if REST_DAMP(max(norm(s.v) / 4.0, womag / 2.0)) > s.x90 else -10.0
        T_ = add(T_, mul(s.L, -2.0 * s.x90)); F_ = add(F_, mul(s.P, -8.0 * s.x90))   # rb+0x150
        dws = []
        for w in s.wh:
            w.T120 = 0.5 * ((w.T - w.r * w.Fx) + w.T120)
            dw = (w.T120 - 0.002 * 1.0 * w.w - 0.1 * 0.0 * w.w) / I_W
            if w.lock: dw = 0.0
            if not w.slide and h > 0.001: dw -= w.s / (w.r * h)
            dws.append(dw)
        # ---- 2. explicit Euler (0x5aa480) + orthonormalise (0x5bb0d0)
        s.pos = add(s.pos, mul(s.v, h))
        s.R = [add(row, mul(cross(s.om, row), h)) for row in s.R]
        s.P = add(s.P, mul(F_, h)); s.L = add(s.L, mul(T_, h)); s.x90 += dx90 * h
        for w, dw in zip(s.wh, dws): w.ang += w.w * h; w.w += dw * h
        r0 = normalize(s.R[0]); r2 = normalize(cross(r0, s.R[1])); r1 = cross(r2, r0)
        s.R = [r0, r1, r2]
        # ---- 3. posts
        s.post_rb();
        for i in range(4): s.post_wheel(s.wh[i]); s.post_susp(i, h)
        # ---- 4. force stage (0x5b0520)
        s.forces(h)

    def post_rb(s):                                      # 0x59c5f0
        s.v = mul(s.P, 1.0 / mass)
        if norm(s.v) > 200.0 and mass > 0.7: s.P = mul(s.P, 200.0 / norm(s.v)); s.v = mul(s.P, 1 / mass)
        s.Flast = s.Fsum
        s.om = s.invIw(s.L)
        if norm(s.om) > 250.0: s.om = mul(s.om, 250.0 / norm(s.om))
        if s.x90 <= 0: s.x90 = 0.0
        s.Fsum = (0.0, 0.0, -mass * G); s.Tsum = (0.0, 0.0, 0.0)   # gravity applied at the CG

    def post_wheel(s, w):                                # 0x59d030
        w.Fx = w.Fy = w.T = w.s = 0.0
        if w.dirstate == 0 and w.w < 0: w.w = 0.0
        elif w.dirstate == 2 and w.w > 0: w.w = 0.0
        w.dirstate = 0 if w.w > 1e-5 else (2 if w.w < -1e-5 else 1)

    def post_susp(s, i, h):                             # 0x5a8790
        su, w = s.su[i], s.wh[i]
        p = add(vrow(su.hp, s.R), s.pos)                 # hardpoint in world
        up_ = s.R[2]
        dn = up_[2]                                      # 0x59d250: flat ground z = 0, n = (0,0,1)
        t = (-p[2]) / dn if dn >= 0.001 else -100.0
        x = t + w.r
        D = su.D; xprev = su.x
        if x > D[5]: su.x = D[5]; su.ground = True
        elif x < D[6]: su.x = D[6]; su.ground = False
        else: su.x = x; su.ground = True
        if h > 0.0001: su.rate = (su.x - xprev) / h
        su.loc = (su.hp[0], su.hp[1], su.hp[2] + su.x)   # wheel centre, body frame
        a = s.steer_angle(i)
        lrows = [(math.cos(a), math.sin(a), 0.0), (-math.sin(a), math.cos(a), 0.0), (0.0, 0.0, 1.0)]
        w.lrows = lrows
        w.rows = [vrow(rw, s.R) for rw in lrows]
        off = vrow(su.loc, s.R)
        w.center = add(s.pos, off)
        w.vel = add(s.v, cross(s.om, off))
        if su.ground:
            xx = max(su.x, 0.0)
            Fs = (1 + D[0] * xx) * D[1] * su.x + (D[2] if su.rate > 0 else D[3]) * su.rate + su.pre \
                 + (su.x - su.partner.x) * D[4]
            su.F = Fs if Fs >= 0 else 0.0
        else: su.F = 0.0

    def steer_angle(s, i):                               # 0x59def0 Ackermann
        if i >= 2: return 0.0
        th = s.steer; pos = th >= 0; th = abs(th)
        Lb = WG[0]['pos'][0] - WG[3]['pos'][0]; Wt = WG[0]['pos'][1] - WG[1]['pos'][1]
        out = Lb * th / (th * Wt + Lb)
        if pos: return th if i == 0 else out
        return -out if i == 0 else -th

    # ================================================================ force stage
    def forces(s, h):
        s.aero()
        for i in range(4): s.tyre_load(i)
        s.drivetrain()
        for w in s.wh: s.tyre(w)
        s.static_solve()
        for w in s.wh: s.friction_circle(w)
        for i, w in enumerate(s.wh):
            if s.su[i].ground: s.add_force(w.Fw, sub(w.cpt, s.pos))
        # 0x5a5b60 angular damping (car+0x424 < 2, car state 0); runs only while |w| < 0.5 rad/s
        wm = norm(s.om)
        if wm < 0.5:
            s.Tsum = add(s.Tsum, mul(s.om, -YAW_DAMP(wm) * mass))

    def aero(s):                                         # 0x59e620 (A.8=A.c=1, A.30=1, A.34=surface 1)
        v = s.v; sp = norm(v); on = sum(1 for su in s.su if su.ground) * 0.25
        Fd = mul(v, -sp * drag_k * (1 + (1 - s.thr) * (2.0 - 1.0)))
        pt = (0.0, 0.0, (1 - s.thr) * -0.1 if on >= 0.5 else 0.0)
        s.add_force(Fd, vrow(pt, s.R))
        u = 1.0 if on >= 0.5 else max(s.R[2][2], 0.0)
        c = dot(mul(v, 1 / sp), s.R[0]) if sp > 0.0001 else s.R[0][0]
        c = max(c, 0.0)
        f = max(c * c, 0.4)                              # (drift/special race: c**4)
        if on == 0:
            Dt = (dfF + dfR) * u * f * sp; Ff, Fr = 0.4 * Dt, 0.41 * Dt
        else: Ff, Fr = dfF * f * u * sp, dfR * f * u * sp
        s.add_force(vrow((0, 0, -Ff), s.R), vrow((1.0, 0, 0), s.R))
        s.add_force(vrow((0, 0, -Fr), s.R), vrow((-1.0, 0, 0), s.R))
        s.df = Ff + Fr

    def tyre_load(s, i):                                 # 0x59d160
        su, w = s.su[i], s.wh[i]
        w.ground = su.ground
        n = (0.0, 0.0, 1.0)
        if su.ground: w.cpt = sub(w.center, mul(n, w.r))
        if su.F > 0:
            a = w.rows[1]
            k = dot(cross(cross(a, n), a), n) * 4.0 - 3.0
            w.load = max(k, 0.3) * su.F
        else: w.load = 0.0

    def drivetrain(s):                                   # APPROX (see part 1 for the exact model)
        dr = [i for i in range(4) if (WG[i]['idx'] >= 2 and bias >= 0.001) or (WG[i]['idx'] < 2 and bias <= 0.999)]
        ws = []
        if bias >= 0.001: ws.append(0.5 * (s.wh[2].w + s.wh[3].w) * fd_r)
        if bias <= 0.999: ws.append(0.5 * (s.wh[0].w + s.wh[1].w) * fd_f)
        x4c = sum(ws) / len(ws) * gear[s.g]
        we = max(x4c, idle * RAD_K); we_t = we
        if s.thr > 0 and x4c < 3500 * RAD_K: we_t = 3500 * RAD_K          # slipping launch clutch
        we = min(we, mx * RAD_K); s.we = we
        rpm = we_t * RPM_K
        th = s.thr if rpm < mx - 25 else 0.0
        Te = TQ(rpm) * th - (1 - th) * EB(rpm)
        if Te < 0: Te *= 0.85
        if s.thr < 0.3 and x4c <= idle * RAD_K: Te = 0.0          # declutched (0x5a1400 condition)
        for i in dr:
            fd = fd_r if WG[i]['idx'] >= 2 else fd_f
            share = bias if WG[i]['idx'] >= 2 else 1 - bias
            s.wh[i].T += 0.5 * share * fd * gear[s.g] * Te

    def tyre(s, w):                                      # 0x5a8280
        Tb = brake_k * w.brk * 4.0; Th = hbrake_k * w.hb * 10.0
        sg = -1.0 if w.w > 0 else 1.0                    # brake torques oppose rotation
        if not w.lock: w.T += sg * Tb + sg * Th
        vx = dot(w.vel, w.rows[0]); vy = dot(w.vel, w.rows[1])
        b = slip_angle(vx, vy); w.beta = b
        sl = w.r * w.w - vx; w.s = sl
        if sl < -0.5 and w.T < 0 and Th == 0.0:          # ABS (car->+0x3c+0x18 == 0)
            w.T *= 0.5 / -sl
        mag = math.hypot(sl, vy)
        if mag != 0 and not (vy == 0 and vx == 0):
            mu = 1.0 * 1.0 * w.g180 * w.ax['mud']
            if w.rear and w.lock == 0 and w.w >= 0:       # 0x59d490 player rear factor (car+0x424 == 1)
                mu *= 0.85 + 0.15 * min(max((norm(s.v) - 13.4108) * 0.11185, 0.0), 1.0)
            k58 = w.load / mag * 1.0 * mu
            arg = w.load / math.hypot(vx, vy) * mu * abs(vx)
            # 0x59d400 wheel lock test
            Tbk = (hbrake_k * w.hb * 10.0 + brake_k * w.brk * 4.0) * 1.2
            if Tbk > abs(w.w) * 0.1 + w.r * arg:
                w.lock = 1 if Tbk > 1.0 else 0; w.w = 0.0
            else: w.lock = 0
            if abs(sl) > 0.5 or abs(b) > w.ax['maxslip'] or w.lock:
                w.slide = True
                w.Fx = sl * 1.0 * k58; w.Fy = -(k58 * 1.0 * vy)
                if norm(s.v) * 2.237 < 1.0 and mu > 0.1: w.Fx /= mu; w.Fy /= mu
                return
        w.slide = False
        w.Fx = 1.0 / w.r * w.T
        sx = 1.0 if vx > 0 else -1.0
        sb = -1.0 if b > 0 else 1.0
        w.Fy = s.lat_force(w, w.load * 1.0, abs(b)) * 1.0 * sb * sx

    def lat_force(s, w, load, ab):                       # 0x59d510 ([0x7fc7f0] = 1 path)
        x = ab * 57.2958 / w.ax['maxslip'] * 12.0 * 0.5
        k = ftol(x); L = 0.8 * load / w.ax['latk']
        if k >= 6: f = LAT[6](L)
        else: f = (x - k) * (LAT[k + 1](L) - LAT[k](L)) + LAT[k](L)
        return f * w.ax['lats'] * 2.5

    def static_solve(s):                                 # 0x5abdf0, 5 passes
        Fb = tolocal(s.Fsum, s.R)
        izz = 1.0 / invI[2]
        for _ in range(5):
            fx, fy, mz = Fb[0], Fb[1], 0.0
            for i, w in enumerate(s.wh):
                lr = w.lrows; hp = s.su[i].hp
                bx = lr[1][0] * w.Fy + lr[0][0] * w.Fx; by = lr[1][1] * w.Fy + lr[0][1] * w.Fx
                fx += bx; fy += by; mz += by * hp[0] - bx * hp[1]
            ax, ay, al = fx / mass, fy / mass, mz / izz
            for i, w in enumerate(s.wh):
                if w.slide or w.lock: continue
                lr = w.lrows; hp = s.su[i].hp
                along = (ay + al * hp[0]) * lr[1][0] + (ax - al * hp[1]) * lr[0][0]   # sic: row1.x, not row0.y
                w.Fx += ((w.T - w.r * w.Fx) / I_W * w.r - along) / w.r * I_W

    def friction_circle(s, w):                           # 0x59d600
        boost = w.T > 0 and not w.lock
        if boost: w.Fx *= 1.5
        Fm = math.hypot(w.Fx, w.Fy)
        lim = 1.0 * w.ax['mus'] * w.g188 * 1.0 * w.g180 * w.load * 1.0
        if w.rear and w.lock == 0 and w.w >= 0:
            lim *= 0.85 + 0.15 * min(max((norm(s.v) - 13.4108) * 0.11185, 0.0), 1.0)
        if Fm > lim and Fm > 0.001:
            k = lim / Fm; w.Fx *= k; w.Fy *= k
        elif boost: w.Fx /= 1.5
        vx = dot(w.vel, w.rows[0]); vy = dot(w.vel, w.rows[1])
        b = slip_angle(vx, vy)
        if abs(vx) > 1.0: w.Fx -= math.sin(b) * w.Fy * 0.15
        rows = w.rows                                    # [0x7fc7fc] = 1: wheel rows
        w.Fw = add(add(mul(rows[0], w.Fx), mul(rows[1], w.Fy)), mul(rows[2], w.load))

# ------------------------------------------------------------------ tests
DT = 1.0 / 60.0
def report(c, t, extra=""):
    vb = tolocal(c.v, c.R); ab = tolocal(c.Flast, c.R)
    print("%6.2f %6.1f %7.3f %6.3f %6.3f %6.1f %6.1f %6.1f %6.1f %5.1f  %s" % (
        t, norm(c.v) * 2.237, c.om[2], ab[1] / mass / 9.8, ab[0] / mass / 9.8,
        math.degrees(c.wh[0].beta), math.degrees(c.wh[2].beta), math.degrees(c.body_slip()),
        sum(w.load for w in c.wh), c.su[0].x * 100, extra))
HDR = "  t[s]   mph  yaw[r/s] lat_g  lon_g  aF[deg] aR[deg] beta  sumLoad xFL[cm]"

def run(c, T, ctl, every=0.5):
    t = 0.0; nxt = 0.0
    print(HDR)
    while t <= T + 1e-9:
        ctl(c, t)
        if t + 1e-9 >= nxt: report(c, t, "g%d %s" % (c.g - 1, "".join("S" if w.slide else ("L" if w.lock else ".") for w in c.wh))); nxt += every
        c.frame(DT); t += DT
    return c

def settle(c, T=1.5):
    for _ in range(int(T / DT)): c.frame(DT)

print("%s: m=%.3f t  I=(%.3f %.3f %.3f*%.1f)  CG=%s  susp F=%s  R=%s" % (NAME, mass, Idiag[0], Idiag[1], Idiag[2], yaw_scale, cg, SU[0], SU[1]))
print("  grip front mu_s/mu_d=%.2f/%.2f rear %.2f/%.2f  maxslip %.0f deg  brake %.3f hbrake %.3f bias %.2f  df %.2f/%.2f"
      % (AX[0]['mus'], AX[0]['mud'], AX[1]['mus'], AX[1]['mud'], AX[0]['maxslip'], brake_k, hbrake_k, brake_bias, dfF, dfR))
if TEST in ("all", "launch"):
    print("\n== full-throttle launch from rest (drivetrain APPROX)")
    c = Car(0.0); settle(c)
    run(c, 12.0, lambda c, t: setattr(c, 'thr', 1.0), 1.0)
if TEST in ("all", "corner"):
    for deg in (1.0, 2.0, 3.0, 4.0):
        print("\n== steady-state cornering, 60 mph, road-wheel steer %.1f deg (speed-hold P controller = harness)" % deg)
        c = Car(60 / 2.237); settle(c, 0.5)
        def ctl(c, t, deg=deg):
            c.steer = math.radians(deg) if t > 0.2 else 0.0
            c.thr = min(1.0, max(0.0, 0.3 + 0.5 * (60 / 2.237 - norm(c.v))))
        run(c, 6.0, ctl, 1.0)
if TEST in ("all", "brake"):
    print("\n== full braking from 60 mph")
    c = Car(60 / 2.237); settle(c, 0.5)
    run(c, 4.0, lambda c, t: (setattr(c, 'brake', 1.0), setattr(c, 'thr', 0.0)), 0.5)
if TEST in ("all", "handbrake"):
    print("\n== handbrake turn from 40 mph: steer 15 deg, handbrake 0.2..1.2 s, then counter-steer 0")
    c = Car(40 / 2.237); settle(c, 0.5)
    def ctl(c, t):
        c.thr = 0.0
        c.steer = math.radians(15.0) if 0.1 < t < 1.6 else 0.0
        c.hbin = 1.0 if 0.2 < t < 1.2 else 0.0
    run(c, 4.0, ctl, 0.2)
