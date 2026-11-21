// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The fixture appliance, `QSDEMO`: what another appliance takes of
//! the base crates, used from a git dependency: the processor check, the
//! QEMU driver, the inventory and the network roles; and what a console
//! takes to service itself as a host does: an update of itself, its
//! service dump, its problem log, a confined service, a release
//! directory with its signed index, the panic capture and the media's
//! check. Each part prints one
//! `QSDEMO <part>: ...` line when it holds, and the run ends with an
//! error when one does not.

use std::path::Path;

/// QSDEMO's two network roles.
const ROLES: &[&str] = &["public", "private"];

/// Its `network.toml`: the public interface by DHCP, the private one on
/// a VLAN of its own link, carrying neither the route, the name servers
/// nor the time.
const NETWORK: &str = "schema_version = 2\n\
    [[interface]]\nrole = \"public\"\n\
    [[interface]]\nrole = \"private\"\ndevice = \"eth1\"\nvlan = 20\nmode = \"static\"\n\
    address = \"10.1.0.1/24\"\ncarries_route = false\ncarries_dns = false\ncarries_time = false\n";

fn processor() -> Result<String, String> {
    let all = processor::lacking_from("avx2 adx aes pclmulqdq");
    if !all.is_empty() {
        return Err(format!("a processor with every feature lacks {all:?}"));
    }
    let lacking = processor::lacking();
    Ok(format!(
        "{} features checked; this processor lacks {}",
        processor::NEEDED.len(),
        if lacking.is_empty() {
            "none".to_string()
        } else {
            lacking.join(", ")
        }
    ))
}

fn qemu() -> Result<String, String> {
    let launch = qemu::Launch {
        qemu: "qemu-system-x86_64".into(),
        firmware: "/fw/OVMF_CODE_4M.fd".into(),
        disk: Some("qsdemo.img".into()),
        disk_serial: "QSDEMOHD0".into(),
        cdrom: Some("qsdemo.iso".into()),
        nic: Some("52:54:00:00:00:01".into()),
        forwards: vec![(40_000, 443)],
        memory_mb: 512,
        cpus: 1,
        snapshot: true,
        ..qemu::Launch::default()
    };
    let words: Vec<String> = qemu::args(&launch)
        .iter()
        .map(|w| w.to_string_lossy().into_owned())
        .collect();
    for want in [
        "virtio-blk-pci,drive=hd,serial=QSDEMOHD0",
        "scsi-cd,drive=cd,bus=cdscsi.0",
        "user,id=net0,hostfwd=tcp:127.0.0.1:40000-10.0.2.15:443",
    ] {
        if !words.iter().any(|w| w == want) {
            return Err(format!("`{want}` is not on the command line: {words:?}"));
        }
    }
    // A store with one boot entry, read back.
    let store = qemu::vars::with_boot_entry(&empty_store(), b"entry")
        .map_err(|e| format!("the variable store: {e}"))?;
    let names: Vec<String> = qemu::vars::variables(&store)?
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    if names != ["Boot0000", "BootOrder"] {
        return Err(format!("the store has {names:?}"));
    }
    let missing = qemu::firmware::locate_vars(Path::new("/nonexistent/CODE.fd"), "QSDEMO_OVMF_VARS");
    match missing {
        Err(e) if e.to_string().contains("QSDEMO_OVMF_VARS") => {}
        other => return Err(format!("a missing store: {other:?}")),
    }
    Ok(format!(
        "{} arguments, the CD-ROM on virtio-scsi; a store with {}",
        words.len(),
        names.join(" and ")
    ))
}

/// An erased variable store: a firmware volume header with an
/// authenticated variable store and nothing in it.
fn empty_store() -> Vec<u8> {
    const LEN: u32 = 0x2_0000;
    let mut s = vec![0xFF_u8; 0x2_0000];
    s[..0x48].fill(0);
    // The volume header: its length at 32, `_FVH` at 40, its header
    // length at 48.
    s[32..40].copy_from_slice(&u64::from(LEN).to_le_bytes());
    s[40..44].copy_from_slice(b"_FVH");
    s[48..50].copy_from_slice(&0x48u16.to_le_bytes());
    // The variable store's header: its GUID, size, formatted and healthy.
    let guid = [
        0x78, 0x2c, 0xf3, 0xaa, 0x7b, 0x94, 0x9a, 0x43, 0xa1, 0x80, 0x2e, 0x14, 0x4e, 0xc3, 0x77,
        0x92,
    ];
    s[0x48..0x58].copy_from_slice(&guid);
    s[0x58..0x5C].copy_from_slice(&(LEN - 0x48).to_le_bytes());
    s[0x5C] = 0x5A;
    s[0x5D] = 0xFE;
    s[0x5E..0x64].fill(0);
    s
}

fn inventory() -> Result<String, String> {
    let roots = inventory::Roots::default();
    let inv = inventory::read(&roots, None);
    let text = state::store::render(&inv).map_err(|e| e.to_string())?;
    let back: state::Inventory = state::store::parse(&text).map_err(|e| e.to_string())?;
    if back != inv {
        return Err("inventory.toml does not read back".into());
    }
    if inv.processors.threads == 0 || inventory::threads(&roots).is_empty() {
        return Err("no processor was read".into());
    }
    Ok(format!(
        "{} threads, {} MiB, {} disks, {} interfaces, {} PCI devices; serial from {}",
        inv.processors.threads,
        inventory::memory_mib(&roots),
        inv.disks.len(),
        inv.interfaces.len(),
        inv.pci.len(),
        inv.system.serial_source
    ))
}

fn network() -> Result<String, String> {
    let network: state::Network = state::store::parse(NETWORK).map_err(|e| e.to_string())?;
    netd::manage::check_roles(&network, ROLES)?;
    let mut moved = network.clone();
    moved.interfaces[1].vlan = Some(30);
    let touched = netd::trial::changed(&network, &moved);
    if touched != ["private"] {
        return Err(format!("a change of the private VLAN touches {touched:?}"));
    }
    let version_1: state::Network =
        state::store::parse("schema_version = 1\nmode = \"static\"\naddress = \"192.0.2.10/24\"\n")
            .map_err(|e| e.to_string())?;
    netd::manage::check_roles(&version_1, ROLES)?;
    Ok(format!(
        "{} interfaces, roles {}; a change of the private one touches it alone; \
         a version 1 file is {} interface",
        network.interfaces.len(),
        ROLES.join(" and "),
        version_1.interfaces.len()
    ))
}

/// A scratch directory of its own.
fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("qsdemo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(&d);
    d
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// QSDEMO's update images: its own magic, first line and fixes.
static FORMAT: update::Format = update::Format {
    magic: *b"QSDEMOUP",
    head: "QSDEMO-UPDATE 1",
    fix_prefix: "QD",
};

/// An update of itself: an image made and verified against its key and
/// refused under another; the state copied at a trial's first IPL, put
/// back when the trial does not confirm, and dropped when one does.
fn updates() -> Result<String, String> {
    use ed25519_dalek::{Signer, SigningKey};
    let key = SigningKey::from_bytes(&[3; 32]);
    let (a, b) = (vec![0xA5u8; 4096], vec![0x5Au8; 4096]);
    let file = |name: &str, bytes: &[u8]| update::manifest::File {
        name: name.into(),
        bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
    };
    let manifest = update::Manifest {
        product: "QSDEMO".into(),
        release: "V1R0M1".into(),
        build: "fixture".into(),
        fixes: vec!["QD00001".into()],
        schemas: vec![("host.toml".into(), 1)],
        files: vec![
            file(update::verify::SLOT_A, &a),
            file(update::verify::SLOT_B, &b),
        ],
    };
    let image = update::make(&FORMAT, &manifest, &a, &b, |t| key.sign(t).to_bytes());
    let running = |keys: Vec<u8>| update::Running {
        format: &FORMAT,
        product: "QSDEMO".into(),
        release: "V1R0M0".parse().expect("a release"),
        keys,
        state: vec![("host.toml".into(), 1)],
    };
    let ours = key.verifying_key().to_bytes().to_vec();
    let verified = update::verify::verify(&image, &running(ours), false)
        .map_err(|e| format!("its own image: {e}"))?;
    let other = SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes().to_vec();
    if update::verify::verify(&image, &running(other), false).is_ok() {
        return Err("an image signed by a key it does not trust was taken".into());
    }

    use update::state_copy::{Done, Ipl, before_state, confirmed};
    let data = scratch("update");
    let state = data.join("state");
    std::fs::create_dir_all(&state).map_err(|e| e.to_string())?;
    std::fs::write(state.join("host.toml"), "before").map_err(|e| e.to_string())?;
    let step = |ipl| before_state(&data, ipl).map_err(|e| e.to_string());
    let trial = step(Ipl::Trial)?;
    std::fs::write(state.join("host.toml"), "during").map_err(|e| e.to_string())?;
    let back = step(Ipl::FellBack)?;
    let after = std::fs::read_to_string(state.join("host.toml")).unwrap_or_default();
    if (trial, back, after.as_str()) != (Done::Copied, Done::Restored, "before") {
        return Err(format!("a trial rolled back: {trial:?}, {back:?}, state `{after}`"));
    }
    step(Ipl::Trial)?;
    std::fs::write(state.join("host.toml"), "kept").map_err(|e| e.to_string())?;
    confirmed(&data).map_err(|e| e.to_string())?;
    let kept = std::fs::read_to_string(state.join("host.toml")).unwrap_or_default();
    if kept != "kept" || step(Ipl::Normal)? != Done::Nothing {
        return Err(format!("a confirmed trial: state `{kept}`"));
    }
    let _ = std::fs::remove_dir_all(&data);
    Ok(format!(
        "an image of {} {} for slots {} staged and verified against its key, refused under \
         another; a trial rolled back restored the state, a confirmed one kept its own",
        verified.manifest.product,
        verified.manifest.release,
        verified.manifest.files.len()
    ))
}

/// QSDEMO's dumps.
static DUMPS: servicedump::Kind = servicedump::Kind {
    prefix: "DEMODUMP-",
    title: "Service dump",
    max_dumps: 4,
    max_total: 1 << 30,
    left_out: &[("pins.toml", "the consoles' pins")],
};

struct Own;

impl servicedump::Sources for Own {
    fn entries(&mut self) -> Vec<(String, Vec<u8>)> {
        vec![("own/sessions.txt".into(), b"2 sessions".to_vec())]
    }
}

/// A service dump with its own entries, and its secret left out.
fn dumps() -> Result<String, String> {
    let d = scratch("dump");
    let state = d.join("state");
    std::fs::create_dir_all(&state).map_err(|e| e.to_string())?;
    std::fs::write(state.join("host.toml"), "name = \"DEMO\"\n").map_err(|e| e.to_string())?;
    std::fs::write(state.join("pins.toml"), "[[console]]\n").map_err(|e| e.to_string())?;
    std::fs::write(state.join("tls.key"), "-----BEGIN PRIVATE KEY-----\n")
        .map_err(|e| e.to_string())?;
    let paths = servicedump::Paths {
        log: d.join("log"),
        state,
        problems: d.join("problems"),
        proc: d.join("proc"),
    };
    let store = servicedump::Store::at(d.join("dumps"), &DUMPS);
    let asked = servicedump::Asked {
        by: "the fixture".into(),
        release: "QSDEMO V1R0M0".into(),
        host: "DEMO".into(),
        status: "all ready".into(),
    };
    let (info, _) = servicedump::take(&store, &paths, &mut Own, &asked, 1_794_834_942)?;
    let bytes = std::fs::read(store.dir().join(&info.name)).map_err(|e| e.to_string())?;
    let entries = servicedump::zip::entries(&bytes).map_err(|e| format!("{e:?}"))?;
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    for gone in ["state/pins.toml", "state/tls.key"] {
        if names.contains(&gone) {
            return Err(format!("{gone} is in the dump"));
        }
    }
    if !names.contains(&"own/sessions.txt") || !names.contains(&"state/host.toml") {
        return Err(format!("the dump holds {names:?}"));
    }
    let _ = std::fs::remove_dir_all(&d);
    Ok(format!(
        "{} of {} entries, its own beside every appliance's; its pins and a key left out",
        info.name,
        names.len()
    ))
}

static NOTE: msg::Message = msg::Message {
    id: "QDM5001",
    text: "Problem &1 is not one of QSDEMO's.",
    help: "Look at the problem log.",
};
static FAILED: msg::Message = msg::Message {
    id: "QDM5101",
    text: "Session &1 failed.",
    help: "Sign on again.",
};

/// QSDEMO's problem table: one condition, cleared by one code.
static RULES: problem::table::Rules = problem::table::Rules {
    conditions: &[problem::table::Condition {
        code: "A5001010",
        opens: Some(problem::table::Opening {
            message: &FAILED,
            resources: &["service"],
            cleared_by: &["C5001010"],
            system_code: false,
        }),
        note: "",
    }],
    severity: |_| problem::Severity::Attention,
    messages: problem::table::Messages {
        no_problem: &NOTE,
        closed: &NOTE,
        holds: &NOTE,
        not_kept: &NOTE,
        note_invalid: &NOTE,
    },
};

/// A problem opened from a posted code, noted, cleared and closed.
fn problems() -> Result<String, String> {
    let posting = |seq: u64, code: &str| problem::book::Posting {
        posted: problem::Posted {
            seq,
            code: code.into(),
            words: Vec::new(),
            text: "session".into(),
            time_ms: seq * 1000,
        },
        about: vec![problem::record::About {
            kind: "service".into(),
            name: "webd".into(),
            ..problem::record::About::default()
        }],
    };
    let mut book = problem::Book::default();
    let opened = book.take(&RULES, &posting(1, "A5001010"), 10);
    let number = opened
        .first()
        .map(|n| n.problem.number)
        .ok_or("the code opened no problem")?;
    let noted = book
        .note(&RULES, number, "Restarted the web server", "OPER", 11)
        .map_err(|e| format!("{e:?}"))?;
    if book.close(&RULES, number, false, "OPER", 12).is_ok() {
        return Err("a problem was closed while its condition held".into());
    }
    book.take(&RULES, &posting(2, "C5001010"), 13);
    let closed = book
        .close(&RULES, number, false, "OPER", 14)
        .map_err(|e| format!("{e:?}"))?;
    Ok(format!(
        "problem {number} opened from A5001010 ({}), `{}`, cleared by C5001010 and closed by {}",
        closed.problem.description,
        noted.note.map(|n| n.line.text).unwrap_or_default(),
        closed.problem.closed_by
    ))
}

/// The calls of the confined child: what writing a line and ending need.
const CONFINED: &[u32] = &[1, 11, 15, 60, 131, 202, 231];

/// The child: confined to [`CONFINED`], it says so, then makes a call
/// it was not given, which ends it.
fn confined_child() -> ! {
    let rules: Vec<confine::bpf::Rule> =
        CONFINED.iter().map(|nr| confine::bpf::Rule::Any(*nr)).collect();
    if let Err(e) = confine::filter("child", &rules, confine::bpf::KILL_PROCESS) {
        eprintln!("not confined: {e}");
        std::process::exit(2);
    }
    println!("QSDEMO confined");
    // `statx` is not one of its calls: the filter ends the process.
    let _ = std::fs::metadata("/");
    println!("QSDEMO not stopped");
    std::process::exit(0);
}

/// A service confined: the fixture again, as a child that installs a
/// filter of its own calls and is ended by the kernel at another.
fn confinement() -> Result<String, String> {
    use std::os::unix::process::ExitStatusExt;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = std::process::Command::new(me)
        .arg("confined")
        .output()
        .map_err(|e| e.to_string())?;
    let said = String::from_utf8_lossy(&out.stdout);
    let signal = out.status.signal();
    if !said.contains("QSDEMO confined") || said.contains("not stopped") || signal != Some(31) {
        return Err(format!(
            "the child said `{}` and ended {:?}: {}",
            said.trim(),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(format!(
        "a child confined to {} calls ran, and a call outside them ended it (SIGSYS)",
        CONFINED.len()
    ))
}

/// A release directory: its stream's index signed, its sums signed, the
/// index verified by the release key.
fn release() -> Result<String, String> {
    use ed25519_dalek::SigningKey;
    let out = scratch("release");
    std::fs::write(out.join("QSDEMO-V1R0M0.iso"), b"an iso").map_err(|e| e.to_string())?;
    std::fs::write(out.join("QSDEMO-V1R0M0-update.upd"), b"an update")
        .map_err(|e| e.to_string())?;
    let key = SigningKey::from_bytes(&[5; 32]);
    let names = buildaid::release::Names {
        prefix: "qsdemo",
        sb_subject: "CN=QSDEMO",
        stream: "qsdemo",
    };
    let mut files = vec!["QSDEMO-V1R0M0.iso".to_string(), "QSDEMO-V1R0M0-update.upd".into()];
    buildaid::release::index_and_sums(
        &out,
        &names,
        "V1R0M0",
        "fixture",
        vec!["QD00001".into()],
        "QSDEMO-V1R0M0-update.upd",
        "QSDEMO-V1R0M0.iso",
        &mut files,
        &key,
    )
    .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(out.join("stream/index.toml")).map_err(|e| e.to_string())?;
    let index = stream::verify(&bytes, &[key.verifying_key().to_bytes()], 0)
        .map_err(|e| e.to_string())?;
    let sums = std::fs::read_to_string(out.join("SHA256SUMS")).map_err(|e| e.to_string())?;
    let listed = buildaid::release::listed(&out).map_err(|e| e.to_string())?;
    let e = index.releases.first().ok_or("an index of no release")?;
    let _ = std::fs::remove_dir_all(&out);
    Ok(format!(
        "{} files, stream {} with {} released {} (notes {}), {} sums signed",
        listed.len(),
        index.stream,
        e.release,
        e.released,
        e.notes,
        sums.lines().count()
    ))
}

/// The kernel's last messages of a panic, taken at the next start.
fn panics() -> Result<String, String> {
    let pstore = scratch("pstore");
    let kept = scratch("panics");
    std::fs::write(
        pstore.join("dmesg-efi_pstore-1"),
        "<0>[ 9.1] Kernel panic - not syncing: Attempted to kill init!\n",
    )
    .map_err(|e| e.to_string())?;
    let (why, files) =
        logd::panics::take(&pstore, &kept, "20261120T230000").ok_or("nothing was taken")?;
    let _ = std::fs::remove_dir_all(&pstore);
    let _ = std::fs::remove_dir_all(&kept);
    Ok(format!("`{why}`, {files} record taken"))
}

/// Its installation media checked: signed by its key, each slot's file
/// its manifest's.
fn media() -> Result<String, String> {
    use ed25519_dalek::{Signer, SigningKey};
    let names = &appliance::LoadSourceNames::DEMO;
    let root = scratch("media");
    let dir = root.join(loadsource::payload::dir(names));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (a, b) = (b"slot a".to_vec(), b"slot b".to_vec());
    let file = |name: &str, bytes: &[u8]| loadsource::payload::File {
        name: name.into(),
        bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
    };
    let manifest = loadsource::payload::Manifest {
        product: "QSDEMO".into(),
        release: "V1R0M0".into(),
        build: "fixture".into(),
        files: vec![
            file(loadsource::payload::SLOT_A, &a),
            file(loadsource::payload::SLOT_B, &b),
        ],
    };
    let key = SigningKey::from_bytes(&[6; 32]);
    let text = manifest.render(names);
    let put = |n: &str, bytes: &[u8]| std::fs::write(dir.join(n), bytes);
    put(loadsource::payload::MANIFEST, text.as_bytes()).map_err(|e| e.to_string())?;
    put(
        loadsource::payload::SIGNATURE,
        hex::encode(key.sign(text.as_bytes()).to_bytes()).as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    put(loadsource::payload::SLOT_A, &a).map_err(|e| e.to_string())?;
    put(loadsource::payload::SLOT_B, &b).map_err(|e| e.to_string())?;
    let keys = key.verifying_key().to_bytes();
    let payload = loadsource::payload::verify(&root, names, "V1R0M0", &keys)?;
    put(loadsource::payload::SLOT_B, b"slot B").map_err(|e| e.to_string())?;
    let refused = loadsource::payload::verify(&root, names, "V1R0M0", &keys).is_err();
    let _ = std::fs::remove_dir_all(&root);
    if !refused {
        return Err("a changed slot was taken".into());
    }
    Ok(format!(
        "{} files verified against its key; a changed slot refused",
        payload.manifest.files.len()
    ))
}

/// One part: what it says when it held, or why it did not.
type Part = fn() -> Result<String, String>;

fn main() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() == Some("confined") {
        confined_child();
    }
    let parts: [(&str, Part); 11] = [
        ("processor", processor),
        ("qemu", qemu),
        ("inventory", inventory),
        ("netd", network),
        ("update", updates),
        ("servicedump", dumps),
        ("problem", problems),
        ("confine", confinement),
        ("buildaid", release),
        ("panics", panics),
        ("payload", media),
    ];
    for (name, part) in parts {
        let line = part().map_err(|e| format!("QSDEMO {name} FAILED: {e}"))?;
        println!("QSDEMO {name}: {line}");
    }
    Ok(())
}
