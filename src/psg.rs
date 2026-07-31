//! The GBA's legacy PSG channels: the Game Boy sound hardware embedded in
//! the GBA. Two square channels (ch1 with sweep), a wavetable channel with
//! two banks, and LFSR noise. Register layout differs from the GB (packed
//! into 16-bit SOUNDxCNT registers) but the channel behavior is identical.

const DUTY: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 0],
];

#[derive(Default, bincode::Encode, bincode::Decode)]
struct Square {
    enabled: bool,
    dac: bool,
    duty: u8,
    duty_pos: u8,
    freq: u16,
    timer: i32,
    length: u16,
    length_enable: bool,
    volume: u8,
    env_vol: u8,
    env_add: bool,
    env_period: u8,
    env_timer: u8,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_timer: u8,
    sweep_enabled: bool,
    sweep_shadow: u16,
}

impl Square {
    fn trigger(&mut self, has_sweep: bool) {
        self.enabled = self.dac;
        if self.length == 0 {
            self.length = 64;
        }
        self.timer = (2048 - self.freq as i32) * 16; // GBA clock = 4x GB
        self.env_vol = self.volume;
        self.env_timer = self.env_period;
        if has_sweep {
            self.sweep_shadow = self.freq;
            self.sweep_timer = if self.sweep_period == 0 { 8 } else { self.sweep_period };
            self.sweep_enabled = self.sweep_period != 0 || self.sweep_shift != 0;
            if self.sweep_shift != 0 && self.sweep_next() > 2047 {
                self.enabled = false;
            }
        }
    }

    fn sweep_next(&self) -> u32 {
        let d = self.sweep_shadow >> self.sweep_shift;
        if self.sweep_negate {
            (self.sweep_shadow - d) as u32
        } else {
            self.sweep_shadow as u32 + d as u32
        }
    }

    fn clock_sweep(&mut self) {
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
        }
        if self.sweep_timer == 0 {
            self.sweep_timer = if self.sweep_period == 0 { 8 } else { self.sweep_period };
            if self.sweep_enabled && self.sweep_period != 0 {
                let next = self.sweep_next();
                if next > 2047 {
                    self.enabled = false;
                } else if self.sweep_shift != 0 {
                    self.sweep_shadow = next as u16;
                    self.freq = next as u16;
                    if self.sweep_next() > 2047 {
                        self.enabled = false;
                    }
                }
            }
        }
    }

    fn clock_length(&mut self) {
        if self.length_enable && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.enabled = false;
            }
        }
    }

    fn clock_env(&mut self) {
        if self.env_period == 0 {
            return;
        }
        if self.env_timer > 0 {
            self.env_timer -= 1;
        }
        if self.env_timer == 0 {
            self.env_timer = self.env_period;
            if self.env_add && self.env_vol < 15 {
                self.env_vol += 1;
            } else if !self.env_add && self.env_vol > 0 {
                self.env_vol -= 1;
            }
        }
    }

    fn tick(&mut self, cycles: i32) {
        if !self.enabled {
            return;
        }
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += (2048 - self.freq as i32) * 16;
            self.duty_pos = (self.duty_pos + 1) & 7;
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && self.dac {
            DUTY[self.duty as usize][self.duty_pos as usize] * self.env_vol
        } else {
            0
        }
    }
}

#[derive(Default, bincode::Encode, bincode::Decode)]
struct Wave {
    enabled: bool,
    dac: bool,
    two_banks: bool,
    bank: u8,
    freq: u16,
    timer: i32,
    length: u16,
    length_enable: bool,
    volume_code: u8, // 0..3 plus force-75% flag as 4
    pos: u8,
    sample: u8,
}

#[derive(Default, bincode::Encode, bincode::Decode)]
struct Noise {
    enabled: bool,
    dac: bool,
    length: u16,
    length_enable: bool,
    volume: u8,
    env_vol: u8,
    env_add: bool,
    env_period: u8,
    env_timer: u8,
    divisor_code: u8,
    width7: bool,
    shift: u8,
    timer: i32,
    lfsr: u16,
}

impl Noise {
    fn period(&self) -> i32 {
        let d = if self.divisor_code == 0 { 8 } else { self.divisor_code as i32 * 16 };
        (d << self.shift) * 4 // GBA clock = 4x GB
    }

    fn tick(&mut self, cycles: i32) {
        if !self.enabled {
            return;
        }
        self.timer -= cycles;
        let mut steps = 0;
        while self.timer <= 0 && steps < 64 {
            self.timer += self.period();
            steps += 1;
            let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            if self.width7 {
                self.lfsr = (self.lfsr & !(1 << 6)) | (bit << 6);
            }
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && self.dac && self.lfsr & 1 == 0 {
            self.env_vol
        } else {
            0
        }
    }
}

#[derive(bincode::Encode, bincode::Decode)]
pub struct Psg {
    ch1: Square,
    ch2: Square,
    ch3: Wave,
    ch4: Noise,
    wave_ram: [[u8; 16]; 2],
    frame_timer: u32,
    frame_step: u8,
    /// SOUNDCNT_L: PSG panning and master volumes.
    pub cnt_l: u16,
}

impl Psg {
    pub fn new() -> Self {
        Self {
            ch1: Square::default(),
            ch2: Square::default(),
            ch3: Wave::default(),
            ch4: Noise { lfsr: 0x7FFF, ..Default::default() },
            wave_ram: [[0; 16]; 2],
            frame_timer: 0,
            frame_step: 0,
            cnt_l: 0,
        }
    }

    /// Advance by CPU cycles (16.78 MHz).
    pub fn tick(&mut self, cycles: u32) {
        let c = cycles as i32;
        self.ch1.tick(c);
        self.ch2.tick(c);
        self.ch4.tick(c);

        if self.ch3.enabled {
            self.ch3.timer -= c;
            while self.ch3.timer <= 0 {
                self.ch3.timer += (2048 - self.ch3.freq as i32) * 8;
                self.ch3.pos = (self.ch3.pos + 1) & 31;
                if self.ch3.pos == 0 && self.ch3.two_banks {
                    self.ch3.bank ^= 1;
                }
                let byte = self.wave_ram[self.ch3.bank as usize][(self.ch3.pos / 2) as usize];
                self.ch3.sample = if self.ch3.pos & 1 == 0 { byte >> 4 } else { byte & 0x0F };
            }
        }

        // Frame sequencer at 512 Hz: 32768 CPU cycles per step.
        self.frame_timer += cycles;
        while self.frame_timer >= 32768 {
            self.frame_timer -= 32768;
            match self.frame_step {
                0 | 4 => self.clock_lengths(),
                2 | 6 => {
                    self.clock_lengths();
                    self.ch1.clock_sweep();
                }
                7 => {
                    self.ch1.clock_env();
                    self.ch2.clock_env();
                    let n = &mut self.ch4;
                    if n.env_period > 0 {
                        if n.env_timer > 0 {
                            n.env_timer -= 1;
                        }
                        if n.env_timer == 0 {
                            n.env_timer = n.env_period;
                            if n.env_add && n.env_vol < 15 {
                                n.env_vol += 1;
                            } else if !n.env_add && n.env_vol > 0 {
                                n.env_vol -= 1;
                            }
                        }
                    }
                }
                _ => {}
            }
            self.frame_step = (self.frame_step + 1) & 7;
        }
    }

    fn clock_lengths(&mut self) {
        self.ch1.clock_length();
        self.ch2.clock_length();
        if self.ch3.length_enable && self.ch3.length > 0 {
            self.ch3.length -= 1;
            if self.ch3.length == 0 {
                self.ch3.enabled = false;
            }
        }
        if self.ch4.length_enable && self.ch4.length > 0 {
            self.ch4.length -= 1;
            if self.ch4.length == 0 {
                self.ch4.enabled = false;
            }
        }
    }

    /// Mixed stereo PSG output in -1..1, before the SOUNDCNT_H master scale.
    pub fn output(&self) -> (f32, f32) {
        let wave_out = if self.ch3.enabled && self.ch3.dac {
            match self.ch3.volume_code {
                0 => 0,
                1 => self.ch3.sample,
                2 => self.ch3.sample >> 1,
                3 => self.ch3.sample >> 2,
                _ => (self.ch3.sample * 3) >> 2,
            }
        } else {
            0
        };
        let outs = [self.ch1.output(), self.ch2.output(), wave_out, self.ch4.output()];
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for (i, &o) in outs.iter().enumerate() {
            let v = if o == 0 { 0.0 } else { (o as f32 / 15.0 * 2.0 - 1.0) * 0.25 };
            if self.cnt_l & (1 << (12 + i)) != 0 {
                l += v;
            }
            if self.cnt_l & (1 << (8 + i)) != 0 {
                r += v;
            }
        }
        let lv = (self.cnt_l >> 4 & 7) as f32 + 1.0;
        let rv = (self.cnt_l & 7) as f32 + 1.0;
        (l * lv / 8.0, r * rv / 8.0)
    }

    pub fn read(&self, off: u32) -> u8 {
        match off {
            0x80 => self.cnt_l as u8,
            0x81 => (self.cnt_l >> 8) as u8,
            0x90..=0x9F => {
                // CPU accesses the bank NOT currently playing.
                let b = (self.ch3.bank ^ 1) as usize;
                self.wave_ram[b][(off - 0x90) as usize]
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, val: u8) {
        match off {
            // SOUND1CNT_L: sweep
            0x60 => {
                self.ch1.sweep_shift = val & 7;
                self.ch1.sweep_negate = val & 0x08 != 0;
                self.ch1.sweep_period = (val >> 4) & 7;
            }
            // SOUND1CNT_H: duty/length (lo), envelope (hi)
            0x62 => {
                self.ch1.length = 64 - (val & 0x3F) as u16;
                self.ch1.duty = val >> 6;
            }
            0x63 => {
                self.ch1.env_period = val & 7;
                self.ch1.env_add = val & 0x08 != 0;
                self.ch1.volume = val >> 4;
                self.ch1.dac = val & 0xF8 != 0;
                if !self.ch1.dac {
                    self.ch1.enabled = false;
                }
            }
            // SOUND1CNT_X: frequency + trigger
            0x64 => self.ch1.freq = (self.ch1.freq & 0x700) | val as u16,
            0x65 => {
                self.ch1.freq = (self.ch1.freq & 0xFF) | ((val as u16 & 7) << 8);
                self.ch1.length_enable = val & 0x40 != 0;
                if val & 0x80 != 0 {
                    self.ch1.trigger(true);
                }
            }
            // SOUND2CNT_L / SOUND2CNT_H
            0x68 => {
                self.ch2.length = 64 - (val & 0x3F) as u16;
                self.ch2.duty = val >> 6;
            }
            0x69 => {
                self.ch2.env_period = val & 7;
                self.ch2.env_add = val & 0x08 != 0;
                self.ch2.volume = val >> 4;
                self.ch2.dac = val & 0xF8 != 0;
                if !self.ch2.dac {
                    self.ch2.enabled = false;
                }
            }
            0x6C => self.ch2.freq = (self.ch2.freq & 0x700) | val as u16,
            0x6D => {
                self.ch2.freq = (self.ch2.freq & 0xFF) | ((val as u16 & 7) << 8);
                self.ch2.length_enable = val & 0x40 != 0;
                if val & 0x80 != 0 {
                    self.ch2.trigger(false);
                }
            }
            // SOUND3CNT_L: enable + bank mode
            0x70 => {
                self.ch3.two_banks = val & 0x20 != 0;
                self.ch3.bank = (val >> 6) & 1;
                self.ch3.dac = val & 0x80 != 0;
                if !self.ch3.dac {
                    self.ch3.enabled = false;
                }
            }
            // SOUND3CNT_H: length (lo), volume (hi)
            0x72 => self.ch3.length = 256 - val as u16,
            0x73 => {
                self.ch3.volume_code = if val & 0x80 != 0 { 4 } else { (val >> 5) & 3 };
            }
            // SOUND3CNT_X
            0x74 => self.ch3.freq = (self.ch3.freq & 0x700) | val as u16,
            0x75 => {
                self.ch3.freq = (self.ch3.freq & 0xFF) | ((val as u16 & 7) << 8);
                self.ch3.length_enable = val & 0x40 != 0;
                if val & 0x80 != 0 {
                    self.ch3.enabled = self.ch3.dac;
                    if self.ch3.length == 0 {
                        self.ch3.length = 256;
                    }
                    self.ch3.timer = (2048 - self.ch3.freq as i32) * 8;
                    self.ch3.pos = 0;
                }
            }
            // SOUND4CNT_L
            0x78 => self.ch4.length = 64 - (val & 0x3F) as u16,
            0x79 => {
                self.ch4.env_period = val & 7;
                self.ch4.env_add = val & 0x08 != 0;
                self.ch4.volume = val >> 4;
                self.ch4.dac = val & 0xF8 != 0;
                if !self.ch4.dac {
                    self.ch4.enabled = false;
                }
            }
            // SOUND4CNT_H
            0x7C => {
                self.ch4.divisor_code = val & 7;
                self.ch4.width7 = val & 0x08 != 0;
                self.ch4.shift = val >> 4;
            }
            0x7D => {
                self.ch4.length_enable = val & 0x40 != 0;
                if val & 0x80 != 0 {
                    self.ch4.enabled = self.ch4.dac;
                    if self.ch4.length == 0 {
                        self.ch4.length = 64;
                    }
                    self.ch4.timer = self.ch4.period();
                    self.ch4.lfsr = 0x7FFF;
                    self.ch4.env_vol = self.ch4.volume;
                    self.ch4.env_timer = self.ch4.env_period;
                }
            }
            0x80 => self.cnt_l = (self.cnt_l & 0xFF00) | val as u16,
            0x81 => self.cnt_l = (self.cnt_l & 0x00FF) | ((val as u16) << 8),
            0x90..=0x9F => {
                let b = (self.ch3.bank ^ 1) as usize;
                self.wave_ram[b][(off - 0x90) as usize] = val;
            }
            _ => {}
        }
    }
}
