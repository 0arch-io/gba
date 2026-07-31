# gba

A Game Boy Advance emulator in Rust. Built from scratch; plays Pokémon
FireRed with music and sound effects, verified frame-identical against
mGBA over long scripted gameplay runs.

## Usage

```
cargo build --release
./target/release/gba <rom.gba>
```

Battery saves are written to `<rom>.gba.sav` next to the ROM (128KB flash,
Sanyo protocol). Save states go to `<rom>.gba.state`.

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
- 128KB flash saves, save states, fast-forward, pause

## Verification tools

- `GBA_FRAMES=N ./target/release/gba rom --headless` renders N frames and
  writes `frame.ppm`
- `GBA_INPUT="100-110:a,200-260:up" ...` scripted input for headless
  playthroughs
- `GBA_WAV=1` captures the last 10s of audio to `samples.raw` (f32le stereo)
- `refprobe.c` links against libmgba (brew mgba) to run the same script in
  mGBA and dump memory + a reference frame for ground-truth diffs
- Debug hooks behind env vars: `GBA_SWILOG`, `GBA_IOLOG`, `GBA_MODELOG`,
  `GBA_BOOTTRACE`, `GBA_PALTRACE`, `GBA_BADPTR`, `GBA_OBJDEBUG`, `GBA_BREAK`

## Known gaps

- No serial/link cable, no RTC (FireRed needs neither)
- Timing is region-approximate (IWRAM 1 / EWRAM 3 / ROM 4 cycles per
  instruction), no prefetch or sub-instruction memory timing
- Mode 1/2 affine backgrounds are lightly exercised
