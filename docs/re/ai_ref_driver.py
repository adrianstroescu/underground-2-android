"""Reference of the NFSU2 PsychoDriver (AI low-level driver, speed2.exe 0x421030) — see ai_re_driver.md.
Covers the parts that matter above 10 m/s with the full physics mover (gain-scheduled steering, speed PID,
rate limits, look-ahead). The online gain adaptation (only active below 10 m/s) is not modelled."""
import math

def graph_even(tbl, x0, x1, x):           # 0x5764d0: evenly spaced table, clamped
    n = len(tbl); t = (x - x0) * (n - 1) / (x1 - x0)
    if t <= 0: return tbl[0]
    i = int(t)
    if i >= n - 1: return tbl[-1]
    f = t - i; return tbl[i] * (1 - f) + tbl[i + 1] * f

def graph_pts(pts, x):                    # 0x576790: (x,y) pairs, clamped
    if x <= pts[0][0]: return pts[0][1]
    if x >= pts[-1][0]: return pts[-1][1]
    for (xa, ya), (xb, yb) in zip(pts, pts[1:]):
        if xa <= x <= xb: return ya + (x - xa) * (yb - ya) / (xb - xa)

LOOKAHEAD = [22.8972, 22.8972, 24.0, 27.9, 33.0, 39.0, 45.8, 49.4, 53.1, 55.9]   # 0x7e9d94, 0..160
GP = [0.328, 0.22, 0.148, 0.115, 0.09, 0.074, 0.057, 0.049, 0.043, 0.04]          # 0x7e9d44
GI = [0.21, 0.244, 0.267, 0.29, 0.305, 0.321, 0.328, 0.336, 0.341, 0.344]         # 0x7e9d80
GD = [0.0, 0.0758, 0.0758, 0.0738, 0.0604, 0.047, 0.047, 0.0403, 0.0403, 0.0403]   # 0x7e9d58/6c
MAX_STEER = [45, 30, 5.5, 5, 3, 2, 2, 2, 2, 2]                                      # 0x7e9e0c deg
STEER_RATE = [4, 3, 2, 1, 0.45, 0.14, 0.1, 0.1, 0.1, 0.1]                           # 0x7e9e20 /s
PEDAL_RATE = [30, 15, 7.5, 6, 6, 6, 6, 6, 6, 6]                                     # 0x7e9d10 /s
ACC = dict(P=[.1,.1,.1,.1,.005,.1,.1], I=[.02,.02,.02,.02,.005,.02,.02], D=[.02,.02,.01,.01,.01,.01,.01])
BRK = dict(P=[.1,.1,.1,.1,.005,.1,.1], I=[.02,.02,.02,.02,0,.02,.02], D=[.02,.02,.01,.01,.008,.01,.01])
THROTTLE_CAP = [0.93, 0.97, 1.0, 1.0]   # 0x783a28[[0x89e804]] for AI racers (+0x22c)

def lookahead_physics(v_fwd, car_class, physics=True):
    k = (1.3 + (1.1 - 1.3) * car_class / 3) if physics else (0.6 + 0.5 * car_class / 3)
    return graph_even(LOOKAHEAD, 0, 160, v_fwd) * k

class Tracker:                              # 0x4016b0 / update 0x576bb0
    def __init__(self, n, k=30.0):
        self.n, self.k = n, k; self.e = 0.0; self.dts = []; self.inc = []; self.ds = []
    def update(self, e, dt, noD=False, noI=False):
        prev, self.e = self.e, e
        for lst, val in ((self.dts, dt), (self.inc, 0 if noI else 0.5 * (e + prev) * dt),
                         (self.ds, 0 if noD else (e - prev) / dt)):
            lst.append(val); del lst[:-self.n]
    P = property(lambda s: s.e)
    @property
    def I(s): return sum(s.inc) * len(s.inc) / (sum(s.dts) * s.k) if s.inc else 0.0
    @property
    def D(s): return sum(s.ds) / len(s.ds) if s.ds else 0.0

def wrap(a): return (a + math.pi) % (2 * math.pi) - math.pi

class PsychoDriver:
    def __init__(self, cap=1.0):
        self.cap = cap; self.head = Tracker(5); self.off = Tracker(5); self.body = Tracker(5)
        self.spd = Tracker(4); self.steer_deg = 0.0; self.gas = 0.0; self.brake = 0.0

    def update(self, dt, car, target_pt, target_speed, offset_d, mode=1):
        """car: dict x,y,heading(rad),vx,vy,v(|v|),v_fwd. Returns (steer_deg, gas, brake)."""
        dt = max(dt, 1e-4)
        ang = math.atan2(target_pt[1] - car['y'], target_pt[0] - car['x'])
        hErr = wrap(car['heading'] - ang)
        w = min(max((car['v'] - 5) / 5, 0), 1)                       # 0x81e7e4
        vErr = (1 - w) * hErr + w * wrap(math.atan2(car['vy'], car['vx']) - ang) if w > 0 else hErr
        oErr = math.atan2(offset_d, 5.0)                               # angle(5, d)
        self.head.update(hErr, dt); self.off.update(oErr, dt); self.body.update(vErr, dt)
        a, b = self.body.P, self.head.P
        if a * b < 0: wa, wb = 0.0, 1.0
        elif a * a + b * b < 1e-5: wa = wb = 0.5
        else: s = a * a + b * b; wa, wb = a * a / s, b * b / s
        eP = wa * a + wb * b
        eI = max(-0.10472, min(0.10472, wa * self.body.I + wb * self.head.I))
        eD = max(-1, min(1, wa * self.body.D + wb * self.head.D))
        v = car['v']
        steer = (graph_even(GP, 0, 160, v) * -eP + graph_even(GI, 0, 160, v) * -eI
                 + graph_even(GD, 0, 160, v) * -eD)                   # valid for v_fwd >= 10
        steer = max(-0.785398, min(0.785398, steer))
        # speed controller (PID path)
        err = abs(target_speed) - v
        self.spd.update(err, dt)
        if abs(target_speed) + 2 > v: k, lo, hi = ACC, 0.0, self.cap
        else: k, lo, hi = BRK, -1.0, 0.0
        u = (k['P'][mode] * self.spd.P + k['I'][mode] * self.spd.I
             + k['D'][mode] * max(-10, min(10, self.spd.D)))
        lim = graph_even(PEDAL_RATE, 0, 160, v) * dt
        x = max(lo, min(hi, (self.gas - self.brake) + max(-lim, min(lim, u))))
        self.gas, self.brake = (min(x, 1.0), 0.0) if x >= 0 else (0.0, min(-x, 1.0))
        self.gas = max(0.0, min(self.gas, self.cap))
        # steering wheel rate limit
        S = max(graph_even(MAX_STEER, 0, 160, v), 0)
        if S > 0:
            tgt = max(-1, min(1, steer / math.radians(S))); cur = self.steer_deg / S
            r = graph_even(STEER_RATE, 0, 160, v)
            if (cur > 0 and cur > tgt) or (cur < 0 and cur < tgt): r *= 2
            d = max(-r * dt, min(r * dt, tgt - cur)); self.steer_deg = (cur + d) * S
        return self.steer_deg, self.gas, self.brake
