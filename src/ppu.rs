/// GBA PPU: 240x160, scanline renderer. Supports text backgrounds (modes
/// 0-1), affine backgrounds (modes 1-2), bitmap modes 3-5, and sprites
/// (regular + affine). Blending and window effects are not yet implemented.
pub const WIDTH: usize = 240;
pub const HEIGHT: usize = 160;

fn rgb555(c: u16) -> u32 {
    let r = (c & 0x1F) as u32;
    let g = (c >> 5 & 0x1F) as u32;
    let b = (c >> 10 & 0x1F) as u32;
    (r << 19 | r >> 2 << 16) | (g << 11 | g >> 2 << 8) | (b << 3 | b >> 2)
}

pub struct Ppu {
    pub framebuffer: [u32; WIDTH * HEIGHT],
}

impl Ppu {
    pub fn new() -> Self {
        Self { framebuffer: [0; WIDTH * HEIGHT] }
    }

    fn pal16(palette: &[u8], bank: u32, idx: u32, obj: bool) -> u16 {
        let base = if obj { 0x200 } else { 0 };
        let off = base + bank as usize * 32 + idx as usize * 2;
        u16::from_le_bytes([palette[off], palette[off + 1]])
    }

    fn pal256(palette: &[u8], idx: u32, obj: bool) -> u16 {
        let base = if obj { 0x200 } else { 0 };
        u16::from_le_bytes([palette[base + idx as usize * 2], palette[base + idx as usize * 2 + 1]])
    }

    /// Render one scanline from the current register/VRAM state.
    /// `io` is the raw I/O register block; `bg_ref` the per-frame affine
    /// reference point accumulators (updated by the caller each line).
    pub fn render_scanline(
        &mut self,
        y: u32,
        io: &[u8],
        palette: &[u8],
        vram: &[u8],
        oam: &[u8],
        bg_ref: &[(i32, i32); 2],
    ) {
        let r16 = |off: usize| u16::from_le_bytes([io[off], io[off + 1]]);
        let dispcnt = r16(0);
        let mode = dispcnt & 7;
        let backdrop = rgb555(u16::from_le_bytes([palette[0], palette[1]]));
        let row = &mut self.framebuffer[y as usize * WIDTH..(y as usize + 1) * WIDTH];

        if dispcnt & 0x80 != 0 {
            row.fill(rgb555(0x7FFF)); // forced blank: white
            return;
        }
        row.fill(backdrop);

        // Sprite line buffer: (color, priority) per pixel, drawn first into
        // a side buffer honoring OAM order.
        let mut obj_line = [(0u32, 255u8, false); WIDTH]; // color, prio, opaque
        if dispcnt & 0x1000 != 0 {
            Self::render_sprites(y, dispcnt, palette, vram, oam, &mut obj_line);
        }

        // Compose: priorities 3 (back) to 0 (front); BGs 3..0 then sprites.
        for prio in (0..4u16).rev() {
            for bg in (0..4u32).rev() {
                if dispcnt & (1 << (8 + bg)) == 0 {
                    continue;
                }
                let bgcnt = r16(0x8 + bg as usize * 2);
                if bgcnt & 3 != prio {
                    continue;
                }
                match (mode, bg) {
                    (0, _) | (1, 0) | (1, 1) => {
                        Self::draw_text_bg(y, bg, io, palette, vram, row)
                    }
                    (1, 2) | (2, 2) | (2, 3) => {
                        Self::draw_affine_bg(y, bg, io, palette, vram, row, bg_ref)
                    }
                    (3, 2) => {
                        for x in 0..WIDTH {
                            let off = (y as usize * WIDTH + x) * 2;
                            row[x] = rgb555(u16::from_le_bytes([vram[off], vram[off + 1]]));
                        }
                    }
                    (4, 2) => {
                        let base = if dispcnt & 0x10 != 0 { 0xA000 } else { 0 };
                        for x in 0..WIDTH {
                            let pi = vram[base + y as usize * WIDTH + x] as u32;
                            if pi != 0 {
                                row[x] = rgb555(Self::pal256(palette, pi, false));
                            }
                        }
                    }
                    (5, 2) => {
                        if (y as usize) < 128 {
                            let base = if dispcnt & 0x10 != 0 { 0xA000 } else { 0 };
                            for x in 0..160.min(WIDTH) {
                                let off = base + (y as usize * 160 + x) * 2;
                                row[x] = rgb555(u16::from_le_bytes([vram[off], vram[off + 1]]));
                            }
                        }
                    }
                    _ => {}
                }
            }
            for x in 0..WIDTH {
                if obj_line[x].2 && obj_line[x].1 as u16 <= prio {
                    if obj_line[x].1 as u16 == prio {
                        row[x] = obj_line[x].0;
                    }
                }
            }
        }
    }

    fn draw_text_bg(
        y: u32,
        bg: u32,
        io: &[u8],
        palette: &[u8],
        vram: &[u8],
        row: &mut [u32],
    ) {
        let r16 = |off: usize| u16::from_le_bytes([io[off], io[off + 1]]) as u32;
        let bgcnt = r16(0x8 + bg as usize * 2);
        let char_base = (bgcnt >> 2 & 3) as usize * 0x4000;
        let screen_base = (bgcnt >> 8 & 0x1F) as usize * 0x800;
        let eight_bpp = bgcnt & 0x80 != 0;
        let size = bgcnt >> 14 & 3;
        let hofs = r16(0x10 + bg as usize * 4) & 0x1FF;
        let vofs = r16(0x12 + bg as usize * 4) & 0x1FF;
        let (w_tiles, h_tiles) = match size {
            0 => (32, 32),
            1 => (64, 32),
            2 => (32, 64),
            _ => (64, 64),
        };
        let py = (y + vofs) % (h_tiles * 8);
        for x in 0..WIDTH as u32 {
            let px = (x + hofs) % (w_tiles * 8);
            // Screen blocks are 32x32 tiles; larger maps chain blocks.
            let sbb = match size {
                0 => 0,
                1 => px / 256,
                2 => py / 256,
                _ => (px / 256) + (py / 256) * 2,
            } as usize;
            let tx = (px / 8) % 32;
            let ty = (py / 8) % 32;
            let entry_off = screen_base + sbb * 0x800 + (ty * 32 + tx) as usize * 2;
            let entry = u16::from_le_bytes([vram[entry_off], vram[entry_off + 1]]) as u32;
            let tile = entry & 0x3FF;
            let mut fx = px % 8;
            let mut fy = py % 8;
            if entry & 0x400 != 0 {
                fx = 7 - fx;
            }
            if entry & 0x800 != 0 {
                fy = 7 - fy;
            }
            let color = if eight_bpp {
                let off = char_base + tile as usize * 64 + (fy * 8 + fx) as usize;
                if off >= 0x10000 { 0 } else { vram[off] as u32 }
            } else {
                let off = char_base + tile as usize * 32 + (fy * 4 + fx / 2) as usize;
                if off >= 0x10000 {
                    0
                } else {
                    let b = vram[off] as u32;
                    if fx & 1 == 0 { b & 0xF } else { b >> 4 }
                }
            };
            if color != 0 {
                let c = if eight_bpp {
                    Self::pal256(palette, color, false)
                } else {
                    Self::pal16(palette, entry >> 12, color, false)
                };
                row[x as usize] = rgb555(c);
            }
        }
    }

    fn draw_affine_bg(
        _y: u32,
        bg: u32,
        io: &[u8],
        palette: &[u8],
        vram: &[u8],
        row: &mut [u32],
        bg_ref: &[(i32, i32); 2],
    ) {
        let r16 = |off: usize| u16::from_le_bytes([io[off], io[off + 1]]) as u32;
        let bgcnt = r16(0x8 + bg as usize * 2);
        let char_base = (bgcnt >> 2 & 3) as usize * 0x4000;
        let screen_base = (bgcnt >> 8 & 0x1F) as usize * 0x800;
        let wrap = bgcnt & 0x2000 != 0;
        let size_tiles: u32 = 16 << (bgcnt >> 14 & 3); // 128..1024 px / 8
        let size_px = (size_tiles * 8) as i32;
        let base = 0x20 + (bg as usize - 2) * 0x10;
        let pa = r16(base) as u16 as i16 as i32;
        let pc = r16(base + 4) as u16 as i16 as i32;
        let (mut rx, mut ry) = bg_ref[bg as usize - 2];
        for x in 0..WIDTH {
            let sx = rx >> 8;
            let sy = ry >> 8;
            rx += pa;
            ry += pc;
            let (sx, sy) = if wrap {
                (sx.rem_euclid(size_px), sy.rem_euclid(size_px))
            } else {
                if sx < 0 || sy < 0 || sx >= size_px || sy >= size_px {
                    continue;
                }
                (sx, sy)
            };
            let tile =
                vram[screen_base + (sy as u32 / 8 * size_tiles + sx as u32 / 8) as usize] as usize;
            let off = char_base + tile * 64 + (sy as usize % 8) * 8 + sx as usize % 8;
            if off < 0x10000 {
                let ci = vram[off] as u32;
                if ci != 0 {
                    row[x] = rgb555(Self::pal256(palette, ci, false));
                }
            }
        }
    }

    fn render_sprites(
        y: u32,
        dispcnt: u16,
        palette: &[u8],
        vram: &[u8],
        oam: &[u8],
        out: &mut [(u32, u8, bool); WIDTH],
    ) {
        let one_dim = dispcnt & 0x40 != 0;
        let bitmap_mode = dispcnt & 7 >= 3;
        for i in (0..128).rev() {
            let a0 = u16::from_le_bytes([oam[i * 8], oam[i * 8 + 1]]) as u32;
            let a1 = u16::from_le_bytes([oam[i * 8 + 2], oam[i * 8 + 3]]) as u32;
            let a2 = u16::from_le_bytes([oam[i * 8 + 4], oam[i * 8 + 5]]) as u32;
            let affine = a0 & 0x100 != 0;
            if !affine && a0 & 0x200 != 0 {
                continue; // disabled
            }
            let shape = a0 >> 14 & 3;
            let size = a1 >> 14 & 3;
            let (w, h): (u32, u32) = match (shape, size) {
                (0, s) => (8 << s, 8 << s),
                (1, 0) => (16, 8),
                (1, 1) => (32, 8),
                (1, 2) => (32, 16),
                (1, 3) => (64, 32),
                (2, 0) => (8, 16),
                (2, 1) => (8, 32),
                (2, 2) => (16, 32),
                _ => (32, 64),
            };
            let double = affine && a0 & 0x200 != 0;
            let (bw, bh) = if double { (w * 2, h * 2) } else { (w, h) };
            let sy = a0 & 0xFF;
            let sx = a1 & 0x1FF;
            let oy = if sy + bh > 256 { sy as i32 - 256 } else { sy as i32 };
            let ox = if sx + bw > 512 { sx as i32 - 512 } else { sx as i32 };
            let line = y as i32 - oy;
            if line < 0 || line >= bh as i32 {
                continue;
            }
            let eight_bpp = a0 & 0x2000 != 0;
            let tile = a2 & 0x3FF;
            let prio = (a2 >> 10 & 3) as u8;
            let pal_bank = a2 >> 12;
            // In bitmap modes tiles 0-511 overlap the framebuffer; skip them.
            if bitmap_mode && tile < 512 {
                continue;
            }
            let row_tiles = if one_dim { w / 8 * if eight_bpp { 2 } else { 1 } } else { 32 };

            let sample = |tx: u32, ty: u32| -> u32 {
                // Tile data for sprites lives at 0x10000; entries are 32
                // bytes (4bpp). 8bpp uses even tile numbers.
                let t = tile + (ty / 8) * row_tiles + (tx / 8) * if eight_bpp { 2 } else { 1 };
                let base = 0x10000 + (t as usize & 0x3FF) * 32;
                if eight_bpp {
                    let off = base + (ty as usize % 8) * 8 + tx as usize % 8;
                    if off >= 0x18000 { 0 } else { vram[off] as u32 }
                } else {
                    let off = base + (ty as usize % 8) * 4 + (tx as usize % 8) / 2;
                    if off >= 0x18000 {
                        0
                    } else {
                        let b = vram[off] as u32;
                        if tx & 1 == 0 { b & 0xF } else { b >> 4 }
                    }
                }
            };

            if affine {
                let idx = (a1 >> 9 & 0x1F) as usize;
                let p = |n: usize| {
                    i16::from_le_bytes([oam[idx * 32 + 6 + n * 8], oam[idx * 32 + 7 + n * 8]])
                        as i32
                };
                let (pa, pb, pc, pd) = (p(0), p(1), p(2), p(3));
                let cx = bw as i32 / 2;
                let cy = bh as i32 / 2;
                for dx in 0..bw as i32 {
                    let x = ox + dx;
                    if !(0..WIDTH as i32).contains(&x) {
                        continue;
                    }
                    let tx = (pa * (dx - cx) + pb * (line - cy) >> 8) + w as i32 / 2;
                    let ty = (pc * (dx - cx) + pd * (line - cy) >> 8) + h as i32 / 2;
                    if tx < 0 || ty < 0 || tx >= w as i32 || ty >= h as i32 {
                        continue;
                    }
                    let ci = sample(tx as u32, ty as u32);
                    if ci != 0 {
                        let c = if eight_bpp {
                            Self::pal256(palette, ci, true)
                        } else {
                            Self::pal16(palette, pal_bank, ci, true)
                        };
                        out[x as usize] = (rgb555(c), prio, true);
                    }
                }
            } else {
                let hflip = a1 & 0x1000 != 0;
                let vflip = a1 & 0x2000 != 0;
                let ty = if vflip { h - 1 - line as u32 } else { line as u32 };
                for dx in 0..w {
                    let x = ox + dx as i32;
                    if !(0..WIDTH as i32).contains(&x) {
                        continue;
                    }
                    let tx = if hflip { w - 1 - dx } else { dx };
                    let ci = sample(tx, ty);
                    if ci != 0 {
                        let c = if eight_bpp {
                            Self::pal256(palette, ci, true)
                        } else {
                            Self::pal16(palette, pal_bank, ci, true)
                        };
                        out[x as usize] = (rgb555(c), prio, true);
                    }
                }
            }
        }
    }
}
