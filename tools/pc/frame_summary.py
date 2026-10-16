"""Summarise one frame of an `apitrace dump` of the PC game (Direct3D 9).

    python -I tools/pc/frame_summary.py frame.txt [FIRST_LINE] [LAST_LINE] [--consts]

Prints one line per draw call with the render target, viewport, shaders, textures
and the blend / depth / fog state, so passes (reflection, scene, cube map, post
processing) can be told apart. With --consts it also prints the shader constants
set since the previous draw.
"""
import re
import sys

CALL = re.compile(r'^(\d+) IDirect3D\w+::(\w+)\((.*)\)( = .*)?$')
ARG = re.compile(r'(\w+) = (\{[^}]*\}|&\{[^}]*\}|[^,]+)')


def args_of(s):
    return {k: v.strip() for k, v in ARG.findall(s)}


def main():
    argv = [a for a in sys.argv[1:] if not a.startswith('--')]
    consts = '--consts' in sys.argv
    path = argv[0]
    first = int(argv[1]) if len(argv) > 1 else 0
    last = int(argv[2]) if len(argv) > 2 else 1 << 60
    rt, ds, vp, vs, ps = '?', '?', '?', '?', '?'
    tex = {}
    rs = {}
    pending = []
    with open(path, encoding='utf-8', errors='replace') as f:
        for n, line in enumerate(f, 1):
            m = CALL.match(line.strip())
            if not m:
                continue
            call, fn, body = m.group(1), m.group(2), m.group(3)
            a = args_of(body)
            if fn == 'SetRenderTarget':
                rt = a.get('pRenderTarget', '?')
            elif fn == 'SetDepthStencilSurface':
                ds = a.get('pNewZStencil', '?')
            elif fn == 'SetViewport':
                v = re.findall(r'(\w+) = (\d+)', body)
                d = dict(v)
                vp = f"{d.get('Width')}x{d.get('Height')}+{d.get('X')}+{d.get('Y')}"
            elif fn == 'SetVertexShader':
                vs = a.get('pShader', '?')
            elif fn == 'SetPixelShader':
                ps = a.get('pShader', '?')
            elif fn == 'SetTexture':
                tex[int(a['Stage'])] = a['pTexture']
            elif fn == 'SetRenderState':
                rs[a['State'].replace('D3DRS_', '')] = a['Value'].replace('D3DBLEND_', '').replace('D3DCMP_', '')
            elif fn in ('SetVertexShaderConstantF', 'SetPixelShaderConstantF') and consts:
                data = re.search(r'pConstantData = \{([^}]*)\}', body)
                pending.append(f"    {fn[3]}S c{a['StartRegister']}: {data.group(1) if data else '?'}")
            elif fn.startswith('Draw') or fn in ('Clear', 'StretchRect'):
                if n < first or n > last:
                    pending = []
                    continue
                blend = '-'
                if rs.get('ALPHABLENDENABLE') == 'TRUE':
                    blend = f"{rs.get('SRCBLEND')}/{rs.get('DESTBLEND')}"
                z = ('Z' if rs.get('ZENABLE', '').endswith('TRUE') else 'z') + \
                    ('W' if rs.get('ZWRITEENABLE') == 'TRUE' else 'w')
                fog = 'F' if rs.get('FOGENABLE') == 'TRUE' else 'f'
                at = 'A' + rs.get('ALPHAREF', '') if rs.get('ALPHATESTENABLE') == 'TRUE' else ''
                ts = ' '.join(f"{k}:{v}" for k, v in sorted(tex.items()) if v != 'NULL')
                extra = ''
                if fn == 'DrawIndexedPrimitive':
                    extra = f"n={a.get('primCount')}"
                elif fn == 'DrawPrimitiveUP':
                    extra = f"up n={a.get('PrimitiveCount')} stride={a.get('VertexStreamZeroStride')}"
                elif fn == 'Clear':
                    extra = f"{a.get('Flags')} {a.get('Color')}"
                elif fn == 'StretchRect':
                    extra = body
                print(f"{n:6} {call} {fn[:12]:12} rt={rt} ds={ds} vp={vp} vs={vs} ps={ps} "
                      f"{blend} {z}{fog}{at} cull={rs.get('CULLMODE', '?').replace('D3DCULL_', '')} "
                      f"cw={rs.get('COLORWRITEENABLE', '15')} {ts} {extra}")
                for p in pending:
                    print(p[:400])
                pending = []


if __name__ == '__main__':
    main()
