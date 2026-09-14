//! VFPv2/v3 floating-point unit for the ARM interpreter.
//!
//! Implements the single- and double-precision encodings Android-era native
//! libraries emit: three-register arithmetic (VMLA/VMLS/VNMLS/VNMLA/VMUL/
//! VNMUL/VADD/VSUB/VDIV), two-register operations (VMOV/VABS/VNEG/VSQRT/
//! VCMP/VCVT), core-register transfers (VMOV core<->single, VMRS/VMSR),
//! VMOV immediates, and the coprocessor load/store family (VLDR/VSTR/VLDM/
//! VSTM, including VPUSH/VPOP and the two-core-register VMOV forms).
//!
//! Register layout follows the classic VFP encoding: 32 single-precision
//! registers s0..s31 where each 5-bit field is `(nibble << 1) | bit`, and 16
//! double registers d0..d15 where `dN` aliases `(s2N, s2N+1)`. FPSCR keeps
//! the comparison N/Z/C/V flags and the rounding-mode field used by VCVT.

use crate::mem::Memory;

/// FPSCR bit groups used here: comparison flags and rounding mode.
pub const FPSCR_NZCV_MASK: u32 = 0xF000_0000;
pub const FPSCR_RMODE_SHIFT: u32 = 22;
pub const FPSCR_RMODE_MASK: u32 = 3 << FPSCR_RMODE_SHIFT;

/// Side effects a VFP instruction requests from the interpreter loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfpEffect {
    /// Nothing outside the VFP unit.
    None,
    /// Write `value` into core register `register` (VMRS, base updates).
    CoreWrite { register: u32, value: u32 },
    /// Write two core registers (VMOV Rt, Rt2, Dm).
    CoreWritePair {
        register1: u32,
        value1: u32,
        register2: u32,
        value2: u32,
    },
    /// Copy comparison flags into APSR (VMRS APSR_nzcv).
    UpdateFlags { n: bool, z: bool, c: bool, v: bool },
}

/// The floating-point register file plus FPSCR.
#[derive(Debug, Clone, Default)]
pub struct VfpUnit {
    /// Raw 32-bit halves: `s[i]`, and `d[n]` = `(s[2n], s[2n+1])`.
    pub s: [u32; 32],
    pub fpscr: u32,
}

impl VfpUnit {
    pub fn new() -> Self {
        Self::default()
    }

    fn s_f32(&self, index: usize) -> f32 {
        f32::from_bits(self.s[index])
    }

    fn set_s_f32(&mut self, index: usize, value: f32) {
        self.s[index] = value.to_bits();
    }

    fn d_f64(&self, index: usize) -> f64 {
        let bits = ((self.s[2 * index + 1] as u64) << 32) | self.s[2 * index] as u64;
        f64::from_bits(bits)
    }

    fn set_d_f64(&mut self, index: usize, value: f64) {
        let bits = value.to_bits();
        self.s[2 * index] = bits as u32;
        self.s[2 * index + 1] = (bits >> 32) as u32;
    }

    /// FPSCR rounding mode: 0=RN, 1=RP, 2=RM, 3=RZ.
    fn rounding(&self) -> u32 {
        (self.fpscr & FPSCR_RMODE_MASK) >> FPSCR_RMODE_SHIFT
    }

    /// Decodes the 8-bit VMOV immediate (`a:~bcd:efgh`) for the given
    /// mantissa width (23 for F32, 52 for F64).
    fn immediate(mantissa_bits: u32, imm8: u32) -> u64 {
        let sign = ((imm8 >> 7) & 1) as u64;
        let bcd = (imm8 >> 4) & 7;
        let efgh = (imm8 & 0xF) as u64;
        let delta = ((bcd << 29) as i32) >> 29; // sign-extend 3 bits
        let (exponent, fraction) = if mantissa_bits == 23 {
            ((128i64 + delta as i64) as u64, efgh << 19)
        } else {
            ((1024i64 + delta as i64) as u64, efgh << 48)
        };
        let sign_shift = if mantissa_bits == 23 { 31 } else { 63 };
        (sign << sign_shift) | (exponent << mantissa_bits) | fraction
    }

    fn set_compare_flags(&mut self, value: f64) {
        let flags: u32 = if value.is_nan() {
            0b0001 // unordered: V
        } else if value == 0.0 {
            0b0110 // equal: Z, C
        } else if value < 0.0 {
            0b1000 // less than: N
        } else {
            0b0010 // greater than: C
        };
        self.fpscr = (self.fpscr & !FPSCR_NZCV_MASK) | (flags << 28);
    }

    /// Converts a floating-point value to a 32-bit integer honouring the
    /// FPSCR rounding mode, saturating out-of-range results like the
    /// hardware's default (non-trapping) behaviour.
    fn convert_to_int(&self, value: f64, unsigned: bool, round_to_nearest: bool) -> u32 {
        if value.is_nan() {
            return 0;
        }
        let rounded = if round_to_nearest {
            value.round_ties_even()
        } else {
            match self.rounding() {
                1 => value.ceil(),  // RP
                2 => value.floor(), // RM
                _ => value.trunc(), // RZ
            }
        };
        if unsigned {
            rounded.clamp(0.0, 4_294_967_295.0) as u32
        } else {
            rounded.clamp(i32::MIN as f64, i32::MAX as f64) as i32 as u32
        }
    }

    /// Decodes and executes a VFP coprocessor load/store instruction
    /// (`bits[27:25] == 0b110`, cp 10/11): VMOV 2-core-reg, VLDR/VSTR, and
    /// VLDM/VSTM (VPUSH/VPOP). Returns the optional base-register writeback.
    pub fn execute_load_store(
        &mut self,
        insn: u32,
        memory: &mut Memory,
        base: u32,
        rt_value: u32,
        rt2_value: u32,
    ) -> Result<VfpEffect, String> {
        let cp = (insn >> 8) & 0xF;
        if cp != 10 && cp != 11 {
            return Err(format!(
                "coprocessor load/store for cp{cp} is not supported ({insn:#010x})"
            ));
        }
        let double = cp == 11;
        let p = insn & 0x0100_0000 != 0;
        let u = insn & 0x0080_0000 != 0;
        let w = insn & 0x0020_0000 != 0;
        let l = insn & 0x0010_0000 != 0;
        let d_bit = (insn >> 22) & 1;
        let vd_field = (insn >> 12) & 0xF;
        let vn_field = (insn >> 16) & 0xF;
        let vm_field = insn & 0xF;
        let m_bit = (insn >> 5) & 1;

        // VMOV between two core registers and a VFP register:
        // cond 1100 010L Rt2 Rt 101x 00M1 Vm. No memory access: Rn and Rt2
        // ARE the core registers.
        if !p && !u && !w && insn & 0x70 == 0x10 {
            let register = if double {
                ((m_bit << 4) | vm_field) as usize
            } else {
                ((vm_field << 1) | m_bit) as usize
            };
            if l {
                // VMOV Rt, Rt2, Dm: VFP register into the core pair.
                let low = if double {
                    self.s[2 * register]
                } else {
                    self.s[register]
                };
                let high = if double {
                    self.s[2 * register + 1]
                } else {
                    self.s[register + 1]
                };
                return Ok(VfpEffect::CoreWritePair {
                    register1: vn_field,
                    value1: low,
                    register2: vd_field,
                    value2: high,
                });
            }
            // VMOV Dm, Rt, Rt2: core pair into the VFP register.
            if double {
                self.s[2 * register] = rt_value;
                self.s[2 * register + 1] = rt2_value;
            } else {
                self.s[register] = rt_value;
                self.s[register + 1] = rt2_value;
            }
            return Ok(VfpEffect::None);
        }

        let count_singles = vm_field;
        let first_single: u32 = if double {
            ((d_bit << 4) | vd_field) * 2
        } else {
            (vd_field << 1) | d_bit
        };

        if !p && u {
            // VLDM/VSTM (IA): transfer `count` registers starting at base.
            let bytes = count_singles * 4;
            self.transfer_block(memory, base, first_single, count_singles, double, l)?;
            return Ok(if w {
                VfpEffect::CoreWrite {
                    register: vn_field,
                    value: base.wrapping_add(bytes),
                }
            } else {
                VfpEffect::None
            });
        }

        if p && u {
            return Err(format!(
                "VFP load/store FA addressing is invalid ({insn:#010x})"
            ));
        }

        let offset = ((insn >> 4) & 0xF0) | (insn & 0xF);
        let delta = if u { offset } else { 0u32.wrapping_sub(offset) };

        if w {
            // VLDM/VSTM (DB): decrement before, always writeback.
            let bytes = count_singles * 4;
            let start = base.wrapping_sub(bytes);
            self.transfer_block(memory, start, first_single, count_singles, double, l)?;
            return Ok(VfpEffect::CoreWrite {
                register: vn_field,
                value: start,
            });
        }

        // VLDR/VSTR: single register at base +/- offset.
        let address = base.wrapping_add(delta);
        if double {
            let low = first_single as usize;
            let high = low + 1;
            if l {
                self.s[low] = memory.read_u32(address).map_err(|e| e.to_string())?;
                self.s[high] = memory.read_u32(address + 4).map_err(|e| e.to_string())?;
            } else {
                memory
                    .write_u32(address, self.s[low])
                    .map_err(|e| e.to_string())?;
                memory
                    .write_u32(address + 4, self.s[high])
                    .map_err(|e| e.to_string())?;
            }
        } else if l {
            self.s[first_single as usize] = memory.read_u32(address).map_err(|e| e.to_string())?;
        } else {
            memory
                .write_u32(address, self.s[first_single as usize])
                .map_err(|e| e.to_string())?;
        }
        Ok(if w {
            VfpEffect::CoreWrite {
                register: vn_field,
                value: address,
            }
        } else {
            VfpEffect::None
        })
    }

    fn transfer_block(
        &mut self,
        memory: &mut Memory,
        start: u32,
        first_single: u32,
        count_singles: u32,
        double: bool,
        load: bool,
    ) -> Result<(), String> {
        if double && !count_singles.is_multiple_of(2) {
            return Err("VLDM/VSTM double register list must contain an even count".to_owned());
        }
        let mut address = start;
        for index in 0..count_singles {
            let register = (first_single + index) as usize;
            if load {
                self.s[register] = memory.read_u32(address).map_err(|e| e.to_string())?;
            } else {
                memory
                    .write_u32(address, self.s[register])
                    .map_err(|e| e.to_string())?;
            }
            address = address.wrapping_add(4);
        }
        Ok(())
    }

    /// Decodes and executes a VFP instruction in the coprocessor
    /// data-processing space (`bits[27:24] == 0b1110`, cp 10/11), plus the
    /// MCR/MRC register-transfer forms (VMOV core<->single, VMRS/VMSR).
    pub fn execute(&mut self, insn: u32, rt_value: u32) -> Result<VfpEffect, String> {
        let cp = (insn >> 8) & 0xF;
        if cp != 10 && cp != 11 {
            return Err(format!(
                "coprocessor data processing for cp{cp} is not supported ({insn:#010x})"
            ));
        }
        let double = cp == 11;
        let op = (insn >> 20) & 0xF;
        let vn_field = (insn >> 16) & 0xF;
        let vd_field = (insn >> 12) & 0xF;
        let vm_field = insn & 0xF;
        let d_bit = (insn >> 22) & 1;
        let n_bit = (insn >> 7) & 1;
        let m_bit = (insn >> 5) & 1;
        let bit4 = insn & 0x10 != 0;

        if bit4 {
            // MCR/MRC register transfers.
            if insn & 0xFEE0_0AFF == 0x0EE0_0A10 {
                // VMRS/VMSR: cond 1110 1110 1111 L Rt 1010 0001 0000.
                let load = insn & 0x0010_0000 != 0;
                let rt = vd_field;
                if load {
                    if rt == 15 {
                        let flags = self.fpscr & FPSCR_NZCV_MASK;
                        return Ok(VfpEffect::UpdateFlags {
                            n: flags & 0x8000_0000 != 0,
                            z: flags & 0x4000_0000 != 0,
                            c: flags & 0x2000_0000 != 0,
                            v: flags & 0x1000_0000 != 0,
                        });
                    }
                    return Ok(VfpEffect::CoreWrite {
                        register: rt,
                        value: self.fpscr,
                    });
                }
                self.fpscr = rt_value;
                return Ok(VfpEffect::None);
            }
            if insn & 0xFEE0_0A70 == 0x0E00_0A10 {
                // VMOV between core register and single-precision register:
                // cond 1110 1110 000L Rt Vd 1010 001N0 Vm.
                let index = ((vd_field << 1) | n_bit) as usize;
                let load = insn & 0x0010_0000 != 0;
                if load {
                    return Ok(VfpEffect::CoreWrite {
                        register: vn_field,
                        value: self.s[index],
                    });
                }
                self.s[index] = rt_value;
                return Ok(VfpEffect::None);
            }
            return Err(format!("unsupported VFP register transfer ({insn:#010x})"));
        }

        // VMOV immediate: cond 1110 1110 1D11 Vd 101x 0000 imm4.
        if op == 0xB && insn & 0xF0 == 0 {
            let imm8 = (vn_field << 4) | vm_field;
            let value = Self::immediate(if double { 52 } else { 23 }, imm8);
            if double {
                self.set_d_f64(((d_bit << 4) | vd_field) as usize, f64::from_bits(value));
            } else {
                self.set_s_f32(
                    ((vd_field << 1) | d_bit) as usize,
                    f32::from_bits(value as u32),
                );
            }
            return Ok(VfpEffect::None);
        }

        if op == 0xB && insn & 0x40 != 0 {
            // Two-register misc: opcode lives in Vn, bit 7 is the sub-op.
            return self.execute_two_register(
                insn, double, vn_field, n_bit, vd_field, d_bit, vm_field, m_bit,
            );
        }

        if op <= 0x8 {
            // Three-register arithmetic; bit 6 selects the negated variant.
            let negate = insn & 0x40 != 0;
            return self.execute_arithmetic(
                insn, double, op, negate, vd_field, d_bit, vn_field, n_bit, vm_field, m_bit,
            );
        }

        Err(format!(
            "unsupported VFP data-processing encoding {insn:#010x}"
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_arithmetic(
        &mut self,
        insn: u32,
        double: bool,
        op: u32,
        negate: bool,
        vd_field: u32,
        d_bit: u32,
        vn_field: u32,
        n_bit: u32,
        vm_field: u32,
        m_bit: u32,
    ) -> Result<VfpEffect, String> {
        if double {
            let vd = ((d_bit << 4) | vd_field) as usize;
            let vn = ((n_bit << 4) | vn_field) as usize;
            let vm = ((m_bit << 4) | vm_field) as usize;
            let n = self.d_f64(vn);
            let m = self.d_f64(vm);
            let d = self.d_f64(vd);
            let result = match (op, negate) {
                (0x0, false) => d + n * m,   // VMLA
                (0x0, true) => d - n * m,    // VMLS
                (0x1, false) => n * m - d,   // VNMLS
                (0x1, true) => -(n * m + d), // VNMLA
                (0x2, false) => n * m,       // VMUL
                (0x2, true) => -(n * m),     // VNMUL
                (0x3, false) => n + m,       // VADD
                (0x3, true) => n - m,        // VSUB
                (0x8, false) => n / m,       // VDIV
                _ => {
                    return Err(format!(
                        "unsupported VFP arithmetic op {op:04b} ({insn:#010x})"
                    ))
                }
            };
            self.set_d_f64(vd, result);
        } else {
            let vd = ((vd_field << 1) | d_bit) as usize;
            let vn = ((vn_field << 1) | n_bit) as usize;
            let vm = ((vm_field << 1) | m_bit) as usize;
            let n = self.s_f32(vn);
            let m = self.s_f32(vm);
            let d = self.s_f32(vd);
            let result = match (op, negate) {
                (0x0, false) => d + n * m,
                (0x0, true) => d - n * m,
                (0x1, false) => n * m - d,
                (0x1, true) => -(n * m + d),
                (0x2, false) => n * m,
                (0x2, true) => -(n * m),
                (0x3, false) => n + m,
                (0x3, true) => n - m,
                (0x8, false) => n / m,
                _ => {
                    return Err(format!(
                        "unsupported VFP arithmetic op {op:04b} ({insn:#010x})"
                    ))
                }
            };
            self.set_s_f32(vd, result);
        }
        Ok(VfpEffect::None)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_two_register(
        &mut self,
        insn: u32,
        double: bool,
        opcode: u32,
        sub_op: u32,
        vd_field: u32,
        d_bit: u32,
        vm_field: u32,
        m_bit: u32,
    ) -> Result<VfpEffect, String> {
        let vd = if double {
            ((d_bit << 4) | vd_field) as usize
        } else {
            ((vd_field << 1) | d_bit) as usize
        };
        // VCVT keeps its source in the single-precision field even when the
        // destination is a double; other operations follow the precision.
        let vm_single = ((vm_field << 1) | m_bit) as usize;
        let vm_reg = if double {
            ((m_bit << 4) | vm_field) as usize
        } else {
            vm_single
        };
        let round_to_nearest = sub_op == 0 && (opcode == 0xC || opcode == 0xD);

        match (opcode, sub_op) {
            (0x0, 0) => {
                // VMOV register-to-register.
                if double {
                    let value = self.d_f64(vm_reg);
                    self.set_d_f64(vd, value);
                } else {
                    let value = self.s_f32(vm_reg);
                    self.set_s_f32(vd, value);
                }
            }
            (0x1, 0) => {
                // VNEG.
                if double {
                    self.set_d_f64(vd, -self.d_f64(vm_reg));
                } else {
                    self.set_s_f32(vd, -self.s_f32(vm_reg));
                }
            }
            (0x1, 1) => {
                // VSQRT.
                if double {
                    self.set_d_f64(vd, self.d_f64(vm_reg).sqrt());
                } else {
                    self.set_s_f32(vd, self.s_f32(vm_reg).sqrt());
                }
            }
            (0x0, 1) => {
                // VABS (sub-op 1).
                if double {
                    self.set_d_f64(vd, self.d_f64(vm_reg).abs());
                } else {
                    self.set_s_f32(vd, self.s_f32(vm_reg).abs());
                }
            }
            (0x4 | 0x5, sub) => {
                // VCMP / VCMPE (bit 7 = E; trap-disabled here, same result).
                // Opcode 5 compares against zero.
                let _ = sub;
                let operand = if opcode == 5 {
                    0.0
                } else if double {
                    self.d_f64(vm_reg)
                } else {
                    self.s_f32(vm_reg) as f64
                };
                self.set_compare_flags(operand);
            }
            (0x7, 1) => {
                // VCVT between single and double.
                if double {
                    // VCVT.F32.F64: single destination, double source.
                    self.set_s_f32(vd, self.d_f64(vm_reg) as f32);
                } else {
                    // VCVT.F64.F32: double destination, single source.
                    self.set_d_f64(vd, f64::from(self.s_f32(vm_reg)));
                }
            }
            (0x8, sub) => {
                // VCVT float <- integer (sub 1 = signed, 0 = unsigned).
                let raw = self.s[vm_single];
                let value = match sub {
                    1 => i32::from_ne_bytes(raw.to_ne_bytes()) as f64,
                    _ => raw as f64,
                };
                if double {
                    self.set_d_f64(vd, value);
                } else {
                    self.set_s_f32(vd, value as f32);
                }
            }
            (0xC | 0xD, _) => {
                // VCVT integer <- float (C = unsigned, D = signed). The
                // integer destination is always a single-precision register.
                let unsigned = opcode == 0xC;
                let value = if double {
                    self.d_f64(vm_reg)
                } else {
                    f64::from(self.s_f32(vm_reg))
                };
                let vd_single = ((vd_field << 1) | d_bit) as usize;
                self.s[vd_single] = self.convert_to_int(value, unsigned, round_to_nearest);
            }
            _ => {
                return Err(format!(
                    "unsupported VFP two-register operation {opcode:04b}/{sub_op} ({insn:#010x})"
                ))
            }
        }
        Ok(VfpEffect::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_encoding() {
        // Values verified against GNU as output.
        assert_eq!(VfpUnit::immediate(23, 0x70), 0x3F80_0000); // 1.0f32
        assert_eq!(VfpUnit::immediate(23, 0x00), 0x4000_0000); // 2.0f32
        assert_eq!(VfpUnit::immediate(23, 0x78), 0x3FC0_0000); // 1.5f32
        assert_eq!(VfpUnit::immediate(23, 0xF0), 0xBF80_0000); // -1.0f32
        assert_eq!(VfpUnit::immediate(52, 0x00), 0x4000_0000_0000_0000); // 2.0f64
        assert_eq!(VfpUnit::immediate(52, 0x70), 0x3FF0_0000_0000_0000); // 1.0f64
        assert_eq!(VfpUnit::immediate(52, 0x08), 0x4008_0000_0000_0000); // 3.0f64
    }
}
