"""x86 helpers for speed2.exe (needs `pip install capstone pefile`).

    python -I tools/re/x86.py EXE xref STRING          # code that pushes/moves the string's address
    python -I tools/re/x86.py EXE xrefva VA            # code that references a VA (hex)
    python -I tools/re/x86.py EXE dis VA [COUNT]       # disassemble from VA
    python -I tools/re/x86.py EXE func VA              # disassemble the function containing VA
    python -I tools/re/x86.py EXE float VA [N]         # read N floats at VA
    python -I tools/re/x86.py EXE calls VA             # callers of the function at VA

VAs are virtual addresses (image base 0x400000).
"""
import struct
import sys

import capstone
import pefile


class Exe:
    def __init__(self, path):
        self.pe = pefile.PE(path, fast_load=True)
        self.base = self.pe.OPTIONAL_HEADER.ImageBase
        self.data = self.pe.get_memory_mapped_image()
        self.md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
        self.md.detail = False
        text = [s for s in self.pe.sections if s.Characteristics & 0x20000000]
        self.text = [(self.base + s.VirtualAddress, self.base + s.VirtualAddress + s.Misc_VirtualSize) for s in text]

    def read(self, va, n):
        o = va - self.base
        return self.data[o:o + n]

    def find_string(self, s):
        b = s.encode() + b'\0'
        out, i = [], 0
        while True:
            i = self.data.find(b, i)
            if i < 0:
                return out
            # whole string: previous byte is a terminator or padding
            if i == 0 or self.data[i - 1] == 0:
                out.append(self.base + i)
            i += 1

    def xref(self, va):
        """Offsets in code where the 4-byte little-endian value `va` appears."""
        needle = struct.pack('<I', va)
        out = []
        for lo, hi in self.text:
            seg = self.data[lo - self.base:hi - self.base]
            i = 0
            while True:
                i = seg.find(needle, i)
                if i < 0:
                    break
                out.append(lo + i)
                i += 1
        return out

    def insn_at(self, ref):
        """The instruction that contains the operand at `ref` (try a few start offsets)."""
        for back in range(1, 8):
            start = ref - back
            for ins in self.md.disasm(self.read(start, 16), start):
                if ins.address == start and start + ins.size > ref:
                    return ins
                break
        return None

    def dis(self, va, count=40):
        out = []
        for ins in self.md.disasm(self.read(va, count * 8), va):
            out.append(ins)
            if len(out) >= count:
                break
        return out

    def func_start(self, va):
        """Walks back to the nearest function prologue or int3/ret padding."""
        for a in range(va, va - 0x4000, -1):
            b = self.read(a - 1, 4)
            if b[0] in (0xCC, 0xC3, 0x90) and b[1:3] in (b'\x55\x8b', b'\x83\xec', b'\x81\xec', b'\x56\x8b', b'\x53\x8b', b'\x51\x53', b'\x51\x56', b'\x6a\xff', b'\x64\xa1') :
                return a
            if b[0] == 0xC2 and False:
                return a
        return va

    def func(self, va):
        start = self.func_start(va)
        out = []
        for ins in self.md.disasm(self.read(start, 0x3000), start):
            out.append(ins)
            if ins.mnemonic == 'int3' or (ins.mnemonic in ('ret', 'retn') and ins.address > va):
                break
        return out


def fmt(ins):
    return f"{ins.address:08x}  {ins.mnemonic:8} {ins.op_str}"


def main():
    exe = Exe(sys.argv[1])
    cmd = sys.argv[2]
    if cmd == 'xref':
        for va in exe.find_string(sys.argv[3]):
            print(f"string at {va:08x}")
            for r in exe.xref(va):
                ins = exe.insn_at(r)
                print('   ', fmt(ins) if ins else f"{r:08x} ?")
    elif cmd == 'xrefva':
        va = int(sys.argv[3], 16)
        for r in exe.xref(va):
            ins = exe.insn_at(r)
            print(fmt(ins) if ins else f"{r:08x} ?")
    elif cmd == 'dis':
        n = int(sys.argv[4]) if len(sys.argv) > 4 else 40
        for ins in exe.dis(int(sys.argv[3], 16), n):
            print(fmt(ins))
    elif cmd == 'func':
        for ins in exe.func(int(sys.argv[3], 16)):
            print(fmt(ins))
    elif cmd == 'float':
        va = int(sys.argv[3], 16)
        n = int(sys.argv[4]) if len(sys.argv) > 4 else 1
        print(struct.unpack(f'<{n}f', exe.read(va, 4 * n)))
    elif cmd == 'calls':
        va = int(sys.argv[3], 16)
        for lo, hi in exe.text:
            seg = exe.data[lo - exe.base:hi - exe.base]
            for i in range(len(seg) - 5):
                if seg[i] == 0xE8:
                    rel = struct.unpack('<i', seg[i + 1:i + 5])[0]
                    if lo + i + 5 + rel == va:
                        print(f"{lo + i:08x}")


if __name__ == '__main__':
    main()
