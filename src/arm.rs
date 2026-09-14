//! ARMv5TE + Thumb-1 interpreter for loaded native libraries.
//!
//! Scope follows the measured instruction mix of the compatibility targets
//! (`docs/ovenbreak.md`): full ARMv4T plus the v5TE additions in use (BLX, CLZ,
//! DSP multiplies, saturation, LDRD/STRD), and full Thumb-1. Anything outside
//! that set (VFP, coprocessors, v6 atomics, Thumb-2) stops the machine with a
//! visible diagnostic instead of executing something wrong.
//!
//! PC convention: `r[15]` holds the address of the instruction being executed;
//! register reads of R15 yield `address + 8` in ARM state and `+ 4` in Thumb
//! state, as the architecture requires.

use anyhow::Result;

use crate::elfload::Linker;
use crate::mem::Memory;

/// Sentinel link-register value: branching to it returns from the current
/// emulated call.
pub const RETURN_SENTINEL: u32 = 0xFFFF_FFFE;

/// How many recent PCs the fault diagnostic keeps.
const TRACE_LEN: usize = 64;

// Data-processing opcodes (bits[24:21]).
const DP_AND: u8 = 0;
const DP_EOR: u8 = 1;
const DP_SUB: u8 = 2;
const DP_RSB: u8 = 3;
const DP_ADD: u8 = 4;
const DP_ADC: u8 = 5;
const DP_SBC: u8 = 6;
const DP_RSC: u8 = 7;
const DP_TST: u8 = 8;
const DP_TEQ: u8 = 9;
const DP_CMP: u8 = 10;
const DP_CMN: u8 = 11;
const DP_ORR: u8 = 12;
const DP_MOV: u8 = 13;
const DP_BIC: u8 = 14;
const DP_MVN: u8 = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Returned,
    StopRequested,
    Exited(i32),
    Aborted,
    Error(String),
}

#[derive(Debug, Clone)]
struct SavedState {
    registers: [u32; 16],
    flags: Flags,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Flags {
    n: bool,
    z: bool,
    c: bool,
    v: bool,
    q: bool,
    thumb: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CpuConfig {
    pub max_steps: u64,
    pub max_call_depth: usize,
}

impl Default for CpuConfig {
    fn default() -> Self {
        Self {
            max_steps: 200_000_000,
            max_call_depth: 64,
        }
    }
}

#[derive(Debug)]
pub struct Cpu {
    pub r: [u32; 16],
    pub(crate) flags: Flags,
    /// VFP floating-point unit state (s0-s31 + FPSCR).
    pub vfp: crate::vfp::VfpUnit,
    /// TLS pointer installed through bionic's `__ARM_NR_set_tls`.
    pub tls: u32,
    steps: u64,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            r: [0; 16],
            flags: Flags {
                n: false,
                z: false,
                c: false,
                v: false,
                q: false,
                thumb: false,
            },
            vfp: crate::vfp::VfpUnit::new(),
            tls: 0,
            steps: 0,
        }
    }

    pub fn thumb(&self) -> bool {
        self.flags.thumb
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    fn snapshot(&self) -> SavedState {
        SavedState {
            registers: self.r,
            flags: self.flags,
        }
    }

    fn restore(&mut self, state: &SavedState) {
        self.r = state.registers;
        self.flags = state.flags;
    }

    fn psr(&self) -> u32 {
        let mut value = 0x1F; // system mode
        if self.flags.n {
            value |= 1 << 31;
        }
        if self.flags.z {
            value |= 1 << 30;
        }
        if self.flags.c {
            value |= 1 << 29;
        }
        if self.flags.v {
            value |= 1 << 28;
        }
        if self.flags.q {
            value |= 1 << 27;
        }
        if self.flags.thumb {
            value |= 1 << 5;
        }
        value
    }

    fn set_psr(&mut self, value: u32) {
        self.flags.n = value & (1 << 31) != 0;
        self.flags.z = value & (1 << 30) != 0;
        self.flags.c = value & (1 << 29) != 0;
        self.flags.v = value & (1 << 28) != 0;
        self.flags.q = value & (1 << 27) != 0;
        // Writes that clear the T bit would switch state; emulated libraries
        // never do this, so Thumb state is kept unless the bit is set.
        if value & (1 << 5) != 0 {
            self.flags.thumb = true;
        }
    }
}

pub trait HostBridge {
    /// Executes the host shim bound to `slot` and returns the value for r0.
    fn call_host(&mut self, machine: &mut Machine, slot: usize) -> u32;

    /// Records a diagnostic (missing import, log output, ...).
    fn diagnostic(&mut self, message: String);

    /// Executes an ARM EABI syscall (number in r7, arguments in r0-r6) and
    /// returns the r0 result. The default implementation covers the common
    /// Linux syscall surface; hosts may override for richer behavior.
    fn syscall(&mut self, machine: &mut Machine, number: u32) -> u32;
}

/// The emulated machine: CPU state, guest memory, and the linker state.
#[derive(Debug)]
pub struct Machine {
    pub cpu: Cpu,
    pub memory: Memory,
    pub linker: Linker,
    pub config: CpuConfig,
    /// Linux syscall bookkeeping (program break, mmap bump pointer).
    pub syscalls: crate::syscalls::SyscallState,
    stop: Option<StopReason>,
    saved: Vec<SavedState>,
    steps_at_call: Vec<u64>,
    /// Most recent program counters, for fault diagnostics.
    trace: std::collections::VecDeque<u32>,
}

impl Machine {
    pub fn new(config: CpuConfig) -> Self {
        Self {
            cpu: Cpu::new(),
            memory: Memory::new(),
            linker: Linker::new(),
            config,
            syscalls: crate::syscalls::SyscallState::default(),
            stop: None,
            saved: Vec::new(),
            steps_at_call: Vec::new(),
            trace: std::collections::VecDeque::with_capacity(TRACE_LEN),
        }
    }

    pub fn stop_reason(&self) -> Option<&StopReason> {
        self.stop.as_ref()
    }

    /// Calls an emulated function and runs until it returns. The caller's CPU
    /// state is preserved, so this is safe to use from inside host shims.
    pub fn call_function(
        &mut self,
        host: &mut dyn HostBridge,
        address: u32,
        args: &[u32],
    ) -> Result<u32, String> {
        if self.saved.len() >= self.config.max_call_depth {
            return Err(format!(
                "native call depth exceeds {} frames",
                self.config.max_call_depth
            ));
        }
        self.saved.push(self.cpu.snapshot());
        self.steps_at_call.push(self.cpu.steps);
        for (index, value) in args.iter().take(4).enumerate() {
            self.cpu.r[index] = *value;
        }
        self.cpu.flags.thumb = address & 1 == 1;
        self.cpu.r[15] = address & !1;
        self.cpu.r[14] = RETURN_SENTINEL;
        let result = self.run(host);
        let return_value = self.cpu.r[0];
        if let Some(state) = self.saved.pop() {
            self.cpu.restore(&state);
        }
        self.steps_at_call.pop();
        result.map(|()| return_value)
    }

    pub(crate) fn run(&mut self, host: &mut dyn HostBridge) -> Result<(), String> {
        let budget = self
            .steps_at_call
            .last()
            .copied()
            .unwrap_or(0)
            .saturating_add(self.config.max_steps);
        while self.stop.is_none() {
            if self.cpu.steps >= budget {
                let message = format!(
                    "native code exceeded {} steps at pc {:#010x}",
                    self.config.max_steps, self.cpu.r[15]
                );
                self.stop = Some(StopReason::Error(message.clone()));
                return Err(message);
            }
            self.step_once(host)?;
        }
        match self.stop.take() {
            Some(StopReason::Returned | StopReason::StopRequested) => Ok(()),
            Some(StopReason::Exited(code)) => {
                Err(format!("emulated process exited with code {code}"))
            }
            Some(StopReason::Aborted) => Err("emulated process aborted".to_owned()),
            Some(StopReason::Error(message)) => Err(message),
            None => Ok(()),
        }
    }

    pub fn request_stop(&mut self) {
        self.stop = Some(StopReason::StopRequested);
    }

    pub fn emulated_exit(&mut self, code: i32) {
        self.stop = Some(StopReason::Exited(code));
    }

    pub fn emulated_abort(&mut self) {
        self.stop = Some(StopReason::Aborted);
    }

    fn step_once(&mut self, host: &mut dyn HostBridge) -> Result<(), String> {
        let pc = self.cpu.r[15] & !1;
        if self.trace.len() == TRACE_LEN {
            self.trace.pop_front();
        }
        self.trace.push_back(pc);
        if pc == RETURN_SENTINEL {
            self.stop = Some(StopReason::Returned);
            return Ok(());
        }
        if let Some(slot) = self.linker.host_slot_for(pc) {
            let value = host.call_host(self, slot);
            self.cpu.r[0] = value;
            let return_address = self.cpu.r[14];
            self.cpu.flags.thumb = return_address & 1 == 1;
            self.cpu.r[15] = return_address & !1;
            self.cpu.steps += 1;
            return Ok(());
        }
        let result = if self.cpu.flags.thumb {
            match self.memory.fetch_u16(pc) {
                Ok(instruction) => self.execute_thumb(instruction, pc, host),
                Err(error) => Err(error.to_string()),
            }
        } else {
            match self.memory.fetch_u32(pc) {
                Ok(instruction) => self.execute_arm(instruction, pc, host),
                Err(error) => Err(error.to_string()),
            }
        };
        self.cpu.steps += 1;
        result.map_err(|message| {
            let message = self.error_at(pc, &message);
            self.stop = Some(StopReason::Error(message.clone()));
            message
        })
    }

    fn error_at(&self, pc: u32, message: &str) -> String {
        let trail = self
            .trace
            .iter()
            .rev()
            .take(std::env::var_os("DONUTHLE_TRACE").map_or(8, |_| 64))
            .rev()
            .map(|address| format!("{address:#010x}"))
            .collect::<Vec<_>>()
            .join(", ");
        // Data accesses that land in the host-slot range are the classic
        // "data symbol bound to a function slot" bug; name the culprit.
        let host_hint = if let Some(begin) = message.find("memory access 0x") {
            let digits = message[begin + 16..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect::<String>();
            u32::from_str_radix(&digits, 16)
                .ok()
                .and_then(|address| self.linker.host_slot_for(address))
                .and_then(|slot| self.linker.host_name(slot))
                .map(|name| format!(" (host slot '{name}')"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        // Stack dump: the top 12 words at r13, so return-address corruption
        // (POP {pc} jumping to 0) is visible in the message itself.
        let stack_dump = (0..12)
            .filter_map(|index| self.memory.read_u32(self.cpu.r[13] + index as u32 * 4).ok())
            .map(|value| format!("{value:08x}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "native CPU fault at pc {pc:#010x} (thumb={}): {message}{host_hint}; r0={:08x} r1={:08x} r2={:08x} r3={:08x} r6={:08x} r7={:08x}; stack@sp: {stack_dump}; recent pc: {trail}",
            self.cpu.flags.thumb,
            self.cpu.r[0],
            self.cpu.r[1],
            self.cpu.r[2],
            self.cpu.r[3],
            self.cpu.r[6],
            self.cpu.r[7],
        )
    }

    /// Architectural read of R15.
    fn read_pc(&self) -> u32 {
        self.cpu.r[15] + if self.cpu.flags.thumb { 4 } else { 8 }
    }

    fn register_value(&self, index: u32) -> u32 {
        if index == 15 {
            self.read_pc()
        } else {
            self.cpu.r[index as usize]
        }
    }

    fn jump_to(&mut self, address: u32) {
        self.cpu.flags.thumb = address & 1 == 1;
        self.cpu.r[15] = address & !1;
    }

    /// Sets the PC without interworking (used by non-interworking writes).
    fn set_pc_no_interwork(&mut self, address: u32) {
        self.cpu.r[15] = address & !1;
    }

    fn condition_holds(&self, code: u8) -> bool {
        let f = &self.cpu.flags;
        match code {
            0x0 => f.z,
            0x1 => !f.z,
            0x2 => f.c,
            0x3 => !f.c,
            0x4 => f.n,
            0x5 => !f.n,
            0x6 => f.v,
            0x7 => !f.v,
            0x8 => f.c && !f.z,
            0x9 => !f.c || f.z,
            0xA => f.n == f.v,
            0xB => f.n != f.v,
            0xC => !f.z && f.n == f.v,
            0xD => f.z || f.n != f.v,
            _ => true,
        }
    }

    fn set_logic_flags(&mut self, result: u32, carry: bool) {
        self.cpu.flags.n = result & 0x8000_0000 != 0;
        self.cpu.flags.z = result == 0;
        self.cpu.flags.c = carry;
    }

    fn set_nz(&mut self, result: u32) {
        self.cpu.flags.n = result & 0x8000_0000 != 0;
        self.cpu.flags.z = result == 0;
    }

    fn add_with_carry(a: u32, b: u32, carry_in: bool) -> (u32, bool, bool) {
        let wide = a as u64 + b as u64 + u32::from(carry_in) as u64;
        let result = wide as u32;
        let carry_out = wide > 0xFFFF_FFFF;
        let overflow = (!(a ^ b) & (a ^ result)) & 0x8000_0000 != 0;
        (result, carry_out, overflow)
    }

    fn sub_with_borrow(a: u32, b: u32, borrow_in: bool) -> (u32, bool, bool) {
        let borrow = u32::from(!borrow_in);
        let result = a.wrapping_sub(b).wrapping_sub(borrow);
        let carry_out = a as u64 >= b as u64 + borrow as u64;
        let overflow = ((a ^ b) & (a ^ result)) & 0x8000_0000 != 0;
        (result, carry_out, overflow)
    }

    /// Barrel shifter; returns (value, carry-out). `reg_form` selects the
    /// register-specified shift semantics (amount 0 = no shift).
    fn shifted(&self, value: u32, shift_type: u8, amount: u32, reg_form: bool) -> (u32, bool) {
        match shift_type {
            0 => match amount {
                0 => (value, self.cpu.flags.c),
                1..=31 => (value << amount, value & (1 << (32 - amount)) != 0),
                32 => (0, value & 1 != 0),
                _ => (0, false),
            },
            1 => {
                if reg_form && amount == 0 {
                    (0, value & 0x8000_0000 != 0)
                } else {
                    match amount {
                        1..=31 => (value >> amount, value & (1 << (amount - 1)) != 0),
                        _ => (0, value & 0x8000_0000 != 0),
                    }
                }
            }
            2 => {
                let sign = value & 0x8000_0000 != 0;
                if reg_form && amount == 0 {
                    (if sign { u32::MAX } else { 0 }, sign)
                } else {
                    match amount {
                        1..=31 => (
                            ((value as i32) >> amount) as u32,
                            value & (1 << (amount - 1)) != 0,
                        ),
                        _ => (if sign { u32::MAX } else { 0 }, sign),
                    }
                }
            }
            _ => {
                if !reg_form && amount == 0 {
                    // RRX
                    (
                        (value >> 1) | (u32::from(self.cpu.flags.c) << 31),
                        value & 1 != 0,
                    )
                } else {
                    match amount & 31 {
                        0 => (value, self.cpu.flags.c),
                        rotated => (
                            value.rotate_right(rotated),
                            value & (1 << (rotated - 1)) != 0,
                        ),
                    }
                }
            }
        }
    }

    fn operand2(&self, insn: u32) -> Result<(u32, bool), String> {
        if insn & 0x0200_0000 != 0 {
            let rotate = ((insn >> 8) & 0xF) * 2;
            let value = (insn & 0xFF).rotate_right(rotate);
            let carry = if rotate == 0 {
                self.cpu.flags.c
            } else {
                value & 0x8000_0000 != 0
            };
            Ok((value, carry))
        } else {
            let register = insn & 0xF;
            let shift_type = ((insn >> 5) & 3) as u8;
            let reg_form = insn & 0x10 != 0;
            let amount = if reg_form {
                self.register_value((insn >> 8) & 0xF) & 0xFF
            } else {
                (insn >> 7) & 0x1F
            };
            Ok(self.shifted(self.register_value(register), shift_type, amount, reg_form))
        }
    }

    fn execute_arm(
        &mut self,
        insn: u32,
        pc: u32,
        host: &mut dyn HostBridge,
    ) -> Result<(), String> {
        let condition = (insn >> 28) as u8;
        if condition == 0xF {
            if insn & 0xFE00_0000 == 0xFA00_0000 {
                // BLX (immediate): link, then branch to Thumb.
                let offset = ((insn << 8) as i32) >> 6; // sign-extended imm24 << 2
                let halfword_offset = ((insn >> 24) & 1) << 1;
                self.cpu.r[14] = pc + 4;
                self.cpu.flags.thumb = true;
                self.cpu.r[15] = (pc + 8).wrapping_add((offset as u32) | halfword_offset) & !3;
                return Ok(());
            }
            if insn & 0x0F70_0000 == 0x0F50_0000 {
                self.cpu.r[15] = pc + 4;
                return Ok(()); // PLD: prefetch hint
            }
            return Err(format!(
                "unconditional instruction {insn:#010x} is not supported"
            ));
        }
        if !self.condition_holds(condition) {
            self.cpu.r[15] = pc + 4;
            return Ok(());
        }
        match (insn >> 25) & 7 {
            0 | 1 => self.execute_data_processing(insn, pc),
            2 | 3 => self.execute_load_store(insn, pc),
            4 => self.execute_load_store_multiple(insn, pc),
            5 => {
                let offset = ((insn << 8) as i32) >> 6;
                if insn & 0x0100_0000 != 0 {
                    self.cpu.r[14] = pc + 4;
                }
                self.cpu.r[15] = (pc + 8).wrapping_add(offset as u32);
                Ok(())
            }
            6 => self.execute_coprocessor_load_store(insn, pc),
            _ => {
                if insn & 0x0F00_0000 == 0x0F00_0000 {
                    // ARM EABI: SWI/SVC with the syscall number in r7.
                    let number = self.register_value(7);
                    let result = host.syscall(self, number);
                    self.cpu.r[0] = result;
                    self.cpu.r[15] = pc + 4;
                    return Ok(());
                }
                if insn & 0x0F00_0000 == 0x0E00_0000 {
                    return self.execute_coprocessor_data_processing(insn, pc);
                }
                Err(format!(
                    "coprocessor instruction {insn:#010x} is not supported"
                ))
            }
        }
    }

    /// Coprocessor load/store (bits[27:25] = 110): VLDR/VSTR/VLDM/VSTM and
    /// the two-core-register VMOV forms.
    fn execute_coprocessor_load_store(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        let register_n = (insn >> 16) & 0xF;
        let cp = (insn >> 8) & 0xF;
        if cp != 10 && cp != 11 {
            return Err(format!(
                "coprocessor load/store for cp{cp} is not supported ({insn:#010x})"
            ));
        }
        let base = self.register_value(register_n);
        let rt_value = self.register_value(register_n);
        let rt2_value = self.register_value((insn >> 12) & 0xF);
        let effect = self
            .cpu
            .vfp
            .execute_load_store(insn, &mut self.memory, base, rt_value, rt2_value)?;
        match effect {
            crate::vfp::VfpEffect::CoreWrite { register, value } if register != 15 => {
                if register != 15 {
                    self.cpu.r[register as usize] = value;
                }
            }
            crate::vfp::VfpEffect::CoreWritePair {
                register1,
                value1,
                register2,
                value2,
            } => {
                if register1 < 15 {
                    self.cpu.r[register1 as usize] = value1;
                }
                if register2 < 15 {
                    self.cpu.r[register2 as usize] = value2;
                }
            }
            _ => {}
        }
        self.cpu.r[15] = pc + 4;
        Ok(())
    }

    /// Coprocessor data processing / register transfer (bits[27:24] = 1110):
    /// the VFP arithmetic space plus VMRS/VMSR.
    fn execute_coprocessor_data_processing(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        let cp = (insn >> 8) & 0xF;
        let bit4 = insn & 0x10 != 0;
        if bit4 && cp == 15 {
            // MCR/MRC p15: only the TLS register (c13, c0, 3) is modeled.
            let opc1 = (insn >> 21) & 7;
            let crm = insn & 0xF;
            let opc2 = (insn >> 5) & 7;
            let load = insn & 0x0010_0000 != 0;
            let rt = (insn >> 12) & 0xF;
            if opc1 == 0 && crm == 13 && opc2 == 3 {
                if load {
                    self.cpu.r[rt as usize] = self.cpu.tls;
                } else {
                    self.cpu.tls = self.cpu.r[rt as usize];
                }
                self.cpu.r[15] = pc + 4;
                return Ok(());
            }
            return Err(format!(
                "coprocessor p15 access ({insn:#010x}) is not supported"
            ));
        }
        if cp != 10 && cp != 11 {
            return Err(format!(
                "coprocessor data processing for cp{cp} is not supported ({insn:#010x})"
            ));
        }
        let rt_value = self.register_value((insn >> 16) & 0xF);
        let effect = self.cpu.vfp.execute(insn, rt_value)?;
        match effect {
            crate::vfp::VfpEffect::CoreWrite { register, value } => {
                if register == 15 {
                    // VMRS APSR_nzcv is handled through UpdateFlags instead.
                } else {
                    self.cpu.r[register as usize] = value;
                }
            }
            crate::vfp::VfpEffect::UpdateFlags { n, z, c, v } => {
                self.cpu.flags.n = n;
                self.cpu.flags.z = z;
                self.cpu.flags.c = c;
                self.cpu.flags.v = v;
            }
            _ => {}
        }
        self.cpu.r[15] = pc + 4;
        Ok(())
    }

    /// ARM data processing and the top==0 miscellaneous space.
    fn execute_data_processing(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        let bit4 = insn & 0x10 != 0;
        let bit7 = insn & 0x80 != 0;
        let sh = (insn >> 5) & 3;
        let bits2720 = (insn >> 20) & 0xFF;
        if (insn >> 25) & 7 == 0 && bit4 {
            // BX / BLX register: cond 0001 0010 1111 1111 1111 00x1 Rm.
            if insn & 0x0FFF_FFF0 == 0x012F_FF30 {
                self.cpu.r[14] = pc + 4;
                self.jump_to(self.register_value(insn & 0xF));
                return Ok(());
            }
            if insn & 0x0FFF_FFF0 == 0x012F_FF10 {
                self.jump_to(self.register_value(insn & 0xF));
                return Ok(());
            }
            // CLZ: cond 0001 0110 1111 Rd 1111 0001 Rm (shares 0x16 with
            // SMULxy; distinguished by the 1111 fields).
            if insn & 0x0FFF_0FF0 == 0x016F_0F10 {
                let value = self.register_value(insn & 0xF);
                self.cpu.r[((insn >> 12) & 0xF) as usize] = value.leading_zeros();
                self.cpu.r[15] = pc + 4;
                return Ok(());
            }
            // SWP/SWPB: bits[27:20] = 0001_0B00 with bits[11:8] = 0; checked
            // before the DSP multiplies, which share bits[27:20] = 0x10-0x16.
            if bits2720 & 0xFB == 0x10
                && bits7_4(insn) == 0b1001
                && insn & 0x0100_0000 != 0
                && insn & 0x0000_F000 == 0
            {
                return self.execute_swap(insn, pc);
            }
            // v5TE DSP multiplies and the QADD family: bits[27:23] = 00010,
            // S = 0; the Q-family is distinguished by bits[7:4] == 0101.
            if bits2720 & 0x78 == 0x10 && bits2720 & 1 == 0 {
                if bits7_4(insn) == 0b0101 {
                    return self.execute_saturating(insn, pc);
                }
                return self.execute_multiply(insn, pc);
            }
            if bit7 && sh == 0 {
                if bits2720 & 0xF0 == 0x00 && insn & 0x0100_0000 == 0 {
                    // MUL/MLA (0x00-0x03) and UMULL/UMLAL/SMULL/SMLAL (0x08-0x0F).
                    return self.execute_multiply(insn, pc);
                }
                return Err(format!(
                    "unallocated multiply/SWP-space instruction {insn:#010x}"
                ));
            }
            if bit7 && sh != 0 {
                return self.execute_halfword_transfer(insn, pc);
            }
        }
        // MRS / MSR (register) live in the DP space with Rd == 15.
        if insn & 0x0FBF_0FFF == 0x010F_0000 {
            self.cpu.r[((insn >> 12) & 0xF) as usize] = self.cpu.psr();
            self.cpu.r[15] = pc + 4;
            return Ok(());
        }
        if insn & 0x0FBF_F000 == 0x0120_F000 && insn & 0x0200_0000 == 0 {
            self.cpu.set_psr(self.register_value(insn & 0xF));
            self.cpu.r[15] = pc + 4;
            return Ok(());
        }
        if insn & 0x0FBF_F000 == 0x0320_F000 {
            // MSR immediate (CPSR).
            self.cpu.set_psr(self.operand2(insn)?.0);
            self.cpu.r[15] = pc + 4;
            return Ok(());
        }
        self.execute_dp_core(insn, pc)
    }

    fn execute_dp_core(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        const OPCODES: [&str; 16] = [
            "AND", "EOR", "SUB", "RSB", "ADD", "ADC", "SBC", "RSC", "TST", "TEQ", "CMP", "CMN",
            "ORR", "MOV", "BIC", "MVN",
        ];
        let opcode = ((insn >> 21) & 0xF) as u8;
        let set_flags = insn & 0x0010_0000 != 0;
        let destination = (insn >> 12) & 0xF;
        let is_compare = matches!(opcode, DP_TST | DP_TEQ | DP_CMP | DP_CMN);
        if destination == 15 && !is_compare && set_flags {
            return Err(format!(
                "data-processing write to PC with S bit ({insn:#010x}) is not supported"
            ));
        }
        let first = self.register_value((insn >> 16) & 0xF);
        let (operand, carry) = self.operand2(insn)?;
        let result: u32;
        match opcode {
            DP_AND => result = first & operand,
            DP_EOR => result = first ^ operand,
            DP_SUB | DP_CMP => result = Self::sub_with_borrow(first, operand, true).0,
            DP_RSB => result = Self::sub_with_borrow(operand, first, true).0,
            DP_ADD | DP_CMN => result = Self::add_with_carry(first, operand, false).0,
            DP_ADC => result = Self::add_with_carry(first, operand, self.cpu.flags.c).0,
            DP_SBC => result = Self::sub_with_borrow(first, operand, self.cpu.flags.c).0,
            DP_RSC => result = Self::sub_with_borrow(operand, first, self.cpu.flags.c).0,
            DP_ORR => result = first | operand,
            DP_MOV => result = operand,
            DP_BIC => result = first & !operand,
            DP_MVN => result = !operand,
            _ => {
                return Err(format!(
                    "data-processing opcode {} ({}) is not supported",
                    opcode, OPCODES[opcode as usize]
                ))
            }
        }
        if set_flags || is_compare {
            match opcode {
                DP_ADD | DP_CMN => {
                    let (_, c, v) = Self::add_with_carry(first, operand, false);
                    self.set_arithmetic_flags(result, c, v);
                }
                DP_ADC => {
                    let (_, c, v) = Self::add_with_carry(first, operand, self.cpu.flags.c);
                    self.set_arithmetic_flags(result, c, v);
                }
                DP_SUB | DP_CMP => {
                    let (_, c, v) = Self::sub_with_borrow(first, operand, true);
                    self.set_arithmetic_flags(result, c, v);
                }
                DP_SBC => {
                    let (_, c, v) = Self::sub_with_borrow(first, operand, self.cpu.flags.c);
                    self.set_arithmetic_flags(result, c, v);
                }
                DP_RSB | DP_RSC => {
                    let borrow_in = opcode == DP_RSC && self.cpu.flags.c;
                    let (_, c, v) = Self::sub_with_borrow(operand, first, borrow_in);
                    self.set_arithmetic_flags(result, c, v);
                }
                _ => self.set_logic_flags(result, carry),
            }
        }
        if is_compare {
            self.cpu.r[15] = pc + 4;
        } else if destination == 15 {
            self.jump_to(result);
        } else {
            self.cpu.r[destination as usize] = result;
            self.cpu.r[15] = pc + 4;
        }
        Ok(())
    }

    fn set_arithmetic_flags(&mut self, result: u32, carry: bool, overflow: bool) {
        self.set_nz(result);
        self.cpu.flags.c = carry;
        self.cpu.flags.v = overflow;
    }

    fn execute_swap(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        let address = self.register_value((insn >> 16) & 0xF);
        let source = self.register_value(insn & 0xF);
        let destination = (insn >> 12) & 0xF;
        if insn & 0x0040_0000 != 0 {
            let loaded = self.memory.read_u8(address).map_err(|e| e.to_string())?;
            self.memory
                .write_u8(address, source as u8)
                .map_err(|e| e.to_string())?;
            self.cpu.r[destination as usize] = u32::from(loaded);
        } else {
            let loaded = self.memory.read_u32(address).map_err(|e| e.to_string())?;
            self.memory
                .write_u32(address, source)
                .map_err(|e| e.to_string())?;
            self.cpu.r[destination as usize] = loaded;
        }
        self.cpu.r[15] = pc + 4;
        Ok(())
    }

    fn execute_multiply(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        let set_flags = insn & 0x0010_0000 != 0;
        let register_m = self.register_value(insn & 0xF) as i32;
        let register_s = self.register_value((insn >> 8) & 0xF) as i32;
        let high = (insn >> 16) & 0xF; // Rd for 32-bit/DSP, RdHi for 64-bit
        let low = (insn >> 12) & 0xF; // addend for MLA/SMLAxy, RdLo for 64-bit
        let next_pc = pc + 4;
        match (insn >> 20) & 0xFF {
            0x00..=0x03 => {
                // MUL (A=0) / MLA (A=1); S = bit20.
                let product = (register_m as i64).wrapping_mul(register_s as i64);
                let result = if insn & 0x0020_0000 != 0 {
                    product + self.register_value(low) as i64
                } else {
                    product
                };
                self.cpu.r[high as usize] = result as u32;
                if set_flags {
                    self.set_nz(result as u32);
                }
            }
            0x08 | 0x0A | 0x0C | 0x0E => {
                // UMULL/UMLAL (U=0) / SMULL/SMLAL (U=1); A = bit22.
                let signed = insn & 0x0040_0000 != 0;
                let wide: i128 = if signed {
                    i128::from(register_m) * i128::from(register_s)
                } else {
                    i128::from(register_m as u32).wrapping_mul(register_s as u32 as i128)
                };
                let mut combined = wide;
                if insn & 0x0020_0000 != 0 {
                    let current = ((self.cpu.r[high as usize] as u128) << 32)
                        | self.cpu.r[low as usize] as u128;
                    combined = combined.wrapping_add(current as i128);
                }
                self.cpu.r[low as usize] = combined as u32;
                self.cpu.r[high as usize] = (combined >> 32) as u32;
                if set_flags {
                    self.cpu.flags.n = combined & (1i128 << 63) != 0;
                    self.cpu.flags.z = combined == 0;
                }
            }
            0x10 | 0x12 | 0x14 | 0x16 => {
                // v5TE DSP multiplies; x = bit5, y = bit6.
                let x = (insn >> 5) & 1;
                let y = (insn >> 6) & 1;
                let half = |value: i32, low_half: u32| -> i32 {
                    if low_half == 0 {
                        i32::from(value as i16)
                    } else {
                        i32::from((value >> 16) as i16)
                    }
                };
                match (insn >> 20) & 0xF {
                    0b0000 => {
                        // SMLAxy: Rd = Rm[x] * Rs[y] + Rn; Q on overflow.
                        let addend = self.register_value(low) as i32;
                        let product = half(register_m, x) as i64 * half(register_s, y) as i64;
                        let result = product + i64::from(addend);
                        self.cpu.flags.q |= result > i32::MAX as i64 || result < i32::MIN as i64;
                        self.cpu.r[high as usize] = result as u32;
                    }
                    0b0010 => {
                        // SMLAWy (bit5 = 0) / SMULWy (bit5 = 1).
                        let product = half(register_m, y) as i64 * i64::from(register_s);
                        if insn & 0x20 != 0 {
                            self.cpu.r[high as usize] = ((product >> 16) & 0xFFFF_FFFF) as u32;
                        } else {
                            let addend = self.register_value(low) as i32;
                            let result = (product >> 16) + i64::from(addend);
                            self.cpu.flags.q |=
                                result > i32::MAX as i64 || result < i32::MIN as i64;
                            self.cpu.r[high as usize] = result as u32;
                        }
                    }
                    0b0100 => {
                        // SMLALxy: RdHi:RdLo += Rm[x] * Rs[y].
                        let product = half(register_m, x) as i64 * half(register_s, y) as i64;
                        let combined = ((self.cpu.r[high as usize] as u64) << 32
                            | self.cpu.r[low as usize] as u64)
                            as i64;
                        let result = combined.wrapping_add(product);
                        self.cpu.r[low as usize] = result as u32;
                        self.cpu.r[high as usize] = (result >> 32) as u32;
                    }
                    0b0110 => {
                        // SMULxy: Rd = Rm[x] * Rs[y].
                        self.cpu.r[high as usize] =
                            half(register_m, x).wrapping_mul(half(register_s, y)) as u32;
                    }
                    other => {
                        return Err(format!(
                            "DSP multiply class {other:04b} is not supported ({insn:#010x})"
                        ))
                    }
                }
            }
            other => {
                return Err(format!(
                    "multiply class {other:02x} is not supported ({insn:#010x})"
                ))
            }
        }
        self.cpu.r[15] = next_pc;
        Ok(())
    }

    fn execute_saturating(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        let class = (insn >> 21) & 0xF;
        let destination = (insn >> 12) & 0xF;
        let first = self.register_value((insn >> 16) & 0xF) as i32;
        let second = self.register_value(insn & 0xF) as i32;
        // Sets Q whenever the final or intermediate result saturates.
        let mut saturating = |raw: i64| -> i32 {
            let value = Self::saturate32(raw);
            if i64::from(value) != raw {
                self.cpu.flags.q = true;
            }
            value
        };
        let result = match class {
            0b1000 => saturating(i64::from(first) + i64::from(second)), // QADD
            0b1001 => saturating(i64::from(first) - i64::from(second)), // QSUB
            0b1010 => {
                // QDADD: Rd = QSAT(Rn + QSAT(2 * Rm)).
                let doubled = saturating(i64::from(second) * 2);
                saturating(i64::from(first) + i64::from(doubled))
            }
            0b1011 => {
                // QDSUB
                let doubled = saturating(i64::from(second) * 2);
                saturating(i64::from(first) - i64::from(doubled))
            }
            _ => return Err(format!("saturating class {class:04b} is not supported")),
        };
        self.cpu.r[destination as usize] = result as u32;
        self.cpu.r[15] = pc + 4;
        Ok(())
    }

    fn saturate32(value: i64) -> i32 {
        value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
    }

    /// Halfword/signed transfers plus v5TE LDRD/STRD.
    ///
    /// bits[6:4]: 011 -> STRH/LDRH, 101 -> LDRD (L=0) / LDRSB (L=1),
    /// 111 -> STRD (L=0) / LDRSH (L=1).
    fn execute_halfword_transfer(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        let register_n = (insn >> 16) & 0xF;
        let register_d = (insn >> 12) & 0xF;
        let up = insn & 0x0080_0000 != 0;
        let pre_indexed = insn & 0x0100_0000 != 0;
        let writeback = insn & 0x0020_0000 != 0;
        let load = insn & 0x0010_0000 != 0;
        let sh = (insn >> 5) & 3;
        // LDRD/STRD always use the immediate form; for the other classes
        // bit22 selects immediate versus register offset.
        let immediate = insn & 0x0040_0000 != 0 || (sh != 0b01 && !load);
        let offset: u32 = if immediate {
            u32::from(((insn >> 4) & 0xF0) as u8) | (insn & 0xF)
        } else {
            self.register_value(insn & 0xF)
        };
        let base = self.register_value(register_n);
        let address = if up {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let effective = if pre_indexed { address } else { base };
        let mut pair_transfer = false;
        match (sh, load) {
            (0b01, false) => {
                let value = self.cpu.r[register_d as usize];
                self.memory
                    .write_u16(effective, value as u16)
                    .map_err(|e| e.to_string())?;
            }
            (0b01, true) => {
                let value = self.memory.read_u16(effective).map_err(|e| e.to_string())?;
                self.write_load_result(register_d, u32::from(value));
            }
            (0b10, true) => {
                let value = self.memory.read_u8(effective).map_err(|e| e.to_string())?;
                self.write_load_result(register_d, (value as i8) as i32 as u32);
            }
            (0b10, false) => {
                // LDRD (bit20 = 0).
                pair_transfer = true;
                self.execute_double_transfer(insn, effective, true)?;
            }
            (0b11, false) => {
                // STRD (bit20 = 0).
                pair_transfer = true;
                self.execute_double_transfer(insn, effective, false)?;
            }
            (_, true) => {
                let value = self.memory.read_u16(effective).map_err(|e| e.to_string())?;
                self.write_load_result(register_d, (value as i16) as i32 as u32);
            }
            (_, false) => {
                return Err(format!(
                    "store with SH={sh:02b} is not a valid encoding ({insn:#010x})"
                ))
            }
        }
        if register_n != 15 && (writeback || !pre_indexed) {
            self.cpu.r[register_n as usize] = address;
        }
        if pair_transfer || self.cpu.r[15] == pc {
            self.cpu.r[15] = pc + 4;
        }
        Ok(())
    }

    fn execute_double_transfer(
        &mut self,
        insn: u32,
        address: u32,
        load: bool,
    ) -> Result<(), String> {
        let first = (insn >> 12) & 0xF;
        if first & 1 == 1 || first >= 14 {
            return Err(format!(
                "double transfer with invalid register pair ({insn:#010x})"
            ));
        }
        let second = first + 1;
        if load {
            let low = self.memory.read_u32(address).map_err(|e| e.to_string())?;
            let high = self
                .memory
                .read_u32(address + 4)
                .map_err(|e| e.to_string())?;
            self.cpu.r[first as usize] = low;
            self.cpu.r[second as usize] = high;
        } else {
            self.memory
                .write_u32(address, self.cpu.r[first as usize])
                .map_err(|e| e.to_string())?;
            self.memory
                .write_u32(address + 4, self.cpu.r[second as usize])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn write_load_result(&mut self, register: u32, value: u32) {
        if register == 15 {
            self.jump_to(value);
        } else {
            self.cpu.r[register as usize] = value;
        }
    }

    fn execute_load_store(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        let register_n = (insn >> 16) & 0xF;
        let register_d = (insn >> 12) & 0xF;
        let up = insn & 0x0080_0000 != 0;
        let byte = insn & 0x0040_0000 != 0;
        let writeback_bit = insn & 0x0020_0000 != 0;
        let load = insn & 0x0010_0000 != 0;
        let pre_indexed = insn & 0x0100_0000 != 0;
        // Bit 25 selects the offset form: 0 = immediate 12-bit, 1 = shifted
        // register (the reverse of the data-processing convention).
        let offset: u32 = if insn & 0x0200_0000 == 0 {
            insn & 0xFFF
        } else {
            let shift_type = ((insn >> 5) & 3) as u8;
            let amount = (insn >> 7) & 0x1F;
            let reg_form = insn & 0x10 != 0;
            self.shifted(
                self.register_value(insn & 0xF),
                shift_type,
                amount,
                reg_form,
            )
            .0
        };
        let base = self.register_value(register_n);
        let address = if up {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let effective = if pre_indexed { address } else { base };
        if load {
            let value = if byte {
                u32::from(self.memory.read_u8(effective).map_err(|e| e.to_string())?)
            } else {
                self.memory.read_u32(effective).map_err(|e| e.to_string())?
            };
            self.write_load_result(register_d, value);
        } else {
            let value = if register_d == 15 {
                pc + 8
            } else {
                self.cpu.r[register_d as usize]
            };
            if byte {
                self.memory
                    .write_u8(effective, value as u8)
                    .map_err(|e| e.to_string())?;
            } else {
                self.memory
                    .write_u32(effective, value)
                    .map_err(|e| e.to_string())?;
            }
        }
        if register_n != 15 && (writeback_bit || !pre_indexed) {
            // P=0 with W=1 selects user-mode transfers; treated as writeback.
            self.cpu.r[register_n as usize] = address;
        }
        if self.cpu.r[15] == pc {
            self.cpu.r[15] = pc + 4;
        }
        Ok(())
    }

    fn execute_load_store_multiple(&mut self, insn: u32, pc: u32) -> Result<(), String> {
        let register_n = (insn >> 16) & 0xF;
        let up = insn & 0x0080_0000 != 0;
        let before = insn & 0x0100_0000 != 0;
        let writeback = insn & 0x0020_0000 != 0;
        let load = insn & 0x0010_0000 != 0;
        let mut registers = Vec::new();
        for index in 0..16u32 {
            if insn & (1 << index) != 0 {
                registers.push(index);
            }
        }
        if registers.is_empty() {
            return Err(format!("empty register list in {insn:#010x}"));
        }
        let base = self.register_value(register_n);
        let count = registers.len() as u32 * 4;
        let (start, final_base) = match (before, up) {
            (false, true) => (base, base.wrapping_add(count)),
            (true, true) => (base.wrapping_add(4), base.wrapping_add(count)),
            (false, false) => (
                base.wrapping_sub(count).wrapping_add(4),
                base.wrapping_sub(count),
            ),
            (true, false) => (base.wrapping_sub(count), base.wrapping_sub(count)),
        };
        let mut address = start;
        let original_base = base;
        let mut pc_loaded = false;
        if load {
            for register in &registers {
                let value = self.memory.read_u32(address).map_err(|e| e.to_string())?;
                if *register == 15 {
                    pc_loaded = true;
                    self.jump_to(value);
                } else {
                    self.cpu.r[*register as usize] = value;
                }
                address += 4;
            }
            if writeback && !registers.contains(&register_n) && register_n != 15 {
                self.cpu.r[register_n as usize] = final_base;
            }
            if !pc_loaded {
                self.cpu.r[15] = pc + 4;
            }
        } else {
            for register in &registers {
                let value = if *register == register_n {
                    original_base
                } else {
                    self.register_value(*register)
                };
                self.memory
                    .write_u32(address, value)
                    .map_err(|e| e.to_string())?;
                address += 4;
            }
            if writeback && register_n != 15 {
                self.cpu.r[register_n as usize] = final_base;
            }
            self.cpu.r[15] = pc + 4;
        }
        Ok(())
    }

    // ---- Thumb ----

    fn execute_thumb(
        &mut self,
        insn: u16,
        pc: u32,
        host: &mut dyn HostBridge,
    ) -> Result<(), String> {
        let next_pc = pc + 2;
        if insn < 0x1800 {
            // Shift by immediate.
            let opcode = ((insn >> 11) & 3) as u8;
            let amount = u32::from((insn >> 6) & 0x1F);
            let source = self.cpu.r[((insn >> 3) & 7) as usize];
            let (value, carry) = self.shifted(source, opcode, amount, false);
            self.cpu.r[(insn & 7) as usize] = value;
            // LSL #0 is a plain register move; every other shift updates flags.
            if !(opcode == 0 && amount == 0) {
                self.set_logic_flags(value, carry);
            }
            self.cpu.r[15] = next_pc;
            return Ok(());
        }
        if insn < 0x2000 {
            // ADD/SUB register or 3-bit immediate; always sets flags.
            // Rd = bits[2:0], Rn = bits[5:3], Rm/imm3 = bits[8:6].
            let immediate = insn & 0x0400 != 0;
            let subtract = insn & 0x0200 != 0;
            let operand_a = self.cpu.r[((insn >> 3) & 7) as usize];
            let operand_b = if immediate {
                u32::from((insn >> 6) & 7)
            } else {
                self.cpu.r[((insn >> 6) & 7) as usize]
            };
            let (result, carry, overflow) = if subtract {
                Self::sub_with_borrow(operand_a, operand_b, true)
            } else {
                Self::add_with_carry(operand_a, operand_b, false)
            };
            self.set_arithmetic_flags(result, carry, overflow);
            self.cpu.r[(insn & 7) as usize] = result;
            self.cpu.r[15] = next_pc;
            return Ok(());
        }
        if insn < 0x4000 {
            // MOV/CMP/ADD/SUB immediate-8; always sets flags.
            let opcode = (insn >> 11) & 3;
            let value = u32::from(insn & 0xFF);
            let destination = ((insn >> 8) & 7) as usize;
            let first = self.cpu.r[destination];
            match opcode {
                0 => {
                    self.cpu.r[destination] = value;
                    self.set_nz(value);
                }
                1 => {
                    let (result, carry, overflow) = Self::sub_with_borrow(first, value, true);
                    self.set_arithmetic_flags(result, carry, overflow);
                    self.cpu.r[15] = next_pc;
                    return Ok(());
                }
                _ => {
                    let (result, carry, overflow) = if opcode == 2 {
                        Self::add_with_carry(first, value, false)
                    } else {
                        Self::sub_with_borrow(first, value, true)
                    };
                    self.set_arithmetic_flags(result, carry, overflow);
                    self.cpu.r[destination] = result;
                }
            }
            self.cpu.r[15] = next_pc;
            return Ok(());
        }
        if insn < 0x4400 {
            // ALU operations (register).
            let opcode = ((insn >> 6) & 0xF) as u8;
            let source = self.cpu.r[((insn >> 3) & 7) as usize];
            let destination = (insn & 7) as usize;
            let first = self.cpu.r[destination];
            match opcode {
                0 => {
                    self.cpu.r[destination] = first & source;
                    self.set_logic_flags(self.cpu.r[destination], self.cpu.flags.c);
                }
                1 => {
                    self.cpu.r[destination] = first ^ source;
                    self.set_logic_flags(self.cpu.r[destination], self.cpu.flags.c);
                }
                2 | 3 | 4 | 7 => {
                    let shift_type = match opcode {
                        2 => 0,
                        3 => 1,
                        4 => 2,
                        _ => 3,
                    };
                    let (value, carry) = self.shifted(first, shift_type, source & 0xFF, true);
                    self.cpu.r[destination] = value;
                    self.set_logic_flags(value, carry);
                }
                5 => {
                    let (value, carry, overflow) =
                        Self::add_with_carry(first, source, self.cpu.flags.c);
                    self.cpu.r[destination] = value;
                    self.set_arithmetic_flags(value, carry, overflow);
                }
                6 => {
                    let (value, carry, overflow) =
                        Self::sub_with_borrow(first, source, self.cpu.flags.c);
                    self.cpu.r[destination] = value;
                    self.set_arithmetic_flags(value, carry, overflow);
                }
                8 => self.set_logic_flags(first & source, self.cpu.flags.c),
                9 => {
                    let (value, carry, overflow) = Self::sub_with_borrow(0, source, true);
                    self.cpu.r[destination] = value;
                    self.set_arithmetic_flags(value, carry, overflow);
                }
                10 => {
                    let (_, carry, overflow) = Self::sub_with_borrow(first, source, true);
                    let value = first.wrapping_sub(source);
                    self.set_arithmetic_flags(value, carry, overflow);
                }
                11 => {
                    let (_, carry, overflow) = Self::add_with_carry(first, source, false);
                    let value = first.wrapping_add(source);
                    self.set_arithmetic_flags(value, carry, overflow);
                }
                12 => {
                    self.cpu.r[destination] = first | source;
                    self.set_logic_flags(self.cpu.r[destination], self.cpu.flags.c);
                }
                13 => {
                    let value = (first as i32).wrapping_mul(source as i32) as u32;
                    self.cpu.r[destination] = value;
                    self.set_logic_flags(value, self.cpu.flags.c);
                }
                14 => {
                    self.cpu.r[destination] = first & !source;
                    self.set_logic_flags(self.cpu.r[destination], self.cpu.flags.c);
                }
                _ => {
                    self.cpu.r[destination] = !source;
                    self.set_logic_flags(self.cpu.r[destination], self.cpu.flags.c);
                }
            }
            self.cpu.r[15] = next_pc;
            return Ok(());
        }
        if insn < 0x4800 {
            // Hi-register operations and BX/BLX register.
            let opcode = (insn >> 8) & 3;
            let destination = ((insn & 7) as usize) | (usize::from(insn & 0x80 != 0) << 3);
            let source_register =
                (((insn >> 3) & 7) as usize) | (usize::from(insn & 0x40 != 0) << 3);
            let source = self.register_value(source_register as u32);
            match opcode {
                0 => {
                    let result = self.cpu.r[destination].wrapping_add(source);
                    if destination == 15 {
                        self.set_pc_no_interwork(result);
                    } else {
                        self.cpu.r[destination] = result;
                        self.cpu.r[15] = next_pc;
                    }
                }
                1 => {
                    let (value, carry, overflow) =
                        Self::sub_with_borrow(self.cpu.r[destination], source, true);
                    self.set_arithmetic_flags(value, carry, overflow);
                    self.cpu.r[15] = next_pc;
                }
                2 => {
                    if destination == 15 {
                        self.jump_to(source);
                    } else {
                        self.cpu.r[destination] = source;
                        self.cpu.r[15] = next_pc;
                    }
                }
                _ => self.jump_to(source),
            }
            return Ok(());
        }
        if insn < 0x5000 {
            // PC-relative load.
            let address = ((pc + 4) & !3).wrapping_add(u32::from(insn & 0xFF) * 4);
            let value = self.memory.read_u32(address).map_err(|e| e.to_string())?;
            self.cpu.r[((insn >> 8) & 7) as usize] = value;
            self.cpu.r[15] = next_pc;
            return Ok(());
        }
        if insn < 0x6000 {
            // Load/store with register offset: base = bits[5:3] (Rn),
            // offset register = bits[8:6] (Rm), data = bits[2:0] (Rt).
            let opcode = u32::from((insn >> 9) & 7);
            let address = self.cpu.r[((insn >> 3) & 7) as usize]
                .wrapping_add(self.cpu.r[((insn >> 6) & 7) as usize]);
            return self.thumb_memory_access(opcode, address, (insn & 7) as usize, next_pc);
        }
        if insn < 0x8000 {
            // Load/store with 5-bit immediate offset: bit 12 selects byte
            // access (B) and bit 11 selects load (L).
            let is_byte = insn & 0x1000 != 0;
            let is_load = insn & 0x0800 != 0;
            let scale: u32 = if is_byte { 1 } else { 4 };
            let offset = u32::from((insn >> 6) & 0x1F) * scale;
            let address = self.cpu.r[((insn >> 3) & 7) as usize].wrapping_add(offset);
            let opcode = match (is_byte, is_load) {
                (false, false) => 0, // STR
                (false, true) => 4,  // LDR
                (true, false) => 2,  // STRB
                (true, true) => 6,   // LDRB
            };
            return self.thumb_memory_access(opcode, address, (insn & 7) as usize, next_pc);
        }
        if insn < 0x9000 {
            // Load/store halfword with 5-bit immediate offset (scaled by 2).
            let offset = u32::from((insn >> 6) & 0x1F) * 2;
            let address = self.cpu.r[((insn >> 3) & 7) as usize].wrapping_add(offset);
            let opcode = if insn & 0x0800 != 0 { 5 } else { 1 };
            return self.thumb_memory_access(opcode, address, (insn & 7) as usize, next_pc);
        }
        if insn < 0xA000 {
            // SP-relative load/store.
            let address = self.cpu.r[13].wrapping_add(u32::from(insn & 0xFF) * 4);
            let opcode = if insn & 0x0800 != 0 { 4 } else { 0 };
            return self.thumb_memory_access(opcode, address, ((insn >> 8) & 7) as usize, next_pc);
        }
        if insn < 0xB000 {
            // ADR / ADD Rd, SP, imm.
            let base = if insn & 0x0800 != 0 {
                self.cpu.r[13]
            } else {
                (pc + 4) & !3
            };
            self.cpu.r[((insn >> 8) & 7) as usize] = base.wrapping_add(u32::from(insn & 0xFF) * 4);
            self.cpu.r[15] = next_pc;
            return Ok(());
        }
        if insn < 0xC000 {
            // Stack and misc.
            match (insn >> 8) & 0xF {
                0 => {
                    let adjustment = u32::from(insn & 0x7F) * 4;
                    self.cpu.r[13] = if insn & 0x80 != 0 {
                        self.cpu.r[13].wrapping_sub(adjustment)
                    } else {
                        self.cpu.r[13].wrapping_add(adjustment)
                    };
                    self.cpu.r[15] = next_pc;
                    Ok(())
                }
                4 | 5 => self.execute_thumb_stack(insn, pc, true),
                12 | 13 => self.execute_thumb_stack(insn, pc, false),
                14 => Err(format!("BKPT {insn:#06x} is not supported")),
                15 => {
                    self.cpu.r[15] = next_pc;
                    Ok(())
                }
                _ => Err(format!("Thumb misc {insn:#06x} is not supported")),
            }
        } else if insn < 0xD000 {
            if insn & 0x0800 == 0 {
                // STMIA (writeback always applied).
                let base = (insn >> 8) & 7;
                let mut address = self.cpu.r[base as usize];
                let mut count = 0u32;
                for index in 0..8u32 {
                    if insn & (1 << index) != 0 {
                        count += 4;
                    }
                }
                for index in 0..8u32 {
                    if insn & (1 << index) != 0 {
                        self.memory
                            .write_u32(address, self.cpu.r[index as usize])
                            .map_err(|e| e.to_string())?;
                        address += 4;
                    }
                }
                self.cpu.r[base as usize] = self.cpu.r[base as usize].wrapping_add(count);
                self.cpu.r[15] = next_pc;
                Ok(())
            } else {
                // LDMIA (writeback skipped when the base is in the list).
                let base = (insn >> 8) & 7;
                let start = self.cpu.r[base as usize];
                let mut count = 0u32;
                for index in 0..8u32 {
                    if insn & (1 << index) != 0 {
                        count += 4;
                    }
                }
                let mut address = start;
                let mut pc_loaded = false;
                for index in 0..8u32 {
                    if insn & (1 << index) != 0 {
                        let value = self.memory.read_u32(address).map_err(|e| e.to_string())?;
                        if index == 15 {
                            pc_loaded = true;
                            self.jump_to(value);
                        } else {
                            self.cpu.r[index as usize] = value;
                        }
                        address += 4;
                    }
                }
                if insn & (1 << base) == 0 {
                    self.cpu.r[base as usize] = start.wrapping_add(count);
                }
                if !pc_loaded {
                    self.cpu.r[15] = next_pc;
                }
                Ok(())
            }
        } else if insn < 0xE000 {
            {
                // Conditional branch.
                let condition = ((insn >> 8) & 0xF) as u8;
                if condition == 0xF {
                    // Thumb SVC: EABI syscall number still arrives in r7.
                    let number = self.register_value(7);
                    let result = host.syscall(self, number);
                    self.cpu.r[0] = result;
                    self.cpu.r[15] = pc + 4;
                    return Ok(());
                }
                if condition == 0xE {
                    return Err(format!("Thumb UDF {insn:#06x} is not supported"));
                }
                if self.condition_holds(condition) {
                    let offset = (((u32::from(insn) & 0xFF) << 24) as i32) >> 23;
                    self.cpu.r[15] = (pc + 4).wrapping_add(offset as u32);
                } else {
                    self.cpu.r[15] = next_pc;
                }
                Ok(())
            }
        } else if insn < 0xF000 {
            // Unconditional branch.
            let offset = (((u32::from(insn) & 0x7FF) << 21) as i32) >> 20;
            self.cpu.r[15] = (pc + 4).wrapping_add(offset as u32);
            Ok(())
        } else {
            // 32-bit Thumb BL / BLX pairs.
            let next = self.memory.read_u16(pc + 2).map_err(|e| e.to_string())?;
            let offset = ((u32::from(insn & 0x7FF) << 12) | (u32::from(next & 0x7FF) << 1)) as i32;
            let offset = if offset & 0x0040_0000 != 0 {
                offset | !0x007F_FFFF
            } else {
                offset
            };
            self.cpu.r[14] = (pc + 4) | 1;
            if next & 0xF800 == 0xF800 {
                self.cpu.r[15] = (pc + 4).wrapping_add(offset as u32);
            } else if next & 0xF800 == 0xE800 {
                self.cpu.r[15] = (pc + 4).wrapping_add(offset as u32) & !3;
                self.cpu.flags.thumb = false;
            } else {
                return Err(format!(
                    "Thumb-2 32-bit instruction pair {insn:#06x} {next:#06x} is not supported"
                ));
            }
            Ok(())
        }
    }

    fn thumb_memory_access(
        &mut self,
        opcode: u32,
        address: u32,
        destination: usize,
        next_pc: u32,
    ) -> Result<(), String> {
        let outcome: Result<(), String> = match opcode {
            // Thumb bits[11:9] order: STR, STRH, STRB, LDRSB, LDR, LDRH,
            // LDRB, LDRSH.
            0 => self
                .memory
                .write_u32(address, self.cpu.r[destination])
                .map_err(|e| e.to_string()),
            1 => {
                let value = self.cpu.r[destination] as u16;
                self.memory
                    .write_u16(address, value)
                    .map_err(|e| e.to_string())
            }
            2 => self
                .memory
                .write_u8(address, self.cpu.r[destination] as u8)
                .map_err(|e| e.to_string()),
            3 => {
                let value = self.memory.read_u8(address).map_err(|e| e.to_string())?;
                self.cpu.r[destination] = i32::from(value as i8) as u32;
                Ok(())
            }
            4 => {
                let value = self.memory.read_u32(address).map_err(|e| e.to_string())?;
                self.cpu.r[destination] = value;
                Ok(())
            }
            5 => {
                let value = self.memory.read_u16(address).map_err(|e| e.to_string())?;
                self.cpu.r[destination] = u32::from(value);
                Ok(())
            }
            6 => {
                let value = self.memory.read_u8(address).map_err(|e| e.to_string())?;
                self.cpu.r[destination] = u32::from(value);
                Ok(())
            }
            _ => {
                let value = self.memory.read_u16(address).map_err(|e| e.to_string())?;
                self.cpu.r[destination] = i32::from(value as i16) as u32;
                Ok(())
            }
        };
        if outcome.is_ok() {
            self.cpu.r[15] = next_pc;
        }
        outcome
    }

    fn execute_thumb_stack(&mut self, insn: u16, pc: u32, push: bool) -> Result<(), String> {
        let extra = insn & 0x0100 != 0;
        let mut count = if extra { 8 } else { 4 };
        for index in 0..8u32 {
            if insn & (1 << index) != 0 {
                count += 4;
            }
        }
        if push {
            let mut address = self.cpu.r[13].wrapping_sub(count);
            for index in 0..8u32 {
                if insn & (1 << index) != 0 {
                    self.memory
                        .write_u32(address, self.cpu.r[index as usize])
                        .map_err(|e| e.to_string())?;
                    address += 4;
                }
            }
            if extra {
                self.memory
                    .write_u32(address, self.cpu.r[14])
                    .map_err(|e| e.to_string())?;
            }
            self.cpu.r[13] = self.cpu.r[13].wrapping_sub(count);
            self.cpu.r[15] = pc + 2;
        } else {
            let mut address = self.cpu.r[13];
            for index in 0..8u32 {
                if insn & (1 << index) != 0 {
                    let value = self.memory.read_u32(address).map_err(|e| e.to_string())?;
                    self.cpu.r[index as usize] = value;
                    address += 4;
                }
            }
            if extra {
                let value = self.memory.read_u32(address).map_err(|e| e.to_string())?;
                self.jump_to(value);
            }
            self.cpu.r[13] = self.cpu.r[13].wrapping_add(count);
            if !extra {
                self.cpu.r[15] = pc + 2;
            }
        }
        Ok(())
    }
}

fn bits7_4(insn: u32) -> u8 {
    ((insn >> 4) & 0xF) as u8
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::BasicHost;

    const CODE: u32 = 0x1000_0000;
    const DATA: u32 = 0x2000_0000;
    const STACK: u32 = 0x7F00_0000;
    const AL: u8 = 0xE;

    // ---- Encoding builders (field positions per the ARM ARM) ----

    fn dp(cond: u8, opcode: u8, s: bool, rn: u8, rd: u8, operand2: u32) -> u32 {
        ((cond as u32) << 28)
            | ((opcode as u32) << 21)
            | ((s as u32) << 20)
            | ((rn as u32) << 16)
            | ((rd as u32) << 12)
            | operand2
    }

    fn dpi(cond: u8, opcode: u8, s: bool, rn: u8, rd: u8, rotate: u8, imm: u8) -> u32 {
        dp(
            cond,
            opcode,
            s,
            rn,
            rd,
            0x0200_0000 | ((rotate as u32) << 8) | (imm as u32),
        )
    }

    fn mov_imm(cond: u8, rd: u8, imm: u8) -> u32 {
        dpi(cond, 13, false, 0, rd, 0, imm)
    }

    fn rm(rm: u8) -> u32 {
        rm as u32
    }
    fn lsl_imm(rm: u8, amount: u8) -> u32 {
        ((amount as u32) << 7) | (rm as u32)
    }
    fn lsl_reg(rm: u8, rs: u8) -> u32 {
        ((rs as u32) << 8) | 0x10 | (rm as u32)
    }

    #[allow(clippy::too_many_arguments)]
    fn ldr_str(
        cond: u8,
        load: bool,
        byte: bool,
        wb: bool,
        up: bool,
        pre: bool,
        rn: u8,
        rd: u8,
        offset: u32,
    ) -> u32 {
        ((cond as u32) << 28)
            | (1 << 26)
            | ((pre as u32) << 24)
            | ((up as u32) << 23)
            | ((byte as u32) << 22)
            | ((wb as u32) << 21)
            | ((load as u32) << 20)
            | ((rn as u32) << 16)
            | ((rd as u32) << 12)
            | offset
    }

    fn ldr_literal(cond: u8, rd: u8, offset: u32) -> u32 {
        ((cond as u32) << 28)
            | (1 << 26)
            | (1 << 24)
            | (1 << 23)
            | (1 << 20)
            | (15 << 16)
            | ((rd as u32) << 12)
            | offset
    }

    fn ldm_stm(cond: u8, load: bool, wb: bool, up: bool, before: bool, rn: u8, regs: u16) -> u32 {
        ((cond as u32) << 28)
            | (1 << 27)
            | ((before as u32) << 24)
            | ((up as u32) << 23)
            | ((wb as u32) << 21)
            | ((load as u32) << 20)
            | ((rn as u32) << 16)
            | (regs as u32)
    }

    fn mul(cond: u8, accumulate: bool, s: bool, rd: u8, rn: u8, rs: u8, rm: u8) -> u32 {
        ((cond as u32) << 28)
            | ((accumulate as u32) << 21)
            | ((s as u32) << 20)
            | ((rd as u32) << 16)
            | ((rn as u32) << 12)
            | ((rs as u32) << 8)
            | (0x9 << 4)
            | (rm as u32)
    }

    #[allow(clippy::too_many_arguments)]
    fn mull(
        cond: u8,
        signed: bool,
        accumulate: bool,
        s: bool,
        rd_hi: u8,
        rd_lo: u8,
        rs: u8,
        rm: u8,
    ) -> u32 {
        ((cond as u32) << 28)
            | (1 << 23)
            | ((signed as u32) << 22)
            | ((accumulate as u32) << 21)
            | ((s as u32) << 20)
            | ((rd_hi as u32) << 16)
            | ((rd_lo as u32) << 12)
            | ((rs as u32) << 8)
            | (0x9 << 4)
            | (rm as u32)
    }

    fn smulxy(cond: u8, x: u8, y: u8, rd: u8, rs: u8, rm: u8) -> u32 {
        ((cond as u32) << 28)
            | (0x16 << 20)
            | ((rd as u32) << 16)
            | ((rs as u32) << 8)
            | (1 << 7)
            | ((y as u32) << 6)
            | ((x as u32) << 5)
            | (1 << 4)
            | (rm as u32)
    }

    fn qadd(cond: u8, rd: u8, rn: u8, rm: u8) -> u32 {
        ((cond as u32) << 28)
            | (0x10 << 20)
            | ((rn as u32) << 16)
            | ((rd as u32) << 12)
            | (0x5 << 4)
            | (rm as u32)
    }

    fn clz(cond: u8, rd: u8, rm: u8) -> u32 {
        ((cond as u32) << 28)
            | (0x16 << 20)
            | (0xF << 16)
            | ((rd as u32) << 12)
            | (0xF << 8)
            | (1 << 4)
            | (rm as u32)
    }

    fn swap(cond: u8, byte: bool, rd: u8, rn: u8, rm: u8) -> u32 {
        ((cond as u32) << 28)
            | (1 << 24)
            | ((byte as u32) << 22)
            | ((rn as u32) << 16)
            | ((rd as u32) << 12)
            | (0x9 << 4)
            | (rm as u32)
    }

    /// Halfword-space transfers; `sh`: 01 = H, 10 = SB/LDRD, 11 = SH/STRD.
    /// `byte_offset == u32::MAX` selects the register form.
    #[allow(clippy::too_many_arguments)]
    fn halfword(
        cond: u8,
        load: bool,
        byte_offset: u32,
        up: bool,
        pre: bool,
        wb: bool,
        rn: u8,
        rd: u8,
        sh: u8,
        rm_or_imm_h: u8,
        low: u8,
    ) -> u32 {
        let offset_field = if byte_offset == u32::MAX {
            rm_or_imm_h as u32
        } else {
            0x0040_0000 | (((rm_or_imm_h as u32) & 0xF) << 8) | ((low as u32) & 0xF)
        };
        ((cond as u32) << 28)
            | ((pre as u32) << 24)
            | ((up as u32) << 23)
            | ((wb as u32) << 21)
            | ((load as u32) << 20)
            | ((rn as u32) << 16)
            | ((rd as u32) << 12)
            | (1 << 7)
            | ((sh as u32) << 5)
            | (1 << 4)
            | offset_field
    }

    fn machine() -> (Machine, BasicHost) {
        let mut machine = Machine::new(CpuConfig::default());
        machine.memory.map_anon(CODE, 0x1_0000).unwrap();
        machine.memory.map_anon(DATA, 0x1_0000).unwrap();
        machine.memory.map_anon(STACK, 0x10_0000).unwrap();
        machine.cpu.r[13] = STACK + 0xF_0000;
        (machine, BasicHost::new())
    }

    fn arm_code(machine: &mut Machine, words: &[u32]) -> u32 {
        let entry = CODE + 0x100;
        for (index, word) in words.iter().enumerate() {
            machine
                .memory
                .write_u32(entry + index as u32 * 4, *word)
                .unwrap();
        }
        machine
            .memory
            .write_u32(entry + words.len() as u32 * 4, 0xE12F_FF1E)
            .unwrap();
        entry
    }

    fn thumb_code(machine: &mut Machine, halfwords: &[u16]) -> u32 {
        let entry = (CODE + 0x100) | 1;
        for (index, halfword) in halfwords.iter().enumerate() {
            machine
                .memory
                .write_u16(CODE + 0x100 + index as u32 * 2, *halfword)
                .unwrap();
        }
        machine
            .memory
            .write_u16(CODE + 0x100 + halfwords.len() as u32 * 2, 0x4770)
            .unwrap();
        entry
    }

    #[test]
    fn arithmetic_flags_via_condition_codes() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                dp(AL, 4, true, 1, 0, rm(2)),          // ADDS r0, r1, r2
                mov_imm(6, 3, 1),                      // MOVVS r3, #1
                mov_imm(4, 4, 1),                      // MOVMI r4, #1
                dp(AL, 4, false, 3, 0, lsl_imm(4, 1)), // ADD r0, r3, r4, LSL #1
            ],
        );
        machine.cpu.r[1] = 0x7FFF_FFFF;
        machine.cpu.r[2] = 1;
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 3, "overflow and negative flags should both hold");
    }

    #[test]
    fn subtraction_borrow_semantics() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                dp(AL, 2, true, 2, 0, rm(1)),  // SUBS r0, r2, r1
                mov_imm(3, 3, 1),              // MOVCC r3, #1
                dp(AL, 4, false, 3, 0, rm(3)), // ADD r0, r3, r3
            ],
        );
        machine.cpu.r[2] = 1;
        machine.cpu.r[1] = 2;
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 2);
    }

    #[test]
    fn shifter_carry_semantics() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                dp(AL, 13, true, 0, 0, lsl_imm(1, 1)), // MOVS r0, r1, LSL #1
                mov_imm(2, 2, 1),                      // MOVCS r2, #1
                dp(AL, 4, false, 0, 0, rm(2)),         // ADD r0, r0, r2
            ],
        );
        machine.cpu.r[1] = 0xC000_0000;
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 0x8000_0001);
    }

    #[test]
    fn register_shifted_operand() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                dp(AL, 4, false, 1, 0, lsl_reg(2, 3)), // ADD r0, r1, r2, LSL r3
            ],
        );
        let result = machine
            .call_function(&mut host, entry, &[0, 0x10, 1, 4])
            .unwrap();
        assert_eq!(result, 0x20);
    }

    #[test]
    fn load_store_addressing_modes() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                ldr_str(AL, false, false, true, true, true, 1, 0, 4), // STR r0, [r1, #4]!
                ldr_str(AL, true, false, false, false, true, 1, 2, 4), // LDR r2, [r1, #-4]
                dp(AL, 4, false, 2, 0, rm(0)),                        // ADD r0, r2, r0
            ],
        );
        machine.cpu.r[0] = 0x42;
        machine.cpu.r[1] = DATA;
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 0x42);
        assert_eq!(machine.memory.read_u32(DATA + 4).unwrap(), 0x42);
        machine.memory.write_u8(DATA + 0x20, 0x80).unwrap();
        let entry = arm_code(
            &mut machine,
            &[
                ldr_str(AL, true, true, false, true, false, 1, 2, 1), // LDRB r2, [r1], #1
                dp(AL, 4, false, 1, 0, rm(2)),                        // ADD r0, r1, r2
            ],
        );
        machine.cpu.r[1] = DATA + 0x20;
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, (DATA + 0x21) + 0x80);
    }

    #[test]
    fn load_store_multiple_with_writeback() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                ldm_stm(AL, false, true, false, true, 13, 0b1_0000_0011), // STMDB sp!
                ldm_stm(AL, true, true, true, false, 13, 0b1_0000_0011),  // LDMIA sp!
                dp(AL, 4, false, 0, 0, rm(1)),                            // ADD r0, r0, r1
            ],
        );
        machine.cpu.r[0] = 0x1111_1111;
        machine.cpu.r[1] = 0x2222_2222;
        machine.cpu.r[8] = 0x3333_3333;
        let sp_before = STACK + 0xF_0000;
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 0x3333_3333);
        assert_eq!(
            machine.memory.read_u32(sp_before - 12).unwrap(),
            0x1111_1111
        );
        assert_eq!(machine.memory.read_u32(sp_before - 8).unwrap(), 0x2222_2222);
        assert_eq!(machine.memory.read_u32(sp_before - 4).unwrap(), 0x3333_3333);
    }

    #[test]
    fn multiply_and_accumulate() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(&mut machine, &[mul(AL, true, false, 0, 3, 2, 1)]);
        let result = machine
            .call_function(&mut host, entry, &[0, 6, 7, 100])
            .unwrap();
        assert_eq!(result, 142);
        let entry = arm_code(
            &mut machine,
            &[
                mull(AL, true, false, false, 3, 2, 0, 1), // SMULL r3:r2, r0, r1
                dp(AL, 4, false, 2, 0, rm(3)),            // ADD r0, r2, r3
            ],
        );
        let result = machine
            .call_function(&mut host, entry, &[5, 0xFFFF_FFFF])
            .unwrap();
        assert_eq!(result, 0xFFFF_FFFA);
    }

    #[test]
    fn dsp_multiply_and_saturation() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(&mut machine, &[smulxy(AL, 0, 0, 0, 1, 2)]);
        let result = machine
            .call_function(&mut host, entry, &[0, 0x0002_0003, 0x0005_0007])
            .unwrap();
        assert_eq!(result, 21);
        let entry = arm_code(
            &mut machine,
            &[
                qadd(AL, 0, 1, 2),             // QADD r0, r1, r2
                0xE10F_1000,                   // MRS r1, CPSR
                dpi(AL, 0, false, 1, 0, 4, 8), // AND r0, r1, #0x08000000
            ],
        );
        machine.cpu.r[1] = 0x7FFF_FFFF;
        machine.cpu.r[2] = 0x7FFF_FFFF;
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 0x0800_0000);
    }

    #[test]
    fn count_leading_zeros_and_swap() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(&mut machine, &[clz(AL, 0, 0)]);
        let result = machine
            .call_function(&mut host, entry, &[0x0008_0000])
            .unwrap();
        assert_eq!(result, 12);
        let entry = arm_code(&mut machine, &[swap(AL, false, 0, 1, 0)]);
        machine.memory.write_u32(DATA, 0xAAAA_AAAA).unwrap();
        let result = machine
            .call_function(&mut host, entry, &[0x1234_5678, DATA])
            .unwrap();
        assert_eq!(result, 0xAAAA_AAAA);
        assert_eq!(machine.memory.read_u32(DATA).unwrap(), 0x1234_5678);
    }

    #[test]
    fn double_word_transfer() {
        let (mut machine, mut host) = machine();
        machine.memory.write_u32(DATA, 0x4444_3333).unwrap();
        machine.memory.write_u32(DATA + 4, 0x6666_5555).unwrap();
        let entry = arm_code(
            &mut machine,
            &[
                halfword(AL, false, 8, true, true, false, 0, 4, 2, 0, 8), // LDRD r4, r5, [r0, #8]
                dp(AL, 4, false, 4, 0, rm(5)),                            // ADD r0, r4, r5
            ],
        );
        let result = machine
            .call_function(&mut host, entry, &[DATA - 8])
            .unwrap();
        assert_eq!(result, 0xAAAA_8888);
        let entry = arm_code(
            &mut machine,
            &[halfword(AL, false, 8, true, true, false, 2, 0, 3, 0, 8)],
        );
        machine
            .call_function(&mut host, entry, &[0x7777_7777, 0x8888_8888, DATA - 8])
            .unwrap();
        assert_eq!(machine.memory.read_u32(DATA).unwrap(), 0x7777_7777);
        assert_eq!(machine.memory.read_u32(DATA + 4).unwrap(), 0x8888_8888);
    }

    #[test]
    fn branch_link_and_state_switch() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                0xE92D_4000,           // STMDB sp!, {lr}
                branch(0xF, false, 1), // BLX to pc+8+4 = entry+16, Thumb state
                0xE8BD_8000,           // LDMIA sp!, {pc}
            ],
        );
        machine.memory.write_u16(CODE + 0x100 + 16, 0x2021).unwrap(); // MOVS r0, #0x21
        machine.memory.write_u16(CODE + 0x100 + 18, 0x4770).unwrap(); // BX lr
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 0x21);
    }

    fn branch(cond: u8, link: bool, offset_words: i32) -> u32 {
        ((cond as u32) << 28)
            | (5 << 25)
            | ((link as u32) << 24)
            | ((offset_words as u32) & 0x00FF_FFFF)
    }

    #[test]
    fn blx_register_interworks() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                0xE92D_4000, // STMDB sp!, {lr}
                0xE12F_FF30, // BLX r0
                0xE8BD_8000, // LDMIA sp!, {pc}
            ],
        );
        machine.memory.write_u16(CODE + 0x200, 0x2044).unwrap(); // MOVS r0, #0x44
        machine.memory.write_u16(CODE + 0x202, 0x4770).unwrap(); // BX lr
        let result = machine
            .call_function(&mut host, entry, &[(CODE + 0x200) | 1])
            .unwrap();
        assert_eq!(result, 0x44);
    }

    #[test]
    fn pc_relative_literal_load() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                ldr_literal(AL, 0, 0), // LDR r0, [pc, #0] -> word at entry+8
                0xE1A0_0000,           // NOP
                DATA,                  // literal
            ],
        );
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, DATA);
    }

    #[test]
    fn thumb_loop_sum() {
        let (mut machine, mut host) = machine();
        let entry = thumb_code(
            &mut machine,
            &[
                0x2100, // MOVS r1, #0
                0x2201, // MOVS r2, #1
                0x4282, // CMP r2, r0
                0xD802, // BHI done
                0x1889, // ADDS r1, r1, r2
                0x1C52, // ADDS r2, r2, #1
                0xE7FA, // B loop (back to CMP)
                0x1C08, // done: ADDS r0, r1, #0
            ],
        );
        let result = machine.call_function(&mut host, entry, &[10]).unwrap();
        assert_eq!(result, 55);
    }

    #[test]
    fn thumb_stack_and_return() {
        let (mut machine, mut host) = machine();
        let entry = thumb_code(&mut machine, &[0xB410, 0x2011, 0xBC10]); // PUSH/MOVS/POP
        let sp_before = STACK + 0xF_0000;
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 0x11);
        assert_eq!(machine.cpu.r[13], sp_before);
        let entry = thumb_code(&mut machine, &[0xB500, 0x2033, 0xBD00]); // PUSH/MOVS/POP {pc}
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 0x33);
    }

    #[test]
    fn thumb_bl_pair_links_and_calls() {
        let (mut machine, mut host) = machine();
        machine.memory.write_u16(CODE + 0x100, 0xB500).unwrap(); // PUSH {lr}
        machine.memory.write_u16(CODE + 0x102, 0x2007).unwrap(); // MOVS r0, #7
        let displacement = (CODE + 0x110) as i32 - (CODE + 0x104 + 4) as i32;
        let imm = ((displacement >> 1) as u32) & 0x3F_FFFF;
        machine
            .memory
            .write_u16(CODE + 0x104, (0xF000 | ((imm >> 11) & 0x7FF)) as u16)
            .unwrap();
        machine
            .memory
            .write_u16(CODE + 0x106, (0xF800 | (imm & 0x7FF)) as u16)
            .unwrap();
        machine.memory.write_u16(CODE + 0x108, 0x1C40).unwrap(); // ADDS r0, r0, #1
        machine.memory.write_u16(CODE + 0x10A, 0xBD00).unwrap(); // POP {pc}
        machine.memory.write_u16(CODE + 0x110, 0x0000).unwrap();
        machine.memory.write_u16(CODE + 0x112, 0x4770).unwrap();
        let entry = (CODE + 0x100) | 1;
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 8, "execution should resume after the BL pair");
    }

    #[test]
    fn host_shim_call_from_thumb_code() {
        let (mut machine, mut host) = machine();
        machine.linker.register_host("memset");
        let memset_address = machine.linker.resolve("memset").expect("memset bound");
        machine.memory.write_u16(CODE + 0x100, 0x4802).unwrap(); // LDR r0, [pc, #8]
        machine.memory.write_u16(CODE + 0x102, 0x21FF).unwrap(); // MOVS r1, #0xFF
        machine.memory.write_u16(CODE + 0x104, 0x2203).unwrap(); // MOVS r2, #3
        machine.memory.write_u16(CODE + 0x106, 0x4B02).unwrap(); // LDR r3, [pc, #8]
        machine.memory.write_u16(CODE + 0x108, 0x4798).unwrap(); // BLX r3
        machine.memory.write_u16(CODE + 0x10A, 0x4770).unwrap(); // BX lr
        machine.memory.write_u32(CODE + 0x10C, DATA).unwrap();
        machine
            .memory
            .write_u32(CODE + 0x110, memset_address)
            .unwrap();
        let entry = (CODE + 0x100) | 1;
        machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(
            machine.memory.read_u32(DATA).unwrap(),
            0x00FF_FFFF,
            "memset should have written three 0xFF bytes plus one old byte"
        );
        assert!(!host.log.iter().any(|line| line.contains("not implemented")));
    }

    #[test]
    fn pc_reads_use_architectural_offsets() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(
            &mut machine,
            &[
                dpi(AL, 4, false, 15, 0, 0, 0), // ADD r0, pc, #0
                0xE1A0_0000,                    // NOP
                0xE1A0_0000,                    // NOP
            ],
        );
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, CODE + 0x100 + 8);
    }

    #[test]
    fn ldr_pc_pre_indexed_writeback() {
        let (mut machine, mut host) = machine();
        // LDR pc, [ip, #0x4F8]! — the slot holds the landing address, and the
        // landing site returns the post-writeback ip in r0.
        let entry = arm_code(&mut machine, &[0xE5BC_F4F8]);
        machine.cpu.r[12] = CODE + 0x2000 - 0x4F8;
        machine
            .memory
            .write_u32(CODE + 0x2000, CODE + 0x3000)
            .unwrap();
        machine
            .memory
            .write_u32(CODE + 0x3000, 0xE1A0_000C)
            .unwrap(); // MOV r0, ip
        machine
            .memory
            .write_u32(CODE + 0x3004, 0xE12F_FF1E)
            .unwrap(); // BX lr
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, CODE + 0x2000, "writeback must update ip");
    }

    #[test]
    fn ldr_word_offset_matches_arm_reference() {
        let (mut machine, mut host) = machine();
        // LDR r2, [ip, #0x4F8]; MOV r0, r2
        let entry = arm_code(&mut machine, &[0xE59C_24F8, 0xE1A0_0002]);
        machine.cpu.r[12] = DATA;
        machine.memory.write_u32(DATA + 0x4F8, 0xABCD_1234).unwrap();
        let result = machine.call_function(&mut host, entry, &[]).unwrap();
        assert_eq!(result, 0xABCD_1234);
    }

    #[test]
    fn unsupported_instruction_stops_visibly() {
        let (mut machine, mut host) = machine();
        let entry = arm_code(&mut machine, &[0xEE11_0F10]); // MRC p15, ...
        let error = machine
            .call_function(&mut host, entry, &[])
            .expect_err("coprocessor instruction should be rejected");
        assert!(error.contains("coprocessor"), "unexpected error: {error}");
    }
}

#[cfg(test)]
mod probe_format5 {
    use super::*;
    use crate::host::BasicHost;

    #[test]
    fn ldr_register_offset_58cb() {
        let mut machine = Machine::new(CpuConfig::default());
        machine.memory.map_anon(0x1000_0000, 0x1000).unwrap();
        machine.memory.map_anon(0x2000_0000, 0x1000).unwrap();
        machine.memory.map_anon(0x7F00_0000, 0x10000).unwrap();
        machine.cpu.r[13] = 0x7F0F_F000;
        let mut host = BasicHost::new();
        // 0x58CB should be: ldr r3, [r1, r3]
        machine.memory.write_u16(0x1000_0000, 0x58CB).unwrap();
        machine.memory.write_u16(0x1000_0002, 0x4770).unwrap(); // bx lr
        machine
            .memory
            .write_u32(0x2000_0100 + 0x2EC, 0x7000_02EC)
            .unwrap();
        machine.cpu.r[1] = 0x2000_0100;
        machine.cpu.r[2] = 0xDEAD_BEEF; // must not be the base
        machine.cpu.r[3] = 0x2EC;
        machine.cpu.flags.thumb = true;
        machine.cpu.r[15] = 0x1000_0000;
        machine.cpu.r[14] = RETURN_SENTINEL;
        machine.step_once(&mut host).unwrap();
        eprintln!(
            "after 0x58CB: r3={:#x} (expect 0x700002ec if base=r1)",
            machine.cpu.r[3]
        );
        assert_eq!(machine.cpu.r[3], 0x7000_02EC, "base must be r1, not r2");
    }
}
