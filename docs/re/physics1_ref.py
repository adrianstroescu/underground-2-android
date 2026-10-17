"""NFSU2 PC (speed2.exe) player-car longitudinal physics, part 1 reference:
engine / clutch / gearbox / drivetrain, straight-line full-throttle launch with automatic shifting.

Usage:  python physics1_ref.py SKYLINE [seconds] [launch_rpm|max] [GLOBALB.BUN]

FAITHFUL (from the exe, see physics1_re.md):
  frame dt -> substeps n = trunc(dt/0.0044)+1 (0x5b72d0), explicit Euler per substep (0x5aa480)
  engine torque 0x594300, engine post-step/limiter 0x5aa760, inertia rec 0x30c
  clutch state machine 0x5a1400 + clutch rate 0x5949b0, slip torque 0x594cb0, Apply 0x594a30
  wheel-side engine speed 0x5945f0, auto-shift 0x5aa960 + shift points 0x5ac7d0/0x594670
  displayed/shift rpm smoothing 0x5a5540, aero drag 0x59e620, speed limiter 0x59c1e0,
  wheel inertia 0.01 (0x5ab990), wheel rolling loss 0.002*w (0x59d350),
  traction check 1.5*Fx > mu_static*load (0x59d600), sliding force mu_dyn*load (0x5a8280)
APPROXIMATED (part 2 = tyres/suspension/chassis):
  1-D, both wheels of an axle identical, static load m*g/4 per wheel (no weight transfer,
  no downforce), the 5-iteration rigid wheel/body solve of 0x5abdf0 and the locked-clutch
  constraint 0x594da0 are replaced by their exact rigid-body limit (virtual work),
  surface coefficients = 1.0 (exe defaults at 0x800f00), random shift offsets = mid-range.
"""
import struct, sys

ARGS = sys.argv[1:]
NAME = ARGS[0] if ARGS else "SKYLINE"
T_END = float(ARGS[1]) if len(ARGS) > 1 else 20.0
LAUNCH = ARGS[2] if len(ARGS) > 2 else "max"      # start rpm: 'max' (revved in N) or a number
BUN = ARGS[3] if len(ARGS) > 3 else "D:/GAME install/Need for Speed - Underground 2-NEW/GLOBAL/GLOBALB.BUN"

# ---------------------------------------------------------------- data
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
        recs = [b[st + i * REC: st + (i + 1) * REC] for i in range((len(b) - st) // REC)]
        break
r = next(x for x in recs if x[:32].split(b'\0')[0].decode().upper() == NAME.upper())
F = lambda o: struct.unpack_from('<f', r, o)[0]
I = lambda o: struct.unpack_from('<I', r, o)[0]

RPM_K = 9.549304008483887      # 0x7a1d28  rad/s -> rpm
RAD_K = 0.10471966862678528    # 0x7a1d38  rpm -> rad/s
MAX_SUB = 0.004399999976158142 # 0x7fc070
G = 9.8                        # 0x7fbef0
I_W = 0.01                     # 0x7fc6f0 wheel inertia (t*m^2)
CLUTCH_RATE = (-3.0, 0.0, 1.5, 0.0)   # 0x7fc298[state]: 0 disengaging,1 disengaged,2 engaging,3 engaged
THR_SCALE = (0.0, 0.0, 0.7, 1.0)      # engine+0x80 written by 0x5a1400 per clutch state
SLIP_CLAMP = 300.0             # 0x7fbf34 rad/s
RPM_DROP = (2.5, 15.0)         # 0x7a1a2c[0 / 4]: display-rpm max fall (torque units / I_e)

class Curve:                   # 0x5764d0
    def __init__(s, y, x0, inv): s.y, s.x0, s.inv = y, x0, inv
    def __call__(s, x):
        t = (x - s.x0) * s.inv; i = int(t)
        if i < 0 or t < 0: return s.y[0]
        if i >= len(s.y) - 1: return s.y[-1]
        f = t - i; return s.y[i] + (s.y[i + 1] - s.y[i]) * f

mass = F(0x220); drag_k = F(0x370); lim_mph = F(0x3a0)
idle, red, mx, I_E = F(0x300), F(0x304), F(0x308), F(0x30c)
TQ = Curve([F(0x310 + 4 * k) + F(0x334 + 4 * k) for k in range(9)], idle, 8.0 / (mx - idle))  # turbo add 0x5a6e30
EB = Curve([F(0x358 + 4 * k) for k in range(3)], idle, 2.0 / (mx - idle))                     # engine braking
fd_f, fd_r, bias = F(0x2c8), F(0x2cc), F(0x2d0)
K_CL, CL_RATE = F(0x2d4), F(0x2dc)
ng = I(0x2d8); gear = [F(0x2e0 + 4 * i) for i in range(ng + 2)]     # [R, N, 1st..]
rad_f, rad_r = F(0x130), F(0x190)
mu_s = (F(0x1ec), F(0x20c)); mu_d = (F(0x1f0), F(0x210))           # static / dynamic grip, front / rear
LOAD = mass * G / 4                                                   # kN per wheel (approximation)

# shift points 0x5ac7d0 ([0x86b33c]==0) and random offsets of 0x594710 (U[-100,0], mid-range here)
up = {g: red - 100.0 for g in range(2, ng + 1)}; up[ng + 1] = mx + 500.0
down = {g + 1: gear[g + 1] / gear[g] * up[g] for g in range(2, ng + 1)}
RND_UP = RND_UP2 = RND_DOWN = -50.0
def upshift_rpm(g, low=False):                                       # 0x594670 (non-drag race: cap = redline)
    base = -100.0 if low else (RND_UP2 if g == 3 else RND_UP)
    return min(red, base + up[g])

LAUNCH_RPM = mx if LAUNCH == "max" else float(LAUNCH)
front_driven, rear_driven = bias <= 0.999, bias >= 0.001

def engine_torque(w, thr_eff, limiting, g):                            # 0x594300 (no NOS, mult 1, no heat)
    rpm = w * RPM_K
    th = 0.0 if limiting else thr_eff
    T = TQ(rpm) * th
    Tb = -(1.0 - th) * EB(rpm) * (2.0 if limiting else 1.0)
    T += Tb
    if T < 0 and not limiting: T *= 0.65 if g == 1 else 0.85
    return T

def wheelside_w(wf, wr, g):                                           # 0x5945f0
    acc = 0.0
    if rear_driven: acc = wr * fd_r
    if front_driven: acc += wf * fd_f
    if rear_driven and front_driven: acc *= 0.5
    return acc * gear[g]

def drag_force(v, thr):                                              # 0x59e620: k*|v|*v*(1+(1-thr))
    return drag_k * v * abs(v) * (2.0 - thr)

def run():
    v = 0.0; wf = wr = 0.0
    # start as after the countdown with the throttle held: neutral, clutch engaged, engine on the limiter
    w_e = LAUNCH_RPM * RAD_K
    g, tgt, state, c = 1, 1, 3, 1.0
    shifting, limiting, prev58 = 0, LAUNCH_RPM > mx - 25.0, 0.0
    slide = [False, False]
    disp = LAUNCH_RPM; thr = 1.0
    dt = 1.0 / 60.0; n = int(dt / MAX_SUB) + 1; h = dt / n
    t = 0.0; next_print = 0.0
    print("%s: m=%.3ft  idle/red/max %g/%g/%g  ng=%d  bias=%.2f  substeps %d x %.5fs" %
          (NAME, mass, idle, red, mx, ng, bias, n, h))
    print("  t[s]   mph    km/h   rpm(eng) rpm(disp) gear clutch state")
    while t <= T_END + 1e-9:
        if t + 1e-9 >= next_print:
            print("%6.2f %6.1f %7.1f %8.0f %8.0f   %s  %5.2f   %d" % (
                t, v * 2.23694, v * 3.6, w_e * RPM_K, disp, "RN1234567"[g], c, state))
            next_print += 0.5
        # ---- per-frame pre-step: input + automatic gearbox 0x5aa960 (uses displayed rpm)
        if g <= 1: tgt = 2
        elif state == 3:
            rpm = disp
            if rpm >= upshift_rpm(g) and g < ng + 1 and not any(slide):
                tgt = g + 1
            elif g > 2:
                lim2 = upshift_rpm(g - 1, True) * gear[g] / gear[g - 1] - 200.0
                if rpm <= down.get(g, 0.0) + RND_DOWN and rpm <= lim2: tgt = g - 1
        for _ in range(n):
            # ---- forces (0x5b0520 / 0x5abdf0) for this substep
            thr_eff = THR_SCALE[state] * thr
            Te = engine_torque(w_e, thr_eff, limiting, g)
            Fd = drag_force(v, thr)
            if state == 3 and g != 1:
                # rigid engine+wheels (0x594cb0 state 3 + 0x594da0 constraint + 0x5a12d0 sync)
                Gf = gear[g] * fd_f * (1 - bias); Gr = gear[g] * fd_r * bias   # torque to axles
                dem = (Gf * Te / 2 / rad_f, Gr * Te / 2 / rad_r)              # static Fx per wheel
                for ax in (0, 1):
                    lim_ = mu_s[ax] * LOAD
                    if 1.5 * abs(dem[ax]) > lim_: slide[ax] = True
                if not any(slide):
                    # virtual work: dwe/da
                    k = gear[g] * ((fd_r / rad_r if rear_driven else 0) + (fd_f / rad_f if front_driven else 0)) \
                        * (0.5 if (rear_driven and front_driven) else 1.0)
                    Meff = mass + 2 * I_W / rad_f ** 2 + 2 * I_W / rad_r ** 2 + I_E * k * k
                    a = (Te * k - Fd) / Meff   # rolling loss 0.002*w is undone by the -s/(r*dt) stick term
                    v += a * h; wf = v / rad_f; wr = v / rad_r; w_e = wheelside_w(wf, wr, g)
                else:
                    # wheelspin: engine + driven wheels one body, Coulomb tyre force
                    Fs = [0.0, 0.0]
                    for ax, (rd, wv) in enumerate(((rad_f, wf), (rad_r, wr))):
                        drv = front_driven if ax == 0 else rear_driven
                        if drv: Fs[ax] = 2 * mu_d[ax] * LOAD * (1 if rd * wv - v >= 0 else -1)
                    # J*dwe = Te - sum(r*F/ratio)
                    J = I_E; Tl = 0.0
                    for ax, rd, fd in ((0, rad_f, fd_f), (1, rad_r, fd_r)):
                        drv = front_driven if ax == 0 else rear_driven
                        if drv:   # wheels synced to engine (0x5a12d0): w_axle = w_e/(gear*fd)
                            J += 2 * I_W / (gear[g] * fd) ** 2
                            Tl += rd * Fs[ax] / (gear[g] * fd)
                    w_e += (Te - Tl) / J * h
                    v += (sum(Fs) - Fd) / mass * h
                    for ax in (0, 1):
                        drv = front_driven if ax == 0 else rear_driven
                        fd = fd_f if ax == 0 else fd_r; rd = rad_f if ax == 0 else rad_r
                        w_ax = w_e / (gear[g] * fd) if drv else v / rd
                        if ax == 0: wf = w_ax
                        else: wr = w_ax
                        if drv and abs(rd * w_ax - v) <= 0.5: slide[ax] = False   # back to static branch
            else:
                # clutch slipping (0,2) or open (1) or neutral: engine and wheels separate
                Tc = 0.0
                if state in (0, 2):
                    x4c = wheelside_w(wf, wr, g)
                    dw = max(-SLIP_CLAMP, min(SLIP_CLAMP, w_e - x4c))
                    drpm = (x4c - w_e) * RPM_K; K = K_CL
                    if prev58 != 0 and drpm * prev58 < 0 and abs(drpm) > 2000 and abs(prev58) > 2000: K *= 0.5
                    prev58 = drpm
                    Tc = K * c * dw
                if g == 1: Tc_w = 0.0
                else: Tc_w = Tc
                w_e += (Te - Tc_w) / I_E * h                       # 0x5a1290 with ext = -Tc
                Tf = Tc_w * gear[g] * fd_f * (1 - bias) if g != 1 else 0.0
                Tr = Tc_w * gear[g] * fd_r * bias if g != 1 else 0.0
                for ax, T_ax, rd in ((0, Tf, rad_f), (1, Tr, rad_r)):
                    if 1.5 * abs(T_ax / 2 / rd) > mu_s[ax] * LOAD: slide[ax] = True
                if not any(slide):
                    Meff = mass + 2 * I_W / rad_f ** 2 + 2 * I_W / rad_r ** 2
                    a = (Tf / rad_f + Tr / rad_r - Fd) / Meff
                    v += a * h; wf = v / rad_f; wr = v / rad_r
                else:
                    Fsum = 0.0
                    for ax, T_ax, rd in ((0, Tf, rad_f), (1, Tr, rad_r)):
                        wv = wf if ax == 0 else wr
                        if slide[ax]:
                            Fx = 2 * mu_d[ax] * LOAD * (1 if rd * wv - v >= 0 else -1)
                            wv += (T_ax - rd * Fx - 0.004 * wv) / (2 * I_W) * h
                            if abs(rd * wv - v) <= 0.5 and 1.5 * abs(T_ax / 2 / rd) <= mu_s[ax] * LOAD:
                                slide[ax] = False
                        else:
                            Fx = T_ax / rd
                        Fsum += Fx
                        if ax == 0: wf = wv
                        else: wr = wv
                    v += (Fsum - Fd) / mass * h
                    if not slide[0]: wf = v / rad_f
                    if not slide[1]: wr = v / rad_r
            # ---- speed limiter (0x59c1e0): above lim+1.9 mph forward force -> min(-5F, -0.5)
            if lim_mph > 0 and v * 2.23602 > lim_mph * 1.0 + 1.9:
                v -= 0.5 / mass * h
            # ---- post-steps: clutch integrate (0x5949b0) and engine clamp/limiter (0x5aa760)
            c += CL_RATE * 1.0 * CLUTCH_RATE[state] * h
            w_e = min(max(w_e, idle * RAD_K), mx * RAD_K)
            rpm = w_e * RPM_K
            if limiting and rpm < mx - mx * 0.0125: limiting = False
            elif not limiting and rpm > mx - 25.0: limiting = True
            # ---- clutch state machine 0x5a1400
            x4c = wheelside_w(wf, wr, g)
            if state == 3:
                if g != tgt: shifting, state = 1, 0
                else: shifting = 0
            elif state == 0:
                if c <= 0: c, state = 0.0, 1
            elif state == 1:
                g = tgt; state = 2                                  # (+0x594710 re-rolls offsets)
            elif state == 2:
                if c >= 1.0:
                    c = 1.0
                    if abs(w_e - x4c) < 50.0 or g == 1: state = 3
        # ---- per-frame post (0x5a5540): displayed rpm used by the auto box
        raw = w_e * RPM_K
        drop = -RPM_DROP[1 if (shifting and tgt > 2) else 0] / I_E
        if (raw - disp) / dt < drop: raw = max(raw, disp + drop * dt)
        disp = 0.45 * disp + 0.55 * raw
        t += dt

run()
