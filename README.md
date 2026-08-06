# gba

A Game Boy Advance emulator in Rust. Built from scratch; plays Pokémon
FireRed with music and sound effects, verified frame-identical against
mGBA over long scripted gameplay runs.

## Usage

```
cargo build --release
./target/release/gba <rom.gba>
```

Battery saves are written to `<rom>.gba.sav` next to the ROM. The save
hardware is detected from the marker string the SDK leaves in the ROM
(`EEPROM_V`, `SRAM_V`, `SRAM_F_V`, `FLASH_V`, `FLASH512_V`, `FLASH1M_V`), so
the .sav is 512B, 8KB, 32KB, 64KB or 128KB depending on the cartridge; the
emulator prints which one it picked at startup. Save states go to
`<rom>.gba.state`.

## Controls

| GBA | Keyboard |
|-----|----------|
| A / B | Z / X |
| Start / Select | Enter / Right Shift |
| D-pad | Arrows |
| L / R | Q / W |
| Save / load state | F5 / F7 |
| Pause | P |
| Fast-forward (hold) | Tab |
| Quit | Esc |

## What's implemented

- ARM7TDMI: full ARM + Thumb instruction sets, mode banking, IRQs
  (passes the jsmolka arm/thumb/memory suites)
- BIOS HLE, no BIOS image needed: CpuSet/CpuFastSet, Div/Sqrt/ArcTan2,
  LZ77/RLE/BitUnPack decompression, ObjAffineSet/BgAffineSet,
  IntrWait/VBlankIntrWait with real flag semantics, IRQ dispatch stub
- PPU: modes 0-5, text + affine backgrounds, regular + affine sprites,
  windows, alpha/brightness blending, scanline timing
- DMA (immediate/vblank/hblank/FIFO, hardware-latched addresses),
  timers with cascade, keypad
- Audio: both DirectSound FIFO channels plus the four PSG channels,
  low-pass filtered, audio-clock-paced frontend (no drift stutter)
- All four save media with auto-detection: 32KB SRAM, 64KB flash
  (Panasonic 0x32/0x1B), 128KB flash in two banks (Sanyo 0x62/0x13), and
  bit-serial EEPROM over DMA3 in both the 512B (6-bit address) and 8KB
  (14-bit address) variants, the width inferred from the transfer length
- Save states, fast-forward, pause

## Verification tools

- `GBA_FRAMES=N ./target/release/gba rom --headless` renders N frames and
  writes `frame.ppm`
- `GBA_INPUT="100-110:a,200-260:up" ...` scripted input for headless
  playthroughs
- `GBA_WAV=1` captures the last 10s of audio to `samples.raw` (f32le stereo)
- `refprobe.c` links against libmgba (brew mgba) to run the same script in
  mGBA and dump memory + a reference frame for ground-truth diffs
- Debug hooks behind env vars: `GBA_SWILOG`, `GBA_IOLOG`, `GBA_MODELOG`,
  `GBA_BOOTTRACE`, `GBA_PALTRACE`, `GBA_BADPTR`, `GBA_OBJDEBUG`, `GBA_BREAK`,
  `GBA_SAVELOG`

## Known gaps

- No serial/link cable, no RTC (FireRed needs neither)
- Timing is region-approximate (IWRAM 1 / EWRAM 3 / ROM 4 cycles per
  instruction), no prefetch or sub-instruction memory timing
- Mode 1/2 affine backgrounds are lightly exercised
- A ROM with no save marker at all falls back to 128KB flash
- `cargo test` covers the save controllers (detection, the flash command
  protocol, the EEPROM bit protocol, .sav sizing); only the 128KB flash path
  has been exercised against a real game
