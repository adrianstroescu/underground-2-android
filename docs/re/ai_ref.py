"""NFSU2 (speed2.exe) AI racer reference: skill table, rubber band ("catch-up"),
target speed. Python mirror of the decoded code paths; see ai_re.md for addresses.

Units: distances m, speeds m/s, times s (game ticks are 1/4000 s).
Data: GLOBALB.BUN chunk 0x34b00 (size 0xae0), runtime pointer [0x81cb60].
"""
import struct

# ---------------------------------------------------------------- data access
def load_ai_chunk(globalb_path):
    d = open(globalb_path, "rb").read()

    def walk(o, end):
        while o + 8 <= end:
            cid, sz = struct.unpack_from("<II", d, o)
            if cid == 0x34B00:
                return d[o + 8:o + 8 + sz]
            if cid & 0x80000000:
                r = walk(o + 8, o + 8 + sz)
                if r:
                    return r
            o += 8 + sz
        return None
    return walk(0, len(d))


class AiData:
    def __init__(self, blob):
        self.b = blob

    def f(self, off):
        return struct.unpack_from("<f", self.b, off)[0]

    def graph(self, off, n):          # n (x, y) float pairs
        return [(self.f(off + 8 * i), self.f(off + 8 * i + 4)) for i in range(n)]

    # per-skill row (0x68 bytes) at +0x524 + skill*0x68
    def row(self, skill, field):
        return self.f(0x524 + skill * 0x68 + field)

    def row_byte(self, skill, field):
        return self.b[0x524 + skill * 0x68 + field]


# 0x576790: piecewise-linear graph, clamped at both ends
def graph_eval(pts, x):
    if len(pts) < 2:
        return pts[0][1]
    if x <= pts[0][0]:
        return pts[0][1]
    if x >= pts[-1][0]:
        return pts[-1][1]
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        if x0 <= x <= x1:
            dx = x1 - x0
            if abs(dx) <= 1e-6:
                return y0 + (y1 - y0) * 0.5
            return y0 + (x - x0) / dx * (y1 - y0)
    return pts[0][1]


def lerp_skill(y0, y9, skill):       # 0x576c60/0x576c90 and 0x413390/0x5764d0
    return y0 + (y9 - y0) * skill / 9.0


# ---------------------------------------------------------------- skill pick
def pick_skill(ai, difficulty, r):
    """0x4fed00: r = randf(1.0); 10 cumulative floats at +0x9f4 + difficulty*40."""
    for k in range(10):
        if r <= ai.f(0x9F4 + difficulty * 40 + 4 * k):
            return k
    return 9


# ---------------------------------------------------------------- rubber band
def rubber_band_a(ai, skill, gap_m, my_speed, player_speed, street_x=False,
                  traffic_level=0, laps=1, sprint=False, race_type=1, my_pos=1):
    """0x4084a0 'a' term (the 'b' position term is 0 in the shipped data).
    gap_m = (my_progress - player_progress) * lap_length, >0 = AI ahead.
    Only called when the Catch Up option [0x89e7f8] is on."""
    if gap_m > ai.f(0xADC):                       # 100000 sanity limit
        return 0.0
    if street_x:
        ga, gb = 0x2C0, 0x318
    else:
        lapcls = 1 if (not sprint and laps >= 4) else 0   # table 0x7e95c8 = [0, 4]
        tr = 1 if traffic_level in (2, 3) else 0
        ga = (tr + lapcls * 2) * 0x58
        gb = (tr + lapcls * 2 + 4) * 0x58
    va = graph_eval(ai.graph(ga, 11), gap_m)
    vb = graph_eval(ai.graph(gb, 11), gap_m)
    a = lerp_skill(va, vb, max(skill, 0))
    if race_type == 7 and my_pos != 1:
        a *= 1.5
    # speed matching near the player
    t = (my_speed - player_speed) * -0.005 / (1.0 + 0.01 * gap_m * gap_m)
    a += min(max(t, -0.05), 0.05)
    return a


def catch_up_multiplier(ai, skill, *, race_type, street_x, catch_up_on,
                        race_clock, action_allows, have_target,
                        gap_m=0.0, my_speed=0.0, player_speed=0.0,
                        player_pos=1, player_speed_ratio=1.0,
                        my_progress_laps=0.0, laps=1, traffic_level=0, sprint=False,
                        my_pos=1, drag_or_drift=False):
    """0x4172d0 (re-evaluated every 0.3 s = data+0xad8). Returns m.
    car+0x468 = max(m, 1)  -> engine torque x, speed limiter x, aero+8 (0x599170)
    ctrl+0x770 = m         -> multiplies the AI target speed (0x41c8a8)."""
    skill = max(skill, 0)
    if race_type == 0:
        return 0.75
    if race_type == 7 and my_pos == 1:
        return 1.0
    if drag_or_drift:
        return 1.0
    m = 1.0
    started = race_clock >= ai.row(skill, 0x5C)          # 3..19 s by skill
    if action_allows and have_target and started:
        a = rubber_band_a(ai, skill, gap_m, my_speed, player_speed, street_x,
                          traffic_level, laps, sprint, race_type, my_pos) if catch_up_on else 0.0
        b = 0.0   # position table data+0x424/+0x464 is all zero
        m = 1.0 * b + 1.0 + a
    if action_allows and have_target and not street_x and catch_up_on:
        base = ai.row(skill, 0x18)
        tbl = (0x49C if player_speed_ratio >= 1.0 else 0x4CC) + player_pos * 8
        v = lerp_skill(ai.f(tbl), ai.f(tbl + 4), skill)
        m = max(1.0 + (player_speed_ratio - 1.0) * v, base) * m
    if action_allows and have_target and not street_x:
        x = my_progress_laps / max(laps, 1)
        g = graph_eval(ai.graph(0x504, 4), x)           # 1 until 2/3 of the race, 0 at the end
        m = (m - 1.0) * g + 1.0
    return m


# ---------------------------------------------------------------- target speed
def ai_target_speed(ai, skill, line_speed_at, dist_m, noise_m, m, street_x=False):
    """0x41c5e0 (normal racing case). line_speed_at(d) = route node byte +0x2c (m/s)
    at distance d along the path (0x411dd0); 0 -> 30 m/s.
    noise_m = rand(2*row+0x60) - row+0x60, re-rolled when the 0x4026e0 timer >= 5 s."""
    scale = ai.row(skill, 0x1C) if street_x else ai.row(skill, 0x14)
    d = dist_m if street_x else dist_m + noise_m
    v = line_speed_at(d)
    if abs(v) < 0.001:
        v = 30.0
    return v * scale * m


if __name__ == "__main__":
    import sys
    g = sys.argv[1] if len(sys.argv) > 1 else \
        "D:/GAME install/Need for Speed - Underground 2-NEW/GLOBAL/GLOBALB.BUN"
    ai = AiData(load_ai_chunk(g))
    for s in range(10):
        print(s, "line x%.2f" % ai.row(s, 0x14), "floor %.2f" % ai.row(s, 0x18),
              "start %.0fs" % ai.row(s, 0x5C), "brake noise +-%.0fm" % ai.row(s, 0x60))
    for gap in (-400, -200, -100, 0, 100, 300, 700):
        print(gap, ["%.3f" % rubber_band_a(ai, s, gap, 50, 50) for s in (0, 4, 8)])
