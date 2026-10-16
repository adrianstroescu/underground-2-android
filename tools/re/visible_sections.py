"""Dumps the visible section manager (chunk 0x80034150) of a region index.
usage: visible_sections.py TRACKS/L4RB.BUN [N]   (N = records to print per table)"""
import struct, sys
d=open(sys.argv[1],"rb").read()
def find(cid):
    off=0
    while off+8<=len(d):
        c,s=struct.unpack_from('<II',d,off)
        if c==0x80034150:
            sub=off+8
            while sub<off+8+s:
                c2,s2=struct.unpack_from('<II',d,sub)
                if c2==cid: return sub+8,s2
                sub+=8+s2
        off+=8+s
    return None
b,s=find(0x34151)
v,n=struct.unpack_from('<II',d,b); print('34151',v,n,list(struct.unpack_from(f'<{n}H',d,b+8)))
b,s=find(0x34152); p=b; recs=[]
while p<b+s:
    h1,h2,sec,npt=struct.unpack_from('<IIHH',d,p)
    bb=struct.unpack_from('<6f',d,p+12)
    pts=[struct.unpack_from('<2f',d,p+36+8*i) for i in range(npt)]
    recs.append((sec,npt,bb)); p+=36+8*npt
print('34152',len(recs)); [print('  ',r[0],r[1],[round(x) for x in r[2]]) for r in recs[:int(sys.argv[2]) if len(sys.argv)>2 else 999]]
b,s=find(0x34153); p=b; k=0
print('34153 size',s)
while p<b+s:
    h1,h2,sec,a,bb,n=struct.unpack_from('<IIHHII',d,p)
    l=list(struct.unpack_from(f'<{n}H',d,p+20))
    if k< (int(sys.argv[2]) if len(sys.argv)>2 else 999): print('  ',h1,h2,sec,a,bb,n,l)
    p+=164; k+=1
print('records',k)
b,s=find(0x34154); p=b
while p<b+s:
    h1,h2,sec,n=struct.unpack_from('<IIII',d,p); l=list(struct.unpack_from(f'<{n}I',d,p+16)); print('34154',sec,n,l); p+=16+4*max(n,4)
