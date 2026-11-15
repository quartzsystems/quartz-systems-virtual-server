// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A dump's notes. Each is the ELF note form: the name's size, the
//! description's size and the type, four bytes each, then the name and
//! the description, each padded to four bytes.
//!
//! | Name | Type | Description |
//! |---|---|---|
//! | `CORE` | 1 (`NT_PRSTATUS`) | a processor: `elf_prstatus` of x86-64, its process number the processor's number plus 1 |
//! | `QSVS` | `0x51530002` | a processor's special registers: its number, then [`Cpu::SPECIAL`] in order, eight bytes each |
//! | `QSVS` | `0x51530001` | the header record: lines `key=value`, [`Header`]'s fields |

/// `NT_PRSTATUS`.
const NT_PRSTATUS: u32 = 1;
/// The header record's type.
const QSVS_HEADER: u32 = 0x5153_0001;
/// A processor's special registers.
const QSVS_SPECIAL: u32 = 0x5153_0002;
/// The size of `elf_prstatus` on x86-64, and where its registers are.
const PRSTATUS: usize = 336;
const PR_REG: usize = 112;
const PR_PID: usize = 32;
/// The most bytes of a header record's value.
const MAX_VALUE: usize = 256;

/// One processor's state when the processors stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)] // the registers' own names
pub struct Cpu {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cs: u64,
    pub ss: u64,
    pub ds: u64,
    pub es: u64,
    pub fs: u64,
    pub gs: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
    pub gdt_base: u64,
    pub idt_base: u64,
}

impl Cpu {
    /// The special registers' names, in the order the note has them.
    pub const SPECIAL: [&str; 7] = ["cr0", "cr2", "cr3", "cr4", "efer", "gdt_base", "idt_base"];

    /// The general registers as `user_regs_struct` orders them.
    fn general(&self) -> [u64; 27] {
        [
            self.r15,
            self.r14,
            self.r13,
            self.r12,
            self.rbp,
            self.rbx,
            self.r11,
            self.r10,
            self.r9,
            self.r8,
            self.rax,
            self.rcx,
            self.rdx,
            self.rsi,
            self.rdi,
            u64::MAX, // orig_rax: no system call
            self.rip,
            self.cs,
            self.rflags,
            self.rsp,
            self.ss,
            self.fs_base,
            self.gs_base,
            self.ds,
            self.es,
            self.fs,
            self.gs,
        ]
    }

    fn special(&self) -> [u64; 7] {
        [
            self.cr0,
            self.cr2,
            self.cr3,
            self.cr4,
            self.efer,
            self.gdt_base,
            self.idt_base,
        ]
    }
}

/// The header record: what the dump is of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    /// The partition's name.
    pub partition: String,
    /// Its ID.
    pub id: u32,
    /// Its serial number.
    pub serial: String,
    /// Its reference code when the dump was taken.
    pub code: String,
    /// The code's text.
    pub text: String,
    /// When, in seconds since the epoch.
    pub time: u64,
    /// The host's release.
    pub release: String,
    /// Why it was taken: the function, or a request.
    pub reason: String,
}

fn note(out: &mut Vec<u8>, name: &str, kind: u32, desc: &[u8]) {
    let name = format!("{name}\0");
    for v in [
        u32::try_from(name.len()).unwrap_or(0),
        u32::try_from(desc.len()).unwrap_or(0),
        kind,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(name.as_bytes());
    out.resize(out.len().next_multiple_of(4), 0);
    out.extend_from_slice(desc);
    out.resize(out.len().next_multiple_of(4), 0);
}

/// A value as a line holds it: no line breaks, bounded.
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_VALUE)
        .collect()
}

impl Header {
    fn record(&self) -> Vec<u8> {
        let mut s = String::new();
        for (k, v) in [
            ("partition", clean(&self.partition)),
            ("id", self.id.to_string()),
            ("serial", clean(&self.serial)),
            ("code", clean(&self.code)),
            ("text", clean(&self.text)),
            ("time", self.time.to_string()),
            ("release", clean(&self.release)),
            ("reason", clean(&self.reason)),
        ] {
            s.push_str(k);
            s.push('=');
            s.push_str(&v);
            s.push('\n');
        }
        s.into_bytes()
    }

    fn parse(bytes: &[u8]) -> Self {
        let mut h = Header::default();
        for line in String::from_utf8_lossy(bytes).lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = clean(v);
            match k {
                "partition" => h.partition = v,
                "id" => h.id = v.parse().unwrap_or(0),
                "serial" => h.serial = v,
                "code" => h.code = v,
                "text" => h.text = v,
                "time" => h.time = v.parse().unwrap_or(0),
                "release" => h.release = v,
                "reason" => h.reason = v,
                _ => {}
            }
        }
        h
    }
}

/// Every note of a dump.
pub(crate) fn all(header: &Header, cpus: &[Cpu]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, cpu) in (0u32..).zip(cpus) {
        let mut status = vec![0u8; PRSTATUS];
        status[PR_PID..PR_PID + 4].copy_from_slice(&(i + 1).to_le_bytes());
        for (r, value) in cpu.general().iter().enumerate() {
            let at = PR_REG + r * 8;
            status[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        note(&mut out, "CORE", NT_PRSTATUS, &status);
        let mut special = Vec::with_capacity(8 * 8);
        special.extend_from_slice(&u64::from(i).to_le_bytes());
        for v in cpu.special() {
            special.extend_from_slice(&v.to_le_bytes());
        }
        note(&mut out, "QSVS", QSVS_SPECIAL, &special);
    }
    note(&mut out, "QSVS", QSVS_HEADER, &header.record());
    out
}

/// The header record among `notes`, read within their bounds.
pub(crate) fn header_of(notes: &[u8]) -> Option<Header> {
    let mut at = 0usize;
    while at + 12 <= notes.len() {
        let word = |o: usize| -> Option<usize> {
            let b = notes.get(o..o + 4)?;
            usize::try_from(u32::from_le_bytes(b.try_into().ok()?)).ok()
        };
        let (name_len, desc_len, kind) = (word(at)?, word(at + 4)?, word(at + 8)?);
        let name_at = at + 12;
        let desc_at = name_at.checked_add(name_len.next_multiple_of(4))?;
        let next = desc_at.checked_add(desc_len.next_multiple_of(4))?;
        let name = notes.get(name_at..name_at + name_len)?;
        let desc = notes.get(desc_at..desc_at.checked_add(desc_len)?)?;
        if name == b"QSVS\0" && kind == QSVS_HEADER as usize {
            return Some(Header::parse(desc));
        }
        at = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notes_carry_each_processor_and_the_header() {
        let cpu = Cpu {
            rip: 0x1000,
            rsp: 0x2000,
            cr3: 0x3000,
            ..Cpu::default()
        };
        let mut header = crate::tests::header();
        header.text = "two\nlines".into();
        let notes = all(&header, &[cpu, cpu]);
        // Two processors, two notes each, then the header.
        let got = header_of(&notes).unwrap();
        assert_eq!(got.text, "two lines");
        assert_eq!(got.code, "B1001234");
        // The registers are where a reader of NT_PRSTATUS looks.
        let desc = 12 + 8;
        let rip = desc + PR_REG + 16 * 8;
        assert_eq!(notes[rip..rip + 8], 0x1000u64.to_le_bytes());
        assert_eq!(notes[desc + PR_PID..desc + PR_PID + 4], 1u32.to_le_bytes());
        // Notes cut short, or that lie about their sizes, give nothing.
        assert!(header_of(&notes[..notes.len() - 5]).is_none());
        let mut lie = notes.clone();
        lie[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(header_of(&lie).is_none());
        assert!(header_of(&[]).is_none());
    }
}
