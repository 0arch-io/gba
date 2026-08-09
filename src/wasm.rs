//! Browser frontend bindings.
//!
//! JavaScript owns the timing: it calls `step_frame` once per animation frame,
//! copies out `framebuffer` for the canvas, and drains `take_audio` into a
//! WebAudio queue. Nothing here touches the filesystem, so battery saves live
//! in memory for the lifetime of the page and are handed to the page through
//! `save_data` if it wants to persist them itself.

use crate::bus::{Bus, CYCLES_PER_FRAME};
use crate::cpi;
use crate::cpu::Cpu;
use wasm_bindgen::prelude::*;

/// The GBA screen, in pixels.
const WIDTH: usize = 240;
const HEIGHT: usize = 160;

/// The rate `Bus::tick` resamples DirectSound and the PSG to.
const SAMPLE_RATE: u32 = 44100;

#[wasm_bindgen]
pub struct Emulator {
    cpu: Cpu,
    rgba: Vec<u8>,
    title: String,
}

#[wasm_bindgen]
impl Emulator {
    /// Load a ROM from raw bytes. Returns an error string if the image is too
    /// small to be a cartridge.
    #[wasm_bindgen(constructor)]
    pub fn new(rom: Vec<u8>) -> Result<Emulator, JsValue> {
        if rom.len() < 0xC0 {
            return Err(JsValue::from_str("not a GBA ROM: image is too small"));
        }
        // The cartridge header keeps a 12-byte ASCII game title at 0xA0.
        let title = String::from_utf8_lossy(&rom[0xA0..0xAC])
            .trim_matches(|c: char| c == '\0' || c.is_whitespace())
            .to_string();
        Ok(Emulator {
            cpu: Cpu::new(Bus::new(rom)),
            rgba: vec![0xFF; WIDTH * HEIGHT * 4],
            title,
        })
    }

    /// Cartridge title from the ROM header, for the page to display.
    pub fn title(&self) -> String {
        self.title.clone()
    }

    /// Run until the PPU has finished the next frame. The cycle cap is a
    /// safety net so a wedged ROM cannot hang the browser tab.
    pub fn step_frame(&mut self) {
        let mut cycles = 0u64;
        while !self.cpu.bus.frame_ready && cycles < CYCLES_PER_FRAME * 2 {
            let c = cpi(self.cpu.regs[15]);
            self.cpu.step();
            self.cpu.bus.tick(c);
            cycles += c;
        }
        self.cpu.bus.frame_ready = false;
    }

    /// 240x160 RGBA bytes, ready for `ImageData`.
    pub fn framebuffer(&mut self) -> Vec<u8> {
        for (px, out) in self
            .cpu
            .bus
            .ppu
            .framebuffer
            .iter()
            .zip(self.rgba.chunks_exact_mut(4))
        {
            out[0] = (px >> 16) as u8;
            out[1] = (px >> 8) as u8;
            out[2] = *px as u8;
            out[3] = 0xFF;
        }
        self.rgba.clone()
    }

    /// Keypad state as a bit set of *pressed* buttons, in KEYINPUT bit order:
    /// 0 A, 1 B, 2 Select, 3 Start, 4 Right, 5 Left, 6 Up, 7 Down, 8 R, 9 L.
    /// The hardware register is active low, so it is inverted here.
    pub fn set_keys(&mut self, pressed: u16) {
        self.cpu.bus.keyinput = !pressed & 0x3FF;
    }

    /// Drain queued audio: interleaved stereo f32 at `sample_rate()`.
    pub fn take_audio(&mut self) -> Vec<f32> {
        self.cpu.bus.audio.drain(..).collect()
    }

    /// Throw away queued audio, used when the page is not keeping up.
    pub fn clear_audio(&mut self) {
        self.cpu.bus.audio.clear();
    }

    pub fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    /// Size in bytes of the save medium this cartridge declared.
    pub fn save_size(&self) -> usize {
        self.cpu.bus.save.len()
    }

    /// True when the cartridge save memory changed since the last `save_data`.
    pub fn save_dirty(&self) -> bool {
        self.cpu.bus.save_dirty
    }

    /// Copy out the cartridge save memory and clear the dirty flag, so the page
    /// can stash it in local storage or offer it as a download.
    pub fn save_data(&mut self) -> Vec<u8> {
        self.cpu.bus.save_dirty = false;
        self.cpu.bus.save.clone()
    }

    /// Restore a previously saved battery file. Wrong-sized blobs are taken on
    /// a best-effort basis rather than rejected, matching the native frontend.
    pub fn load_save(&mut self, data: Vec<u8>) {
        self.cpu.bus.load_save(&data);
    }
}
