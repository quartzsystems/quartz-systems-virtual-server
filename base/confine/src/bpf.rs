// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A seccomp filter as classic BPF, built from a list of rules: the
//! architecture checked first (a call of another ABI is refused, x32's
//! numbers included), then each allowed system call, some only with an
//! argument among a few values (an `ioctl`'s request, a signal's
//! target). Everything else gets the default action. Every jump is
//! short (within 255 instructions), so the program's length is not
//! bounded by its jumps.

/// `struct sock_filter`, as the kernel reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insn {
    /// The operation.
    pub code: u16,
    /// Where to go when true, after the next instruction.
    pub jt: u8,
    /// Where to go when false.
    pub jf: u8,
    /// The constant.
    pub k: u32,
}

const LD_W_ABS: u16 = 0x20;
const JEQ_K: u16 = 0x15;
const JGE_K: u16 = 0x35;
const RET_K: u16 = 0x06;

/// `AUDIT_ARCH_X86_64`.
pub const ARCH_X86_64: u32 = 0xC000_003E;
/// The first number of the x32 ABI.
const X32_BIT: u32 = 0x4000_0000;

/// `SECCOMP_RET_ALLOW`.
pub const ALLOW: u32 = 0x7FFF_0000;
/// `SECCOMP_RET_KILL_PROCESS`.
pub const KILL_PROCESS: u32 = 0x8000_0000;
/// `SECCOMP_RET_TRAP`: `SIGSYS` to the thread.
pub const TRAP: u32 = 0x0003_0000;

/// Offsets in `struct seccomp_data`.
const NR: u32 = 0;
const ARCH: u32 = 4;
/// The low half of argument `i`.
fn arg_low(i: u8) -> u32 {
    16 + 8 * u32::from(i)
}

/// One allowed system call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The call, with any arguments.
    Any(u32),
    /// The call when the low half of argument `.1` is one of `.2`.
    When(u32, u8, Vec<u32>),
}

impl Rule {
    fn nr(&self) -> u32 {
        match self {
            Rule::Any(nr) | Rule::When(nr, ..) => *nr,
        }
    }
}

/// Why a program could not be built.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// A rule had more values than a short jump can pass over.
    TooManyValues(u32),
    /// A call was named twice.
    Twice(u32),
    /// The program is longer than the kernel takes (`BPF_MAXINSNS`).
    TooLong(usize),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TooManyValues(nr) => write!(f, "system call {nr} has too many values"),
            Error::Twice(nr) => write!(f, "system call {nr} is named twice"),
            Error::TooLong(n) => write!(f, "{n} instructions are too many"),
        }
    }
}

fn insn(code: u16, jt: u8, jf: u8, k: u32) -> Insn {
    Insn { code, jt, jf, k }
}

/// The most instructions the kernel takes.
const MAX_INSNS: usize = 4096;

/// The program for `rules`, with `default` for every other call.
pub fn program(rules: &[Rule], default: u32) -> Result<Vec<Insn>, Error> {
    let mut seen: Vec<u32> = Vec::new();
    for r in rules {
        if seen.contains(&r.nr()) {
            return Err(Error::Twice(r.nr()));
        }
        seen.push(r.nr());
    }
    let mut out = vec![
        insn(LD_W_ABS, 0, 0, ARCH),
        insn(JEQ_K, 1, 0, ARCH_X86_64),
        insn(RET_K, 0, 0, KILL_PROCESS),
        insn(LD_W_ABS, 0, 0, NR),
        insn(JGE_K, 0, 1, X32_BIT),
        insn(RET_K, 0, 0, KILL_PROCESS),
    ];
    for r in rules {
        match r {
            Rule::Any(nr) => {
                out.push(insn(JEQ_K, 0, 1, *nr));
                out.push(insn(RET_K, 0, 0, ALLOW));
            }
            Rule::When(nr, arg, values) => {
                // The load, two for each value, and the default.
                let block = 1 + 2 * values.len() + 1;
                let skip = u8::try_from(block).map_err(|_| Error::TooManyValues(*nr))?;
                out.push(insn(JEQ_K, 0, skip, *nr));
                out.push(insn(LD_W_ABS, 0, 0, arg_low(*arg)));
                for v in values {
                    out.push(insn(JEQ_K, 0, 1, *v));
                    out.push(insn(RET_K, 0, 0, ALLOW));
                }
                out.push(insn(RET_K, 0, 0, default));
            }
        }
    }
    out.push(insn(RET_K, 0, 0, default));
    if out.len() > MAX_INSNS {
        return Err(Error::TooLong(out.len()));
    }
    Ok(out)
}

/// Run `program` on a call, as the kernel would: its action. For the
/// tests, and for the policy's own checks.
pub fn run(program: &[Insn], arch: u32, nr: u32, args: [u64; 6]) -> u32 {
    let mut data = [0u32; 16];
    data[0] = nr;
    data[1] = arch;
    for (i, a) in args.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)] // the two halves
        {
            data[4 + 2 * i] = *a as u32;
            data[5 + 2 * i] = (*a >> 32) as u32;
        }
    }
    let mut acc = 0u32;
    let mut pc = 0usize;
    while let Some(i) = program.get(pc) {
        pc += 1;
        match i.code {
            LD_W_ABS => acc = data.get(i.k as usize / 4).copied().unwrap_or(0),
            JEQ_K | JGE_K => {
                let hit = if i.code == JEQ_K {
                    acc == i.k
                } else {
                    acc >= i.k
                };
                pc += usize::from(if hit { i.jt } else { i.jf });
            }
            RET_K => return i.k,
            _ => return KILL_PROCESS,
        }
    }
    KILL_PROCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ: u32 = 0;
    const WRITE: u32 = 1;
    const IOCTL: u32 = 16;

    fn rules() -> Vec<Rule> {
        vec![
            Rule::Any(READ),
            Rule::When(IOCTL, 1, vec![0xAE80, 0x4004_AE86]),
            Rule::Any(WRITE),
        ]
    }

    #[test]
    fn the_rules_allow_what_they_name_and_nothing_else() {
        let p = program(&rules(), KILL_PROCESS).unwrap();
        let call = |nr, a1| run(&p, ARCH_X86_64, nr, [3, a1, 0, 0, 0, 0]);
        assert_eq!(call(READ, 0), ALLOW);
        assert_eq!(call(WRITE, 0), ALLOW);
        assert_eq!(call(IOCTL, 0xAE80), ALLOW);
        assert_eq!(call(IOCTL, 0x4004_AE86), ALLOW);
        // Only the low half of the request counts, as the kernel's does.
        assert_eq!(call(IOCTL, 0x1_0000_AE80), ALLOW);
        assert_eq!(call(IOCTL, 0x5401), KILL_PROCESS);
        assert_eq!(call(59, 0), KILL_PROCESS);
        assert_eq!(call(2, 0), KILL_PROCESS);
    }

    #[test]
    fn another_abi_is_refused_whatever_it_names() {
        let p = program(&rules(), TRAP).unwrap();
        assert_eq!(run(&p, 0x4000_0003, READ, [0; 6]), KILL_PROCESS);
        assert_eq!(run(&p, ARCH_X86_64, READ | X32_BIT, [0; 6]), KILL_PROCESS);
        assert_eq!(run(&p, ARCH_X86_64, 59, [0; 6]), TRAP);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn an_instruction_is_the_kernel_s() {
        assert_eq!(size_of::<Insn>(), size_of::<libc::sock_filter>());
        assert_eq!(align_of::<Insn>(), align_of::<libc::sock_filter>());
    }

    #[test]
    fn a_rule_too_long_or_twice_is_refused() {
        let many = (0..200).collect();
        assert_eq!(
            program(&[Rule::When(IOCTL, 1, many)], KILL_PROCESS),
            Err(Error::TooManyValues(IOCTL))
        );
        assert_eq!(
            program(&[Rule::Any(READ), Rule::Any(READ)], KILL_PROCESS),
            Err(Error::Twice(READ))
        );
        assert!(program(&[Rule::When(IOCTL, 1, (0..120).collect())], KILL_PROCESS).is_ok());
    }
}
