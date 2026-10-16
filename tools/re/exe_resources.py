"""Extracts the PE resources of speed2.exe (the D3DX effects IDI_*_FX are RCDATA,
type 10). usage: exe_resources.py speed2.exe OUT_DIR -> OUT_DIR/IDI_<NAME>_FX.bin
Then disassemble with d3dasm.py."""
import struct, sys, os
d=open(sys.argv[1],'rb').read(); out=sys.argv[2]
pe=struct.unpack_from('<I',d,0x3c)[0]
nsec=struct.unpack_from('<H',d,pe+6)[0]; optsz=struct.unpack_from('<H',d,pe+20)[0]
opt=pe+24
rsrc_rva,rsrc_size=struct.unpack_from('<II',d,opt+96+8*2)
secs=[]
for i in range(nsec):
    o=opt+optsz+40*i
    name=d[o:o+8].rstrip(b'\0'); vsz,va,rsz,rptr=struct.unpack_from('<IIII',d,o+8)
    secs.append((va,max(vsz,rsz),rptr))
def off(rva):
    for va,sz,rp in secs:
        if va<=rva<va+sz: return rva-va+rp
base=off(rsrc_rva)
def name_at(o):
    n=struct.unpack_from('<H',d,base+o)[0]; return d[base+o+2:base+o+2+2*n].decode('utf-16le')
def walk(o,path):
    nnamed,nid=struct.unpack_from('<HH',d,base+o+12)
    for k in range(nnamed+nid):
        nm,ptr=struct.unpack_from('<II',d,base+o+16+8*k)
        key=name_at(nm&0x7fffffff) if nm&0x80000000 else str(nm)
        if ptr&0x80000000: walk(ptr&0x7fffffff,path+[key])
        else:
            rva,size=struct.unpack_from('<II',d,base+ptr)
            data=d[off(rva):off(rva)+size]
            p=path+[key]
            print('/'.join(p),size,data[:40])
            if any('FX' in x for x in p):
                open(os.path.join(out,p[1]+'.bin'),'wb').write(data)
walk(0,[])
