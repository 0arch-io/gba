use crate::ppu::Ppu;

/// Scanline timing in CPU cycles (16.78 MHz): 1232 per line, 228 lines.
const CYCLES_PER_LINE: u64 = 1232;
const VISIBLE_LINES: u32 = 160;
const TOTAL_LINES: u32 = 228;
pub const CYCLES_PER_FRAME: u64 = CYCLES_PER_LINE * TOTAL_LINES as u64;

/// IRQ bits in IE/IF.
const IRQ_VBLANK: u16 = 1 << 0;
const IRQ_HBLANK: u16 = 1 << 1;
const IRQ_VCOUNT: u16 = 1 << 2;
const IRQ_TIMER0: u16 = 1 << 3;
const IRQ_DMA0: u16 = 1 << 8;

pub struct Bus {
    pub bios: Vec<u8>,
    pub ewram: Vec<u8>,
    pub iwram: Vec<u8>,
    pub palette: [u8; 0x400],
    pub vram: Vec<u8>,
    pub oam: [u8; 0x400],
    pub io: [u8; 0x800],
    pub rom: Vec<u8>,
    pub ppu: Ppu,

    // Interrupts
    pub ime: bool,
    pub ie: u16,
    pub if_: u16,

    // Display timing
    cycles: u64,
    line_cycles: u64,
    pub vcount: u32,
    in_hblank: bool,
    pub frame_ready: bool,
    bg_ref: [(i32, i32); 2], // internal affine reference accumulators

    // Timers: (counter, fractional prescaler accumulator)
    timer_counter: [u32; 4],
    timer_frac: [u64; 4],

    // Keypad, set by the frontend: active-low bit per key.
    pub keyinput: u16,

    // 128KB flash (two 64KB banks) with the standard command protocol.
    pub flash: Vec<u8>,
    flash_bank: usize,
    flash_state: u8, // 0 idle, 1 got AA, 2 got 55
    flash_id_mode: bool,
    flash_erase_mode: bool,
    flash_write_byte: bool,
    flash_bank_select: bool,
    pub flash_dirty: bool,

    // DirectSound: two FIFO channels fed by timer-paced DMA1/DMA2.
    fifo_a: std::collections::VecDeque<i8>,
    fifo_b: std::collections::VecDeque<i8>,
    sample_a: i8,
    sample_b: i8,
    sample_timer: u64,
    lp_l: f32,
    lp_r: f32,
    /// Stereo interleaved f32 samples for the frontend.
    pub audio: Vec<f32>,
}

impl Bus {
    pub fn new(rom: Vec<u8>) -> Self {
        let mut bios = vec![0u8; 0x4000];
        // Minimal HLE BIOS IRQ dispatcher at the hardware IRQ vector (0x18):
        // saves registers, calls the user handler pointed to by [0x03007FFC]
        // (via its 0x03FFFFFC mirror), restores, returns.
        let irq_stub: [u32; 6] = [
            0xE92D500F, // stmfd sp!, {r0-r3, r12, lr}
            0xE3A00301, // mov r0, #0x04000000
            0xE28FE000, // add lr, pc, #0
            0xE510F004, // ldr pc, [r0, #-4]
            0xE8BD500F, // ldmfd sp!, {r0-r3, r12, lr}
            0xE25EF004, // subs pc, lr, #4
        ];
        for (i, w) in irq_stub.iter().enumerate() {
            bios[0x18 + i * 4..0x18 + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        Self {
            bios,
            ewram: vec![0; 0x40000],
            iwram: vec![0; 0x8000],
            palette: [0; 0x400],
            vram: vec![0; 0x18000],
            oam: [0; 0x400],
            io: [0; 0x800],
            rom,
            ppu: Ppu::new(),
            ime: false,
            ie: 0,
            if_: 0,
            cycles: 0,
            line_cycles: 0,
            vcount: 0,
            in_hblank: false,
            frame_ready: false,
            bg_ref: [(0, 0); 2],
            timer_counter: [0; 4],
            timer_frac: [0; 4],
            keyinput: 0x3FF,
            flash: vec![0xFF; 0x20000],
            flash_bank: 0,
            flash_state: 0,
            flash_id_mode: false,
            flash_erase_mode: false,
            flash_write_byte: false,
            flash_bank_select: false,
            flash_dirty: false,
            fifo_a: std::collections::VecDeque::new(),
            fifo_b: std::collections::VecDeque::new(),
            sample_a: 0,
            sample_b: 0,
            sample_timer: 0,
            lp_l: 0.0,
            lp_r: 0.0,
            audio: Vec::new(),
        }
    }

    fn io16(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.io[off], self.io[off + 1]])
    }

    pub fn request_irq(&mut self, bit: u16) {
        self.if_ |= bit;
    }

    fn reload_affine_refs(&mut self) {
        for bg in 0..2 {
            let base = 0x28 + bg * 0x10;
            let rd = |o: usize| {
                let v = u32::from_le_bytes([
                    self.io[o],
                    self.io[o + 1],
                    self.io[o + 2],
                    self.io[o + 3],
                ]);
                ((v << 4) as i32) >> 4 // sign-extend 28-bit
            };
            self.bg_ref[bg] = (rd(base), rd(base + 4));
        }
    }

    /// Advance display, timers, and scheduled DMA by CPU cycles.
    pub fn tick(&mut self, cycles: u64) {
        self.cycles += cycles;
        self.line_cycles += cycles;
        self.tick_timers(cycles);

        // Resample DirectSound output to 44.1kHz (2^24 Hz CPU clock).
        self.sample_timer += cycles * 44100;
        while self.sample_timer >= 1 << 24 {
            self.sample_timer -= 1 << 24;
            let cnt_h = self.io16(0x82);
            let master_on = self.io[0x84] & 0x80 != 0;
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            if master_on {
                let va = if cnt_h & 0x04 != 0 { 1.0 } else { 0.5 };
                let vb = if cnt_h & 0x08 != 0 { 1.0 } else { 0.5 };
                let a = self.sample_a as f32 / 128.0 * va * 0.35;
                let b = self.sample_b as f32 / 128.0 * vb * 0.35;
                if cnt_h & 0x0200 != 0 { l += a }
                if cnt_h & 0x0100 != 0 { r += a }
                if cnt_h & 0x2000 != 0 { l += b }
                if cnt_h & 0x1000 != 0 { r += b }
            }
            // One-pole low-pass (~7 kHz), approximating the GBA's output filter.
            self.lp_l += 0.5 * (l - self.lp_l);
            self.lp_r += 0.5 * (r - self.lp_r);
            self.audio.push(self.lp_l);
            self.audio.push(self.lp_r);
        }

        // Hblank starts at cycle 960 of a line.
        if !self.in_hblank && self.line_cycles >= 960 {
            self.in_hblank = true;
            if self.vcount < VISIBLE_LINES {
                let bg_ref = self.bg_ref;
                let io = self.io; // arrays are Copy; cheap enough per line
                let palette = self.palette;
                self.ppu.render_scanline(
                    self.vcount,
                    &io,
                    &palette,
                    &self.vram,
                    &self.oam,
                    &bg_ref,
                );
                // Step affine reference points by PB/PD per line.
                for bg in 0..2 {
                    let base = 0x22 + bg * 0x10;
                    let pb = self.io16(base) as i16 as i32;
                    let pd = self.io16(base + 4) as i16 as i32;
                    self.bg_ref[bg].0 += pb;
                    self.bg_ref[bg].1 += pd;
                }
                self.run_dma(2); // hblank-timed DMA
            }
            if self.io[4] & 0x10 != 0 {
                self.request_irq(IRQ_HBLANK);
            }
        }

        while self.line_cycles >= CYCLES_PER_LINE {
            self.line_cycles -= CYCLES_PER_LINE;
            self.in_hblank = false;
            self.vcount += 1;
            if self.vcount == VISIBLE_LINES {
                self.frame_ready = true;
                if self.io[4] & 0x08 != 0 {
                    self.request_irq(IRQ_VBLANK);
                }
                self.run_dma(1); // vblank-timed DMA
            }
            if self.vcount >= TOTAL_LINES {
                self.vcount = 0;
                self.reload_affine_refs();
            }
            let lyc = self.io[5];
            if self.vcount == lyc as u32 && self.io[4] & 0x20 != 0 {
                self.request_irq(IRQ_VCOUNT);
            }
        }
    }

    fn tick_timers(&mut self, cycles: u64) {
        let mut overflowed = [false; 4];
        for t in 0..4 {
            let cnt = self.io16(0x102 + t * 4);
            if cnt & 0x80 == 0 {
                continue;
            }
            let cascade = t > 0 && cnt & 0x04 != 0;
            let increments = if cascade {
                overflowed[t - 1] as u64
            } else {
                let shift = match cnt & 3 {
                    0 => 0,
                    1 => 6,
                    2 => 8,
                    _ => 10,
                };
                self.timer_frac[t] += cycles;
                let inc = self.timer_frac[t] >> shift;
                self.timer_frac[t] &= (1 << shift) - 1;
                inc
            };
            if increments == 0 {
                continue;
            }
            let reload = self.io16(0x100 + t * 4) as u32;
            let mut c = self.timer_counter[t] as u64 + increments;
            while c > 0xFFFF {
                c = reload as u64 + (c - 0x10000);
                overflowed[t] = true;
            }
            self.timer_counter[t] = c as u32;
            if overflowed[t] && cnt & 0x40 != 0 {
                self.request_irq(IRQ_TIMER0 << t);
            }
            if overflowed[t] && t < 2 {
                let cnt_h = self.io16(0x82);
                if (cnt_h >> 10 & 1) as usize == t {
                    self.sample_a = self.fifo_a.pop_front().unwrap_or(self.sample_a);
                    if self.fifo_a.len() <= 16 {
                        self.run_fifo_dma(0x0400_00A0);
                    }
                }
                if (cnt_h >> 14 & 1) as usize == t {
                    self.sample_b = self.fifo_b.pop_front().unwrap_or(self.sample_b);
                    if self.fifo_b.len() <= 16 {
                        self.run_fifo_dma(0x0400_00A4);
                    }
                }
            }
        }
    }

    /// Run all enabled DMA channels whose start timing matches
    /// (0 = immediate, 1 = vblank, 2 = hblank).
    fn run_dma(&mut self, timing: u16) {
        for ch in 0..4usize {
            let base = 0xB0 + ch * 12;
            let cnt = self.io16(base + 10);
            if cnt & 0x8000 == 0 || (cnt >> 12 & 3) != timing {
                continue;
            }
            let src = u32::from_le_bytes([
                self.io[base],
                self.io[base + 1],
                self.io[base + 2],
                self.io[base + 3],
            ]);
            let dst = u32::from_le_bytes([
                self.io[base + 4],
                self.io[base + 5],
                self.io[base + 6],
                self.io[base + 7],
            ]);
            let mut count = self.io16(base + 8) as u32;
            if count == 0 {
                count = if ch == 3 { 0x10000 } else { 0x4000 };
            }
            let word = cnt & 0x0400 != 0;
            let unit = if word { 4u32 } else { 2 };
            let dst_ctl = cnt >> 5 & 3;
            let src_ctl = cnt >> 7 & 3;
            let mut s = src & !(unit - 1);
            let mut d = dst & !(unit - 1);
            for _ in 0..count {
                if word {
                    let v = self.read32(s);
                    self.write32(d, v);
                } else {
                    let v = self.read16(s);
                    self.write16(d, v);
                }
                s = match src_ctl {
                    0 => s.wrapping_add(unit),
                    1 => s.wrapping_sub(unit),
                    _ => s,
                };
                d = match dst_ctl {
                    0 | 3 => d.wrapping_add(unit),
                    1 => d.wrapping_sub(unit),
                    _ => d,
                };
            }
            // Write back incremented source; dst "increment+reload" keeps
            // the register value.
            self.io[base..base + 4].copy_from_slice(&s.to_le_bytes());
            if dst_ctl != 3 {
                self.io[base + 4..base + 8].copy_from_slice(&d.to_le_bytes());
            }
            if cnt & 0x4000 != 0 {
                self.request_irq(IRQ_DMA0 << ch);
            }
            if cnt & 0x0200 == 0 || timing == 0 {
                let ncnt = cnt & !0x8000;
                self.io[base + 10..base + 12].copy_from_slice(&ncnt.to_le_bytes());
            }
        }
    }

    /// DMA1/DMA2 in "special" timing refill a sound FIFO: 4 words, fixed dst.
    fn run_fifo_dma(&mut self, fifo_addr: u32) {
        for ch in 1..=2usize {
            let base = 0xB0 + ch * 12;
            let cnt = self.io16(base + 10);
            if cnt & 0x8000 == 0 || cnt >> 12 & 3 != 3 {
                continue;
            }
            let dst = u32::from_le_bytes([
                self.io[base + 4],
                self.io[base + 5],
                self.io[base + 6],
                self.io[base + 7],
            ]);
            if dst != fifo_addr {
                continue;
            }
            let mut src = u32::from_le_bytes([
                self.io[base],
                self.io[base + 1],
                self.io[base + 2],
                self.io[base + 3],
            ]) & !3;
            for _ in 0..4 {
                let v = self.read32(src);
                for b in v.to_le_bytes() {
                    let f = if fifo_addr == 0x0400_00A0 { &mut self.fifo_a } else { &mut self.fifo_b };
                    if f.len() < 32 {
                        f.push_back(b as i8);
                    }
                }
                src += 4;
            }
            self.io[base..base + 4].copy_from_slice(&src.to_le_bytes());
            if cnt & 0x4000 != 0 {
                self.request_irq(IRQ_DMA0 << ch);
            }
        }
    }

    fn vram_idx(addr: u32) -> usize {
        let a = (addr & 0x1FFFF) as usize;
        if a >= 0x18000 { a - 0x8000 } else { a }
    }

    fn flash_read(&self, addr: u32) -> u8 {
        let a = addr as usize & 0xFFFF;
        if self.flash_id_mode {
            // Sanyo 128K: manufacturer 0x62, device 0x13.
            return if a == 0 {
                0x62
            } else if a == 1 {
                0x13
            } else {
                0xFF
            };
        }
        self.flash[self.flash_bank * 0x10000 + a]
    }

    fn flash_write(&mut self, addr: u32, val: u8) {
        let a = addr as usize & 0xFFFF;
        if self.flash_write_byte {
            self.flash[self.flash_bank * 0x10000 + a] &= val; // program clears bits
            self.flash_write_byte = false;
            self.flash_dirty = true;
            return;
        }
        if self.flash_bank_select && a == 0 {
            self.flash_bank = (val & 1) as usize;
            self.flash_bank_select = false;
            return;
        }
        match (self.flash_state, a, val) {
            (0, 0x5555, 0xAA) => self.flash_state = 1,
            (1, 0x2AAA, 0x55) => self.flash_state = 2,
            (2, 0x5555, cmd) => {
                self.flash_state = 0;
                match cmd {
                    0x90 => self.flash_id_mode = true,
                    0xF0 => self.flash_id_mode = false,
                    0x80 => self.flash_erase_mode = true,
                    0x10 if self.flash_erase_mode => {
                        self.flash.fill(0xFF);
                        self.flash_erase_mode = false;
                        self.flash_dirty = true;
                    }
                    0xA0 => self.flash_write_byte = true,
                    0xB0 => self.flash_bank_select = true,
                    _ => {}
                }
            }
            (2, _, 0x30) if self.flash_erase_mode => {
                // 4KB sector erase
                let start = self.flash_bank * 0x10000 + (a & 0xF000);
                self.flash[start..start + 0x1000].fill(0xFF);
                self.flash_erase_mode = false;
                self.flash_state = 0;
                self.flash_dirty = true;
            }
            _ => self.flash_state = 0,
        }
    }

    pub fn read8(&self, addr: u32) -> u8 {
        match addr >> 24 {
            0x00 => *self.bios.get(addr as usize & 0x3FFF).unwrap_or(&0),
            0x02 => self.ewram[addr as usize & 0x3FFFF],
            0x03 => self.iwram[addr as usize & 0x7FFF],
            0x04 => self.io_read(addr & 0xFFFFFF),
            0x05 => self.palette[addr as usize & 0x3FF],
            0x06 => self.vram[Self::vram_idx(addr)],
            0x07 => self.oam[addr as usize & 0x3FF],
            0x08..=0x0D => {
                let idx = (addr & 0x01FF_FFFF) as usize;
                *self.rom.get(idx).unwrap_or(&0xFF)
            }
            0x0E | 0x0F => self.flash_read(addr),
            _ => 0,
        }
    }

    fn raw8(&mut self, addr: u32, val: u8) {
        match addr >> 24 {
            0x02 => self.ewram[addr as usize & 0x3FFFF] = val,
            0x03 => self.iwram[addr as usize & 0x7FFF] = val,
            0x04 => self.io_write(addr & 0xFFFFFF, val),
            0x05 => self.palette[addr as usize & 0x3FF] = val,
            0x06 => self.vram[Self::vram_idx(addr)] = val,
            0x07 => self.oam[addr as usize & 0x3FF] = val,
            0x0E | 0x0F => self.flash_write(addr, val),
            _ => {}
        }
    }

    /// Byte write without the video-memory byte-store quirks; used by HLE
    /// BIOS decompression, which legitimately writes bytes into VRAM.
    pub fn write8_lenient(&mut self, addr: u32, val: u8) {
        self.raw8(addr, val);
    }

    pub fn write8(&mut self, addr: u32, val: u8) {
        match addr >> 24 {
            // Byte stores to 16-bit video memory: palette duplicates, VRAM
            // duplicates in the BG region / ignores in OBJ, OAM ignores.
            0x05 => {
                let a = addr as usize & 0x3FE;
                self.palette[a] = val;
                self.palette[a + 1] = val;
            }
            0x06 => {
                let idx = Self::vram_idx(addr & !1);
                let bitmap = self.io[0] & 7 >= 3;
                let obj_start = if bitmap { 0x14000 } else { 0x10000 };
                if idx < obj_start {
                    self.vram[idx] = val;
                    self.vram[idx + 1] = val;
                }
            }
            0x07 => {}
            _ => self.raw8(addr, val),
        }
    }

    pub fn read16(&self, addr: u32) -> u16 {
        let a = addr & !1;
        u16::from_le_bytes([self.read8(a), self.read8(a + 1)])
    }

    pub fn read32(&self, addr: u32) -> u32 {
        let a = addr & !3;
        u32::from_le_bytes([
            self.read8(a),
            self.read8(a + 1),
            self.read8(a + 2),
            self.read8(a + 3),
        ])
    }

    pub fn write16(&mut self, addr: u32, val: u16) {
        let a = addr & !1;
        let [b0, b1] = val.to_le_bytes();
        self.raw8(a, b0);
        self.raw8(a + 1, b1);
    }

    pub fn write32(&mut self, addr: u32, val: u32) {
        let a = addr & !3;
        for (i, b) in val.to_le_bytes().iter().enumerate() {
            self.raw8(a + i as u32, *b);
        }
    }

    fn io_read(&self, off: u32) -> u8 {
        match off {
            0x004 => {
                let vb = (self.vcount >= VISIBLE_LINES && self.vcount != 227) as u8;
                let hb = self.in_hblank as u8;
                let vc = (self.vcount == self.io[5] as u32) as u8;
                self.io[4] & 0xF8 | vb | hb << 1 | vc << 2
            }
            0x006 => self.vcount as u8,
            0x007 => 0,
            0x100 | 0x104 | 0x108 | 0x10C => self.timer_counter[(off as usize - 0x100) / 4] as u8,
            0x101 | 0x105 | 0x109 | 0x10D => {
                (self.timer_counter[(off as usize - 0x101) / 4] >> 8) as u8
            }
            0x130 => self.keyinput as u8,
            0x131 => (self.keyinput >> 8) as u8 & 3,
            0x200 => self.ie as u8,
            0x201 => (self.ie >> 8) as u8,
            0x202 => self.if_ as u8,
            0x203 => (self.if_ >> 8) as u8,
            0x208 => self.ime as u8,
            0x209 => 0,
            _ => *self.io.get(off as usize).unwrap_or(&0),
        }
    }

    fn io_write(&mut self, off: u32, val: u8) {
        match off {
            0x006 | 0x007 => {} // VCOUNT read-only
            0x102 | 0x106 | 0x10A | 0x10E => {
                let t = (off as usize - 0x102) / 4;
                let was = self.io[off as usize] & 0x80;
                self.io[off as usize] = val;
                if was == 0 && val & 0x80 != 0 {
                    self.timer_counter[t] = self.io16(0x100 + t * 4) as u32;
                    self.timer_frac[t] = 0;
                }
            }
            0x0A0..=0x0A3 => {
                if self.fifo_a.len() < 32 {
                    self.fifo_a.push_back(val as i8);
                }
            }
            0x0A4..=0x0A7 => {
                if self.fifo_b.len() < 32 {
                    self.fifo_b.push_back(val as i8);
                }
            }
            0x083 => {
                self.io[0x83] = val;
                // FIFO reset bits (11/15 of SOUNDCNT_H → bits 3/7 of high byte)
                if val & 0x08 != 0 {
                    self.fifo_a.clear();
                }
                if val & 0x80 != 0 {
                    self.fifo_b.clear();
                }
            }
            0x202 => self.if_ &= !(val as u16), // write-1-to-clear
            0x203 => self.if_ &= !((val as u16) << 8),
            0x200 => self.ie = (self.ie & 0xFF00) | val as u16,
            0x201 => self.ie = (self.ie & 0x00FF) | ((val as u16 & 0x3F) << 8),
            0x208 => self.ime = val & 1 != 0,
            _ => {
                if let Some(b) = self.io.get_mut(off as usize) {
                    *b = val;
                }
                // Affine reference writes reload the internal accumulators.
                if (0x28..0x30).contains(&off) || (0x38..0x40).contains(&off) {
                    self.reload_affine_refs();
                }
                // DMA enable with immediate timing fires right away.
                if let 0xBB | 0xC7 | 0xD3 | 0xDF = off {
                    if val & 0x80 != 0 {
                        self.run_dma(0);
                    }
                }
            }
        }
    }
}
