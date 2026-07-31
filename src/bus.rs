/// GBA memory map (by top byte of the address):
///   00  BIOS (16KB)
///   02  EWRAM (256KB, on-board)
///   03  IWRAM (32KB, on-chip)
///   04  I/O registers
///   05  palette RAM (1KB)
///   06  VRAM (96KB)
///   07  OAM (1KB)
///   08+ cartridge ROM (mirrors), 0E = SRAM/flash
pub struct Bus {
    pub bios: Vec<u8>,
    pub ewram: Vec<u8>,
    pub iwram: Vec<u8>,
    pub palette: [u8; 0x400],
    pub vram: Vec<u8>,
    pub oam: [u8; 0x400],
    pub io: [u8; 0x800],
    pub rom: Vec<u8>,
    pub sram: Vec<u8>,
    /// Crude global clock: incremented once per CPU step by the frontend.
    pub ticks: u64,
}

impl Bus {
    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            bios: vec![0; 0x4000],
            ewram: vec![0; 0x40000],
            iwram: vec![0; 0x8000],
            palette: [0; 0x400],
            vram: vec![0; 0x18000],
            oam: [0; 0x400],
            io: [0; 0x800],
            rom,
            sram: vec![0xFF; 0x10000],
            ticks: 0,
        }
    }

    fn vram_idx(addr: u32) -> usize {
        let a = (addr & 0x1FFFF) as usize;
        if a >= 0x18000 { a - 0x8000 } else { a }
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
            0x0E | 0x0F => self.sram[addr as usize & 0xFFFF],
            _ => 0,
        }
    }

    /// Raw byte write used by 16/32-bit stores (no byte-store quirks).
    fn raw8(&mut self, addr: u32, val: u8) {
        match addr >> 24 {
            0x02 => self.ewram[addr as usize & 0x3FFFF] = val,
            0x03 => self.iwram[addr as usize & 0x7FFF] = val,
            0x04 => self.io_write(addr & 0xFFFFFF, val),
            0x05 => self.palette[addr as usize & 0x3FF] = val,
            0x06 => self.vram[Self::vram_idx(addr)] = val,
            0x07 => self.oam[addr as usize & 0x3FF] = val,
            0x0E | 0x0F => self.sram[addr as usize & 0xFFFF] = val,
            _ => {}
        }
    }

    pub fn write8(&mut self, addr: u32, val: u8) {
        match addr >> 24 {
            0x02 => self.ewram[addr as usize & 0x3FFFF] = val,
            0x03 => self.iwram[addr as usize & 0x7FFF] = val,
            0x04 => self.io_write(addr & 0xFFFFFF, val),
            // Byte stores to 16-bit-only video memory behave specially:
            // palette duplicates the byte into both halves, VRAM does the
            // same in the BG region and ignores it in the OBJ region, and
            // OAM ignores byte stores entirely.
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
            0x0E | 0x0F => self.sram[addr as usize & 0xFFFF] = val,
            _ => {}
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

    /// VCOUNT derived from the global clock: ~308 steps per scanline,
    /// 228 lines per frame (160 visible + 68 vblank).
    fn vcount(&self) -> u8 {
        (self.ticks / 308 % 228) as u8
    }

    fn io_read(&self, off: u32) -> u8 {
        match off {
            0x004 => {
                // DISPSTAT: bit 0 = in vblank, bit 1 = in hblank
                let vb = (self.vcount() >= 160) as u8;
                let hb = ((self.ticks % 308) >= 240) as u8;
                self.io[4] & 0xF8 | vb | hb << 1
            }
            0x006 => self.vcount(),
            0x007 => 0,
            0x130 => 0xFF, // KEYINPUT low: no keys held (active low)
            0x131 => 0x03,
            _ => *self.io.get(off as usize).unwrap_or(&0),
        }
    }

    fn io_write(&mut self, off: u32, val: u8) {
        if let Some(b) = self.io.get_mut(off as usize) {
            *b = val;
        }
    }
}
