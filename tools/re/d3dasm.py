"""Minimal D3D9 shader bytecode disassembler (SM 1.x - 3.0) for effect blobs.
usage: d3dasm.py effect.bin   -> disassembles every shader found in the file."""
import struct, sys

OPS = {0: ('nop', 0), 1: ('mov', 2), 2: ('add', 3), 3: ('sub', 3), 4: ('mad', 4), 5: ('mul', 3), 6: ('rcp', 2),
       7: ('rsq', 2), 8: ('dp3', 3), 9: ('dp4', 3), 10: ('min', 3), 11: ('max', 3), 12: ('slt', 3), 13: ('sge', 3),
       14: ('exp', 2), 15: ('log', 2), 16: ('lit', 2), 17: ('dst', 3), 18: ('lrp', 4), 19: ('frc', 2), 20: ('m4x4', 3),
       21: ('m4x3', 3), 22: ('m3x4', 3), 23: ('m3x3', 3), 24: ('m3x2', 3), 25: ('call', 1), 26: ('callnz', 2),
       27: ('loop', 2), 28: ('ret', 0), 29: ('endloop', 0), 30: ('label', 1), 31: ('dcl', None), 32: ('pow', 3),
       33: ('crs', 3), 34: ('sgn', 4), 35: ('abs', 2), 36: ('nrm', 2), 37: ('sincos', None), 38: ('rep', 1),
       39: ('endrep', 0), 40: ('if', 1), 41: ('ifc', 2), 42: ('else', 0), 43: ('endif', 0), 44: ('break', 0),
       45: ('breakc', 2), 46: ('mova', 2), 47: ('defb', None), 48: ('defi', None), 64: ('texcoord', None),
       65: ('texkill', 1), 66: ('tex', None), 67: ('texbem', 2), 68: ('texbeml', 2), 69: ('texreg2ar', 2),
       70: ('texreg2gb', 2), 71: ('texm3x2pad', 2), 72: ('texm3x2tex', 2), 73: ('texm3x3pad', 2),
       74: ('texm3x3tex', 2), 76: ('texm3x3spec', 3), 77: ('texm3x3vspec', 2), 78: ('expp', 2), 79: ('logp', 2),
       80: ('cnd', 4), 81: ('def', None), 82: ('texreg2rgb', 2), 83: ('texdp3tex', 2), 84: ('texm3x2depth', 2),
       85: ('texdp3', 2), 86: ('texm3x3', 2), 87: ('texdepth', 1), 88: ('cmp', 4), 89: ('bem', 3), 90: ('dp2add', 4),
       91: ('dsx', 2), 92: ('dsy', 2), 93: ('texldd', 5), 94: ('setp', 3), 95: ('texldl', 3), 96: ('breakp', 1),
       0xFFFD: ('phase', 0)}

USAGE = ['position', 'blendweight', 'blendindices', 'normal', 'psize', 'texcoord', 'tangent', 'binormal',
         'tessfactor', 'positiont', 'color', 'fog', 'depth', 'sample']


def regname(t, num, ps, ver):
    ver = (ver >> 8) & 0xFF
    names = {0: 'r', 1: 'v', 2: 'c', 3: ('t' if ps else 'a'), 4: None, 5: 'oD', 6: ('oT' if ver < 3 else 'o'),
             7: 'i', 8: 'oC', 9: 'oDepth', 10: 's', 11: 'c', 12: 'c', 13: 'c', 14: 'b', 15: 'aL', 17: 'misc',
             18: 'l', 19: 'p'}
    if t == 4:
        return ['oPos', 'oFog', 'oPts'][num]
    if t in (11, 12, 13):
        num += {11: 2048, 12: 4096, 13: 6144}[t]
    return f"{names.get(t, '?%d' % t)}{num}"


def reg_of(tok):
    return ((tok >> 28) & 7) | ((tok >> 8) & 0x18), tok & 0x7FF


def dst_str(tok, ps, ver):
    t, n = reg_of(tok)
    mask = (tok >> 16) & 0xF
    mod = (tok >> 20) & 0xF
    shift = (tok >> 24) & 0xF
    s = regname(t, n, ps, ver)
    if mask != 0xF:
        s += '.' + ''.join('xyzw'[i] for i in range(4) if mask >> i & 1)
    pre = ''
    if mod & 1: pre += '_sat'
    if mod & 2: pre += '_pp'
    if mod & 4: pre += '_centroid'
    if shift:
        sh = shift if shift < 8 else shift - 16
        pre += {1: '_x2', 2: '_x4', 3: '_x8', -1: '_d2', -2: '_d4', -3: '_d8'}.get(sh, f'_sh{sh}')
    return pre, s


SRCMOD = {0: '{}', 1: '-{}', 2: '{}_bias', 3: '-{}_bias', 4: '{}_bx2', 5: '-{}_bx2', 6: '1-{}', 7: '{}_x2',
          8: '-{}_x2', 9: '{}_dz', 10: '{}_dw', 11: 'abs({})', 12: '-abs({})', 13: '!{}'}


def src_str(tok, ps, ver, rel=None):
    t, n = reg_of(tok)
    s = regname(t, n, ps, ver)
    if tok & (1 << 13):
        s = s[0] + f"[a0.x + {n}]" if rel is None else f"{s[0]}[{rel} + {n}]"
    sw = (tok >> 16) & 0xFF
    comp = [(sw >> (2 * i)) & 3 for i in range(4)]
    if comp != [0, 1, 2, 3]:
        c = ''.join('xyzw'[k] for k in comp)
        if len(set(comp)) == 1:
            c = c[0]
        s += '.' + c
    return SRCMOD.get((tok >> 24) & 0xF, '{}?').format(s)


def disasm(words, consts=None):
    ver = words[0]
    ps = (ver >> 16) == 0xFFFF
    major, minor = (ver >> 8) & 0xFF, ver & 0xFF
    out = [f"{'ps' if ps else 'vs'}_{major}_{minor}"]
    i = 1
    while i < len(words):
        tok = words[i]
        op = tok & 0xFFFF
        if tok == 0x0000FFFF:
            break
        if op == 0xFFFE:
            n = (tok >> 16) & 0x7FFF
            blob = struct.pack(f'<{n}I', *words[i + 1:i + 1 + n])
            if blob[:4] == b'CTAB' and consts is not None:
                consts.extend(parse_ctab(blob[4:]))
            i += 1 + n
            continue
        length = (tok >> 24) & 0xF
        name, nops = OPS.get(op, (f'op{op}', None))
        coissue = '+' if tok & (1 << 30) else ''
        if major >= 2:
            ops = words[i + 1:i + 1 + length]
            i += 1 + length
        else:
            if name == 'dcl':
                nops = 2
            elif name == 'def':
                nops = 5
            elif name in ('tex', 'texcoord'):
                nops = 1 if (major, minor) < (1, 4) else 2
            elif name == 'sincos':
                nops = 4
            elif nops is None:
                nops = 2
            ops = words[i + 1:i + 1 + nops]
            i += 1 + nops
        if name == 'dcl':
            u = ops[0]
            pre, d = dst_str(ops[1], ps, ver)
            t, _ = reg_of(ops[1])
            if t == 10:
                out.append(f"dcl_{['?', '?', '2d', 'cube', 'volume'][(u >> 27) & 0xF]} {d}")
            else:
                out.append(f"dcl_{USAGE[u & 0x1F]}{(u >> 16) & 0xF or ''} {d}")
            continue
        if name in ('def', 'defi'):
            pre, d = dst_str(ops[0], ps, ver)
            vals = struct.unpack('<4f', struct.pack('<4I', *ops[1:5])) if name == 'def' else ops[1:5]
            out.append(f"{name} {d}, " + ', '.join(f'{v:g}' for v in vals))
            continue
        if not ops:
            out.append(name)
            continue
        if name in ('texkill', 'call', 'label', 'rep', 'if', 'loop', 'breakp'):
            out.append(f"{coissue}{name} " + ', '.join(src_str(o, ps, ver) for o in ops))
            continue
        pre, d = dst_str(ops[0], ps, ver)
        srcs = [src_str(o, ps, ver) for o in ops[1:]]
        out.append(f"{coissue}{name}{pre} {d}" + (', ' + ', '.join(srcs) if srcs else ''))
    return out


def parse_ctab(b):
    size, creator, version, n, info = struct.unpack_from('<5I', b, 0)
    res = []

    def cstr(o):
        e = b.index(b'\0', o)
        return b[o:e].decode('latin1')

    for k in range(n):
        name_o, rset, ridx, rcount, _, type_o, def_o = struct.unpack_from('<IHHHHII', b, info + 20 * k)
        res.append((cstr(name_o), ['b', 'i', 'c', 's'][rset], ridx, rcount))
    return res


def shaders_in(d):
    """Yields (offset, words) for each shader blob (version token ... end token)."""
    o = 0
    while o + 4 <= len(d):
        w = struct.unpack_from('<I', d, o)[0]
        hi, lo = w >> 16, w & 0xFFFF
        if hi in (0xFFFE, 0xFFFF) and (lo >> 8) in (1, 2, 3) and (lo & 0xFF) <= 4:
            words = []
            p = o
            while p + 4 <= len(d):
                t = struct.unpack_from('<I', d, p)[0]
                words.append(t)
                p += 4
                if t == 0x0000FFFF:
                    break
                if len(words) > 20000:
                    break
            yield o, words
            o = p
        else:
            o += 4


if __name__ == '__main__':
    d = open(sys.argv[1], 'rb').read()
    for off, words in shaders_in(d):
        consts = []
        try:
            lines = disasm(words, consts)
        except Exception as e:  # noqa
            print(f'--- {off:#x}: failed {e}')
            continue
        print(f'--- shader at {off:#x} ({len(words)} words)')
        for c in consts:
            print(f'   // {c[0]}: {c[1]}{c[2]} ({c[3]})')
        for l in lines:
            print('   ' + l)
