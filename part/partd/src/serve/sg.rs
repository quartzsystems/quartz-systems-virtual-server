// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A physical tape drive through the kernel's SCSI generic driver
//! (`SG_IO` on `/dev/sgN`), and the relay that carries a partition's
//! commands to it (`docs/STORAGE.md`). The relay runs in the partition
//! manager, which has the rights `SG_IO` wants for a tape's commands;
//! the VMM has its far end and nothing else of the drive. Every command
//! is checked against the allow-list here, whatever the VMM checked.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::thread;

use tape::passthrough::{self as pt, Answer, Command, Drive, Gone};

/// `SG_IO`.
const SG_IO: libc::c_ulong = 0x2285;
/// No data.
const SG_DXFER_NONE: i32 = -1;
/// Data to the drive.
const SG_DXFER_TO_DEV: i32 = -2;
/// Data from the drive.
const SG_DXFER_FROM_DEV: i32 = -3;
/// The host could not reach the target, or the target is not there.
const DID_NO_CONNECT: u16 = 0x01;
const DID_BAD_TARGET: u16 = 0x04;
/// A tape's longest command (an erase of a whole cartridge) is hours.
const TIMEOUT_MS: u32 = 4 * 3600 * 1000;

/// `struct sg_io_hdr`.
#[repr(C)]
struct SgIoHdr {
    interface_id: i32,
    dxfer_direction: i32,
    cmd_len: u8,
    mx_sb_len: u8,
    iovec_count: u16,
    dxfer_len: u32,
    dxferp: *mut libc::c_void,
    cmdp: *const u8,
    sbp: *mut u8,
    timeout: u32,
    flags: u32,
    pack_id: i32,
    usr_ptr: *mut libc::c_void,
    status: u8,
    masked_status: u8,
    msg_status: u8,
    sb_len_wr: u8,
    host_status: u16,
    driver_status: u16,
    resid: i32,
    duration: u32,
    info: u32,
}

/// The commands that move no data, whatever room the guest has.
fn moves_nothing(op: u8) -> bool {
    matches!(
        op,
        0x00 | 0x01 | 0x10 | 0x11 | 0x16 | 0x17 | 0x19 | 0x1B | 0x1E | 0x2B | 0x80 | 0x91 | 0x92
    )
}

/// A drive at `/dev/sgN`.
pub struct SgDrive {
    file: File,
}

impl SgDrive {
    /// Open the drive's generic device `sg` (`sg2`).
    pub fn open(sg: &str) -> io::Result<Self> {
        if sg.is_empty() || !sg.starts_with("sg") || !sg[2..].bytes().all(|b| b.is_ascii_digit()) {
            return Err(io::Error::other(format!(
                "{sg:?} is not a SCSI generic device"
            )));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/dev/{sg}"))?;
        Ok(Self { file })
    }
}

impl Drive for SgDrive {
    fn execute(&mut self, command: &Command) -> Result<Answer, Gone> {
        let mut sense = [0u8; pt::MAX_SENSE];
        let (direction, mut buffer) = if !command.data_out.is_empty() {
            (SG_DXFER_TO_DEV, command.data_out.clone())
        } else if command.data_in == 0 || moves_nothing(command.cdb[0]) {
            (SG_DXFER_NONE, Vec::new())
        } else {
            (SG_DXFER_FROM_DEV, vec![0u8; command.data_in as usize])
        };
        let mut hdr = SgIoHdr {
            interface_id: i32::from(b'S'),
            dxfer_direction: direction,
            cmd_len: u8::try_from(command.cdb.len()).unwrap_or(16),
            mx_sb_len: u8::try_from(sense.len()).unwrap_or(0),
            iovec_count: 0,
            dxfer_len: u32::try_from(buffer.len()).unwrap_or(0),
            dxferp: if buffer.is_empty() {
                std::ptr::null_mut()
            } else {
                buffer.as_mut_ptr().cast()
            },
            cmdp: command.cdb.as_ptr(),
            sbp: sense.as_mut_ptr(),
            timeout: TIMEOUT_MS,
            flags: 0,
            pack_id: 0,
            usr_ptr: std::ptr::null_mut(),
            status: 0,
            masked_status: 0,
            msg_status: 0,
            sb_len_wr: 0,
            host_status: 0,
            driver_status: 0,
            resid: 0,
            duration: 0,
            info: 0,
        };
        // SAFETY: SG_IO reads the header and the command, sense and data
        // buffers it points to, each of the length the header says, and
        // writes the header, the sense buffer and (from the drive) the
        // data buffer; every buffer lives on this frame for the call.
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), SG_IO, &raw mut hdr) };
        if r < 0 {
            let e = io::Error::last_os_error();
            return Err(Gone(e.to_string()));
        }
        if matches!(hdr.host_status, DID_NO_CONNECT | DID_BAD_TARGET) {
            return Err(Gone(format!("host status {:#x}", hdr.host_status)));
        }
        let resid = u32::try_from(hdr.resid.max(0)).unwrap_or(0);
        let data = if direction == SG_DXFER_FROM_DEV {
            let got = buffer.len().saturating_sub(resid as usize);
            buffer.truncate(got);
            buffer
        } else {
            Vec::new()
        };
        Ok(Answer {
            status: hdr.status,
            sense: sense[..usize::from(hdr.sb_len_wr).min(sense.len())].to_vec(),
            data,
            resid,
            gone: false,
        })
    }
}

/// Whether a cartridge is in the drive: TEST UNIT READY answers GOOD.
pub fn loaded(sg: &str) -> bool {
    let Ok(mut drive) = SgDrive::open(sg) else {
        return false;
    };
    let tur = Command {
        cdb: vec![0; 6],
        data_out: Vec::new(),
        data_in: 0,
    };
    drive
        .execute(&tur)
        .is_ok_and(|a| a.status == pt::STATUS_GOOD)
}

/// What the relay says of a drive: posted through PID 1.
pub type Posts = Sender<(String, String, Vec<refcode::about::About>)>;

/// Relay the VMM's commands on `link` to `drive` until the VMM closes
/// it. A drive that goes away is posted once; each refusal is logged.
pub fn spawn_relay(
    mut link: UnixStream,
    mut drive: Box<dyn Drive>,
    location: String,
    posts: Posts,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name(format!("relay {location}"))
        .spawn(move || {
            let mut said_gone = false;
            while let Ok(Some(body)) = pt::read_frame(&mut link) {
                // Nonsense from the VMM ends the relay; its partition's
                // tape device fails from then on.
                let Some(command) = Command::decode(&body) else {
                    eprintln!("relay {location}: a frame that is no command");
                    break;
                };
                let (answer, reached) = pt::relay(drive.as_mut(), &command);
                if let Err(why) = reached {
                    eprintln!("relay {location}: refused: {}", why.words());
                }
                if answer.gone && !said_gone {
                    said_gone = true;
                    let code = &refcode::DRIVE_GONE;
                    let _ = posts.send((
                        code.code.to_string(),
                        code.text_with(&location),
                        vec![refcode::about::tape_drive(&location, "")],
                    ));
                }
                if link.write_all(&answer.encode()).is_err() {
                    break;
                }
            }
        })
}
