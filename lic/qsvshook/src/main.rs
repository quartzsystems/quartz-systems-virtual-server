// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The test hooks' binary: `qsvshook channel`, `qsvshook flaky` and
//! `qsvshook partition`, the services a test image's PID 1 has in its
//! table, and `qsvshook soak`, which PID 1 starts in a soak's partition.

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
        Some("soak") => {
            qsvshook::soak::run(&std::env::args().nth(2).unwrap_or_default());
        }
        Some("soak-child") => {
            let mib = std::env::args()
                .nth(2)
                .and_then(|m| m.parse().ok())
                .unwrap_or(1);
            std::process::exit(qsvshook::soak::child(mib));
        }
        other => eprintln!("qsvshook channel | flaky | partition | soak, not {other:?}"),
    }
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvshook runs in a QS/VS test image only");
}
