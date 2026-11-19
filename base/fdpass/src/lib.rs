// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Open files handed from one process to another over a Unix stream
//! socket. The descriptors ride on one byte of their own, sent with
//! `SCM_RIGHTS` between the frames of whatever protocol the socket
//! carries: a frame is read to its exact length, so the byte that
//! carries them is never read by anything but [`recv`].
//!
//! The receiver takes at most the number it asked for; any more that
//! came are closed, and a byte that carries none gives an empty list.
//! More than one byte carries ride on several ([`send_all`]).

#![forbid(unsafe_code)]

use std::io::{self, IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
};

/// The most descriptors one byte carries.
pub const MAX_FDS: usize = 16;

/// Send `fds` on `stream`, on one byte.
pub fn send(stream: &UnixStream, fds: &[BorrowedFd<'_>]) -> io::Result<()> {
    if fds.len() > MAX_FDS {
        return Err(io::Error::other(format!(
            "{} descriptors; one byte carries {MAX_FDS}",
            fds.len()
        )));
    }
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() && !control.push(SendAncillaryMessage::ScmRights(fds)) {
        return Err(io::Error::other("the descriptors do not fit"));
    }
    let byte = [0u8];
    let sent = sendmsg(
        stream,
        &[IoSlice::new(&byte)],
        &mut control,
        SendFlags::empty(),
    )?;
    if sent == 1 {
        Ok(())
    } else {
        Err(io::Error::other("the byte was not sent"))
    }
}

/// Receive the byte [`send`] sent and the descriptors on it: at most
/// `max`.
pub fn recv(stream: &UnixStream, max: usize) -> io::Result<Vec<OwnedFd>> {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let mut byte = [0u8];
    let got = recvmsg(
        stream,
        &mut [IoSliceMut::new(&mut byte)],
        &mut control,
        RecvFlags::CMSG_CLOEXEC,
    )?;
    if got.bytes != 1 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the connection ended before the descriptors",
        ));
    }
    let mut fds = Vec::new();
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(received) = message {
            // Each one not kept is closed as it is dropped.
            for fd in received {
                if fds.len() < max {
                    fds.push(fd);
                }
            }
        }
    }
    Ok(fds)
}

/// The bytes `count` descriptors ride on: [`MAX_FDS`] a byte, and one
/// byte for none.
fn bytes_for(count: usize) -> usize {
    count.div_ceil(MAX_FDS).max(1)
}

/// Send `fds` on as many bytes as they need, [`MAX_FDS`] on each.
pub fn send_all(stream: &UnixStream, fds: &[BorrowedFd<'_>]) -> io::Result<()> {
    if fds.is_empty() {
        return send(stream, &[]);
    }
    for some in fds.chunks(MAX_FDS) {
        send(stream, some)?;
    }
    Ok(())
}

/// Receive the `count` descriptors [`send_all`] sent: fewer when the
/// sender sent fewer, never more.
pub fn recv_all(stream: &UnixStream, count: usize) -> io::Result<Vec<OwnedFd>> {
    let mut fds = Vec::new();
    for _ in 0..bytes_for(count) {
        let want = count.saturating_sub(fds.len()).min(MAX_FDS);
        fds.extend(recv(stream, want)?);
    }
    Ok(fds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::fd::AsFd;

    #[test]
    fn an_open_file_crosses_the_socket_between_frames() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let path = std::env::temp_dir().join(format!("fdpass-{}", std::process::id()));
        let mut file = File::create(&path).unwrap();
        file.write_all(b"the virtual disk").unwrap();
        // A frame before, the descriptor, a frame after.
        (&a).write_all(b"before").unwrap();
        send(&a, &[file.as_fd()]).unwrap();
        (&a).write_all(b"after").unwrap();
        let mut frame = [0u8; 6];
        b.read_exact(&mut frame).unwrap();
        assert_eq!(&frame, b"before");
        let fds = recv(&b, 4).unwrap();
        assert_eq!(fds.len(), 1);
        let mut frame = [0u8; 5];
        b.read_exact(&mut frame).unwrap();
        assert_eq!(&frame, b"after");
        // The same file, through the descriptor received.
        let mut got = File::from(fds.into_iter().next().unwrap());
        got.seek(SeekFrom::Start(0)).unwrap();
        let mut text = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "the virtual disk");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn none_or_too_many_are_taken_as_they_come() {
        let (a, b) = UnixStream::pair().unwrap();
        send(&a, &[]).unwrap();
        assert!(recv(&b, 4).unwrap().is_empty());
        let f = File::open("/dev/null").unwrap();
        send(&a, &[f.as_fd(), f.as_fd(), f.as_fd()]).unwrap();
        assert_eq!(recv(&b, 2).unwrap().len(), 2);
        let many: Vec<BorrowedFd<'_>> = (0..=MAX_FDS).map(|_| f.as_fd()).collect();
        assert!(send(&a, &many).is_err());
        drop(a);
        assert!(recv(&b, 1).is_err());
    }

    #[test]
    fn more_than_one_byte_carries_ride_on_several() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let f = File::open("/dev/null").unwrap();
        for count in [0, 1, MAX_FDS, MAX_FDS + 1, 69] {
            let many: Vec<BorrowedFd<'_>> = (0..count).map(|_| f.as_fd()).collect();
            send_all(&a, &many).unwrap();
            (&a).write_all(b"next").unwrap();
            assert_eq!(recv_all(&b, count).unwrap().len(), count);
            let mut frame = [0u8; 4];
            b.read_exact(&mut frame).unwrap();
            assert_eq!(&frame, b"next");
        }
    }
}
