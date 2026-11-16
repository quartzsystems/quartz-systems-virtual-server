// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What is more than one request and one reply: the events followed,
//! a partition's console in the terminal, a dump downloaded and an
//! image uploaded, each over its stream.

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use qsi::any::{AnyEvent, AnyReply, AnyRequest};
use qsi::base;
use qsi::host::console::{In, Out, Pieces};
use qsi::host::{console, dumps, vres};
use qsipart::console::{FromSystem, Grid, Key};
use qsisess::Incoming;

use crate::requests::refused;
use qsictl::link::Link;

/// Print every event from after `since`, until interrupted.
///
/// # Errors
/// The subscription was refused, or the connection ended.
pub fn events(link: &mut Link, since: Option<u64>) -> Result<(), String> {
    let reply = link.ask(&AnyRequest::Base(base::Request::Subscribe(
        base::Subscribe { since },
    )))?;
    match reply.decode() {
        Ok(Some(AnyReply::Base(base::Reply::Subscribe(s)))) => {
            println!(
                "subscribed: {:?}, {} replayed, next {}",
                s.replay, s.replayed, s.next
            );
        }
        _ => return Err(refused(&reply)),
    }
    loop {
        let mut got = std::mem::take(&mut link.besides);
        got.extend(link.step()?);
        for i in got {
            if let Incoming::Event(e) = i {
                match e.decode() {
                    Ok(Some(event)) => println!("{} {}", e.seq, describe(&event)),
                    _ => println!("{} family {:#06x} event {:#06x}", e.seq, e.family, e.op),
                }
            }
        }
    }
}

fn describe(e: &AnyEvent) -> String {
    format!("{e:?}")
}

/// A line typed at the terminal, as the console's keys: the text, then
/// Enter; `:F3`, `:enter`, `:up` and the like for one key alone.
fn keys(line: &str) -> Vec<Key> {
    let named = |w: &str| -> Option<Key> {
        let w = w.to_ascii_lowercase();
        Some(match w.as_str() {
            "enter" => Key::Enter,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "home" => Key::Home,
            "attn" => Key::Attention,
            "sysreq" => Key::SysReq,
            "reset" => Key::Reset,
            "tab" => Key::FieldAdvance,
            "exit" => Key::FieldExit,
            f if f.starts_with('f') => Key::Function(f[1..].parse().ok()?),
            _ => return None,
        })
    };
    if let Some(word) = line.strip_prefix(':')
        && let Some(k) = named(word.trim())
    {
        return vec![k];
    }
    let mut out: Vec<Key> = line.chars().map(Key::Char).collect();
    out.push(Key::Enter);
    out
}

/// Attach to partition `id`'s console: its screen drawn in the
/// terminal, or its serial port's bytes; each line typed sent.
///
/// # Errors
/// The attach was refused, or the connection ended.
pub fn attach(link: &mut Link, id: u32, take_over: bool) -> Result<(), String> {
    let reply = link.ask(&AnyRequest::Console(console::Request::Attach(
        console::Attach { id, take_over },
    )))?;
    let (Some(stream), Ok(Some(AnyReply::Console(console::Reply::Attach(a))))) =
        (reply.stream, reply.decode())
    else {
        return Err(refused(&reply));
    };
    let (tx, typed) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut pieces = Pieces::default();
    let mut grid = Grid::default();
    let mut out = std::io::stdout();
    loop {
        while let Ok(line) = typed.try_recv() {
            let messages: Vec<In> = if a.serial {
                vec![In::Bytes(format!("{line}\r").into_bytes())]
            } else {
                keys(&line).into_iter().map(In::Key).collect()
            };
            for m in messages {
                let piece = m.piece().map_err(|e| e.to_string())?;
                link.send_all(stream, &piece)?;
            }
        }
        let mut got = std::mem::take(&mut link.besides);
        got.extend(link.step()?);
        for i in got {
            match i {
                Incoming::Data { stream: s, bytes } if s == stream => {
                    let _ = link
                        .session
                        .grant(stream, u32::try_from(bytes.len()).unwrap_or(0));
                    pieces.push(&bytes);
                    while let Some(piece) = pieces.next_piece() {
                        match Out::decode(&piece) {
                            Ok(Out::Frame(f)) => {
                                let _ = grid.apply(&FromSystem::Frame(f));
                                let _ = write!(out, "\x1b[H\x1b[2J{}", grid.text());
                            }
                            Ok(Out::Update(u)) => {
                                let _ = grid.apply(&FromSystem::Update(u));
                                let _ = write!(out, "\x1b[H\x1b[2J{}", grid.text());
                            }
                            Ok(Out::Serial(b)) => {
                                let _ = out.write_all(&b);
                            }
                            Ok(Out::Ended(why)) => {
                                println!("\nthe console ended: {why}");
                                return Ok(());
                            }
                            _ => {}
                        }
                        let _ = out.flush();
                    }
                }
                Incoming::StreamClosed { stream: s } | Incoming::StreamReset { stream: s, .. }
                    if s == stream =>
                {
                    println!("\nthe console's stream ended");
                    return Ok(());
                }
                _ => {}
            }
        }
    }
}

/// Download partition `id`'s dump `name` to `file`.
///
/// # Errors
/// Refused, short, or the file could not be written.
pub fn download(link: &mut Link, id: u32, name: &str, file: &Path) -> Result<(), String> {
    let reply = link.ask(&AnyRequest::Dumps(dumps::Request::Download(
        dumps::Download {
            id,
            name: name.to_string(),
        },
    )))?;
    let (Some(stream), Ok(Some(AnyReply::Dumps(dumps::Reply::Download(d))))) =
        (reply.stream, reply.decode())
    else {
        return Err(refused(&reply));
    };
    let mut out = std::fs::File::create(file).map_err(|e| e.to_string())?;
    let mut have = 0u64;
    let t0 = Instant::now();
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
                    println!(
                        "{} bytes of {} in {:.1} s",
                        have,
                        d.name,
                        t0.elapsed().as_secs_f64()
                    );
                    return Ok(());
                }
                Incoming::StreamReset {
                    stream: s,
                    msgid,
                    text,
                    ..
                } if s == stream => {
                    return Err(format!("{msgid} {text}"));
                }
                _ => {}
            }
        }
    }
}

/// Upload `file` as image `name`, and wait for how it ended.
///
/// # Errors
/// Refused, cut short, or the file could not be read.
pub fn upload(link: &mut Link, name: &str, file: &Path) -> Result<(), String> {
    let bytes = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    // The end is an event: subscribe first, so it is not missed.
    let sub = link.ask(&AnyRequest::Base(base::Request::Subscribe(
        base::Subscribe { since: None },
    )))?;
    if sub.refusal().is_some() {
        return Err(refused(&sub));
    }
    let reply = link.ask(&AnyRequest::Vres(vres::Request::ImageUpload(
        vres::ImageUpload {
            name: name.to_string(),
            bytes,
        },
    )))?;
    let Some(stream) = reply.stream else {
        return Err(refused(&reply));
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
    let until = Instant::now() + Duration::from_secs(600);
    while Instant::now() < until {
        let mut got = std::mem::take(&mut link.besides);
        got.extend(link.step()?);
        for i in got {
            if let Incoming::Event(e) = i
                && let Ok(Some(AnyEvent::Vres(vres::Event::Upload(u)))) = e.decode()
                && u.name == name
            {
                return match (u.image, u.error) {
                    (Some(i), _) => {
                        println!(
                            "uploaded {} ({} bytes, label {:?})",
                            i.name, i.bytes, i.label
                        );
                        Ok(())
                    }
                    (None, Some(n)) => Err(format!("{} {}", n.msgid, n.text)),
                    (None, None) => Err("the upload ended with no image".into()),
                };
            }
        }
    }
    Err("no word of the upload's end".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_its_keys_then_enter_and_a_name_is_one_key() {
        assert_eq!(keys("ab"), vec![Key::Char('a'), Key::Char('b'), Key::Enter]);
        assert_eq!(keys(":F3"), vec![Key::Function(3)]);
        assert_eq!(keys(":enter"), vec![Key::Enter]);
        assert_eq!(keys(":nonsense").len(), 10);
    }
}
