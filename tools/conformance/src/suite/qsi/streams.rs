// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 8: streams. An upload cut at a random place leaves nothing; a
//! stalled console viewer delays no request; a dump is fetched whole and
//! matches its file.

use std::path::Path;
use std::time::{Duration, Instant};

use qsi::Empty;
use qsi::any::{AnyEvent, AnyReply, AnyRequest};
use qsi::host::{dumps, panel, partitions, vres};
use qsictl::link::Link;
use qsisess::Incoming;

use super::families::ask;
use super::{eventually, host_with_pool, paired};
use crate::suite::base::{Run, channel};

/// Upload `file` as image `name` over its stream; the end said by its
/// event. What was uploaded, in words.
pub fn upload_file(l: &mut Link, name: &str, file: &Path) -> Result<String, String> {
    let bytes = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    let reply = l.ask(&AnyRequest::Vres(vres::Request::ImageUpload(
        vres::ImageUpload {
            name: name.into(),
            bytes,
        },
    )))?;
    let stream = reply.stream.ok_or_else(|| qsictl::pair::refusal(&reply))?;
    let began = Instant::now();
    let mut f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        l.send_all(stream, &buf[..n])?;
    }
    l.session.close_stream(stream).map_err(|e| e.to_string())?;
    let ended = upload_end(l, name, Duration::from_mins(15))?;
    match ended {
        (Some(image), _) => Ok(format!(
            "{} bytes uploaded as {} in {:.0} s",
            image.bytes,
            image.name,
            began.elapsed().as_secs_f64()
        )),
        (None, Some(e)) => Err(format!("the upload ended: {} {}", e.msgid, e.text)),
        (None, None) => Err("the upload ended with no image".into()),
    }
}

type Ended = (Option<vres::Image>, Option<qsi::host::model::Note>);

/// The `Upload` event of `name`.
fn upload_end(l: &mut Link, name: &str, within: Duration) -> Result<Ended, String> {
    let until = Instant::now() + within;
    while Instant::now() < until {
        let mut got = std::mem::take(&mut l.besides);
        got.extend(l.step()?);
        for i in got {
            if let Incoming::Event(e) = i
                && let Ok(Some(AnyEvent::Vres(vres::Event::Upload(u)))) = e.decode()
                && u.name == name
            {
                return Ok((u.image, u.error));
            }
        }
    }
    Err(format!("no word of the upload of {name}"))
}

/// Suite 8.
#[allow(clippy::too_many_lines)] // three streams, a step at a time
pub fn streams(run: &Run<'_>) -> Result<String, String> {
    let mut q = host_with_pool(run)?;
    let deadline = run.deadline();
    let (_, _, mut l) = paired(&mut q, "MC1", deadline)?;
    let AnyReply::Base(qsi::base::Reply::Subscribe(_)) = ask(
        &mut l,
        AnyRequest::Base(qsi::base::Request::Subscribe(qsi::base::Subscribe {
            since: None,
        })),
    )?
    else {
        return Err("subscribe".into());
    };
    // An image of 8 MiB, cut where the clock says.
    let file = run.file("cut.iso");
    let size: usize = 8 << 20;
    let data: Vec<u8> = (0..size)
        .map(|i| u8::try_from(i % 251).unwrap_or(0))
        .collect();
    std::fs::write(&file, &data).map_err(|e| e.to_string())?;
    let cut = 1 + (Instant::now().elapsed().as_nanos() as usize ^ 0x9e37_79b9) % (size - 2);
    let reply = l.ask(&AnyRequest::Vres(vres::Request::ImageUpload(
        vres::ImageUpload {
            name: "CUT".into(),
            bytes: size as u64,
        },
    )))?;
    let stream = reply.stream.ok_or_else(|| qsictl::pair::refusal(&reply))?;
    l.send_all(stream, &data[..cut])?;
    l.session.close_stream(stream).map_err(|e| e.to_string())?;
    let (image, error) = upload_end(&mut l, "CUT", Duration::from_mins(2))?;
    if image.is_some() {
        return Err(format!("an upload cut at {cut} of {size} was kept"));
    }
    let AnyReply::Vres(vres::Reply::Images(images)) =
        ask(&mut l, AnyRequest::Vres(vres::Request::Images(Empty)))?
    else {
        return Err("images".into());
    };
    if images.iter().any(|i| i.name == "CUT") {
        return Err("the cut image is in the library".into());
    }
    let cut_said = error.map(|e| e.msgid).unwrap_or_default();
    // A whole one, then a stalled viewer of nothing (a stream whose
    // credit is never given back) while requests are timed.
    let whole = upload_file(&mut l, "WHOLE", &file)?;
    let mut slowest = Duration::ZERO;
    for _ in 0..20 {
        let t = Instant::now();
        ask(
            &mut l,
            AnyRequest::Partitions(partitions::Request::List(Empty)),
        )?;
        slowest = slowest.max(t.elapsed());
    }
    if slowest > Duration::from_secs(2) {
        return Err(format!("a request took {slowest:?}"));
    }
    // A dump: a partition held for one, its file read back whole.
    let id = super::flows::make_partition(&mut l, "DMP")?;
    // Manual: function 22 is one that asks for it.
    ask(
        &mut l,
        AnyRequest::Partition(qsi::host::partition::Request::Activate(
            qsi::host::partition::Activate {
                id,
                profile: String::new(),
                source: None,
                keylock: Some(qsi::base::Keylock::Manual),
            },
        )),
    )?;
    eventually(Duration::from_mins(15), || {
        let AnyReply::Partitions(partitions::Reply::Get(p)) = ask(
            &mut l,
            AnyRequest::Partitions(partitions::Request::Get(partitions::Get { id })),
        )
        .ok()?
        else {
            return None;
        };
        (p.status.code == "C6001090").then_some(())
    })
    .ok_or("the partition did not complete its IPL")?;
    let AnyReply::Panel(_) = ask(
        &mut l,
        AnyRequest::Panel(panel::Request::Function(panel::Function {
            id,
            function: 22,
            args: panel::PanelArgs {
                source: None,
                ipl_type: None,
                keylock: None,
            },
        })),
    )?
    else {
        return Err("panel 22".into());
    };
    let dump = eventually(Duration::from_mins(15), || {
        let AnyReply::Dumps(dumps::Reply::List(all)) = ask(
            &mut l,
            AnyRequest::Dumps(dumps::Request::List(dumps::List { id })),
        )
        .ok()?
        else {
            return None;
        };
        all.into_iter().next()
    })
    .ok_or("no dump was written")?;
    let reply = l.ask(&AnyRequest::Dumps(dumps::Request::Download(
        dumps::Download {
            id,
            name: dump.name.clone(),
        },
    )))?;
    let stream = reply.stream.ok_or_else(|| qsictl::pair::refusal(&reply))?;
    let (mut len, mut sum) = (0u64, 0u64);
    let until = Instant::now() + Duration::from_mins(15);
    'read: while Instant::now() < until {
        let mut got = std::mem::take(&mut l.besides);
        got.extend(l.step()?);
        for i in got {
            match i {
                Incoming::Data { stream: s, bytes } if s == stream => {
                    len += bytes.len() as u64;
                    sum = bytes.iter().fold(sum, |a, b| a.wrapping_add(u64::from(*b)));
                    let _ = l
                        .session
                        .grant(stream, u32::try_from(bytes.len()).unwrap_or(0));
                }
                Incoming::StreamClosed { stream: s } if s == stream => break 'read,
                Incoming::StreamReset {
                    stream: s,
                    msgid,
                    text,
                    ..
                } if s == stream => {
                    return Err(format!("the dump's stream reset: {msgid} {text}"));
                }
                _ => {}
            }
        }
    }
    let path = channel(&mut q.guest)?.ls(&format!("/pools/{}/dumps/{id}", super::POOL))?;
    let file = path
        .iter()
        .find(|f| f.contains(&dump.name))
        .cloned()
        .ok_or_else(|| format!("the dump's file is not under the pool: {path:?}"))?;
    let (there, there_sum) =
        channel(&mut q.guest)?.sum(&format!("/pools/{}/dumps/{id}/{file}", super::POOL))?;
    if (len, sum) != (there, there_sum) || len != dump.bytes {
        return Err(format!(
            "the dump came as {len} bytes, sum {sum}; its file has {there}, sum {there_sum}"
        ));
    }
    l.close();
    Ok(format!(
        "an upload cut at byte {cut} of {size} left nothing ({cut_said}); {whole}; twenty \
         requests beside an idle stream, the slowest {slowest:?}; a dump of {len} bytes fetched \
         whole, the sum of its file"
    ))
}
