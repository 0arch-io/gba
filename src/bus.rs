use crate::ppu::Ppu;
use crate::psg::Psg;

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

/// Which battery-backed save chip the cartridge carries. Detected from the
/// ASCII marker the SDK leaves in the ROM image (see `detect_save_type`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, bincode::Encode, bincode::Decode)]
pub enum SaveType {
    /// 32KB static RAM at 0x0E000000, plain 8-bit reads and writes.
    Sram,
    /// 64KB flash, one bank, Panasonic MN63F805MNP (0x32 / 0x1B).
    Flash64,
    /// 128KB flash, two 64KB banks, Sanyo LE26FV10N1TS (0x62 / 0x13).
    Flash128,
    /// 512 byte serial EEPROM, 6-bit block address.
    Eeprom512,
    /// 8KB serial EEPROM, 14-bit block address.
    Eeprom8k,
    /// EEPROM of not-yet-known size; resolved by the first DMA3 transfer.
    EepromUnknown,
}

impl SaveType {
    /// Size in bytes of the save medium (and therefore of the .sav file).
    pub fn size(self) -> usize {
        match self {
            SaveType::Sram => 0x8000,
            SaveType::Flash64 => 0x10000,
            SaveType::Flash128 => 0x20000,
            SaveType::Eeprom512 => 512,
            // Provisional until the transfer width settles it; nothing can be
            // written before that happens, so the .sav is never this size.
            SaveType::Eeprom8k | SaveType::EepromUnknown => 8192,
        }
    }

    pub fn is_eeprom(self) -> bool {
        matches!(
            self,
            SaveType::Eeprom512 | SaveType::Eeprom8k | SaveType::EepromUnknown
        )
    }

    pub fn is_flash(self) -> bool {
        matches!(self, SaveType::Flash64 | SaveType::Flash128)
    }

    /// Byte the medium is erased to (flash erases to 0xFF; SRAM/EEPROM start
    /// blank, but 0xFF is what a fresh cartridge reads back as either way).
    fn fill(self) -> u8 {
        0xFF
    }

    pub fn name(self) -> &'static str {
        match self {
            SaveType::Sram => "SRAM 32KB",
            SaveType::Flash64 => "Flash 64KB",
            SaveType::Flash128 => "Flash 128KB",
            SaveType::Eeprom512 => "EEPROM 512B",
            SaveType::Eeprom8k => "EEPROM 8KB",
            SaveType::EepromUnknown => "EEPROM (size pending)",
        }
    }
}

/// Scan a ROM image for the save-hardware marker string the Nintendo SDK
/// links into every commercial cartridge, e.g. "FLASH1M_V103". The first
/// marker in address order wins; the six strings have no common prefixes so
/// the match is unambiguous.
pub fn detect_save_type(rom: &[u8]) -> SaveType {
    for (i, &c) in rom.iter().enumerate() {
        let at = |pat: &[u8]| rom.len() - i >= pat.len() && &rom[i..i + pat.len()] == pat;
        match c {
            b'E' if at(b"EEPROM_V") => return SaveType::EepromUnknown,
            b'S' if at(b"SRAM_V") || at(b"SRAM_F_V") => return SaveType::Sram,
            b'F' if at(b"FLASH1M_V") => return SaveType::Flash128,
            b'F' if at(b"FLASH512_V") || at(b"FLASH_V") => return SaveType::Flash64,
            _ => {}
        }
    }
    // No marker (homebrew, prototypes, trimmed dumps): assume the largest
    // flash part, which is what this emulator shipped with before detection
    // existed and is harmless for a game that never touches the save region.
    SaveType::Flash128
}

#[derive(bincode::Encode, bincode::Decode)]
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
    pub psg: Psg,

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
    /// Debug: counts writes of 0 to IME (used by boot-divergence tracing).
    pub ime_off_count: u32,
    /// Debug: PC of the currently executing instruction (set by the CPU).
    pub last_pc: u32,
    pub pal_trace: bool,

    // DMA internal address latches. Hardware latches SAD/DAD on enable and
    // never exposes the incremented values back through the registers.
    dma_src: [u32; 4],
    dma_dst: [u32; 4],

    // Battery-backed save. `save` is always exactly `save_type.size()` bytes.
    pub save_type: SaveType,
    pub save: Vec<u8>,
    pub save_dirty: bool,

    // Flash command-protocol state (64KB and 128KB parts share it).
    flash_bank: usize,
    flash_state: u8, // 0 idle, 1 got AA, 2 got 55
    flash_id_mode: bool,
    flash_erase_mode: bool,
    flash_write_byte: bool,
    flash_bank_select: bool,

    // EEPROM bit-serial protocol state.
    ee_addr_bits: u8, // 0 = not yet known, else 6 or 14
    ee_buf: Vec<u8>,  // command bits received so far, one per element
    ee_reading: bool,
    ee_out: u32,        // bits already shifted out of the current read
    ee_read_off: usize, // byte offset of the block being read

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
        // Hardware reset state: the affine parameter registers PA/PD start
        // at 0x0100 (identity); games can and do rely on this.
        let mut io = [0u8; 0x800];
        for base in [0x20usize, 0x26, 0x30, 0x36] {
            io[base] = 0x00;
            io[base + 1] = 0x01;
        }
        let save_type = detect_save_type(&rom);
        Self {
            bios,
            ewram: vec![0; 0x40000],
            iwram: vec![0; 0x8000],
            palette: [0; 0x400],
            vram: vec![0; 0x18000],
            oam: [0; 0x400],
            io,
            rom,
            ppu: Ppu::new(),
            psg: Psg::new(),
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
            ime_off_count: 0,
            last_pc: 0,
            pal_trace: false,
            dma_src: [0; 4],
            dma_dst: [0; 4],
            save_type,
            save: vec![save_type.fill(); save_type.size()],
            save_dirty: false,
            flash_bank: 0,
            flash_state: 0,
            flash_id_mode: false,
            flash_erase_mode: false,
            flash_write_byte: false,
            flash_bank_select: false,
            ee_addr_bits: 0,
            ee_buf: Vec::new(),
            ee_reading: false,
            ee_out: 0,
            ee_read_off: 0,
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

        self.psg.tick(cycles as u32);

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
                if cnt_h & 0x0200 != 0 {
                    l += a
                }
                if cnt_h & 0x0100 != 0 {
                    r += a
                }
                if cnt_h & 0x2000 != 0 {
                    l += b
                }
                if cnt_h & 0x1000 != 0 {
                    r += b
                }
                let (pl, pr) = self.psg.output();
                let pv = match cnt_h & 3 {
                    0 => 0.25,
                    1 => 0.5,
                    _ => 1.0,
                };
                l += pl * pv;
                r += pr * pv;
            }
            // Headroom so DirectSound + PSG at full tilt cannot clip.
            let l = l * 0.7;
            let r = r * 0.7;
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
            let src = self.dma_src[ch];
            // Destination "increment+reload" mode re-latches DAD each trigger.
            let dst = if cnt >> 5 & 3 == 3 {
                u32::from_le_bytes([
                    self.io[base + 4],
                    self.io[base + 5],
                    self.io[base + 6],
                    self.io[base + 7],
                ])
            } else {
                self.dma_dst[ch]
            };
            let mut count = self.io16(base + 8) as u32;
            if count == 0 {
                count = if ch == 3 { 0x10000 } else { 0x4000 };
            }
            // EEPROM is clocked bit-serially by DMA3. The length of the first
            // transfer into the chip is what reveals its address width.
            if ch == 3 && dst >> 24 == 0x0D && self.eeprom_selected(dst) {
                self.eeprom_dma_begin(count);
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
            // Update internal latches only; the visible registers are
            // write-only on hardware and keep their programmed values.
            self.dma_src[ch] = s;
            if dst_ctl != 3 {
                self.dma_dst[ch] = d;
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
            let mut src = self.dma_src[ch] & !3;
            for _ in 0..4 {
                let v = self.read32(src);
                for b in v.to_le_bytes() {
                    let f = if fifo_addr == 0x0400_00A0 {
                        &mut self.fifo_a
                    } else {
                        &mut self.fifo_b
                    };
                    if f.len() < 32 {
                        f.push_back(b as i8);
                    }
                }
                src += 4;
            }
            self.dma_src[ch] = src;
            if cnt & 0x4000 != 0 {
                self.request_irq(IRQ_DMA0 << ch);
            }
        }
    }

    fn vram_idx(addr: u32) -> usize {
        let a = (addr & 0x1FFFF) as usize;
        if a >= 0x18000 { a - 0x8000 } else { a }
    }

    /// Adopt a previously written .sav blob. A blob whose length does not
    /// match the detected medium is taken on a best-effort basis rather than
    /// rejected or, worse, allowed to panic: a Rust panic aborts the host app.
    /// Returns true when the blob matched the medium exactly.
    pub fn load_save(&mut self, data: &[u8]) -> bool {
        // An existing EEPROM save settles the 512B / 8KB question by itself.
        if self.save_type == SaveType::EepromUnknown {
            match data.len() {
                512 => self.set_save_type(SaveType::Eeprom512),
                8192 => self.set_save_type(SaveType::Eeprom8k),
                _ => {}
            }
        }
        let n = data.len().min(self.save.len());
        self.save[..n].copy_from_slice(&data[..n]);
        for b in self.save[n..].iter_mut() {
            *b = 0xFF;
        }
        data.len() == self.save.len()
    }

    fn set_save_type(&mut self, t: SaveType) {
        if self.save_type == t {
            return;
        }
        self.save_type = t;
        self.save.resize(t.size(), t.fill());
        self.ee_addr_bits = match t {
            SaveType::Eeprom512 => 6,
            SaveType::Eeprom8k => 14,
            _ => self.ee_addr_bits,
        };
    }

    // ---- Flash (64KB single bank, or 128KB in two banks) ----

    fn flash_offset(&self, a: usize) -> usize {
        let off = if self.save_type == SaveType::Flash128 {
            self.flash_bank * 0x10000 + a
        } else {
            a & 0xFFFF
        };
        // Never index past the buffer, whatever the game does.
        if off < self.save.len() {
            off
        } else {
            off % self.save.len().max(1)
        }
    }

    fn flash_read(&self, addr: u32) -> u8 {
        let a = addr as usize & 0xFFFF;
        if self.flash_id_mode {
            // Sanyo 128K: 0x62/0x13. Panasonic 64K: 0x32/0x1B.
            let (man, dev) = if self.save_type == SaveType::Flash128 {
                (0x62, 0x13)
            } else {
                (0x32, 0x1B)
            };
            return match a {
                0 => man,
                1 => dev,
                _ => 0xFF,
            };
        }
        self.save.get(self.flash_offset(a)).copied().unwrap_or(0xFF)
    }

    fn flash_write(&mut self, addr: u32, val: u8) {
        let a = addr as usize & 0xFFFF;
        if self.flash_write_byte {
            let off = self.flash_offset(a);
            if let Some(b) = self.save.get_mut(off) {
                *b &= val; // programming can only clear bits
            }
            self.flash_write_byte = false;
            self.save_dirty = true;
            if std::env::var("GBA_SAVELOG").is_ok() {
                eprintln!("flash program {:05X} = {:02X}", off, val);
            }
            return;
        }
        if self.flash_bank_select && a == 0 {
            // Only the 128KB part has a second bank.
            if self.save_type == SaveType::Flash128 {
                self.flash_bank = (val & 1) as usize;
            }
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
                        self.save.fill(0xFF);
                        self.flash_erase_mode = false;
                        self.save_dirty = true;
                    }
                    0xA0 => self.flash_write_byte = true,
                    0xB0 => self.flash_bank_select = true,
                    _ => {}
                }
            }
            (2, _, 0x30) if self.flash_erase_mode => {
                // 4KB sector erase
                let start = self.flash_offset(a & 0xF000);
                let end = (start + 0x1000).min(self.save.len());
                if start < end {
                    self.save[start..end].fill(0xFF);
                }
                self.flash_erase_mode = false;
                self.flash_state = 0;
                self.save_dirty = true;
            }
            _ => self.flash_state = 0,
        }
    }

    // ---- EEPROM (bit-serial, driven by DMA3 through the 0x0D region) ----

    /// True when `addr` decodes to the EEPROM chip rather than a ROM mirror.
    /// Cartridges larger than 16MB only expose EEPROM in the last 256 bytes
    /// of the 0x0D region; smaller ones answer anywhere in it.
    fn eeprom_selected(&self, addr: u32) -> bool {
        self.save_type.is_eeprom()
            && (self.rom.len() <= 0x0100_0000 || addr & 0x00FF_FF00 == 0x00FF_FF00)
    }

    /// A DMA3 transfer of `count` halfwords into the EEPROM is starting.
    ///
    /// Hardware deselects the chip between transfers, so the command shift
    /// register starts empty each time; without that a single malformed
    /// stream would desync the chip forever.
    ///
    /// The transfer length is also what distinguishes the 6-bit part from the
    /// 14-bit one: a read request is 2+n+1 bits and a write is 2+n+64+1, so
    /// 9/73 means 6-bit and 17/81 means 14-bit. (Some SDKs omit the trailing
    /// stop bit, hence 72/80 too.)
    pub fn eeprom_dma_begin(&mut self, count: u32) {
        self.ee_buf.clear();
        if self.ee_addr_bits != 0 {
            return;
        }
        match count {
            9 | 72 | 73 => self.set_save_type(SaveType::Eeprom512),
            17 | 80 | 81 => self.set_save_type(SaveType::Eeprom8k),
            _ => {}
        }
    }

    fn eeprom_block_offset(&self, block: usize) -> usize {
        let blocks = (self.save.len() / 8).max(1);
        (block % blocks) * 8
    }

    fn eeprom_write_bit(&mut self, bit: u8) {
        if self.ee_addr_bits == 0 {
            // Nothing told us the width yet; 6-bit is the safer guess because
            // a 14-bit game always issues a 17- or 81-halfword transfer first.
            self.set_save_type(SaveType::Eeprom512);
        }
        let n = self.ee_addr_bits as usize;
        self.ee_buf.push(bit & 1);
        // Every command starts with a 1; a leading 0 is the stop bit of the
        // previous request, so drop it and stay in sync.
        if self.ee_buf[0] == 0 {
            self.ee_buf.clear();
            return;
        }
        if self.ee_buf.len() < 2 {
            return;
        }
        let bits = |s: &[u8]| s.iter().fold(0usize, |a, &b| a << 1 | b as usize);
        if self.ee_buf[1] == 1 {
            // "11" = read request: address, then the game clocks the data out.
            if self.ee_buf.len() == 2 + n {
                let block = bits(&self.ee_buf[2..]);
                self.ee_read_off = self.eeprom_block_offset(block);
                self.ee_reading = true;
                self.ee_out = 0;
                self.ee_buf.clear();
            }
        } else if self.ee_buf.len() == 2 + n + 64 {
            // "10" = write request: address followed by 64 data bits.
            let block = bits(&self.ee_buf[2..2 + n]);
            let off = self.eeprom_block_offset(block);
            for i in 0..8 {
                let byte = bits(&self.ee_buf[2 + n + i * 8..2 + n + i * 8 + 8]) as u8;
                if let Some(b) = self.save.get_mut(off + i) {
                    *b = byte;
                }
            }
            self.save_dirty = true;
            self.ee_buf.clear();
        } else if self.ee_buf.len() > 2 + n + 64 {
            self.ee_buf.clear(); // desynced; resynchronise on the next command
        }
    }

    fn eeprom_read_bit(&mut self) -> u8 {
        if !self.ee_reading {
            return 1; // idle chip reads back as ready
        }
        let i = self.ee_out;
        self.ee_out += 1;
        if i < 4 {
            return 0; // four dummy bits precede the data
        }
        let b = (i - 4) as usize;
        if b >= 64 {
            self.ee_reading = false;
            return 1;
        }
        if b == 63 {
            self.ee_reading = false;
        }
        let byte = self
            .save
            .get(self.ee_read_off + b / 8)
            .copied()
            .unwrap_or(0xFF);
        byte >> (7 - b % 8) & 1
    }

    /// Byte read. Takes `&mut self` because reading the EEPROM region clocks
    /// the chip's serial output on, which is a side effect.
    pub fn read8(&mut self, addr: u32) -> u8 {
        match addr >> 24 {
            0x00 => *self.bios.get(addr as usize & 0x3FFF).unwrap_or(&0),
            0x02 => self.ewram[addr as usize & 0x3FFFF],
            0x03 => self.iwram[addr as usize & 0x7FFF],
            0x04 => self.io_read(addr & 0xFFFFFF),
            0x05 => self.palette[addr as usize & 0x3FF],
            0x06 => self.vram[Self::vram_idx(addr)],
            0x07 => self.oam[addr as usize & 0x3FF],
            0x0D if self.eeprom_selected(addr) => {
                // One data bit per halfword, in bit 0 of the low byte.
                if addr & 1 == 0 {
                    self.eeprom_read_bit()
                } else {
                    0
                }
            }
            0x08..=0x0D => {
                let idx = (addr & 0x01FF_FFFF) as usize;
                *self.rom.get(idx).unwrap_or(&0xFF)
            }
            0x0E | 0x0F => match self.save_type {
                SaveType::Sram => self
                    .save
                    .get(addr as usize & 0x7FFF)
                    .copied()
                    .unwrap_or(0xFF),
                t if t.is_flash() => self.flash_read(addr),
                _ => 0xFF, // EEPROM carts leave this region unmapped
            },
            _ => 0,
        }
    }

    fn raw8(&mut self, addr: u32, val: u8) {
        if self.pal_trace && (0x02037418..0x02037438).contains(&addr) {
            eprintln!(
                "palbuf write {:08X} = {:02X} from pc={:08X}",
                addr, val, self.last_pc
            );
        }
        match addr >> 24 {
            0x02 => self.ewram[addr as usize & 0x3FFFF] = val,
            0x03 => self.iwram[addr as usize & 0x7FFF] = val,
            0x04 => self.io_write(addr & 0xFFFFFF, val),
            0x05 => self.palette[addr as usize & 0x3FF] = val,
            0x06 => self.vram[Self::vram_idx(addr)] = val,
            0x07 => self.oam[addr as usize & 0x3FF] = val,
            0x0D if self.eeprom_selected(addr) => {
                // One command bit per halfword; ignore the high byte.
                if addr & 1 == 0 {
                    self.eeprom_write_bit(val);
                }
            }
            0x0E | 0x0F => match self.save_type {
                SaveType::Sram => {
                    let i = addr as usize & 0x7FFF;
                    if let Some(b) = self.save.get_mut(i) {
                        if *b != val {
                            *b = val;
                            self.save_dirty = true;
                        }
                    }
                }
                t if t.is_flash() => self.flash_write(addr, val),
                _ => {}
            },
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

    /// True when this address sits on the cartridge's 8-bit SRAM bus, where a
    /// wider access sees one byte replicated instead of consecutive bytes.
    fn sram_bus(&self, addr: u32) -> bool {
        self.save_type == SaveType::Sram && matches!(addr >> 24, 0x0E | 0x0F)
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        if self.sram_bus(addr) {
            let b = self.read8(addr) as u16;
            return b | b << 8;
        }
        let a = addr & !1;
        u16::from_le_bytes([self.read8(a), self.read8(a + 1)])
    }

    pub fn read32(&mut self, addr: u32) -> u32 {
        if self.sram_bus(addr) {
            let b = self.read8(addr) as u32;
            return b * 0x0101_0101;
        }
        let a = addr & !3;
        u32::from_le_bytes([
            self.read8(a),
            self.read8(a + 1),
            self.read8(a + 2),
            self.read8(a + 3),
        ])
    }

    pub fn write16(&mut self, addr: u32, val: u16) {
        if self.sram_bus(addr) {
            // Only the byte lane matching the address reaches the chip.
            self.raw8(addr, (val >> ((addr & 1) * 8)) as u8);
            return;
        }
        let a = addr & !1;
        let [b0, b1] = val.to_le_bytes();
        self.raw8(a, b0);
        self.raw8(a + 1, b1);
    }

    pub fn write32(&mut self, addr: u32, val: u32) {
        if self.sram_bus(addr) {
            self.raw8(addr, (val >> ((addr & 3) * 8)) as u8);
            return;
        }
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
            0x060..=0x081 | 0x090..=0x09F => self.psg.read(off),
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
        if std::env::var("GBA_IOLOG").is_ok()
            && matches!(off, 0x004 | 0x005 | 0x128 | 0x134 | 0x200 | 0x201 | 0x208)
        {
            eprintln!(
                "io write {:03X} = {:02X} (frame ~{})",
                off,
                val,
                self.cycles / 280896
            );
        }
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
            0x060..=0x07F | 0x080 | 0x081 | 0x090..=0x09F => {
                self.psg.write(off, val);
                if let Some(b) = self.io.get_mut(off as usize) {
                    *b = val; // SOUNDCNT_H etc. still visible via io[]
                }
            }
            0x202 => self.if_ &= !(val as u16), // write-1-to-clear
            0x203 => self.if_ &= !((val as u16) << 8),
            0x200 => self.ie = (self.ie & 0xFF00) | val as u16,
            0x201 => self.ie = (self.ie & 0x00FF) | ((val as u16 & 0x3F) << 8),
            0x208 => {
                self.ime = val & 1 != 0;
                if val & 1 == 0 {
                    self.ime_off_count += 1;
                }
            }
            _ => {
                if let Some(b) = self.io.get_mut(off as usize) {
                    *b = val;
                }
                // Affine reference writes reload the internal accumulators.
                if (0x28..0x30).contains(&off) || (0x38..0x40).contains(&off) {
                    self.reload_affine_refs();
                }
                // DMA enable rising edge: latch SAD/DAD like hardware, and
                // fire immediately-timed transfers.
                if let 0xBB | 0xC7 | 0xD3 | 0xDF = off {
                    if val & 0x80 != 0 {
                        let ch = (off as usize - 0xBB) / 12;
                        let base = 0xB0 + ch * 12;
                        self.dma_src[ch] = u32::from_le_bytes([
                            self.io[base],
                            self.io[base + 1],
                            self.io[base + 2],
                            self.io[base + 3],
                        ]);
                        self.dma_dst[ch] = u32::from_le_bytes([
                            self.io[base + 4],
                            self.io[base + 5],
                            self.io[base + 6],
                            self.io[base + 7],
                        ]);
                        self.run_dma(0);
                    }
                }
            }
        }
    }
}
