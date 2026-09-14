//! Linux syscall layer for the ARM interpreter.
//!
//! Android-era bionic and native engines invoke the kernel directly through
//! SVC with the EABI convention: syscall number in r7, arguments in r0-r6,
//! negative-errno result in r0. This module implements the common surface
//! (process identity, time, memory management, stdio) on top of the guest
//! memory map, and fails closed for anything dangerous (file I/O, network,
//! signals are stubbed or refused with visible diagnostics).

use crate::arm::{HostBridge, Machine};
use crate::mem::Permissions;

/// Sentinel returned by mmap syscalls on failure (Linux convention).
pub const MAP_FAILED: u32 = 0xFFFF_F000;

/// Anonymous mapping arena for `mmap2`/`mmap` syscalls.
const SYS_MMAP_BASE: u32 = 0x4400_0000;
const SYS_MMAP_SIZE: u32 = 0x0800_0000; // 128 MiB
/// Program break arena for `brk`.
const SYS_BRK_BASE: u32 = 0x4C00_0000;
const SYS_BRK_SIZE: u32 = 0x0400_0000; // 64 MiB
const PAGE: u32 = 0x1000;

const EBADF: i32 = 9;
const ENOENT: i32 = 2;
const ENOMEM: i32 = 12;
const EACCES: i32 = 13;
const EINVAL: i32 = 22;
const ENOTTY: i32 = 25;
const ENOSYS: i32 = 38;

const MAP_FIXED: u32 = 0x10;
const MAP_ANONYMOUS: u32 = 0x20;

/// Persistent syscall layer state (program break, mmap bump pointer).
#[derive(Debug, Clone, Default)]
pub struct SyscallState {
    /// Current program break; 0 until the first brk call.
    pub brk: u32,
    /// Extent of the brk arena currently backed by mapped pages.
    brk_mapped: u32,
    /// Next free address in the mmap arena; 0 until the first mmap.
    pub mmap_next: u32,
}

fn errno(code: i32) -> u32 {
    (code as u32).wrapping_neg()
}

fn align_up(value: u32) -> u32 {
    value.next_multiple_of(PAGE)
}

fn now_epoch() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as u32)
        .unwrap_or(0)
}

fn write_u64_words(memory: &mut crate::mem::Memory, address: u32, low: u32, high: u32) {
    let _ = memory.write_u32(address, low);
    let _ = memory.write_u32(address.wrapping_add(4), high);
}

/// Writes a NUL-padded fixed-size string field (utsname layout).
fn write_field(memory: &mut crate::mem::Memory, address: u32, text: &str, size: usize) {
    let mut buffer = vec![0u8; size];
    let bytes = text.as_bytes();
    let copied = bytes.len().min(size - 1);
    buffer[..copied].copy_from_slice(&bytes[..copied]);
    let _ = memory.write_bytes(address, &buffer);
}

const UTS_FIELD: usize = 65;

/// Dispatches an ARM EABI syscall. Number arrives in r7, arguments in r0-r6;
/// the return value becomes the new r0.
pub fn dispatch(host: &mut dyn HostBridge, machine: &mut Machine, number: u32) -> u32 {
    let args: [u32; 7] = core::array::from_fn(|index| machine.cpu.r[index]);

    match number {
        0 => 0, // restart_syscall: treat as completed
        1 | 248 => {
            // exit / exit_group
            machine.emulated_exit(args[0] as i32);
            0
        }
        2 | 190 => errno(ENOSYS), // fork / vfork: single-threaded machine

        // ---- stdio ----
        4 => {
            let (fd, address, count) = (args[0], args[1], args[2]);
            if fd == 1 || fd == 2 {
                let text = machine
                    .memory
                    .read_bytes(address, count as usize)
                    .unwrap_or_default();
                let label = if fd == 1 { "stdout" } else { "stderr" };
                host.diagnostic(format!("{label}: {}", String::from_utf8_lossy(&text)));
                count
            } else {
                errno(EBADF)
            }
        }
        146 => {
            // writev: log the iovec contents the same way as write(2).
            let (fd, iov, iovcnt) = (args[0], args[1], args[2]);
            if fd != 1 && fd != 2 {
                return errno(EBADF);
            }
            let mut total = 0u32;
            for index in 0..iovcnt.min(1024) {
                let entry = iov.wrapping_add(index * 8);
                let pointer = machine.memory.read_u32(entry).unwrap_or(0);
                let length = machine.memory.read_u32(entry.wrapping_add(4)).unwrap_or(0);
                let text = machine
                    .memory
                    .read_bytes(pointer, length as usize)
                    .unwrap_or_default();
                host.diagnostic(format!("stdout: {}", String::from_utf8_lossy(&text)));
                total = total.wrapping_add(length);
            }
            total
        }
        3 => errno(EBADF), // read: no backing files yet
        6 => 0,            // close
        5 => {
            host.diagnostic(format!(
                "open(\"{:#x}\") refused: file I/O disabled",
                args[0]
            ));
            errno(EACCES)
        }
        322 => {
            host.diagnostic("openat refused: file I/O disabled".to_owned());
            errno(EACCES)
        }

        // ---- process identity ----
        20 | 224 => 42,                // getpid / gettid
        64 => 1,                       // getppid
        24 | 199 | 47 | 200 => 10_000, // getuid*/getgid*
        49 | 201 | 50 | 202 => 10_000, // geteuid*/getegid*
        256 => 42,                     // set_tid_address: return this tid
        283 => {
            // set_thread_area
            machine.cpu.tls = args[0];
            0
        }
        0x00F0_0005 => {
            // ARM private: set_tls (bionic < 2.9)
            machine.cpu.tls = args[0];
            0
        }
        0x00F0_0002 => 0, // ARM private: cacheflush (no I-cache model)

        // ---- time ----
        13 => {
            let now = now_epoch();
            if args[0] != 0 {
                let _ = machine.memory.write_u32(args[0], now);
            }
            now
        }
        78 => {
            let now = now_epoch();
            if args[0] != 0 {
                write_u64_words(&mut machine.memory, args[0], now, 0);
            }
            if args[1] != 0 {
                let _ = machine.memory.write_u64(args[1], 0);
            }
            0
        }
        263 => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            if args[1] != 0 {
                write_u64_words(
                    &mut machine.memory,
                    args[1],
                    now.as_secs() as u32,
                    now.subsec_nanos(),
                );
            }
            0
        }
        264 => {
            if args[1] != 0 {
                write_u64_words(&mut machine.memory, args[1], 0, 1);
            }
            0
        }
        162 => 0, // nanosleep: instant

        // ---- memory management ----
        45 => {
            // brk
            let requested = args[0];
            if machine.syscalls.brk == 0 {
                machine.syscalls.brk = SYS_BRK_BASE;
                machine.syscalls.brk_mapped = SYS_BRK_BASE;
                let _ = machine.memory.map_anon(SYS_BRK_BASE, SYS_BRK_SIZE);
            }
            if requested == 0 {
                return machine.syscalls.brk;
            }
            if !(SYS_BRK_BASE..=SYS_BRK_BASE + SYS_BRK_SIZE).contains(&requested) {
                return machine.syscalls.brk;
            }
            if requested > machine.syscalls.brk {
                let to = align_up(requested);
                if to > machine.syscalls.brk_mapped {
                    let _ = machine.memory.map_anon(
                        machine.syscalls.brk_mapped,
                        to - machine.syscalls.brk_mapped,
                    );
                    machine.syscalls.brk_mapped = to;
                }
            }
            machine.syscalls.brk = requested;
            machine.syscalls.brk
        }
        192 | 90 => {
            // mmap2 / mmap (anonymous mappings only)
            let length = align_up(args[1].max(1));
            let prot = args[2];
            let flags = args[3];
            let fd = args[4] as i32;
            let anonymous = fd == -1 || flags & MAP_ANONYMOUS != 0;
            if !anonymous {
                host.diagnostic("mmap of file descriptors is not supported".to_owned());
                return errno(EBADF);
            }
            if length > SYS_MMAP_SIZE {
                host.diagnostic(format!("mmap of {length:#x} bytes exceeds the arena"));
                return errno(ENOMEM);
            }
            let perms = Permissions::from_prot(prot);
            if args[0] != 0 && flags & MAP_FIXED != 0 {
                if machine.memory.map(args[0], length, perms).is_err() {
                    return errno(ENOMEM);
                }
                return args[0];
            }
            if machine.syscalls.mmap_next == 0 {
                machine.syscalls.mmap_next = SYS_MMAP_BASE;
            }
            if machine.syscalls.mmap_next + length > SYS_MMAP_BASE + SYS_MMAP_SIZE {
                host.diagnostic("mmap arena exhausted".to_owned());
                return errno(ENOMEM);
            }
            let address = machine.syscalls.mmap_next;
            machine.syscalls.mmap_next += length;
            if machine.memory.map(address, length, perms).is_err() {
                return errno(ENOMEM);
            }
            address
        }
        91 => {
            // munmap
            let base = align_up(args[0]);
            let length = align_up(args[1].max(1));
            match machine.memory.unmap(base, length) {
                Ok(()) => 0,
                Err(_) => errno(EINVAL),
            }
        }
        125 => {
            // mprotect
            let base = align_up(args[0]);
            let length = align_up(args[1].max(1));
            match machine
                .memory
                .mprotect(base, length, Permissions::from_prot(args[2]))
            {
                Ok(()) => 0,
                Err(_) => errno(EINVAL),
            }
        }
        163 => {
            // mremap: only moves are supported (MREMAP_MAYMOVE).
            let flags = args[3];
            if flags & 1 == 0 {
                return errno(ENOMEM);
            }
            let new_length = align_up(args[2].max(1));
            let old_length = align_up(args[1].max(1));
            let old_address = args[0];
            if machine.syscalls.mmap_next == 0 {
                machine.syscalls.mmap_next = SYS_MMAP_BASE;
            }
            if machine.syscalls.mmap_next + new_length > SYS_MMAP_BASE + SYS_MMAP_SIZE {
                return errno(ENOMEM);
            }
            let address = machine.syscalls.mmap_next;
            machine.syscalls.mmap_next += new_length;
            let _ = machine.memory.map(address, new_length, Permissions::RWX);
            let _ = machine
                .memory
                .copy_within(address, old_address, old_length.min(new_length));
            let _ = machine.memory.unmap(old_address, old_length);
            address
        }
        220 => 0, // madvise

        // ---- system metadata ----
        122 => {
            // uname
            let buffer = args[0];
            write_field(&mut machine.memory, buffer, "Linux", UTS_FIELD);
            write_field(&mut machine.memory, buffer + 65, "donuthle", UTS_FIELD);
            write_field(
                &mut machine.memory,
                buffer + 130,
                "2.6.29-donuthle",
                UTS_FIELD,
            );
            write_field(&mut machine.memory, buffer + 195, "#1 SMP", UTS_FIELD);
            write_field(&mut machine.memory, buffer + 260, "armv7l", UTS_FIELD);
            write_field(&mut machine.memory, buffer + 325, "", UTS_FIELD);
            0
        }
        116 => {
            // sysinfo: zeroed structure (no swap, no load model)
            if args[0] != 0 {
                let _ = machine.memory.fill(args[0], 0, 64);
            }
            0
        }
        76 | 191 => {
            // getrlimit / ugetrlimit: RLIM_INFINITY for both fields
            if args[1] != 0 {
                write_u64_words(&mut machine.memory, args[1], 0xFFFF_FFFF, 0xFFFF_FFFF);
            }
            0
        }
        75 => 0, // setrlimit

        // ---- signals: pass-through (nothing is delivered) ----
        67 | 126 | 174 | 175 => 0,
        240 => 0, // futex: no waiters are ever woken, no waits block

        // ---- descriptor metadata: succeed vacuously ----
        108 | 197 => {
            // fstat / fstat64: zeroed stat buffer
            if args[1] != 0 {
                let _ = machine.memory.fill(args[1], 0, 96);
            }
            0
        }
        327 => 0,       // fstatat64
        141 | 217 => 0, // getdents / getdents64: empty directory
        140 => {
            // llseek: position 0
            if args[3] != 0 {
                write_u64_words(&mut machine.memory, args[3], 0, 0);
            }
            0
        }
        180 | 181 => 0,      // pread64 / pwrite64
        55 | 221 => 0,       // fcntl / fcntl64
        54 => errno(ENOTTY), // ioctl
        168 => 0,            // poll: nothing is ever ready
        85 => errno(ENOENT), // readlink
        33 => errno(ENOENT), // access
        136 => 0,            // personality
        153 => 0,            // prctl

        other => {
            host.diagnostic(format!("syscall {other} is not implemented (fail-closed)"));
            errno(ENOSYS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arm::{CpuConfig, Machine};
    use crate::host::BasicHost;

    struct TestHost(BasicHost);
    impl HostBridge for TestHost {
        fn syscall(&mut self, machine: &mut Machine, number: u32) -> u32 {
            crate::syscalls::dispatch(self, machine, number)
        }

        fn call_host(&mut self, machine: &mut Machine, slot: usize) -> u32 {
            self.0.call_host(machine, slot)
        }
        fn diagnostic(&mut self, message: String) {
            self.0.diagnostic(message)
        }
    }

    fn machine() -> Machine {
        Machine::new(CpuConfig::default())
    }

    /// Runs `code` at a scratch address with r7/args preset, returns r0.
    #[allow(clippy::assertions_on_constants)]
    fn pad5(v: [u32; 4]) -> [u32; 5] {
        let mut out = [0u32; 5];
        out[..4].copy_from_slice(&v);
        out
    }

    fn run_syscall(number: u32, args: [u32; 5]) -> (u32, Machine) {
        let mut machine = machine();
        machine.memory.map_anon(0x1000_0000, 0x1_0000).unwrap();
        // svc #0 encoded at the scratch address; return to the sentinel.
        machine.memory.write_u32(0x1000_0000, 0xEF00_0000).unwrap();
        machine.memory.write_u32(0x1000_0004, 0xE1A0_F00E).unwrap(); // mov pc, lr
        machine.cpu.r[7] = number;
        for (index, value) in args.iter().enumerate() {
            machine.cpu.r[index] = *value;
        }
        machine.cpu.r[14] = crate::arm::RETURN_SENTINEL;
        machine.cpu.r[15] = 0x1000_0000;
        let mut host = TestHost(BasicHost::new());
        let _ = machine.run(&mut host);
        (machine.cpu.r[0], machine)
    }

    #[test]
    fn getpid_and_exit() {
        let (result, _machine) = run_syscall(20, [0; 5]);
        assert_eq!(result, 42);
        let (result, _) = run_syscall(1, pad5([7, 0, 0, 0]));
        assert_eq!(result, 0);
    }

    #[test]
    fn brk_grows_and_reports() {
        let (initial, mut machine) = run_syscall(45, pad5([0; 4]));
        assert_eq!(initial, SYS_BRK_BASE);
        // Reuse the mapped machine for a second call with a real break.
        machine.memory.write_u32(0x1000_0000, 0xEF00_0000).unwrap();
        machine.cpu.r[7] = 45;
        machine.cpu.r[0] = SYS_BRK_BASE + 0x3000;
        machine.cpu.r[15] = 0x1000_0000;
        let mut host = TestHost(BasicHost::new());
        let _ = machine.run(&mut host);
        assert_eq!(machine.cpu.r[0], SYS_BRK_BASE + 0x3000);
        // The grown area must be writable.
        machine
            .memory
            .write_u32(SYS_BRK_BASE + 0x2000, 0x1234)
            .unwrap();
    }

    #[test]
    fn mmap2_maps_readable_memory() {
        let (address, mut machine) = run_syscall(192, [0, 0x2000, 3, 0x20 | 0x800, (-1i32) as u32]);
        assert_ne!(address, MAP_FAILED);
        assert_ne!(address, 0);
        machine.memory.write_u32(address, 0xABCD).unwrap();
        assert_eq!(machine.memory.read_u32(address).unwrap(), 0xABCD);
    }

    #[test]
    fn unknown_syscall_returns_enosys() {
        let (result, _) = run_syscall(9999, pad5([0; 4]));
        assert_eq!(result, (-38i32) as u32);
    }

    #[test]
    fn uname_writes_utsname() {
        let (result, machine) = run_syscall(122, pad5([0x1000_0100, 0, 0, 0]));
        assert_eq!(result, 0);
        let name = machine.memory.read_cstr(0x1000_0100, 65).unwrap();
        assert_eq!(name, "Linux");
        let machine_name = machine.memory.read_cstr(0x1000_0100 + 65, 65).unwrap();
        assert_eq!(machine_name, "donuthle");
    }
}
