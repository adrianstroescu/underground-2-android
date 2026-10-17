import struct
from car_chunks import C, S, g
def u32(b,o): return struct.unpack_from('<I',b,o)[0]
# CarTypeInfo chunk 0x34600 at top level of GLOBALB
o=0
while o+8<=len(g):
    cid,sz=struct.unpack_from('<II',g,o)
    if cid==0x34600: body=g[o+8:o+8+sz]; break
    o+=8+sz
R=0x890; st=len(body)%R
recs=[body[st+i*R:st+(i+1)*R] for i in range(len(body)//R)]
def cs(b,o): return b[o:o+32].split(b'\0')[0].decode('latin1')
byh={}
for i,r in enumerate(recs):
    byh[u32(r,0xd0)]=i
    print(i, cs(r,0).ljust(12), cs(r,0x40).ljust(12), 'h=%08x'%u32(r,0xd0), 'da=%02x'%r[0xda], 'type840=%d'%u32(r,0x840), 'c0=%s'%cs(r,0xc0))
print('chunk 0x34a1f', len(C[0x34a1f]))
races=[C[0x34a11][i:i+0x88] for i in range(0,len(C[0x34a11]),0x88)]
rb={u32(r,8):S(struct.unpack_from('<H',r,0)[0]) for r in races}
for k in range(0,len(C[0x34a1f]),12):
    a,b,c=struct.unpack_from('<3I',C[0x34a1f],k)
    print(' car %08x (%s) normal %s alt %s'%(a, cs(recs[byh[a]],0) if a in byh else '?', rb.get(b,hex(b)), rb.get(c,hex(c))))
print('chunk 0x34a1e', len(C[0x34a1e]))
for k in range(0,len(C[0x34a1e]),12):
    print(' ', ['%08x'%x for x in struct.unpack_from('<3I',C[0x34a1e],k)])
print('--- +0x844, +0x88a, +0xda per record')
for i,r in enumerate(recs):
    print(i, cs(r,0), 'x844=%d'%u32(r,0x844), 'suv88a=%d'%r[0x88a], 'da=%02x'%r[0xda])
