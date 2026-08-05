//! Focused tests for save-memory detection and the three cartridge save
//! media. These drive the memory controller directly (command sequences,
//! address decoding, blob sizing) because only a FLASH1M ROM is available
//! locally for end-to-end testing.

use crate::bus::{Bus, SaveType, detect_save_type};

/// A minimal ROM image carrying one SDK save marker.
fn rom_with(marker: &[u8]) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    rom[0x4000..0x4000 + marker.len()].copy_from_slice(marker);
    rom
}

fn bus_with(marker: &[u8]) -> Bus {
    Bus::new(rom_with(marker))
}

// ---------------------------------------------------------------- detection

#[test]
fn detects_every_marker() {
    let cases: &[(&[u8], SaveType, usize)] = &[
        (b"EEPROM_V122", SaveType::EepromUnknown, 8192),
        (b"SRAM_V113", SaveType::Sram, 0x8000),
        (b"SRAM_F_V102", SaveType::Sram, 0x8000),
        (b"FLASH_V131", SaveType::Flash64, 0x10000),
        (b"FLASH512_V130", SaveType::Flash64, 0x10000),
        (b"FLASH1M_V103", SaveType::Flash128, 0x20000),
    ];
    for &(marker, want, size) in cases {
        let b = bus_with(marker);
        assert_eq!(b.save_type, want, "marker {:?}", std::str::from_utf8(marker));
        assert_eq!(b.save.len(), size, "marker {:?}", std::str::from_utf8(marker));
    }
}

#[test]
fn unmarked_rom_falls_back_to_flash_128k() {
    let b = Bus::new(vec![0u8; 0x8000]);
    assert_eq!(b.save_type, SaveType::Flash128);
    assert_eq!(b.save.len(), 0x20000);
}

#[test]
fn marker_scan_does_not_run_off_the_end() {
    // A marker prefix at the very end of the image must not read past it.
    let mut rom = vec![0u8; 16];
    rom[10..16].copy_from_slice(b"FLASH1");
    assert_eq!(detect_save_type(&rom), SaveType::Flash128); // fallback, no panic
    assert_eq!(detect_save_type(&[]), SaveType::Flash128);
}

// --------------------------------------------------------------------- SRAM

#[test]
fn sram_reads_and_writes() {
    let mut b = bus_with(b"SRAM_V113");
    b.write8(0x0E00_0000, 0x12);
    b.write8(0x0E00_7FFF, 0x34);
    assert_eq!(b.read8(0x0E00_0000), 0x12);
    assert_eq!(b.read8(0x0E00_7FFF), 0x34);
    assert!(b.save_dirty);
    // 32KB mirrors across the whole 0x0E/0x0F region.
    assert_eq!(b.read8(0x0E00_8000), 0x12);
    assert_eq!(b.read8(0x0F00_0000), 0x12);
    assert_eq!(b.save[0], 0x12);
}

#[test]
fn sram_is_an_eight_bit_bus() {
    let mut b = bus_with(b"SRAM_V113");
    b.write8(0x0E00_0000, 0xAB);
    b.write8(0x0E00_0001, 0xCD);
    // A halfword read sees one byte replicated, not two consecutive bytes.
    assert_eq!(b.read16(0x0E00_0000), 0xABAB);
    assert_eq!(b.read32(0x0E00_0000), 0xABAB_ABAB);
    // A halfword write only puts one byte on the bus.
    b.write16(0x0E00_0010, 0x1234);
    assert_eq!(b.save[0x10], 0x34);
    assert_eq!(b.save[0x11], 0xFF);
}

// -------------------------------------------------------------------- flash

/// Send the AA/55 unlock pair followed by a command byte.
fn flash_cmd(b: &mut Bus, cmd: u8) {
    b.write8(0x0E00_5555, 0xAA);
    b.write8(0x0E00_2AAA, 0x55);
    b.write8(0x0E00_5555, cmd);
}

#[test]
fn flash64_identifies_as_panasonic() {
    let mut b = bus_with(b"FLASH_V131");
    flash_cmd(&mut b, 0x90);
    assert_eq!(b.read8(0x0E00_0000), 0x32);
    assert_eq!(b.read8(0x0E00_0001), 0x1B);
    flash_cmd(&mut b, 0xF0);
    assert_eq!(b.read8(0x0E00_0000), 0xFF); // erased state, not the ID
}

#[test]
fn flash128_identifies_as_sanyo() {
    let mut b = bus_with(b"FLASH1M_V103");
    flash_cmd(&mut b, 0x90);
    assert_eq!(b.read8(0x0E00_0000), 0x62);
    assert_eq!(b.read8(0x0E00_0001), 0x13);
    flash_cmd(&mut b, 0xF0);
    assert_eq!(b.read8(0x0E00_0000), 0xFF);
}

#[test]
fn flash_programs_erases_by_sector_and_erases_whole_chip() {
    let mut b = bus_with(b"FLASH_V131");
    flash_cmd(&mut b, 0xA0);
    b.write8(0x0E00_1234, 0x5A);
    assert_eq!(b.read8(0x0E00_1234), 0x5A);
    assert_eq!(b.save[0x1234], 0x5A);
    assert!(b.save_dirty);

    // Programming can only clear bits, like real flash.
    flash_cmd(&mut b, 0xA0);
    b.write8(0x0E00_1234, 0x0F);
    assert_eq!(b.read8(0x0E00_1234), 0x0A);

    // Another byte in a different 4KB sector survives that sector's erase.
    flash_cmd(&mut b, 0xA0);
    b.write8(0x0E00_2000, 0x77);
    b.write8(0x0E00_5555, 0xAA);
    b.write8(0x0E00_2AAA, 0x55);
    b.write8(0x0E00_5555, 0x80);
    b.write8(0x0E00_5555, 0xAA);
    b.write8(0x0E00_2AAA, 0x55);
    b.write8(0x0E00_1000, 0x30); // erase the 0x1000 sector only
    assert_eq!(b.read8(0x0E00_1234), 0xFF);
    assert_eq!(b.read8(0x0E00_2000), 0x77);

    // Chip erase clears everything.
    b.write8(0x0E00_5555, 0xAA);
    b.write8(0x0E00_2AAA, 0x55);
    b.write8(0x0E00_5555, 0x80);
    flash_cmd(&mut b, 0x10);
    assert!(b.save.iter().all(|&x| x == 0xFF));
}

#[test]
fn flash128_bank_switching_reaches_the_upper_64k() {
    let mut b = bus_with(b"FLASH1M_V103");
    flash_cmd(&mut b, 0xB0);
    b.write8(0x0E00_0000, 1); // select bank 1
    flash_cmd(&mut b, 0xA0);
    b.write8(0x0E00_0100, 0x42);
    assert_eq!(b.save[0x1_0100], 0x42);
    assert_eq!(b.save[0x0_0100], 0xFF);
    assert_eq!(b.read8(0x0E00_0100), 0x42);

    flash_cmd(&mut b, 0xB0);
    b.write8(0x0E00_0000, 0); // back to bank 0
    assert_eq!(b.read8(0x0E00_0100), 0xFF);
}

#[test]
fn flash64_ignores_bank_select() {
    // The 64KB part has no second bank; a stray bank write must not push
    // accesses out of the buffer.
    let mut b = bus_with(b"FLASH_V131");
    flash_cmd(&mut b, 0xB0);
    b.write8(0x0E00_0000, 1);
    flash_cmd(&mut b, 0xA0);
    b.write8(0x0E00_0100, 0x42);
    assert_eq!(b.save[0x0100], 0x42);
    assert_eq!(b.save.len(), 0x10000);
}

// ------------------------------------------------------------------- EEPROM

const EE_BASE: u32 = 0x0D00_0000;

fn bits_of_addr(block: usize, n: usize) -> Vec<u8> {
    (0..n).rev().map(|i| (block >> i & 1) as u8).collect()
}

/// The bit stream for a write request: "10", address, 64 data bits, stop.
fn write_stream(block: usize, n: usize, data: &[u8; 8]) -> Vec<u8> {
    let mut s = vec![1, 0];
    s.extend(bits_of_addr(block, n));
    for byte in data {
        for i in (0..8).rev() {
            s.push(byte >> i & 1);
        }
    }
    s.push(0);
    s
}

/// The bit stream for a read request: "11", address, stop.
fn read_stream(block: usize, n: usize) -> Vec<u8> {
    let mut s = vec![1, 1];
    s.extend(bits_of_addr(block, n));
    s.push(0);
    s
}

/// Clock a bit stream into the chip the way DMA3 does: one halfword each.
fn clock_in(b: &mut Bus, stream: &[u8]) {
    b.eeprom_dma_begin(stream.len() as u32);
    for (i, &bit) in stream.iter().enumerate() {
        b.write16(EE_BASE + i as u32 * 2, bit as u16);
    }
}

/// Clock 68 halfwords out and reassemble the 8 data bytes (4 dummy bits
/// come first).
fn clock_out(b: &mut Bus) -> [u8; 8] {
    let mut out = [0u8; 8];
    for i in 0..68 {
        let bit = (b.read16(EE_BASE + i as u32 * 2) & 1) as u8;
        if i >= 4 {
            let j = i - 4;
            out[j / 8] = out[j / 8] << 1 | bit;
        }
    }
    out
}

#[test]
fn eeprom_512_round_trip() {
    let mut b = bus_with(b"EEPROM_V122");
    assert_eq!(b.save_type, SaveType::EepromUnknown);
    let data = [0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45, 0x67];
    clock_in(&mut b, &write_stream(3, 6, &data));
    // A 73-halfword transfer identifies the 6-bit part.
    assert_eq!(b.save_type, SaveType::Eeprom512);
    assert_eq!(b.save.len(), 512);
    assert_eq!(&b.save[24..32], &data);
    assert!(b.save_dirty);

    clock_in(&mut b, &read_stream(3, 6));
    assert_eq!(clock_out(&mut b), data);
}

#[test]
fn eeprom_8k_round_trip() {
    let mut b = bus_with(b"EEPROM_V126");
    let data = [1, 2, 3, 4, 5, 6, 7, 8];
    // The read request comes first here: 17 halfwords identifies the 14-bit
    // part just as well as the 81-halfword write does.
    clock_in(&mut b, &read_stream(0, 14));
    assert_eq!(b.save_type, SaveType::Eeprom8k);
    assert_eq!(b.save.len(), 8192);
    let _ = clock_out(&mut b);

    clock_in(&mut b, &write_stream(1023, 14, &data));
    assert_eq!(&b.save[1023 * 8..1023 * 8 + 8], &data);
    clock_in(&mut b, &read_stream(1023, 14));
    assert_eq!(clock_out(&mut b), data);
}

#[test]
fn eeprom_addresses_past_the_chip_wrap_instead_of_panicking() {
    let mut b = bus_with(b"EEPROM_V126");
    clock_in(&mut b, &read_stream(0, 14));
    let _ = clock_out(&mut b);
    let data = [9u8; 8];
    // 14-bit addressing can name blocks the 8KB part does not have.
    clock_in(&mut b, &write_stream(9000, 14, &data));
    clock_in(&mut b, &read_stream(9000, 14));
    assert_eq!(clock_out(&mut b), data);
}

#[test]
fn eeprom_resynchronises_after_a_malformed_stream() {
    let mut b = bus_with(b"EEPROM_V122");
    b.eeprom_dma_begin(73);
    // Garbage: leading zeros and a truncated command.
    for bit in [0u16, 0, 1, 0, 1, 1, 0] {
        b.write16(EE_BASE, bit);
    }
    let data = [0xA5; 8];
    clock_in(&mut b, &write_stream(5, 6, &data));
    clock_in(&mut b, &read_stream(5, 6));
    assert_eq!(clock_out(&mut b), data);
}

#[test]
fn eeprom_region_is_rom_when_the_cart_has_no_eeprom() {
    let mut b = bus_with(b"FLASH1M_V103");
    // 0x0D must keep mirroring ROM (reads past the image give 0xFF) and must
    // not swallow writes as EEPROM bits.
    assert_eq!(b.read16(EE_BASE), 0xFFFF);
    b.write16(EE_BASE, 1);
    assert_eq!(b.read16(EE_BASE), 0xFFFF);
}

#[test]
fn eeprom_driven_by_a_real_dma3_transfer() {
    let mut b = bus_with(b"EEPROM_V126");
    let data = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    let stream = write_stream(7, 14, &data);
    assert_eq!(stream.len(), 81);

    // Stage the bit stream in EWRAM, one bit per halfword, then program DMA3.
    let stage = |b: &mut Bus, bits: &[u8]| {
        for (i, &bit) in bits.iter().enumerate() {
            b.write16(0x0200_0000 + i as u32 * 2, bit as u16);
        }
    };
    let dma3 = |b: &mut Bus, src: u32, dst: u32, count: u16| {
        b.write32(0x0400_00D4, src);
        b.write32(0x0400_00D8, dst);
        b.write16(0x0400_00DC, count);
        b.write16(0x0400_00DE, 0x8000); // enable, immediate, 16-bit
    };

    stage(&mut b, &stream);
    dma3(&mut b, 0x0200_0000, EE_BASE, 81);
    assert_eq!(b.save_type, SaveType::Eeprom8k);
    assert_eq!(&b.save[56..64], &data);

    // Read it back the same way: request, then a 68-halfword fetch.
    let req = read_stream(7, 14);
    stage(&mut b, &req);
    dma3(&mut b, 0x0200_0000, EE_BASE, req.len() as u16);
    dma3(&mut b, EE_BASE, 0x0200_1000, 68);
    let mut out = [0u8; 8];
    for i in 4..68u32 {
        let bit = (b.read16(0x0200_1000 + i * 2) & 1) as u8;
        let j = (i - 4) as usize;
        out[j / 8] = out[j / 8] << 1 | bit;
    }
    assert_eq!(out, data);
}

// ----------------------------------------------------------- .sav handling

#[test]
fn existing_sav_settles_the_eeprom_size() {
    let mut b = bus_with(b"EEPROM_V122");
    assert!(b.load_save(&[0x7Eu8; 512]));
    assert_eq!(b.save_type, SaveType::Eeprom512);
    assert_eq!(b.save.len(), 512);

    let mut b = bus_with(b"EEPROM_V122");
    assert!(b.load_save(&vec![0x7Eu8; 8192]));
    assert_eq!(b.save_type, SaveType::Eeprom8k);
}

#[test]
fn wrong_sized_sav_is_taken_gracefully() {
    // Too small: the tail stays erased.
    let mut b = bus_with(b"FLASH1M_V103");
    assert!(!b.load_save(&[0xAAu8; 0x8000]));
    assert_eq!(b.save.len(), 0x20000);
    assert_eq!(b.save[0], 0xAA);
    assert_eq!(b.save[0x8000], 0xFF);

    // Too big: the excess is dropped.
    let mut b = bus_with(b"SRAM_V113");
    assert!(!b.load_save(&vec![0xBBu8; 0x20000]));
    assert_eq!(b.save.len(), 0x8000);
    assert_eq!(b.save[0x7FFF], 0xBB);

    // Degenerate inputs must not panic.
    let mut b = bus_with(b"EEPROM_V122");
    assert!(!b.load_save(&[]));
    assert!(!b.load_save(&[1, 2, 3]));
}
