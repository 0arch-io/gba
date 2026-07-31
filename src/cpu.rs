use crate::bus::Bus;

/// CPSR flag bits.
const N: u32 = 1 << 31;
const Z: u32 = 1 << 30;
const C: u32 = 1 << 29;
const V: u32 = 1 << 28;
const T: u32 = 1 << 5; // Thumb state

/// The ARM7TDMI. Executes both the 32-bit ARM and 16-bit Thumb instruction
/// sets. No pipeline emulation: r15 reads as the architectural PC+8 (ARM) or
/// PC+4 (Thumb), which is what a flushed three-stage pipeline exposes.
pub struct Cpu {
    pub regs: [u32; 16],
    pub cpsr: u32,
    spsr: u32,
    // Banked registers per mode: usr/sys share, fiq, svc, abt, irq, und.
    bank_usr: [u32; 2], // r13, r14
    bank_fiq: [u32; 7], // r8-r14
    bank_svc: [u32; 2],
    bank_abt: [u32; 2],
    bank_irq: [u32; 2],
    bank_und: [u32; 2],
    bank_fiq_usr: [u32; 5], // usr r8-r12 while in FIQ
    spsr_fiq: u32,
    spsr_svc: u32,
    spsr_abt: u32,
    spsr_irq: u32,
    spsr_und: u32,
    pub halted: bool,
    pub bus: Bus,
}

impl Cpu {
    /// State after the BIOS hands off to a cartridge: PC at ROM start,
    /// system mode, stacks set the way the real BIOS leaves them.
    pub fn new(bus: Bus) -> Self {
        let mut regs = [0u32; 16];
        regs[13] = 0x0300_7F00; // SP (usr/sys)
        regs[15] = 0x0800_0000;
        Self {
            regs,
            cpsr: 0x1F, // system mode, ARM state
            spsr: 0,
            bank_usr: [0x0300_7F00, 0],
            bank_fiq: [0; 7],
            bank_svc: [0x0300_7FE0, 0],
            bank_abt: [0; 2],
            bank_irq: [0x0300_7FA0, 0],
            bank_und: [0; 2],
            bank_fiq_usr: [0; 5],
            spsr_fiq: 0,
            spsr_svc: 0,
            spsr_abt: 0,
            spsr_irq: 0,
            spsr_und: 0,
            halted: false,
            bus,
        }
    }

    fn thumb(&self) -> bool {
        self.cpsr & T != 0
    }

    fn flag(&self, f: u32) -> bool {
        self.cpsr & f != 0
    }

    fn set_flag(&mut self, f: u32, on: bool) {
        if on { self.cpsr |= f } else { self.cpsr &= !f }
    }

    fn set_nz(&mut self, v: u32) {
        self.set_flag(N, v & 0x8000_0000 != 0);
        self.set_flag(Z, v == 0);
    }

    /// Reading a register; r15 shows PC + pipeline offset.
    fn r(&self, i: u32) -> u32 {
        if i == 15 {
            self.regs[15].wrapping_add(if self.thumb() { 4 } else { 8 })
        } else {
            self.regs[i as usize]
        }
    }

    /// Writing r15 branches (the caller must not then advance PC).
    fn set_r(&mut self, i: u32, v: u32) {
        if i == 15 {
            self.regs[15] = v & if self.thumb() { !1 } else { !3 };
        } else {
            self.regs[i as usize] = v;
        }
    }

    fn mode(&self) -> u32 {
        self.cpsr & 0x1F
    }

    /// Swap banked registers when leaving `old` mode for the current one.
    fn switch_mode(&mut self, old: u32) {
        let new = self.mode();
        if std::env::var("GBA_MODELOG").is_ok() {
            eprintln!("mode {:#04X} -> {:#04X} at pc={:#010X} sp={:#010X}", old, new, self.regs[15], self.regs[13]);
        }
        if old == new || (old | new) & !0xF == 0x10 && (old & 0xF) == (new & 0xF) {
            return;
        }
        // Save current r13/r14 (and FIQ r8-12) into the old mode's bank.
        match old {
            0x11 => {
                self.bank_fiq.copy_from_slice(&self.regs[8..15]);
                self.regs[8..13].copy_from_slice(&self.bank_fiq_usr);
            }
            0x13 => self.bank_svc.copy_from_slice(&self.regs[13..15]),
            0x17 => self.bank_abt.copy_from_slice(&self.regs[13..15]),
            0x12 => self.bank_irq.copy_from_slice(&self.regs[13..15]),
            0x1B => self.bank_und.copy_from_slice(&self.regs[13..15]),
            _ => self.bank_usr.copy_from_slice(&self.regs[13..15]),
        }
        match new {
            0x11 => {
                self.bank_fiq_usr.copy_from_slice(&self.regs[8..13]);
                self.regs[8..15].copy_from_slice(&self.bank_fiq);
            }
            0x13 => self.regs[13..15].copy_from_slice(&self.bank_svc),
            0x17 => self.regs[13..15].copy_from_slice(&self.bank_abt),
            0x12 => self.regs[13..15].copy_from_slice(&self.bank_irq),
            0x1B => self.regs[13..15].copy_from_slice(&self.bank_und),
            _ => self.regs[13..15].copy_from_slice(&self.bank_usr),
        }
    }

    fn spsr_for_mode(&mut self) -> &mut u32 {
        match self.mode() {
            0x11 => &mut self.spsr_fiq,
            0x13 => &mut self.spsr_svc,
            0x17 => &mut self.spsr_abt,
            0x12 => &mut self.spsr_irq,
            0x1B => &mut self.spsr_und,
            _ => &mut self.spsr, // usr/sys: reading SPSR is unpredictable; give scratch
        }
    }

    /// Read/write the user-bank view of a register from any mode (LDM/STM ^).
    fn user_reg(&self, i: u32) -> u32 {
        let m = self.mode();
        match i {
            8..=12 if m == 0x11 => self.bank_fiq_usr[i as usize - 8],
            13 | 14 if m != 0x10 && m != 0x1F => self.bank_usr[i as usize - 13],
            _ => self.regs[i as usize],
        }
    }

    fn set_user_reg(&mut self, i: u32, v: u32) {
        let m = self.mode();
        match i {
            8..=12 if m == 0x11 => self.bank_fiq_usr[i as usize - 8] = v,
            13 | 14 if m != 0x10 && m != 0x1F => self.bank_usr[i as usize - 13] = v,
            _ => self.regs[i as usize] = v,
        }
    }

    fn cond(&self, c: u32) -> bool {
        match c {
            0x0 => self.flag(Z),
            0x1 => !self.flag(Z),
            0x2 => self.flag(C),
            0x3 => !self.flag(C),
            0x4 => self.flag(N),
            0x5 => !self.flag(N),
            0x6 => self.flag(V),
            0x7 => !self.flag(V),
            0x8 => self.flag(C) && !self.flag(Z),
            0x9 => !self.flag(C) || self.flag(Z),
            0xA => self.flag(N) == self.flag(V),
            0xB => self.flag(N) != self.flag(V),
            0xC => !self.flag(Z) && self.flag(N) == self.flag(V),
            0xD => self.flag(Z) || self.flag(N) != self.flag(V),
            _ => true,
        }
    }

    /// Barrel shifter. `imm` = amount came from an immediate field (special
    /// cases for 0); returns (result, carry_out).
    fn shift(&self, ty: u32, val: u32, amount: u32, imm: bool) -> (u32, bool) {
        let c_in = self.flag(C);
        match ty {
            0 => {
                // LSL
                if amount == 0 {
                    (val, c_in)
                } else if amount < 32 {
                    (val << amount, val >> (32 - amount) & 1 != 0)
                } else if amount == 32 {
                    (0, val & 1 != 0)
                } else {
                    (0, false)
                }
            }
            1 => {
                // LSR (imm 0 encodes 32)
                let amount = if imm && amount == 0 { 32 } else { amount };
                if amount == 0 {
                    (val, c_in)
                } else if amount < 32 {
                    (val >> amount, val >> (amount - 1) & 1 != 0)
                } else if amount == 32 {
                    (0, val >> 31 != 0)
                } else {
                    (0, false)
                }
            }
            2 => {
                // ASR (imm 0 encodes 32)
                let amount = if imm && amount == 0 { 32 } else { amount };
                if amount == 0 {
                    (val, c_in)
                } else if amount < 32 {
                    (((val as i32) >> amount) as u32, val >> (amount - 1) & 1 != 0)
                } else {
                    let fill = if val >> 31 != 0 { u32::MAX } else { 0 };
                    (fill, val >> 31 != 0)
                }
            }
            _ => {
                // ROR (imm 0 encodes RRX)
                if imm && amount == 0 {
                    ((val >> 1) | ((c_in as u32) << 31), val & 1 != 0)
                } else if amount == 0 {
                    (val, c_in)
                } else {
                    let a = amount & 31;
                    if a == 0 {
                        (val, val >> 31 != 0)
                    } else {
                        (val.rotate_right(a), val >> (a - 1) & 1 != 0)
                    }
                }
            }
        }
    }

    fn add_with_flags(&mut self, a: u32, b: u32, carry: u32, set: bool) -> u32 {
        let r64 = a as u64 + b as u64 + carry as u64;
        let r = r64 as u32;
        if set {
            self.set_nz(r);
            self.set_flag(C, r64 > 0xFFFF_FFFF);
            self.set_flag(V, (!(a ^ b) & (a ^ r)) >> 31 != 0);
        }
        r
    }

    fn sub_with_flags(&mut self, a: u32, b: u32, carry: u32, set: bool) -> u32 {
        // carry: 1 for SUB, C flag for SBC.
        self.add_with_flags(a, !b, carry, set)
    }

    pub fn step(&mut self) {
        // IRQ delivery: wakes from halt; taken when IME, IE&IF, and CPSR.I allow.
        let pending = self.bus.ie & self.bus.if_ != 0;
        if pending {
            self.halted = false;
            if self.bus.ime && self.cpsr & 0x80 == 0 {
                let thumb = self.thumb();
                let old_cpsr = self.cpsr;
                let old_mode = self.mode();
                // LR_irq = next instruction + 4
                let ret = self.regs[15].wrapping_add(if thumb { 4 } else { 4 });
                self.cpsr = (self.cpsr & !0x3F) | 0x12 | 0x80; // IRQ mode, ARM, I set
                self.switch_mode(old_mode);
                *self.spsr_for_mode() = old_cpsr;
                self.regs[14] = ret;
                self.regs[15] = 0x18;
                return;
            }
        }
        if self.halted {
            return;
        }
        if self.thumb() {
            let op = self.bus.read16(self.regs[15]);
            let pc_before = self.regs[15];
            self.exec_thumb(op);
            if self.regs[15] == pc_before {
                self.regs[15] = self.regs[15].wrapping_add(2);
            }
        } else {
            let op = self.bus.read32(self.regs[15]);
            let pc_before = self.regs[15];
            if self.cond(op >> 28) {
                self.exec_arm(op);
            }
            if self.regs[15] == pc_before {
                self.regs[15] = self.regs[15].wrapping_add(4);
            }
        }
    }

    // ===================== ARM =====================

    fn exec_arm(&mut self, op: u32) {
        if op & 0x0FFF_FFF0 == 0x012F_FF10 {
            // BX
            let v = self.r(op & 0xF);
            self.set_flag(T, v & 1 != 0);
            self.regs[15] = v & !1 & if v & 1 != 0 { !0 } else { !3 };
            return;
        }
        if op & 0x0E00_0000 == 0x0A00_0000 {
            // B/BL
            let off = ((op << 8) as i32 >> 6) as u32; // sign-extend 24-bit, <<2
            if op & 0x0100_0000 != 0 {
                self.regs[14] = self.regs[15].wrapping_add(4);
            }
            self.regs[15] = self.regs[15].wrapping_add(8).wrapping_add(off);
            return;
        }
        if op & 0x0FC0_00F0 == 0x0000_0090 {
            // MUL/MLA
            let rd = op >> 16 & 0xF;
            let rn = op >> 12 & 0xF;
            let rs = op >> 8 & 0xF;
            let rm = op & 0xF;
            let mut r = self.r(rm).wrapping_mul(self.r(rs));
            if op & 0x0020_0000 != 0 {
                r = r.wrapping_add(self.r(rn));
            }
            self.set_r(rd, r);
            if op & 0x0010_0000 != 0 {
                self.set_nz(r);
            }
            return;
        }
        if op & 0x0F80_00F0 == 0x0080_0090 {
            // UMULL/UMLAL/SMULL/SMLAL
            let rdhi = op >> 16 & 0xF;
            let rdlo = op >> 12 & 0xF;
            let rs = op >> 8 & 0xF;
            let rm = op & 0xF;
            let signed = op & 0x0040_0000 != 0;
            let acc = op & 0x0020_0000 != 0;
            let mut r: u64 = if signed {
                (self.r(rm) as i32 as i64).wrapping_mul(self.r(rs) as i32 as i64) as u64
            } else {
                (self.r(rm) as u64).wrapping_mul(self.r(rs) as u64)
            };
            if acc {
                r = r.wrapping_add((self.r(rdhi) as u64) << 32 | self.r(rdlo) as u64);
            }
            self.set_r(rdlo, r as u32);
            self.set_r(rdhi, (r >> 32) as u32);
            if op & 0x0010_0000 != 0 {
                self.set_flag(N, r >> 63 != 0);
                self.set_flag(Z, r == 0);
            }
            return;
        }
        if op & 0x0FB0_0FF0 == 0x0100_0090 {
            // SWP/SWPB
            let addr = self.r(op >> 16 & 0xF);
            let rm = self.r(op & 0xF);
            let rd = op >> 12 & 0xF;
            if op & 0x0040_0000 != 0 {
                let old = self.bus.read8(addr) as u32;
                self.bus.write8(addr, rm as u8);
                self.set_r(rd, old);
            } else {
                let old = self.bus.read32(addr).rotate_right((addr & 3) * 8);
                self.bus.write32(addr, rm);
                self.set_r(rd, old);
            }
            return;
        }
        if op & 0x0E00_0090 == 0x0000_0090 && op & 0x60 != 0 {
            // Halfword / signed transfers: LDRH/STRH/LDRSB/LDRSH
            self.arm_halfword(op);
            return;
        }
        if op & 0x0FBF_0FFF == 0x010F_0000 {
            // MRS
            let v = if op & 0x0040_0000 != 0 { *self.spsr_for_mode() } else { self.cpsr };
            self.set_r(op >> 12 & 0xF, v);
            return;
        }
        if op & 0x0DB0_F000 == 0x0120_F000 {
            // MSR
            let val = if op & 0x0200_0000 != 0 {
                let imm = op & 0xFF;
                imm.rotate_right((op >> 8 & 0xF) * 2)
            } else {
                self.r(op & 0xF)
            };
            let mut mask = 0u32;
            if op & 0x0008_0000 != 0 {
                mask |= 0xFF00_0000;
            }
            if op & 0x0001_0000 != 0 {
                mask |= 0x0000_00FF;
            }
            if op & 0x0040_0000 != 0 {
                let s = self.spsr_for_mode();
                *s = (*s & !mask) | (val & mask);
            } else {
                if self.mode() == 0x10 {
                    mask &= 0xFF00_0000; // user mode can't change control bits
                }
                let old = self.mode();
                self.cpsr = (self.cpsr & !mask) | (val & mask);
                self.switch_mode(old);
            }
            return;
        }
        if op & 0x0C00_0000 == 0x0000_0000 {
            self.arm_data_processing(op);
            return;
        }
        if op & 0x0C00_0000 == 0x0400_0000 {
            self.arm_single_transfer(op);
            return;
        }
        if op & 0x0E00_0000 == 0x0800_0000 {
            self.arm_block_transfer(op);
            return;
        }
        if op & 0x0F00_0000 == 0x0F00_0000 {
            self.hle_swi(op >> 16 & 0xFF);
            return;
        }
        panic!("unimplemented ARM op {op:#010X} at {:#010X}", self.regs[15]);
    }

    fn exception(&mut self, vector: u32, mode: u32) {
        let old_cpsr = self.cpsr;
        let old_mode = self.mode();
        let ret = self.regs[15].wrapping_add(4);
        self.cpsr = (self.cpsr & !0x3F) | mode | 0x80; // new mode, IRQs off, ARM state
        self.switch_mode(old_mode);
        *self.spsr_for_mode() = old_cpsr;
        self.regs[14] = ret;
        self.regs[15] = vector;
    }

    /// Operand 2 for data processing: (value, shifter carry).
    fn dp_operand2(&mut self, op: u32) -> (u32, bool) {
        if op & 0x0200_0000 != 0 {
            let imm = op & 0xFF;
            let rot = (op >> 8 & 0xF) * 2;
            if rot == 0 {
                (imm, self.flag(C))
            } else {
                let v = imm.rotate_right(rot);
                (v, v >> 31 != 0)
            }
        } else {
            let rm = op & 0xF;
            let ty = op >> 5 & 3;
            if op & 0x10 != 0 {
                // Register shift amount: r15 reads +12 here (pipeline quirk).
                let amount = self.r(op >> 8 & 0xF) & 0xFF;
                let val = if rm == 15 { self.r(15).wrapping_add(4) } else { self.r(rm) };
                self.shift(ty, val, amount, false)
            } else {
                let amount = op >> 7 & 0x1F;
                self.shift(ty, self.r(rm), amount, true)
            }
        }
    }

    fn arm_data_processing(&mut self, op: u32) {
        let opcode = op >> 21 & 0xF;
        let set = op & 0x0010_0000 != 0;
        let rn = op >> 16 & 0xF;
        let rd = op >> 12 & 0xF;
        // TST/TEQ/CMP/CMN with Rd=15: result discarded, CPSR <- SPSR (TEQP idiom).
        if (0x8..=0xB).contains(&opcode) && rd == 15 {
            let old = self.mode();
            self.cpsr = *self.spsr_for_mode();
            self.switch_mode(old);
            return;
        }
        let (op2, sh_carry) = self.dp_operand2(op);
        let a = if rn == 15 && op & 0x0200_0000 == 0 && op & 0x10 != 0 {
            self.r(15).wrapping_add(4)
        } else {
            self.r(rn)
        };
        let logical_flags = |cpu: &mut Cpu, r: u32| {
            cpu.set_nz(r);
            cpu.set_flag(C, sh_carry);
        };
        let c = self.flag(C) as u32;
        let result = match opcode {
            0x0 => { let r = a & op2; if set { logical_flags(self, r) } Some(r) } // AND
            0x1 => { let r = a ^ op2; if set { logical_flags(self, r) } Some(r) } // EOR
            0x2 => Some(self.sub_with_flags(a, op2, 1, set)),                     // SUB
            0x3 => Some(self.sub_with_flags(op2, a, 1, set)),                     // RSB
            0x4 => Some(self.add_with_flags(a, op2, 0, set)),                     // ADD
            0x5 => Some(self.add_with_flags(a, op2, c, set)),                     // ADC
            0x6 => Some(self.sub_with_flags(a, op2, c, set)),                     // SBC
            0x7 => Some(self.sub_with_flags(op2, a, c, set)),                     // RSC
            0x8 => { let r = a & op2; logical_flags(self, r); None }              // TST
            0x9 => { let r = a ^ op2; logical_flags(self, r); None }              // TEQ
            0xA => { self.sub_with_flags(a, op2, 1, true); None }                 // CMP
            0xB => { self.add_with_flags(a, op2, 0, true); None }                 // CMN
            0xC => { let r = a | op2; if set { logical_flags(self, r) } Some(r) } // ORR
            0xD => { let r = op2; if set { logical_flags(self, r) } Some(r) }     // MOV
            0xE => { let r = a & !op2; if set { logical_flags(self, r) } Some(r) }// BIC
            _ => { let r = !op2; if set { logical_flags(self, r) } Some(r) }      // MVN
        };
        if let Some(r) = result {
            if rd == 15 {
                if set {
                    // Return from exception: restore CPSR from SPSR.
                    let old = self.mode();
                    self.cpsr = *self.spsr_for_mode();
                    self.switch_mode(old);
                }
                self.regs[15] = r & if self.thumb() { !1 } else { !3 };
            } else {
                self.set_r(rd, r);
            }
        }
    }

    fn arm_single_transfer(&mut self, op: u32) {
        let rn = op >> 16 & 0xF;
        let rd = op >> 12 & 0xF;
        let offset = if op & 0x0200_0000 != 0 {
            let (v, _) = self.shift(op >> 5 & 3, self.r(op & 0xF), op >> 7 & 0x1F, true);
            v
        } else {
            op & 0xFFF
        };
        let base = self.r(rn);
        let up = op & 0x0080_0000 != 0;
        let pre = op & 0x0100_0000 != 0;
        let byte = op & 0x0040_0000 != 0;
        let load = op & 0x0010_0000 != 0;
        let wb = op & 0x0020_0000 != 0;
        let off_base = if up { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
        let addr = if pre { off_base } else { base };
        if load {
            let v = if byte {
                self.bus.read8(addr) as u32
            } else {
                self.bus.read32(addr).rotate_right((addr & 3) * 8)
            };
            if !pre || wb {
                self.set_r(rn, off_base);
            }
            self.set_r(rd, v); // load wins over writeback on same register
        } else {
            let v = if rd == 15 { self.r(15).wrapping_add(4) } else { self.r(rd) };
            if byte {
                self.bus.write8(addr, v as u8);
            } else {
                self.bus.write32(addr, v);
            }
            if !pre || wb {
                self.set_r(rn, off_base);
            }
        }
    }

    fn arm_halfword(&mut self, op: u32) {
        let rn = op >> 16 & 0xF;
        let rd = op >> 12 & 0xF;
        let offset = if op & 0x0040_0000 != 0 {
            (op >> 4 & 0xF0) | (op & 0xF)
        } else {
            self.r(op & 0xF)
        };
        let base = self.r(rn);
        let up = op & 0x0080_0000 != 0;
        let pre = op & 0x0100_0000 != 0;
        let load = op & 0x0010_0000 != 0;
        let wb = op & 0x0020_0000 != 0;
        let off_base = if up { base.wrapping_add(offset) } else { base.wrapping_sub(offset) };
        let addr = if pre { off_base } else { base };
        let ty = op >> 5 & 3;
        if load {
            let v = match ty {
                1 => {
                    // LDRH: unaligned rotates like LDR on ARM7
                    let v = self.bus.read16(addr) as u32;
                    v.rotate_right((addr & 1) * 8)
                }
                2 => self.bus.read8(addr) as i8 as i32 as u32, // LDRSB
                _ => {
                    // LDRSH: unaligned behaves as LDRSB
                    if addr & 1 != 0 {
                        self.bus.read8(addr) as i8 as i32 as u32
                    } else {
                        self.bus.read16(addr) as i16 as i32 as u32
                    }
                }
            };
            if !pre || wb {
                self.set_r(rn, off_base);
            }
            self.set_r(rd, v);
        } else {
            let v = self.r(rd);
            self.bus.write16(addr, v as u16);
            if !pre || wb {
                self.set_r(rn, off_base);
            }
        }
    }

    fn arm_block_transfer(&mut self, op: u32) {
        let rn = op >> 16 & 0xF;
        let list = op & 0xFFFF;
        let load = op & 0x0010_0000 != 0;
        let wb = op & 0x0020_0000 != 0;
        let s_bit = op & 0x0040_0000 != 0;
        let up = op & 0x0080_0000 != 0;
        let pre = op & 0x0100_0000 != 0;
        let n = list.count_ones();
        let base = self.r(rn);

        // Empty list: transfers r15, base +/- 0x40 (ARM7 quirk).
        if list == 0 {
            let addr = if up {
                if pre { base + 4 } else { base }
            } else if pre {
                base - 0x40
            } else {
                base - 0x3C
            };
            if load {
                self.regs[15] = self.bus.read32(addr) & !3;
            } else {
                self.bus.write32(addr, self.r(15).wrapping_add(4));
            }
            if wb {
                self.set_r(rn, if up { base + 0x40 } else { base - 0x40 });
            }
            return;
        }

        let start = if up {
            if pre { base.wrapping_add(4) } else { base }
        } else {
            let s = base.wrapping_sub(n * 4);
            if pre { s } else { s.wrapping_add(4) }
        };
        let new_base = if up { base.wrapping_add(n * 4) } else { base.wrapping_sub(n * 4) };

        // S bit without r15 in a load: transfer the user-mode register bank.
        let user_bank = s_bit && !(load && list & 0x8000 != 0);

        let mut addr = start;
        let first_reg = list.trailing_zeros();
        if load {
            if wb {
                self.set_r(rn, new_base);
            }
            for i in 0..16 {
                if list & (1 << i) != 0 {
                    let v = self.bus.read32(addr);
                    if user_bank && i < 15 {
                        self.set_user_reg(i, v);
                        addr = addr.wrapping_add(4);
                        continue;
                    }
                    if i == 15 {
                        if s_bit {
                            let old = self.mode();
                            self.cpsr = *self.spsr_for_mode();
                            self.switch_mode(old);
                        }
                        self.regs[15] = v & if self.thumb() { !1 } else { !3 };
                    } else {
                        self.regs[i as usize] = v;
                    }
                    addr = addr.wrapping_add(4);
                }
            }
        } else {
            for i in 0..16 {
                if list & (1 << i) != 0 {
                    let v = if i == 15 {
                        self.r(15).wrapping_add(4)
                    } else if i == rn && i != first_reg && wb {
                        new_base // store of base after writeback point
                    } else if user_bank {
                        self.user_reg(i)
                    } else {
                        self.regs[i as usize]
                    };
                    self.bus.write32(addr, v);
                    addr = addr.wrapping_add(4);
                }
            }
            if wb {
                self.set_r(rn, new_base);
            }
        }
    }

    /// High-level emulation of BIOS calls (no BIOS image needed).
    fn hle_swi(&mut self, n: u32) {
        match n {
            0x00 => { // SoftReset: jump to ROM entry
                self.regs[15] = 0x0800_0000;
                self.set_flag(T, false);
            }
            0x01 => {} // RegisterRamReset: ignore
            0x02 | 0x03 => self.halted = true, // Halt/Stop
            0x04 => self.halted = true,        // IntrWait: wake on any enabled IRQ
            0x05 => {
                // VBlankIntrWait: enable vblank in IE-of-interest and halt.
                self.halted = true;
            }
            0x06 => { // Div: r0/r1 -> r0=quot, r1=rem, r3=|quot|
                let num = self.regs[0] as i32;
                let den = self.regs[1] as i32;
                if den != 0 {
                    let q = num.wrapping_div(den);
                    self.regs[0] = q as u32;
                    self.regs[1] = num.wrapping_rem(den) as u32;
                    self.regs[3] = q.unsigned_abs();
                }
            }
            0x07 => { // DivArm: r1/r0
                let num = self.regs[1] as i32;
                let den = self.regs[0] as i32;
                if den != 0 {
                    let q = num.wrapping_div(den);
                    self.regs[0] = q as u32;
                    self.regs[1] = num.wrapping_rem(den) as u32;
                    self.regs[3] = q.unsigned_abs();
                }
            }
            0x08 => self.regs[0] = (self.regs[0] as f64).sqrt() as u32, // Sqrt
            0x09 => { // ArcTan (approximation adequate for games)
                let x = self.regs[0] as i16 as f64 / 16384.0;
                self.regs[0] = ((x.atan() / std::f64::consts::PI * 0x8000 as f64) as i32) as u32;
            }
            0x0A => { // ArcTan2
                let x = self.regs[0] as i16 as f64;
                let y = self.regs[1] as i16 as f64;
                let a = y.atan2(x) / (2.0 * std::f64::consts::PI) * 65536.0;
                self.regs[0] = (a as i32 as u32) & 0xFFFF;
            }
            0x0B => { // CpuSet
                let src = self.regs[0];
                let dst = self.regs[1];
                let cnt = self.regs[2];
                let count = cnt & 0x1F_FFFF;
                let fill = cnt & 0x0100_0000 != 0;
                if cnt & 0x0400_0000 != 0 {
                    // 32-bit
                    let v0 = self.bus.read32(src);
                    for i in 0..count {
                        let v = if fill { v0 } else { self.bus.read32(src + i * 4) };
                        self.bus.write32(dst + i * 4, v);
                    }
                } else {
                    let v0 = self.bus.read16(src);
                    for i in 0..count {
                        let v = if fill { v0 } else { self.bus.read16(src + i * 2) };
                        self.bus.write16(dst + i * 2, v);
                    }
                }
            }
            0x0C => { // CpuFastSet: 32-bit only, chunks of 8 words
                let src = self.regs[0];
                let dst = self.regs[1];
                let cnt = self.regs[2];
                let count = ((cnt & 0x1F_FFFF) + 7) & !7;
                let fill = cnt & 0x0100_0000 != 0;
                let v0 = self.bus.read32(src);
                for i in 0..count {
                    let v = if fill { v0 } else { self.bus.read32(src + i * 4) };
                    self.bus.write32(dst + i * 4, v);
                }
            }
            0x10 => {
                // BitUnPack: expand sub-byte-width data (used for 1bpp fonts).
                let src = self.regs[0];
                let dst = self.regs[1];
                let info = self.regs[2];
                let len = self.bus.read16(info) as u32;
                let src_w = self.bus.read8(info + 2) as u32;
                let dst_w = self.bus.read8(info + 3) as u32;
                let off_flags = self.bus.read32(info + 4);
                let data_off = off_flags & 0x7FFF_FFFF;
                let zero_flag = off_flags & 0x8000_0000 != 0;
                let mut out: u32 = 0;
                let mut out_bits = 0;
                let mut d = dst;
                for i in 0..len {
                    let b = self.bus.read8(src + i) as u32;
                    let mut bit = 0;
                    while bit < 8 {
                        let v = b >> bit & ((1 << src_w) - 1);
                        let v = if v != 0 || zero_flag { v + data_off } else { v };
                        out |= v << out_bits;
                        out_bits += dst_w;
                        if out_bits >= 32 {
                            self.bus.write32(d, out);
                            d += 4;
                            out = 0;
                            out_bits = 0;
                        }
                        bit += src_w;
                    }
                }
                if out_bits > 0 {
                    self.bus.write32(d, out);
                }
            }
            0x11 | 0x12 => {
                // LZ77UnCompWram / LZ77UnCompVram
                let mut src = self.regs[0];
                let dst = self.regs[1];
                let header = self.bus.read32(src);
                let size = header >> 8;
                src += 4;
                let mut written = 0u32;
                while written < size {
                    let flags = self.bus.read8(src);
                    src += 1;
                    for bit in (0..8).rev() {
                        if written >= size {
                            break;
                        }
                        if flags >> bit & 1 == 0 {
                            let b = self.bus.read8(src);
                            src += 1;
                            self.bus.write8_lenient(dst + written, b);
                            written += 1;
                        } else {
                            let b0 = self.bus.read8(src) as u32;
                            let b1 = self.bus.read8(src + 1) as u32;
                            src += 2;
                            let len = (b0 >> 4) + 3;
                            let disp = ((b0 & 0xF) << 8 | b1) + 1;
                            for _ in 0..len {
                                if written >= size {
                                    break;
                                }
                                let b = self.bus.read8(dst + written - disp);
                                self.bus.write8_lenient(dst + written, b);
                                written += 1;
                            }
                        }
                    }
                }
            }
            0x14 | 0x15 => {
                // RLUnCompWram / RLUnCompVram
                let mut src = self.regs[0];
                let dst = self.regs[1];
                let size = self.bus.read32(src) >> 8;
                src += 4;
                let mut written = 0u32;
                while written < size {
                    let flag = self.bus.read8(src);
                    src += 1;
                    if flag & 0x80 != 0 {
                        let len = (flag as u32 & 0x7F) + 3;
                        let b = self.bus.read8(src);
                        src += 1;
                        for _ in 0..len.min(size - written) {
                            self.bus.write8_lenient(dst + written, b);
                            written += 1;
                        }
                    } else {
                        let len = (flag as u32 & 0x7F) + 1;
                        for _ in 0..len.min(size - written) {
                            let b = self.bus.read8(src);
                            src += 1;
                            self.bus.write8_lenient(dst + written, b);
                            written += 1;
                        }
                    }
                }
            }
            _ => {} // unimplemented BIOS call: ignore
        }
    }

    // ===================== Thumb =====================

    fn exec_thumb(&mut self, op: u16) {
        let op = op as u32;
        match op >> 13 {
            0b000 => {
                if op >> 11 & 3 == 3 {
                    // ADD/SUB register or 3-bit immediate
                    let rd = op & 7;
                    let rs = op >> 3 & 7;
                    let v = if op & 0x400 != 0 { op >> 6 & 7 } else { self.r(op >> 6 & 7) };
                    let a = self.r(rs);
                    let r = if op & 0x200 != 0 {
                        self.sub_with_flags(a, v, 1, true)
                    } else {
                        self.add_with_flags(a, v, 0, true)
                    };
                    self.set_r(rd, r);
                } else {
                    // LSL/LSR/ASR by immediate
                    let rd = op & 7;
                    let rs = op >> 3 & 7;
                    let amount = op >> 6 & 0x1F;
                    let (r, c) = self.shift(op >> 11 & 3, self.r(rs), amount, true);
                    self.set_r(rd, r);
                    self.set_nz(r);
                    self.set_flag(C, c);
                }
            }
            0b001 => {
                // MOV/CMP/ADD/SUB 8-bit immediate
                let rd = op >> 8 & 7;
                let imm = op & 0xFF;
                let a = self.r(rd);
                match op >> 11 & 3 {
                    0 => {
                        self.set_r(rd, imm);
                        self.set_nz(imm);
                    }
                    1 => {
                        self.sub_with_flags(a, imm, 1, true);
                    }
                    2 => {
                        let r = self.add_with_flags(a, imm, 0, true);
                        self.set_r(rd, r);
                    }
                    _ => {
                        let r = self.sub_with_flags(a, imm, 1, true);
                        self.set_r(rd, r);
                    }
                }
            }
            0b010 => self.thumb_group_010(op),
            0b011 => {
                // LDR/STR with 5-bit immediate offset (word or byte)
                let rd = op & 7;
                let base = self.r(op >> 3 & 7);
                let imm = op >> 6 & 0x1F;
                let byte = op & 0x1000 != 0;
                let load = op & 0x0800 != 0;
                let addr = base.wrapping_add(if byte { imm } else { imm << 2 });
                match (load, byte) {
                    (false, false) => self.bus.write32(addr, self.r(rd)),
                    (false, true) => self.bus.write8(addr, self.r(rd) as u8),
                    (true, false) => {
                        let v = self.bus.read32(addr).rotate_right((addr & 3) * 8);
                        self.set_r(rd, v);
                    }
                    (true, true) => {
                        let v = self.bus.read8(addr) as u32;
                        self.set_r(rd, v);
                    }
                }
            }
            0b100 => {
                if op & 0x1000 == 0 {
                    // LDRH/STRH immediate
                    let rd = op & 7;
                    let addr = self.r(op >> 3 & 7).wrapping_add((op >> 6 & 0x1F) << 1);
                    if op & 0x0800 != 0 {
                        let v = (self.bus.read16(addr) as u32).rotate_right((addr & 1) * 8);
                        self.set_r(rd, v);
                    } else {
                        self.bus.write16(addr, self.r(rd) as u16);
                    }
                } else {
                    // LDR/STR sp-relative
                    let rd = op >> 8 & 7;
                    let addr = self.r(13).wrapping_add((op & 0xFF) << 2);
                    if op & 0x0800 != 0 {
                        let v = self.bus.read32(addr).rotate_right((addr & 3) * 8);
                        self.set_r(rd, v);
                    } else {
                        self.bus.write32(addr, self.r(rd));
                    }
                }
            }
            0b101 => {
                if op & 0x1000 == 0 {
                    // ADD rd, pc/sp, imm
                    let rd = op >> 8 & 7;
                    let imm = (op & 0xFF) << 2;
                    let base = if op & 0x0800 != 0 { self.r(13) } else { self.r(15) & !3 };
                    self.set_r(rd, base.wrapping_add(imm));
                } else if op & 0x0F00 == 0 {
                    // ADD SP, +/-imm
                    let imm = (op & 0x7F) << 2;
                    let sp = self.r(13);
                    self.set_r(13, if op & 0x80 != 0 { sp.wrapping_sub(imm) } else { sp.wrapping_add(imm) });
                } else if op & 0x0600 == 0x0400 {
                    // PUSH/POP
                    let load = op & 0x0800 != 0;
                    let r_bit = op & 0x0100 != 0;
                    let list = op & 0xFF;
                    let n = list.count_ones() + r_bit as u32;
                    if load {
                        let mut addr = self.r(13);
                        for i in 0..8 {
                            if list & (1 << i) != 0 {
                                self.regs[i as usize] = self.bus.read32(addr);
                                addr = addr.wrapping_add(4);
                            }
                        }
                        if r_bit {
                            self.regs[15] = self.bus.read32(addr) & !1;
                            addr = addr.wrapping_add(4);
                        }
                        self.set_r(13, addr);
                    } else {
                        let base = self.r(13).wrapping_sub(n * 4);
                        let mut addr = base;
                        for i in 0..8 {
                            if list & (1 << i) != 0 {
                                let v = self.regs[i as usize];
                                self.bus.write32(addr, v);
                                addr = addr.wrapping_add(4);
                            }
                        }
                        if r_bit {
                            self.bus.write32(addr, self.regs[14]);
                        }
                        self.set_r(13, base);
                    }
                } else {
                    panic!("unimplemented Thumb op {op:#06X} at {:#010X}", self.regs[15]);
                }
            }
            0b110 => {
                if op & 0x1000 == 0 {
                    // LDMIA/STMIA
                    let rb = op >> 8 & 7;
                    let list = op & 0xFF;
                    let mut addr = self.r(rb);
                    let load = op & 0x0800 != 0;
                    if list == 0 {
                        // Empty list quirk: r15, base += 0x40
                        if load {
                            self.regs[15] = self.bus.read32(addr) & !1;
                        } else {
                            self.bus.write32(addr, self.r(15).wrapping_add(2));
                        }
                        self.set_r(rb, addr.wrapping_add(0x40));
                        return;
                    }
                    let first = list.trailing_zeros();
                    let new_base = addr.wrapping_add(list.count_ones() * 4);
                    for i in 0..8 {
                        if list & (1 << i) != 0 {
                            if load {
                                self.regs[i as usize] = self.bus.read32(addr);
                            } else {
                                let v = if i == rb && i != first {
                                    new_base
                                } else {
                                    self.regs[i as usize]
                                };
                                self.bus.write32(addr, v);
                            }
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if !(load && list & (1 << rb) != 0) {
                        self.set_r(rb, addr);
                    }
                } else {
                    let cond = op >> 8 & 0xF;
                    if cond == 0xF {
                        self.hle_swi(op & 0xFF);
                    } else if self.cond(cond) {
                        // Conditional branch
                        let off = ((op & 0xFF) as i8 as i32) << 1;
                        self.regs[15] = self.r(15).wrapping_add(off as u32);
                    }
                }
            }
            _ => {
                if op & 0x1800 == 0x0000 {
                    // B unconditional
                    let off = ((op << 21) as i32 >> 20) as u32;
                    self.regs[15] = self.r(15).wrapping_add(off);
                } else if op & 0x1800 == 0x1000 {
                    // BL first half: LR = PC + offset<<12
                    let off = ((op << 21) as i32 >> 9) as u32;
                    self.regs[14] = self.r(15).wrapping_add(off);
                } else if op & 0x1800 == 0x1800 {
                    // BL second half
                    let lr = self.regs[14].wrapping_add((op & 0x7FF) << 1);
                    self.regs[14] = self.regs[15].wrapping_add(2) | 1;
                    self.regs[15] = lr & !1;
                } else {
                    panic!("unimplemented Thumb op {op:#06X} at {:#010X}", self.regs[15]);
                }
            }
        }
    }

    fn thumb_group_010(&mut self, op: u32) {
        if op & 0x1C00 == 0x0000 {
            // ALU operations
            let rd = op & 7;
            let rs = op >> 3 & 7;
            let a = self.r(rd);
            let b = self.r(rs);
            let c = self.flag(C) as u32;
            match op >> 6 & 0xF {
                0x0 => { let r = a & b; self.set_r(rd, r); self.set_nz(r); }
                0x1 => { let r = a ^ b; self.set_r(rd, r); self.set_nz(r); }
                0x2 => {
                    let (r, cy) = self.shift(0, a, b & 0xFF, false);
                    self.set_r(rd, r); self.set_nz(r); self.set_flag(C, cy);
                }
                0x3 => {
                    let (r, cy) = self.shift(1, a, b & 0xFF, false);
                    self.set_r(rd, r); self.set_nz(r); self.set_flag(C, cy);
                }
                0x4 => {
                    let (r, cy) = self.shift(2, a, b & 0xFF, false);
                    self.set_r(rd, r); self.set_nz(r); self.set_flag(C, cy);
                }
                0x5 => { let r = self.add_with_flags(a, b, c, true); self.set_r(rd, r); }
                0x6 => { let r = self.sub_with_flags(a, b, c, true); self.set_r(rd, r); }
                0x7 => {
                    let (r, cy) = self.shift(3, a, b & 0xFF, false);
                    self.set_r(rd, r); self.set_nz(r); self.set_flag(C, cy);
                }
                0x8 => { let r = a & b; self.set_nz(r); }
                0x9 => { let r = self.sub_with_flags(0, b, 1, true); self.set_r(rd, r); }
                0xA => { self.sub_with_flags(a, b, 1, true); }
                0xB => { self.add_with_flags(a, b, 0, true); }
                0xC => { let r = a | b; self.set_r(rd, r); self.set_nz(r); }
                0xD => { let r = a.wrapping_mul(b); self.set_r(rd, r); self.set_nz(r); }
                0xE => { let r = a & !b; self.set_r(rd, r); self.set_nz(r); }
                _ => { let r = !b; self.set_r(rd, r); self.set_nz(r); }
            }
        } else if op & 0x1C00 == 0x0400 {
            // Hi register ops / BX
            let rd = (op & 7) | (op >> 4 & 8);
            let rs = op >> 3 & 0xF;
            match op >> 8 & 3 {
                0 => {
                    let r = self.r(rd).wrapping_add(self.r(rs));
                    if rd == 15 {
                        self.regs[15] = r & !1;
                    } else {
                        self.set_r(rd, r);
                    }
                }
                1 => {
                    let a = self.r(rd);
                    let b = self.r(rs);
                    self.sub_with_flags(a, b, 1, true);
                }
                2 => {
                    let v = self.r(rs);
                    if rd == 15 {
                        self.regs[15] = v & !1;
                    } else {
                        self.set_r(rd, v);
                    }
                }
                _ => {
                    // BX
                    let v = self.r(rs);
                    self.set_flag(T, v & 1 != 0);
                    self.regs[15] = v & !1;
                    if v & 1 == 0 {
                        self.regs[15] &= !3;
                    }
                }
            }
        } else if op & 0x1800 == 0x0800 {
            // PC-relative load
            let rd = op >> 8 & 7;
            let addr = (self.r(15) & !3).wrapping_add((op & 0xFF) << 2);
            let v = self.bus.read32(addr);
            self.set_r(rd, v);
        } else {
            // Load/store with register offset (word/byte/half/signed)
            let rd = op & 7;
            let addr = self.r(op >> 3 & 7).wrapping_add(self.r(op >> 6 & 7));
            if op & 0x0200 != 0 {
                // sign-extended group
                match op >> 10 & 3 {
                    0 => self.bus.write16(addr, self.r(rd) as u16), // STRH
                    1 => { let v = self.bus.read8(addr) as i8 as i32 as u32; self.set_r(rd, v); }
                    2 => {
                        let v = (self.bus.read16(addr) as u32).rotate_right((addr & 1) * 8);
                        self.set_r(rd, v);
                    }
                    _ => {
                        let v = if addr & 1 != 0 {
                            self.bus.read8(addr) as i8 as i32 as u32
                        } else {
                            self.bus.read16(addr) as i16 as i32 as u32
                        };
                        self.set_r(rd, v);
                    }
                }
            } else {
                match op >> 10 & 3 {
                    0 => self.bus.write32(addr, self.r(rd)),
                    1 => self.bus.write8(addr, self.r(rd) as u8),
                    2 => {
                        let v = self.bus.read32(addr).rotate_right((addr & 3) * 8);
                        self.set_r(rd, v);
                    }
                    _ => { let v = self.bus.read8(addr) as u32; self.set_r(rd, v); }
                }
            }
        }
    }
}
