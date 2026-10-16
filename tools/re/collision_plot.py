"""Top-down PNG of the track collision (0x80034130) around a point: ground polygons
blue, walls red (yellow = flag 0x04, infinite walls). Optional race line CSV
(track,x,y,z per line) and a height filter (+-5 m around Z).
usage: collision_plot.py STREAML4RA.BUN X Y RADIUS out.png [line.csv] [Z]"""
import struct, sys
from PIL import Image, ImageDraw
d=open(sys.argv[1],"rb").read(); cx,cy,R=float(sys.argv[2]),float(sys.argv[3]),float(sys.argv[4]); out=sys.argv[5]
pts=[tuple(map(float,l.split(',')[1:])) for l in open(sys.argv[6])] if len(sys.argv)>6 else []
ZC=float(sys.argv[7]) if len(sys.argv)>7 else None
W=900; sc=W/(2*R)
im=Image.new('RGB',(W,W),(0,0,0)); dr=ImageDraw.Draw(im)
P=lambda x,y:((x-cx)*sc+W/2, W/2-(y-cy)*sc)
off=0
while off+8<=len(d):
    cid,s=struct.unpack_from('<II',d,off)
    if cid==0x80034130:
        sub=off+8; parts={}
        while sub<off+8+s:
            c2,s2=struct.unpack_from('<II',d,sub); parts[c2]=(sub+8,s2); sub+=8+s2
        qb,qs=parts.get(0x34134,(0,0))
        for i in range(qs//32):
            r=d[qb+32*i:qb+32*i+32]
            X=[v/8 for v in struct.unpack_from('<4h',r,8)]; Y=[v/8 for v in struct.unpack_from('<4h',r,16)]; zb=struct.unpack_from('<h',r,4)[0]; Z=[zb+v/256 for v in struct.unpack_from('<4h',r,24)]
            if min(abs(x-cx) for x in X)>R*1.5 or min(abs(y-cy) for y in Y)>R*1.5: continue
            wall=max(Z)-min(Z)>1.5
            if ZC is not None and (min(Z)>ZC+5 or max(Z)<ZC-5): continue
            col=(255,60,60) if r[3]&4==0 else (255,200,0)
            poly=[P(x,y) for x,y in zip(X,Y)]
            if wall: dr.line(poly+[poly[0]],fill=col,width=3)
            else: dr.polygon(poly,outline=(80,200,255),fill=(20,50,70))
    off+=8+s
for x,y,z in pts:
    if abs(x-cx)<R and abs(y-cy)<R: dr.ellipse([P(x,y)[0]-3,P(x,y)[1]-3,P(x,y)[0]+3,P(x,y)[1]+3],fill=(255,255,0))
im.save(out)
