//! Software BC1/BC2/BC3 (DXT1/3/5) decoding, for GPUs without BC texture support
//! (most Android GPUs) and for exporting textures to PNG.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bc {
    Dxt1,
    Dxt3,
    Dxt5,
}

impl Bc {
    pub fn block_bytes(self) -> usize {
        match self {
            Bc::Dxt1 => 8,
            Bc::Dxt3 | Bc::Dxt5 => 16,
        }
    }

    pub fn level_size(self, w: u32, h: u32) -> usize {
        (w.max(1).div_ceil(4) * h.max(1).div_ceil(4)) as usize * self.block_bytes()
    }
}

fn rgb565(c: u16) -> [u8; 4] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [((r * 527 + 23) >> 6) as u8, ((g * 259 + 33) >> 6) as u8, ((b * 527 + 23) >> 6) as u8, 255]
}

fn color_block(b: &[u8], dxt1: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let p0 = rgb565(c0);
    let p1 = rgb565(c1);
    let mix = |a: u8, b: u8, wa: u32, wb: u32| ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8;
    let mut pal = [p0, p1, [0; 4], [0; 4]];
    if c0 > c1 || !dxt1 {
        for i in 0..3 {
            pal[2][i] = mix(p0[i], p1[i], 2, 1);
            pal[3][i] = mix(p0[i], p1[i], 1, 2);
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for i in 0..3 {
            pal[2][i] = mix(p0[i], p1[i], 1, 1);
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0];
    }
    let idx = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let mut px = [[0u8; 4]; 16];
    for (i, p) in px.iter_mut().enumerate() {
        *p = pal[((idx >> (2 * i)) & 3) as usize];
    }
    px
}

fn alpha_dxt5(b: &[u8]) -> [u8; 16] {
    let a0 = b[0] as u32;
    let a1 = b[1] as u32;
    let mut pal = [0u8; 8];
    pal[0] = a0 as u8;
    pal[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7u32 {
            pal[i as usize + 1] = ((a0 * (7 - i) + a1 * i) / 7) as u8;
        }
    } else {
        for i in 1..5u32 {
            pal[i as usize + 1] = ((a0 * (5 - i) + a1 * i) / 5) as u8;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let bits = b[2..8].iter().rev().fold(0u64, |acc, &x| (acc << 8) | x as u64);
    let mut out = [0u8; 16];
    for (i, a) in out.iter_mut().enumerate() {
        *a = pal[((bits >> (3 * i)) & 7) as usize];
    }
    out
}

/// Decodes one mip level to tightly packed RGBA8.
pub fn decode(format: Bc, data: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    let bs = format.block_bytes();
    let mut out = vec![0u8; w * h * 4];
    for by in 0..bh {
        for bx in 0..bw {
            let o = (by * bw + bx) * bs;
            let Some(blk) = data.get(o..o + bs) else { return out };
            let px = match format {
                Bc::Dxt1 => color_block(blk, true),
                Bc::Dxt3 => {
                    let mut px = color_block(&blk[8..], false);
                    for (i, p) in px.iter_mut().enumerate() {
                        let nib = (blk[i / 2] >> ((i & 1) * 4)) & 0xF;
                        p[3] = nib * 17;
                    }
                    px
                }
                Bc::Dxt5 => {
                    let mut px = color_block(&blk[8..], false);
                    let a = alpha_dxt5(blk);
                    for (p, a) in px.iter_mut().zip(a) {
                        p[3] = a;
                    }
                    px
                }
            };
            for (i, p) in px.iter().enumerate() {
                let x = bx * 4 + (i & 3);
                let y = by * 4 + (i >> 2);
                if x < w && y < h {
                    let d = (y * w + x) * 4;
                    out[d..d + 4].copy_from_slice(p);
                }
            }
        }
    }
    out
}
