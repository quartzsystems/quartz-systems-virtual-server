// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsictl`: a developer's QSI console (`docs/QSICTL.md`). It pairs
//! with a QS/VS host, asks it each request of QSI's families and prints
//! the answer, follows its events, moves a dump or an image, and
//! attaches to a partition's console in a terminal. It is in no image.

mod console;
mod home;
mod link;
mod requests;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use home::Home;
use link::Link;

/// A developer's QSI console.
#[derive(Parser, Debug)]
#[command(name = "qsictl", version)]
struct Cli {
    /// The directory of its identity and hosts.
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    /// The host, by the name it was paired as; the one host when there
    /// is one.
    #[arg(long, short = 'H', global = true)]
    host: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Make this console's identity.
    Init {
        /// The name it pairs as.
        #[arg(long, default_value = "qsictl")]
        name: String,
    },
    /// Pair with a host, by its management address and the password
    /// shown at its Service Tools.
    Pair {
        /// `address` or `address:port`.
        address: String,
        /// The pairing password.
        password: String,
        /// The name to keep the host as; its address by default.
        #[arg(long = "as")]
        name: Option<String>,
    },
    /// The hosts paired with.
    Hosts,
    /// Forget a host.
    Forget {
        /// Its name.
        name: String,
    },
    /// Follow the host's events, from after `since`.
    Events {
        /// The last event seen.
        #[arg(long)]
        since: Option<u64>,
    },
    /// Attach to a partition's console.
    Console {
        /// The partition.
        id: u32,
        /// Take it over from another viewer.
        #[arg(long)]
        take_over: bool,
    },
    /// Download a partition's dump to a file.
    Download {
        /// The partition.
        id: u32,
        /// The dump.
        name: String,
        /// Where to write it.
        file: PathBuf,
    },
    /// Upload an optical image from a file.
    Upload {
        /// The image's name in the library.
        name: String,
        /// The file.
        file: PathBuf,
    },
    /// Any other request: `qsictl request --help` lists them.
    #[command(flatten)]
    Request(requests::Ask),
}

fn with_port(address: &str) -> String {
    if address.contains(':') {
        address.to_string()
    } else {
        format!("{address}:2300")
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let home = Home::at(cli.dir.unwrap_or_else(home::default_dir));
    match cli.command {
        Command::Init { name } => {
            let id = home.init(&name)?;
            println!("identity {}", id.fingerprint().hex());
            Ok(())
        }
        Command::Pair {
            address,
            password,
            name,
        } => pair(&home, &with_port(&address), &password, name),
        Command::Hosts => {
            for h in home.hosts() {
                println!("{}  {}  {}", h.name, h.address, h.fingerprint.hex());
            }
            Ok(())
        }
        Command::Forget { name } => home.forget(&name),
        command => {
            let host = home.host(cli.host.as_deref())?;
            let identity = home.identity()?;
            let mut link = Link::connect(&host.address, &identity, Some(host.fingerprint))?;
            let done = match command {
                Command::Events { since } => console::events(&mut link, since),
                Command::Console { id, take_over } => console::attach(&mut link, id, take_over),
                Command::Download { id, name, file } => {
                    console::download(&mut link, id, &name, &file)
                }
                Command::Upload { name, file } => console::upload(&mut link, &name, &file),
                Command::Request(ask) => requests::run(&mut link, ask),
                _ => Ok(()),
            };
            link.close();
            done
        }
    }
}

fn pair(home: &Home, address: &str, password: &str, name: Option<String>) -> Result<(), String> {
    use qsi::any::AnyRequest;
    use qsi::pair;
    let identity = home.identity()?;
    let mut link = Link::connect(address, &identity, None)?;
    let host = link.peer().ok_or("the host presented no certificate")?;
    println!("host certificate {}", host.hex());
    println!("check it is the one the host's Service Tools shows");
    let binding = link.binding(&identity)?;
    let (exchange, start) =
        qsitrust::pair::Console::start(password, binding, "qsictl").map_err(|e| e.to_string())?;
    let reply = link.ask(&AnyRequest::Pair(pair::Request::Start(pair::Start {
        spake: start.spake,
        name: start.name,
    })))?;
    let Ok(Some(pair::Reply::Answer(a))) = reply.typed::<pair::Reply>() else {
        return Err(requests::refused(&reply));
    };
    let (confirm, paired) = exchange
        .answer(&qsitrust::pair::Answer {
            spake: a.spake,
            confirm: a.confirm,
            name: a.name,
        })
        .map_err(|_| "the host's confirmation does not hold: a wrong password, or not that host")?;
    let done = link.ask(&AnyRequest::Pair(pair::Request::Confirm(pair::Confirm {
        confirm: confirm.confirm,
    })))?;
    if done.refusal().is_some() {
        return Err(requests::refused(&done));
    }
    link.close();
    let kept = home::Host {
        name: name.unwrap_or_else(|| paired.name.clone()),
        address: address.to_string(),
        fingerprint: paired.fingerprint,
    };
    println!("paired with {} as {}", paired.name, kept.name);
    home.keep(kept)
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("qsictl: {e}");
            ExitCode::FAILURE
        }
    }
}
