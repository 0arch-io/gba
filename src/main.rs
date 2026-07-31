mod bus;
mod cpu;

use std::env;
use std::process::ExitCode;

/// Render the mode-4 frame (8bpp paletted bitmap) as a PPM. jsmolka's test
/// ROMs draw PASS/FAIL text in this mode.
fn dump_mode4(b: &bus::Bus, path: &str) {
    let mut out = String::from("P3\n240 160\n255\n");
    for i in 0..240 * 160 {
        let pi = b.vram[i] as usize * 2;
        let c = u16::from_le_bytes([b.palette[pi], b.palette[pi + 1]]);
        let r = (c & 0x1F) << 3;
        let g = (c >> 5 & 0x1F) << 3;
        let bl = (c >> 10 & 0x1F) << 3;
        out += &format!("{r} {g} {bl}\n");
    }
    std::fs::write(path, out).unwrap();
}

fn main() -> ExitCode {
    let Some(rom_path) = env::args().nth(1) else {
        eprintln!("usage: gba <rom.gba>");
        return ExitCode::FAILURE;
    };
    let rom = std::fs::read(&rom_path).expect("failed to read ROM");
    let mut cpu = cpu::Cpu::new(bus::Bus::new(rom));

    let steps: u64 = env::var("GBA_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(5_000_000);
    let mut ring: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(400);
    for _ in 0..steps {
        let pc = cpu.regs[15];
        let thumb = cpu.cpsr & 0x20 != 0;
        let op = if thumb { cpu.bus.read16(pc) as u32 } else { cpu.bus.read32(pc) };
        let _ = thumb;
        if ring.len() == 400 { ring.remove(0); }
        ring.push((pc, op, cpu.regs[13], cpu.regs[14]));
        cpu.step();
        cpu.bus.ticks += 1;
        let top = cpu.regs[15] >> 24;
        if (cpu.cpsr & 0x1F == 0x1F && cpu.regs[13] >> 24 != 3) || top != 8 {
            eprintln!("PC escaped to {:#010X}; last instructions:", cpu.regs[15]);
            for (p, o, sp, lr) in &ring {
                eprintln!("  {:#010X}: {:#010X} sp={:#010X} lr={:#010X}", p, o, sp, lr);
            }
            break;
        }
    }
    eprintln!(
        "after {steps} steps: pc={:#010X} r7={:#X} r0-3={:X?}",
        cpu.regs[15],
        cpu.regs[7],
        &cpu.regs[0..4]
    );
    dump_mode4(&cpu.bus, "frame.ppm");
    ExitCode::SUCCESS
}
