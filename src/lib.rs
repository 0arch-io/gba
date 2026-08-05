pub mod bus;
pub mod cpu;
pub mod ppu;
pub mod psg;

#[cfg(test)]
mod save_tests;

use std::ffi::c_void;

/// C FFI for embedding the emulator core (iOS app frontend).
/// All functions take the opaque handle returned by `gba_create`.

#[unsafe(no_mangle)]
pub extern "C" fn gba_create(
    rom: *const u8,
    rom_len: usize,
    sav: *const u8,
    sav_len: usize,
) -> *mut c_void {
    let rom = unsafe { std::slice::from_raw_parts(rom, rom_len) }.to_vec();
    let mut b = bus::Bus::new(rom);
    if !sav.is_null() && sav_len > 0 {
        // A blob of the wrong size for the detected save chip is taken on a
        // best-effort basis; never panic, that would abort the host app.
        b.load_save(unsafe { std::slice::from_raw_parts(sav, sav_len) });
    }
    Box::into_raw(Box::new(cpu::Cpu::new(b))) as *mut c_void
}

#[unsafe(no_mangle)]
pub extern "C" fn gba_destroy(h: *mut c_void) {
    if !h.is_null() {
        drop(unsafe { Box::from_raw(h as *mut cpu::Cpu) });
    }
}

fn cpi(pc: u32) -> u64 {
    match pc >> 24 {
        0x03 => 1,
        0x02 => 3,
        _ => 4,
    }
}

/// Run emulation until the next completed video frame. `keys` is the raw
/// active-low KEYINPUT value; pass 0x3FF for "nothing held".
#[unsafe(no_mangle)]
pub extern "C" fn gba_run_frame(h: *mut c_void, keys: u16) {
    let cpu = unsafe { &mut *(h as *mut cpu::Cpu) };
    cpu.bus.keyinput = keys;
    let mut cycles = 0u64;
    while !cpu.bus.frame_ready && cycles < bus::CYCLES_PER_FRAME * 2 {
        let c = cpi(cpu.regs[15]);
        cpu.step();
        cpu.bus.tick(c);
        cycles += c;
    }
    cpu.bus.frame_ready = false;
}

/// Pointer to the 240x160 ARGB framebuffer (valid until the next run/destroy).
#[unsafe(no_mangle)]
pub extern "C" fn gba_framebuffer(h: *mut c_void) -> *const u32 {
    let cpu = unsafe { &mut *(h as *mut cpu::Cpu) };
    cpu.bus.ppu.framebuffer.as_ptr()
}

/// Drain up to `max` stereo-interleaved f32 samples; returns the count taken.
#[unsafe(no_mangle)]
pub extern "C" fn gba_audio_read(h: *mut c_void, out: *mut f32, max: usize) -> usize {
    let cpu = unsafe { &mut *(h as *mut cpu::Cpu) };
    let n = cpu.bus.audio.len().min(max);
    unsafe { std::ptr::copy_nonoverlapping(cpu.bus.audio.as_ptr(), out, n) };
    cpu.bus.audio.drain(..n);
    n
}

/// True when the cartridge save memory changed since the last
/// `gba_flash_read`, whichever medium the game actually uses.
#[unsafe(no_mangle)]
pub extern "C" fn gba_flash_dirty(h: *mut c_void) -> bool {
    let cpu = unsafe { &mut *(h as *mut cpu::Cpu) };
    cpu.bus.save_dirty
}

/// Copy the cartridge save memory into `out` and clear the dirty flag.
/// Returns the number of bytes written, which is the true size of the
/// detected medium (512B/8KB EEPROM, 32KB SRAM, 64KB/128KB flash).
#[unsafe(no_mangle)]
pub extern "C" fn gba_flash_read(h: *mut c_void, out: *mut u8, max: usize) -> usize {
    let cpu = unsafe { &mut *(h as *mut cpu::Cpu) };
    let n = cpu.bus.save.len().min(max);
    unsafe { std::ptr::copy_nonoverlapping(cpu.bus.save.as_ptr(), out, n) };
    cpu.bus.save_dirty = false;
    n
}

/// Size in bytes of the detected save medium (0 if the handle is null).
#[unsafe(no_mangle)]
pub extern "C" fn gba_save_size(h: *mut c_void) -> usize {
    if h.is_null() {
        return 0;
    }
    let cpu = unsafe { &mut *(h as *mut cpu::Cpu) };
    cpu.bus.save.len()
}

/// Serialize the whole machine. Returns bytes written, or 0 on failure /
/// insufficient buffer. Call with out=null to query the needed size.
#[unsafe(no_mangle)]
pub extern "C" fn gba_state_save(h: *mut c_void, out: *mut u8, max: usize) -> usize {
    let cpu = unsafe { &mut *(h as *mut cpu::Cpu) };
    let Ok(bytes) = bincode::encode_to_vec(&*cpu, bincode::config::standard()) else {
        return 0;
    };
    if out.is_null() {
        return bytes.len();
    }
    if bytes.len() > max {
        return 0;
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    bytes.len()
}

/// Restore a snapshot produced by `gba_state_save`. Returns true on success.
#[unsafe(no_mangle)]
pub extern "C" fn gba_state_load(h: *mut c_void, data: *const u8, len: usize) -> bool {
    let cpu = unsafe { &mut *(h as *mut cpu::Cpu) };
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    match bincode::decode_from_slice::<cpu::Cpu, _>(bytes, bincode::config::standard()) {
        Ok((loaded, _)) => {
            *cpu = loaded;
            true
        }
        Err(_) => false,
    }
}
