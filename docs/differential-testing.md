# Differential testing against mGBA

This is how I convinced myself the emulator is actually correct rather than merely convincing-looking. The short version: I ran Pokémon FireRed for 9,000 frames under a scripted input sequence, ran the identical script through mGBA, and compared the framebuffer plus the video memories frame by frame. They matched everywhere. This document covers why that test exists, how the harness works, what it found, and what it cannot find.

## The problem: emulator bugs are invisible until they are not

An emulator is a very large pile of behavior with almost no natural error signal. If I decode an instruction wrong, nothing throws. The wrong number lands in a register, the game keeps running, and a hundred thousand cycles later a sprite is one pixel off or a menu draws garbage. The symptom rarely resembles the cause: a missing BIOS call looks like a broken renderer, a DMA bug looks like an audio bug, timing that is slightly too generous looks like a hang in the boot code.

The standard answer is test ROMs, and they are useful. The jsmolka `arm`, `thumb` and `memory` suites got the CPU core from "crashes immediately" to "executes correctly", and this emulator passes all three in full. But a test ROM only covers what its author thought to test. Nobody writes one for "the sound driver re-enables DMA1 every frame and expects the source register to still hold the value it originally programmed", because nobody thinks of that until a real game does it.

## The idea: a commercial game as the oracle, another emulator as the answer key

A real game exercises the hardware the way the hardware was meant to be exercised. FireRed alone touches the ARM and Thumb instruction sets, four background layers with windows and blending, affine sprites, all four DMA channels, timers, interrupts, the flash save chip, both DirectSound FIFO channels and a dozen BIOS calls, all within the first two minutes. It is a far better workload than anything I would write by hand. What it lacks is assertions, so I needed an answer key, and mGBA is a mature, widely trusted emulator that is right about nearly everything I could plausibly get wrong. If both emulators consume the exact same input and produce the exact same observable state, every disagreement is a bug (almost always mine), and the frame number of the first disagreement says roughly when it happened. That is differential testing: don't ask "is this output correct", which requires knowing the right answer, ask "do these two independent implementations agree", which does not.

## The harness

Determinism is the whole game. Two emulators fed identical input from identical reset state must, if both are correct, produce identical frames forever. There is no wall clock and no randomness in a GBA beyond what the game derives from its own state. The pieces:

**Scripted input.** This emulator accepts `GBA_INPUT="100-110:a,200-260:up"`, meaning "hold A from frame 100 to 110, hold Up from 200 to 260". Frame ranges, not timestamps, so it is reproducible regardless of host speed.

**A reference driver that speaks the same script.** `tools/refprobe.c` is about 130 lines of C that links against libmgba (installed with `brew install mgba`, so there is no vendored copy of anyone else's emulator here) and parses the same `GBA_INPUT` format with the same key-bit assignments. It creates a GBA core, points it at a video buffer, loads the ROM, and runs frames in a loop, calling `setKeys` before each `runFrame`.

One subtlety is worth calling out, because getting it wrong wasted an afternoon. The Rust runner computes which keys are held from the count of completed frames, so the keys visible during frame `f` come from index `f - 1`. The reference driver replicates that off-by-one deliberately:

```c
// The Rust runner computes the held keys from the count of COMPLETED
// frames, so the keys seen during frame f come from f-1. Match that.
int n = f - 1;
```

If the two sides disagree about input phase by a single frame, everything downstream diverges and the diff is useless noise. Aligning the input timing exactly is a precondition for the whole method.

**Matched dumps on both sides.** At chosen frame numbers, both emulators write the same four things: the framebuffer as a PPM image, all of VRAM (0x06000000, 96 KB), palette RAM (0x05000000, 1 KB) and OAM (0x07000000, 1 KB). The reference side also prints the video registers (DISPCNT, the four BGCNTs, the blend and window registers, the scroll offsets) so I can compare configuration, not just results. On this side, `GBA_DUMP_EVERY` with `GBA_DUMP_DIR` writes a filmstrip of frames at a chosen interval. Dumping the memories and not just the picture is the single highest-value decision in the harness, for reasons the first war story makes obvious.

## The bisect

A raw diff at frame 9,000 tells me nothing: once two emulators diverge they stay diverged, and by the end the screens are unrelated. What I want is the first frame where they differ, because that is where the bug is close to the surface. So the loop is a bisect. Run both sides to frame N and compare. If they match the bug is later, if they differ it is earlier. Halve, repeat. Each step is a fresh run from reset with the same script, which is only tolerable because both emulators run far faster than real time headless. Ten or eleven iterations pin down the exact frame out of 9,000.

Then the four dumps disambiguate the layer. If the framebuffers differ but VRAM, palette and OAM are identical, the bug is in the renderer: right data, drawn wrong. If VRAM already differs, the renderer is innocent and something upstream (a DMA, a decompression routine, a CPU write) put wrong bytes there. If only OAM differs it is sprite setup, and if only the palette differs it is a color write or a fade. That is the difference between "somewhere in the emulator" and "in one of these three functions".

## War stories

### The missing Huffman decompressor that looked like a PPU bug

Pressing Start in Kirby: Nightmare in Dream Land replaced the picture with repeating garbage. Every instinct said renderer: the layout was wrong, the tiles repeated, it had the texture of a bad tile-map fetch. I could have spent a long time in `ppu.rs`.

The filmstrip found the frame the picture broke, and the memory dumps settled it immediately: video memory was byte-identical to mGBA right up to the Start press, and 61% different two frames after it. The renderer was innocent, since it never sees anything but VRAM, and VRAM was already wrong. Something was writing garbage into it.

The cause: the pause screen's graphics are Huffman compressed, the game asks the BIOS to expand them with `SWI 0x13` (`HuffUnComp`), and I had not implemented that call. What made it so hard to see is that unimplemented software interrupts fell through to a match arm that ignored them silently. No panic, no log, nothing. The destination buffer kept whatever happened to be in it, the game copied that to VRAM in good faith, and the PPU drew it faithfully. Every layer downstream of the hole behaved correctly on wrong data.

With `HuffUnComp` implemented, video memory, palette and the rendered frame all became identical to mGBA at that frame, with FireRed's output byte-for-byte unchanged (no collateral drift, which is what you want from a fix). I also made unimplemented BIOS calls log under `GBA_SWILOG` instead of vanishing, because the silence was the actual bug. A missing feature that announces itself costs minutes; one that fails quietly costs a day.

### Write-only DMA latches, surfacing as mangled audio

The DMA source and destination registers on real hardware are write-only latches. The CPU writes an address, the DMA controller copies it into an internal pointer, and that internal pointer is what advances during the transfer. The register itself never changes.

I had implemented this the intuitive way instead, advancing the address and writing it back to the register. That is invisible for any transfer programmed fresh each time. But the m4a sound engine re-enables audio DMA every frame and expects the source address to still be the value it originally programmed. With my version, that "original" value had already been walked forward by the previous frame's transfer, so every frame streamed a little further past the end of the mix buffer into whatever memory came next.

What I observed was mangled audio. Not a crash, not a black screen: noise where music should be. The chain from "register semantics" to "the music is wrong" is four subsystems long, and I do not think I would have reasoned my way backward along it. Separating the register from the internal pointer fixed it.

### Timing that has to be roughly right or FireRed will not boot

FireRed deadlocks during boot unless instruction timing is approximately correct. Its boot code and its audio mixer both assume things about how much work fits between interrupts, and a uniform cycle cost for every instruction fails in one of two directions: too slow and boot hangs, too fast and the mixer starves.

The compromise is region-aware timing: instructions are costed by which memory region the program counter is in, 1 cycle in IWRAM, 3 in EWRAM, 4 in ROM. That is not cycle accuracy (no prefetch buffer, no sub-instruction memory timing), but it captures what matters, which is that the IWRAM audio mixer runs several times faster than ROM code. It is tuned rather than derived, correct enough for the games tested, and listed as a known gap in the README for that reason.

### The RNG bug that was not a bug

Scripted walks through the tall grass on Route 1 produced zero wild Pokémon encounters. Given how many you would expect from that much walking, this looked like a broken random number generator, probably a wrong multiply or a seed that never advanced.

Then I ran the same script through mGBA and got the identical result: zero encounters, at exactly the same positions. Both emulators agreeing is strong evidence that the emulation is fine and my model of the game was wrong. FireRed's encounter RNG is deterministic and derives from state that a fixed input script drives identically every run, so a scripted walk does not sample randomly. It replays one trajectory through the RNG sequence, and that trajectory happened to contain no encounters.

This is the underrated half of differential testing: it clears things as well as finding them. Stopping an investigation into a non-bug in ten minutes is worth as much as finding a real one, because the alternative is days spent rewriting a correct RNG.

## What this does not prove

I would rather state the limits than let the "frame-identical" claim carry more weight than it can.

**Agreement is not correctness.** If mGBA and this emulator are wrong in the same way, the diff is clean and both are wrong. That is unlikely, since the implementations are independent and mGBA is heavily validated against hardware, but it is possible where mGBA itself is approximating. The oracle is a very good proxy for hardware. It is not hardware.

**The granularity is one frame.** I compare state at frame boundaries, so anything that goes wrong and comes back within a single frame is invisible: a mid-scanline register write that lands a few cycles early, a DMA that completes at the wrong point but before anything reads the result, an interrupt that fires late but still inside the frame. Sub-frame timing is exactly the class of bug this method is blindest to, which matters given that this emulator's timing is region-aware rather than cycle-accurate. The two limitations compound.

**Coverage is only what the script walks through.** 9,000 frames of FireRed (loading a save, navigating menus, walking Viridian and Route 1 grass) is a serious workload, but it is one path through one game. It does not touch Mode 1 and 2 affine backgrounds, the link cable, the real-time clock, or EEPROM saves in anger.

**Audio is not in the diff, and the diff cannot assign blame.** The comparison covers video output and video memory only, so the audio bugs above were caught by listening rather than by the harness. A sample-level audio diff would be a real improvement and does not exist yet. And a divergence says the two implementations disagree, not which one is at fault. In practice it has always been me, but that is an observation, not a guarantee.

So the claim I am comfortable making is the narrow one: under a specific 9,000-frame scripted run of Pokémon FireRed, this emulator's framebuffer, VRAM, palette RAM and OAM matched mGBA's exactly at every frame checked. Most of the hard bugs in this project (the DMA latching, the affine transforms, the IntrWait flag semantics, the missing Huffman decompressor) were found by that harness rather than by reading code, which is the real argument for building it.
