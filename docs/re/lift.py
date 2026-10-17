# lift.py START [END]: x87 symbolic lifter. Prints each insn with symbolic annotations.
import sys, struct, re
from dz import d, off, md

def fl(a):
    o = off(a)
    if o is None: return None
    return struct.unpack_from('<f', d, o)[0]
def dbl(a):
    o = off(a)
    return struct.unpack_from('<d', d, o)[0]

_retcache = {}
def callee_pop(t):
    if t in _retcache: return _retcache[t]
    o = off(t); n = 0
    if o is None: _retcache[t] = 0; return 0
    # follow a leading jmp thunk
    for ins in md.disasm(d[o:o + 0x3000], t):
        if ins.mnemonic == 'ret':
            n = int(ins.op_str, 16) if ins.op_str else 0
            break
    _retcache[t] = n
    return n

REGS = ['eax', 'ecx', 'edx', 'ebx', 'esi', 'edi', 'ebp']
LIM = 160
def short(e):
    return e if len(e) <= LIM else e[:LIM] + '...'

def run(st, en=None, maxn=6000):
    o = off(st)
    insns = []
    for ins in md.disasm(d[o:o + 0x10000], st):
        insns.append(ins)
        if en and ins.address >= en: break
        if not en and ins.mnemonic == 'ret':
            nb = d[off(ins.address + ins.size)]
            if nb in (0x90, 0xcc): break
        if len(insns) > maxn: break
    targets = set()
    for ins in insns:
        if ins.mnemonic.startswith('j') and ins.op_str.startswith('0x'):
            targets.add(int(ins.op_str, 16))
    R = {r: r for r in REGS}; R['ecx'] = 'this'
    espd = 0; ebp_is_frame = False
    fst = []  # x87 stack, fst[-1] = st0
    pushes = []
    out = []
    lastcmp = ''
    def memname(op):
        m = re.search(r'\[([^\]]+)\]', op)
        if not m: return None
        inner = m.group(1).replace(' ', '')
        mm = re.fullmatch(r'0x([0-9a-f]+)', inner)
        if mm:
            a = int(mm.group(1), 16)
            if 'qword' in op and 0x780000 <= a < 0x900000 and off(a) is not None:
                v = dbl(a)
                return '%g' % v if abs(v) < 1e7 else '[%s]' % hex(a)
            if 'dword' in op and 0x780000 <= a < 0x830000 and off(a) is not None:
                v = fl(a)
                if v is not None and (v == 0 or 1e-8 < abs(v) < 1e8):
                    return '%g' % v if a < 0x7e8000 else '%s{%g}' % (hex(a)[2:], v)
            return 'G[%s]' % hex(a)
        mm = re.fullmatch(r'([a-z]+)([+-]0x[0-9a-f]+|[+-]\d+)?', inner)
        if mm:
            reg = mm.group(1); disp = int(mm.group(2), 0) if mm.group(2) else 0
            if reg == 'esp':
                return 'L%x' % ((espd + disp) & 0xffff) if espd + disp >= 0 else 'L-%x' % (-(espd + disp))
            if reg == 'ebp' and R.get('ebp') == 'FRAME':
                return 'ARG%d' % ((disp - 8) // 4) if disp >= 8 else 'BP%x' % (-disp)
            base = R.get(reg, reg)
            if disp == 0: return '%s.0' % base if not base.startswith('&') else '*' + base[1:]
            if base.startswith('&'):
                return '(%s+%x)' % (base[1:], disp) if disp > 0 else '(%s-%x)' % (base[1:], -disp)
            return '%s.%x' % (base, disp) if disp >= 0 else '%s.-%x' % (base, -disp)
        # index forms
        def sub(mo):
            t = mo.group(0)
            return R.get(t, t) if t in REGS else t
        return '[' + re.sub(r'[a-z]{3}', sub, inner) + ']'
    def st(i):
        return fst[-1 - i] if len(fst) > i else 'st%d?' % i
    def setst(i, v):
        while len(fst) <= i: fst.insert(0, 'st?')
        fst[-1 - i] = short(v)
    for ins in insns:
        a = ins.address; mn = ins.mnemonic; op = ins.op_str; b = bytes(ins.bytes)
        note = ''
        if a in targets:
            out.append('    L_%x:' % a)
        ops = [x.strip() for x in op.split(',')] if op else []
        try:
            if mn in ('fld', 'fild'):
                if ops[0].startswith('st('):
                    fst.append(st(int(ops[0][3])))
                else:
                    fst.append(memname(op))
                note = 'push ' + st(0)
            elif mn == 'fld1': fst.append('1'); note = 'push 1'
            elif mn == 'fldz': fst.append('0'); note = 'push 0'
            elif mn == 'fldpi': fst.append('pi'); note = 'push pi'
            elif mn in ('fst', 'fstp', 'fist', 'fistp'):
                v = st(0)
                if ops[0].startswith('st('):
                    i = int(ops[0][3]); setst(i, v)
                    note = 'st%d := %s' % (i, v)
                else:
                    note = '>>> %s := %s' % (memname(op), v)
                if mn.endswith('p') and fst: fst.pop()
            elif mn in ('fadd', 'fsub', 'fsubr', 'fmul', 'fdiv', 'fdivr', 'fiadd', 'fisub', 'fimul', 'fidiv') and '[' in op:
                m = memname(op); x = st(0); k = mn.replace('fi', 'f')
                e = {'fadd': '(%s + %s)', 'fsub': '(%s - %s)', 'fmul': '%s*%s', 'fdiv': '%s/%s'}
                if k == 'fsubr': v = '(%s - %s)' % (m, x)
                elif k == 'fdivr': v = '%s/(%s)' % (m, x)
                else: v = e[k] % (x, m)
                setst(0, v); note = 'st0 = ' + st(0)
            elif b[0] in (0xd8, 0xdc, 0xde) and len(b) == 2 and b[1] >= 0xc0:
                i = b[1] & 7; g = (b[1] >> 3) & 7  # 0 add 1 mul 2 com 3 comp 4 sub 5 subr 6 div 7 divr
                if b[0] == 0xd8:
                    x, y = st(0), st(i)
                    r = {0: '(%s + %s)' % (x, y), 1: '%s*%s' % (x, y), 4: '(%s - %s)' % (x, y), 5: '(%s - %s)' % (y, x), 6: '%s/(%s)' % (x, y), 7: '%s/(%s)' % (y, x)}
                    if g in r: setst(0, r[g]); note = 'st0 = ' + st(0)
                    else:
                        lastcmp = 'cmp %s : %s' % (x, y); note = lastcmp
                        if g == 3: fst.pop()
                else:
                    x, y = st(i), st(0)  # dest st(i)
                    # DC/DE: E0+i = subr (st0 - sti), E8+i = sub (sti - st0), F0+i divr (st0/sti), F8+i div (sti/st0)
                    r = {0: '(%s + %s)' % (x, y), 1: '%s*%s' % (x, y), 4: '(%s - %s)' % (y, x), 5: '(%s - %s)' % (x, y), 6: '%s/(%s)' % (y, x), 7: '%s/(%s)' % (x, y)}
                    if g in r:
                        setst(i, r[g]); note = 'st%d = %s' % (i, st(i))
                        if b[0] == 0xde: fst.pop(); note = 'pop; st%d = %s' % (i - 1, st(i - 1))
                    else:
                        lastcmp = 'cmp %s : %s' % (st(0), st(i)); note = lastcmp
                        if b[0] == 0xde: fst.pop(); fst.pop()
            elif mn in ('fcom', 'fcomp', 'fcompp', 'fucom', 'fucomp', 'fucompp', 'ficomp', 'ficom', 'fcomi', 'fcomip', 'fucomip'):
                if '[' in op: y = memname(op)
                elif ops: y = st(int(ops[-1][3])) if ops[-1].startswith('st(') else st(1)
                else: y = st(1)
                lastcmp = 'cmp %s : %s' % (st(0), y); note = lastcmp
                n = 2 if mn.endswith('pp') else (1 if mn.endswith('p') else 0)
                for _ in range(n):
                    if fst: fst.pop()
            elif mn == 'ftst':
                lastcmp = 'cmp %s : 0' % st(0); note = lastcmp
            elif mn == 'fxch':
                i = int(ops[0][3]) if ops else 1
                x, y = st(0), st(i); setst(0, y); setst(i, x); note = 'xch'
            elif mn in ('fchs', 'fabs', 'fsqrt', 'fsin', 'fcos', 'frndint'):
                f = {'fchs': '-(%s)', 'fabs': 'abs(%s)', 'fsqrt': 'sqrt(%s)', 'fsin': 'sin(%s)', 'fcos': 'cos(%s)', 'frndint': 'rnd(%s)'}[mn]
                setst(0, f % st(0)); note = 'st0 = ' + st(0)
            elif mn == 'fpatan':
                v = 'atan2(%s, %s)' % (st(1), st(0)); fst.pop(); setst(0, v); note = 'st0 = ' + v
            elif mn == 'fsincos':
                x = st(0); setst(0, 'sin(%s)' % x); fst.append('cos(%s)' % x); note = 'sincos'
            elif mn == 'fstp' or mn == 'ffree':
                pass
            elif mn == 'fnstsw':
                pass
            elif mn == 'push':
                espd -= 4
                v = memname(op) if '[' in op else R.get(op, op)
                pushes.append(v)
                if '[' in op: note = 'push ' + str(v)
                if op == 'ebp': pass
            elif mn == 'pop':
                espd += 4
                if op in R: R[op] = op if op != 'ecx' else 'ecx'
                if op == 'ebp': R['ebp'] = 'ebp'
            elif mn == 'sub' and ops[0] == 'esp':
                espd -= int(ops[1], 0)
            elif mn == 'add' and ops[0] == 'esp':
                espd += int(ops[1], 0)
                pushes = pushes[:max(0, len(pushes) - int(ops[1], 0) // 4)]
            elif mn == 'and' and ops[0] == 'esp':
                espd = 0
            elif mn == 'mov' and ops[0] == 'ebp' and ops[1] == 'esp':
                R['ebp'] = 'FRAME'
            elif mn == 'mov' and ops[0] == 'esp' and ops[1] == 'ebp':
                pass
            elif mn in ('mov', 'movzx', 'movsx') and ops[0] in R:
                if '[' in op: v = memname(op)
                elif ops[1] in R: v = R[ops[1]]
                else: v = ops[1]
                R[ops[0]] = v; note = '%s = %s' % (ops[0], v)
            elif mn == 'lea' and ops[0] in R:
                m = memname(op); R[ops[0]] = '&' + (m or '?'); note = '%s = %s' % (ops[0], R[ops[0]])
            elif mn == 'mov' and '[' in ops[0]:
                v = R.get(ops[1], ops[1]) if not ops[1].startswith('0x') else ops[1]
                note = '>>> %s := %s' % (memname(op.split(',')[0]), v)
            elif mn == 'call':
                args = list(reversed(pushes))
                t = int(op, 16) if op.startswith('0x') else None
                tgt = op if t is None else hex(t)
                if '[' in op: tgt = memname(op)
                note = 'CALL %s(this=%s; %s)' % (tgt, R['ecx'], ', '.join(str(x) for x in args[:6]))
                if t:
                    n = callee_pop(t)
                    espd += n; pushes = pushes[:max(0, len(pushes) - n // 4)]
                else:
                    # indirect: assume thiscall pops all args seen
                    pass
                R['eax'] = 'ret_%x' % a; R['ecx'] = 'ecx?'; R['edx'] = 'edx?'
            elif mn in ('xor',) and ops[0] == ops[1] and ops[0] in R:
                R[ops[0]] = '0'
            elif ops and ops[0] in R and mn not in ('cmp', 'test'):
                R[ops[0]] = '%s(%s)' % (mn, R[ops[0]] if len(ops) == 1 else R[ops[0]] + ',' + (R.get(ops[1], ops[1]) if len(ops) > 1 else ''))
                if mn in ('inc', 'dec', 'add', 'sub', 'shl', 'imul') and len(R[ops[0]]) > 60: R[ops[0]] = ops[0] + "'"
            elif mn in ('cmp', 'test'):
                note = '%s %s' % (mn, ', '.join(memname(x) if '[' in x else R.get(x, x) for x in ops))
        except Exception as ex:
            note = 'ERR %s' % ex
        s = '%06x  %-38s' % (a, mn + ' ' + op)
        if note: s += ' ; ' + note
        out.append(s)
    return out

if __name__ == '__main__':
    st0 = int(sys.argv[1], 16); en = int(sys.argv[2], 16) if len(sys.argv) > 2 else None
    print('\n'.join(run(st0, en)))
