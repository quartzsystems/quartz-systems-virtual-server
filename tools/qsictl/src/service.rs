// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Servicing over QSI: the host's problems, its code update and its own
//! dumps (`docs/QSICTL.md`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::Subcommand;
use qsi::Empty;
use qsi::any::{AnyEvent, AnyReply, AnyRequest};
use qsi::{base, service, update};
use qsisess::Incoming;

use crate::link::Link;
use crate::pair::refusal;

/// `qsictl problems`.
#[derive(Subcommand, Debug)]
pub enum Problems {
    /// The problems, newest first.
    List {
        /// The open ones only.
        #[arg(long)]
        open: bool,
        /// How many.
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// One problem whole.
    Show {
        /// Its number.
        number: u64,
    },
    /// Close a problem.
    Close {
        /// Its number.
        number: u64,
        /// While its condition holds.
        #[arg(long)]
        anyway: bool,
    },
    /// Add a note to an open problem's analysis log.
    Note {
        /// Its number.
        number: u64,
        /// The note, one line.
        text: String,
    },
    /// The problems with their severity, newest first.
    Rated {
        /// How many.
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// One problem whole, with its severity.
    ShowRated {
        /// Its number.
        number: u64,
    },
}

/// `qsictl update`.
#[derive(Subcommand, Debug)]
pub enum Update {
    /// The slots, what is staged, what is being installed.
    Status,
    /// Stage an update image from a file, and wait for its verification.
    Stage {
        /// The image.
        file: PathBuf,
        /// An older release may be installed.
        #[arg(long)]
        allow_older: bool,
    },
    /// Install what is staged into the slot that does not run, and wait
    /// for it to be pending.
    Install,
    /// Restart into the pending slot.
    Activate {
        /// Shut the partitions down through their systems first.
        #[arg(long)]
        shut_down: bool,
        /// Seconds each partition is given.
        #[arg(long, default_value_t = 300)]
        timeout: u32,
    },
    /// The pending slot is pending no more.
    Cancel,
    /// Throw away what is staged.
    Discard,
}

/// `qsictl hostdump`.
#[derive(Subcommand, Debug)]
pub enum HostDump {
    /// Take one, and wait for it to be written.
    Take,
    /// The host's dumps, oldest first.
    List,
    /// Download one to a file.
    Fetch {
        /// Its name.
        name: String,
        /// Where to write it.
        file: PathBuf,
    },
    /// Delete one.
    Delete {
        /// Its name.
        name: String,
    },
    /// What the host's dumps take, and their limits.
    Space,
}

/// Ask `r`; its reply, or the refusal whole.
fn ask(link: &mut Link, r: &AnyRequest) -> Result<AnyReply, String> {
    let reply = link.ask(r)?;
    if reply.refusal().is_some() {
        return Err(refusal(&reply));
    }
    reply
        .decode()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "a reply this console does not read".to_string())
}

/// Subscribe, so an end said as an event is not missed.
fn subscribe(link: &mut Link) -> Result<(), String> {
    ask(
        link,
        &AnyRequest::Base(base::Request::Subscribe(base::Subscribe { since: None })),
    )
    .map(drop)
}

/// The events until `f` says done, for at most `wait`.
fn until<T>(
    link: &mut Link,
    wait: Duration,
    mut f: impl FnMut(&AnyEvent) -> Option<Result<T, String>>,
) -> Result<T, String> {
    let t0 = Instant::now();
    while t0.elapsed() < wait {
        let mut got = std::mem::take(&mut link.besides);
        got.extend(link.step()?);
        for i in got {
            if let Incoming::Event(e) = i
                && let Ok(Some(e)) = e.decode()
                && let Some(done) = f(&e)
            {
                return done;
            }
        }
    }
    Err("no answer in time".into())
}

fn note(n: Option<&qsi::host::model::Note>) -> String {
    n.map_or_else(|| "refused".into(), |n| format!("{} {}", n.msgid, n.text))
}

/// Run `qsictl problems`.
///
/// # Errors
/// The request was refused.
pub fn problems(link: &mut Link, p: &Problems) -> Result<(), String> {
    match p {
        Problems::Note { number, text } => {
            let r = ask(
                link,
                &AnyRequest::Service(service::Request::Note(service::Note {
                    number: *number,
                    text: text.clone(),
                })),
            )?;
            let AnyReply::Service(service::Reply::Note(written)) = r else {
                return Err(format!("{r:?}"));
            };
            println!("{}  {}", written.time, written.text);
            return Ok(());
        }
        Problems::Rated { limit } => {
            let r = ask(
                link,
                &AnyRequest::Service(service::Request::ListRated(service::List {
                    filter: service::Filter::default(),
                    before: None,
                    limit: *limit,
                })),
            )?;
            let AnyReply::Service(service::Reply::ListRated(page)) = r else {
                return Err(format!("{r:?}"));
            };
            for r in page.problems {
                println!(
                    "{:>6}  {:<9}  {}  {}",
                    r.summary.number,
                    format!("{:?}", r.severity),
                    r.summary.refcode.code,
                    r.summary.description
                );
            }
            return Ok(());
        }
        Problems::ShowRated { number } => {
            let r = ask(
                link,
                &AnyRequest::Service(service::Request::GetRated(service::Get { number: *number })),
            )?;
            println!("{r:#?}");
            return Ok(());
        }
        _ => {}
    }
    match *p {
        Problems::Note { .. } | Problems::Rated { .. } | Problems::ShowRated { .. } => Ok(()),
        Problems::List { open, limit } => {
            let r = ask(
                link,
                &AnyRequest::Service(service::Request::List(service::List {
                    filter: service::Filter {
                        status: open.then_some(service::ProblemStatus::Open),
                        ..service::Filter::default()
                    },
                    before: None,
                    limit,
                })),
            )?;
            let AnyReply::Service(service::Reply::List(page)) = r else {
                return Err(format!("{r:?}"));
            };
            for s in page.problems {
                let status = match (s.status, s.holds) {
                    (service::ProblemStatus::Closed, _) => "closed",
                    (_, true) => "open",
                    (_, false) => "cleared",
                };
                println!(
                    "{:>6}  {status:<7}  {}  {:>3}x  {}",
                    s.number, s.refcode.code, s.count, s.description
                );
            }
            Ok(())
        }
        Problems::Show { number } => {
            let r = ask(
                link,
                &AnyRequest::Service(service::Request::Get(service::Get { number })),
            )?;
            println!("{r:#?}");
            Ok(())
        }
        Problems::Close { number, anyway } => {
            let r = ask(
                link,
                &AnyRequest::Service(service::Request::Close(service::Close { number, anyway })),
            )?;
            let AnyReply::Service(service::Reply::Close(s)) = r else {
                return Err(format!("{r:?}"));
            };
            println!("problem {} closed by {}", s.number, s.closed_by);
            Ok(())
        }
    }
}

/// Upload `file` on a stage's stream, and wait for its verification.
fn stage(link: &mut Link, file: &Path, allow_older: bool) -> Result<(), String> {
    let bytes = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    let name = file.file_name().map_or_else(
        || "update".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let name: String = name.chars().take(64).collect();
    subscribe(link)?;
    let reply = link.ask(&AnyRequest::Update(update::Request::Stage(update::Stage {
        name: name.clone(),
        bytes,
        allow_older,
    })))?;
    let Some(stream) = reply.stream else {
        return Err(refusal(&reply));
    };
    let mut f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        link.send_all(stream, &buf[..n])?;
    }
    link.session
        .close_stream(stream)
        .map_err(|e| e.to_string())?;
    until(link, Duration::from_mins(10), |e| match e {
        AnyEvent::Update(update::Event::Staged(s)) if s.name == name && s.error.is_some() => {
            Some(Err(note(s.error.as_ref())))
        }
        AnyEvent::Update(update::Event::Verified(v)) if v.name == name => Some(match &v.image {
            Some(i) => {
                println!(
                    "staged {name}: {} {} build {}, fixes {}",
                    i.product,
                    i.release,
                    i.build,
                    if i.fixes.is_empty() {
                        "none".into()
                    } else {
                        i.fixes.join(",")
                    }
                );
                Ok(())
            }
            None => Err(note(v.error.as_ref())),
        }),
        _ => None,
    })
}

/// Run `qsictl update`.
///
/// # Errors
/// The request was refused, or an install failed.
pub fn update(link: &mut Link, u: Update) -> Result<(), String> {
    match u {
        Update::Status => {
            let r = ask(link, &AnyRequest::Update(update::Request::Status(Empty)))?;
            println!("{r:#?}");
            Ok(())
        }
        Update::Stage { file, allow_older } => stage(link, &file, allow_older),
        Update::Install => {
            subscribe(link)?;
            let r = ask(link, &AnyRequest::Update(update::Request::Install(Empty)))?;
            let AnyReply::Update(update::Reply::Install(t)) = r else {
                return Err(format!("{r:?}"));
            };
            println!("installing into slot {}", t.slot);
            until(link, Duration::from_mins(15), |e| match e {
                AnyEvent::Update(update::Event::Installing(p)) => {
                    println!("  {}%", p.percent);
                    None
                }
                AnyEvent::Update(update::Event::Installed(i)) => Some(match &i.image {
                    Some(img) => {
                        println!(
                            "slot {} holds {} build {}: pending",
                            i.slot, img.release, img.build
                        );
                        Ok(())
                    }
                    None => Err(note(i.error.as_ref())),
                }),
                _ => None,
            })
        }
        Update::Activate { shut_down, timeout } => {
            let partitions = if shut_down {
                update::Partitions::ShutDown
            } else {
                update::Partitions::Refuse
            };
            ask(
                link,
                &AnyRequest::Update(update::Request::Activate(update::Activate {
                    partitions,
                    timeout,
                })),
            )?;
            println!("the restart into the pending slot was begun");
            Ok(())
        }
        Update::Cancel => {
            ask(link, &AnyRequest::Update(update::Request::Cancel(Empty)))?;
            println!("nothing is pending");
            Ok(())
        }
        Update::Discard => {
            ask(link, &AnyRequest::Update(update::Request::Discard(Empty)))?;
            println!("what was staged is discarded");
            Ok(())
        }
    }
}

/// Download host dump `name` to `file`.
fn fetch(link: &mut Link, name: &str, file: &Path) -> Result<(), String> {
    let reply = link.ask(&AnyRequest::Base(base::Request::DownloadDump(
        base::DownloadDump {
            name: name.to_string(),
        },
    )))?;
    let (Some(stream), Ok(Some(AnyReply::Base(base::Reply::DownloadDump(d))))) =
        (reply.stream, reply.decode())
    else {
        return Err(refusal(&reply));
    };
    let mut out = std::fs::File::create(file).map_err(|e| e.to_string())?;
    let mut have = 0u64;
    loop {
        let mut got = std::mem::take(&mut link.besides);
        got.extend(link.step()?);
        for i in got {
            match i {
                Incoming::Data { stream: s, bytes } if s == stream => {
                    out.write_all(&bytes).map_err(|e| e.to_string())?;
                    have += bytes.len() as u64;
                    let _ = link
                        .session
                        .grant(stream, u32::try_from(bytes.len()).unwrap_or(0));
                }
                Incoming::StreamClosed { stream: s } if s == stream => {
                    if have != d.bytes {
                        return Err(format!("{have} of {} bytes", d.bytes));
                    }
                    println!("{have} bytes of {}", d.name);
                    return Ok(());
                }
                Incoming::StreamReset {
                    stream: s,
                    msgid,
                    text,
                    ..
                } if s == stream => return Err(format!("{msgid} {text}")),
                _ => {}
            }
        }
    }
}

/// Run `qsictl hostdump`.
///
/// # Errors
/// The request was refused, or the dump was not written.
pub fn host_dump(link: &mut Link, what: HostDump) -> Result<(), String> {
    match what {
        HostDump::Take => {
            subscribe(link)?;
            let reply = ask(
                link,
                &AnyRequest::Base(base::Request::TakeDump(base::TakeDump {
                    kind: base::DumpKind::ServiceData,
                    kind_name: String::new(),
                })),
            )?;
            let AnyReply::Base(base::Reply::TakeDump(begun)) = reply else {
                return Err(format!("{reply:?}"));
            };
            println!("taking {}", begun.name);
            until(link, Duration::from_mins(10), |e| match e {
                AnyEvent::Base(base::Event::DumpTaken(taken)) if taken.name == begun.name => {
                    Some(match &taken.dump {
                        Some(dump) => {
                            println!("{} written, {} bytes", dump.name, dump.bytes);
                            Ok(())
                        }
                        None => Err(note(taken.error.as_ref())),
                    })
                }
                _ => None,
            })
        }
        HostDump::List => {
            let reply = ask(
                link,
                &AnyRequest::Base(base::Request::Dumps(base::ListDumps {})),
            )?;
            let AnyReply::Base(base::Reply::Dumps(all)) = reply else {
                return Err(format!("{reply:?}"));
            };
            for dump in all {
                println!(
                    "{}  {:>10}  {}  {}",
                    dump.name, dump.bytes, dump.time, dump.by
                );
            }
            Ok(())
        }
        HostDump::Fetch { name, file } => fetch(link, &name, &file),
        HostDump::Delete { name } => {
            ask(
                link,
                &AnyRequest::Base(base::Request::DeleteDump(base::DeleteDump {
                    name: name.clone(),
                })),
            )?;
            println!("{name} deleted");
            Ok(())
        }
        HostDump::Space => {
            let reply = ask(
                link,
                &AnyRequest::Base(base::Request::DumpSpace(qsi::Empty)),
            )?;
            println!("{reply:#?}");
            Ok(())
        }
    }
}
