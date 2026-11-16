// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The streams' far ends: the partition manager's console and dump
//! connections, and the pool service's upload. Each is read by a thread
//! of its own into a bounded channel, so a slow console holds back only
//! its own stream and the connection's loop never waits.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread;
use std::time::Duration;

use qsi::any::AnyEvent;
use qsi::host::console::{In, Out, Pieces};
use qsi::host::vres;
use qsisess::Note;

use crate::conn::pipe::{Flow, Opener, Pipe};
use crate::events::hub::{self, Shared};
use crate::families::{convert, down, vres as v};

/// How many pieces wait between a far end's thread and the session.
const QUEUE: usize = 8;

/// A pipe's thread's messages.
enum Piece {
    Bytes(Vec<u8>),
    End(Option<Note>),
}

/// The thread's side as a pipe's flow.
fn flow(rx: &Receiver<Piece>, ended: &mut bool) -> Flow {
    if *ended {
        return Flow::End(None);
    }
    match rx.try_recv() {
        Ok(Piece::Bytes(b)) => Flow::Bytes(b),
        Ok(Piece::End(why)) => {
            *ended = true;
            Flow::End(why)
        }
        Err(TryRecvError::Empty) => Flow::Idle,
        Err(TryRecvError::Disconnected) => {
            *ended = true;
            Flow::End(None)
        }
    }
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    let _ = thread::Builder::new().name(name.into()).spawn(f);
}

fn note(msgid: &str, text: impl Into<String>) -> Note {
    Note {
        msgid: msgid.into(),
        text: text.into(),
        help: String::new(),
    }
}

/// A partition's console: the manager's frames as QSI's pieces, the
/// console's pieces as its keys.
struct ConsolePipe {
    rx: Receiver<Piece>,
    ended: bool,
    to: partd::proto::ConsoleSession,
    pieces: Pieces,
}

impl Pipe for ConsolePipe {
    fn poll(&mut self) -> Flow {
        flow(&self.rx, &mut self.ended)
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), Note> {
        self.pieces.push(bytes);
        while let Some(piece) = self.pieces.next_piece() {
            let message = In::decode(&piece)
                .ok()
                .filter(|i| i.check().is_ok())
                .ok_or_else(|| note(qsi::msgid::BAD, "A console piece that does not read."))?;
            let local = match message {
                In::Key(key) => partd::proto::ConsoleIn::Key { key },
                In::Refresh => partd::proto::ConsoleIn::Refresh,
                In::Bytes(bytes) => partd::proto::ConsoleIn::Bytes { bytes },
                // A later version's: passed over.
                In::Other(_) => continue,
            };
            self.to
                .send(&local)
                .map_err(|e| note(qsi::host::host::msgid::SERVICE_DOWN, e.to_string()))?;
        }
        Ok(())
    }

    fn closed(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        if let Ok(s) = self.to.try_clone() {
            s.close();
        }
    }
}

fn console_reader(mut from: partd::proto::ConsoleSession, tx: &SyncSender<Piece>) {
    loop {
        let out = match from.next(Duration::from_secs(3600)) {
            Ok(o) => o,
            Err(ipc::WireError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
        };
        let (out, last) = match out {
            partd::proto::ConsoleOut::Frame { frame } => (Out::Frame(frame), false),
            partd::proto::ConsoleOut::Update { update } => (Out::Update(update), false),
            partd::proto::ConsoleOut::Serial { bytes } => (Out::Serial(bytes), false),
            partd::proto::ConsoleOut::Ended { why } => (Out::Ended(why), true),
        };
        let Ok(piece) = out.piece() else { continue };
        if tx.send(Piece::Bytes(piece)).is_err() || last {
            break;
        }
    }
    let _ = tx.send(Piece::End(None));
}

/// A dump's bytes, a piece at a time.
struct DumpPipe {
    rx: Receiver<Piece>,
    ended: bool,
}

impl Pipe for DumpPipe {
    fn poll(&mut self) -> Flow {
        flow(&self.rx, &mut self.ended)
    }
    fn put(&mut self, _: &[u8]) -> Result<(), Note> {
        Err(note(qsi::msgid::BAD, "A dump's stream takes no bytes."))
    }
    fn closed(&mut self) {}
}

/// A writer that hands what it is given to the session's thread.
struct Sender(SyncSender<Piece>);

impl Write for Sender {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(Piece::Bytes(buf.to_vec()))
            .map_err(|_| std::io::Error::other("the console went"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// An upload: the console's bytes to the pool service, counted; how it
/// ended is an event.
struct UploadPipe {
    to: Option<UnixStream>,
    left: u64,
    rx: Receiver<Piece>,
    ended: bool,
}

impl Pipe for UploadPipe {
    fn poll(&mut self) -> Flow {
        flow(&self.rx, &mut self.ended)
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), Note> {
        let n = bytes.len() as u64;
        if n > self.left {
            self.reset();
            return Err(note(
                vres::msgid::UPLOAD_LONG,
                "More bytes than the upload named.",
            ));
        }
        let to = self
            .to
            .as_mut()
            .ok_or_else(|| note(qsi::msgid::BAD, "The upload is over."))?;
        to.write_all(bytes)
            .map_err(|e| note(qsi::host::host::msgid::SERVICE_DOWN, e.to_string()))?;
        self.left -= n;
        Ok(())
    }

    fn closed(&mut self) {
        if self.left > 0 {
            // Fewer than were named: the pool service sees the end.
            self.reset();
        }
    }

    fn reset(&mut self) {
        if let Some(s) = self.to.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// The far ends, on the services' sockets.
#[derive(Clone, Debug)]
pub struct Sockets {
    /// The partition manager's.
    pub partd: PathBuf,
    /// The pool service's.
    pub poold: PathBuf,
    /// The hub an upload's end is said on.
    pub hub: Shared,
}

const PARTD: &str = "partition manager";
const POOLD: &str = "pool service";

fn now_secs() -> u64 {
    u64::try_from(appliance::time::now_secs()).unwrap_or(0)
}

impl Opener for Sockets {
    fn console(&mut self, id: u32, take_over: bool) -> Result<(bool, Box<dyn Pipe>), Vec<Note>> {
        let client = partd::Client::new(&self.partd);
        let (serial, session) = client
            .console(id, take_over)
            .map_err(|e| down(PARTD, &e))?
            .map_err(|issued| convert::notes(&issued))?;
        let reader = session.try_clone().map_err(|e| down(PARTD, &e.into()))?;
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        spawn("console", move || console_reader(reader, &tx));
        Ok((
            serial,
            Box::new(ConsolePipe {
                rx,
                ended: false,
                to: session,
                pieces: Pieces::default(),
            }),
        ))
    }

    fn dump(&mut self, id: u32, name: &str) -> Result<(u64, Box<dyn Pipe>), Vec<Note>> {
        // The size is asked first, so a refusal is a refusal and not a
        // stream that resets.
        let mut infos = partd::Client::new(&self.partd);
        let all = {
            use partd::Calls as _;
            infos.dumps(id)
        }
        .map_err(|e| down(PARTD, &e))?
        .map_err(|issued| convert::notes(&issued))?;
        let bytes = all
            .iter()
            .find(|d| d.name == name)
            .map(|d| d.bytes)
            .ok_or_else(|| convert::notes(&[qvsmsg::DUMP_MISSING.with(&[name])]))?;
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let client = partd::Client::new(&self.partd);
        let name = name.to_string();
        spawn("dump", move || {
            let mut out = Sender(tx.clone());
            let end = match client.read_dump_into(id, &name, &mut out) {
                Ok(Ok(_)) => None,
                Ok(Err(issued)) => convert::notes(&issued).into_iter().next(),
                Err(e) => down(PARTD, &e).into_iter().next(),
            };
            let _ = tx.send(Piece::End(end));
        });
        Ok((bytes, Box::new(DumpPipe { rx, ended: false })))
    }

    fn upload(&mut self, name: &str, bytes: u64) -> Result<Box<dyn Pipe>, Vec<Note>> {
        let mut stream = UnixStream::connect(&self.poold).map_err(|e| down(POOLD, &e.into()))?;
        let _ = stream.set_read_timeout(Some(poold::proto::ANSWER_WAIT));
        let request = poold::Request::Upload {
            name: name.to_string(),
            bytes,
        };
        ipc::frame::write_frame(&mut stream, &ipc::encode(&request))
            .map_err(|e| down(POOLD, &e))?;
        let answer = ipc::frame::read_frame_max(&mut stream, poold::proto::MAX_FRAME)
            .map_err(|e| down(POOLD, &e))?;
        let accepted: Result<(), poold::Refused> =
            ipc::decode(&answer).map_err(|e| down(POOLD, &e))?;
        accepted.map_err(|issued| convert::notes(&issued))?;
        let mut reader = stream.try_clone().map_err(|e| down(POOLD, &e.into()))?;
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let hub = self.hub.clone();
        let name = name.to_string();
        spawn("upload", move || {
            // The pool service's second answer, once every byte is in
            // or the connection ends.
            let _ = reader.set_read_timeout(None);
            let ended: Result<poold::Image, Note> =
                ipc::frame::read_frame_max(&mut reader, poold::proto::MAX_FRAME)
                    .map_err(|e| note(vres::msgid::UPLOAD_CUT, e.to_string()))
                    .and_then(|a| {
                        ipc::decode::<Result<poold::Image, poold::Refused>>(&a)
                            .map_err(|e| note(vres::msgid::UPLOAD_CUT, e.to_string()))
                    })
                    .and_then(|r| {
                        r.map_err(|issued| {
                            convert::notes(&issued)
                                .into_iter()
                                .next()
                                .unwrap_or_else(|| note(vres::msgid::UPLOAD_CUT, "refused"))
                        })
                    });
            let (image, error) = match &ended {
                Ok(i) => (Some(v::image(i)), None),
                Err(n) => (
                    None,
                    Some(qsi::host::model::Note {
                        msgid: n.msgid.clone(),
                        text: n.text.clone(),
                        help: n.help.clone(),
                    }),
                ),
            };
            let event = AnyEvent::Vres(vres::Event::Upload(vres::UploadEnded {
                name,
                image,
                error,
                time: now_secs(),
            }));
            let _ = hub::lock(&hub).push(&event);
            let _ = tx.send(Piece::End(ended.err()));
        });
        let _ = stream.set_read_timeout(None);
        Ok(Box::new(UploadPipe {
            to: Some(stream),
            left: bytes,
            rx,
            ended: false,
        }))
    }
}
