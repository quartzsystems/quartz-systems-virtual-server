// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The test hooks' binary: `qsvshook channel`, `qsvshook flaky` and
//! `qsvshook partition`, the services a test image's PID 1 has in its
//! table.

#[cfg(target_os = "linux")]
fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("channel") => {
            let Err(e) = qsvshook::channel::run();
            eprintln!("the channel did not start: {e}");
        }
        Some("flaky") => qsvshook::flaky::run(),
        Some(qsvshook::TRY) => {
            let mut args = std::env::args().skip(2);
            let (verb, what) = (
                args.next().unwrap_or_default(),
                args.next().unwrap_or_default(),
            );
            std::process::exit(qsvshook::channel::try_it(&verb, &what));
        }
        Some("partition") => {
            let Err(e) = qsvshook::partition::relay::run();
            eprintln!("the partition hook did not start: {e}");
        }
        other => eprintln!("qsvshook channel | flaky | partition, not {other:?}"),
    }
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvshook runs in a QS/VS test image only");
}
