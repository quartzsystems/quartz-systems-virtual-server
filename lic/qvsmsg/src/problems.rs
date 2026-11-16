// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The problem table (`docs/PROBLEMS.md`): for every attention and
//! fatal code of the host (`docs/REFCODES.md`), whether it opens a
//! problem, and when it does, the message that describes it, the kinds
//! of resource it names and the progress codes that clear it.
//!
//! A message's values are the names of the resources the posting is
//! about, one for each kind the row names and in that order, then the
//! text the code was posted with.

use crate::Message;
#[allow(clippy::wildcard_imports)] // the table names the messages
use crate::catalogue::*;

/// A resource's kind, as a posting says it (`supervise::control::about`).
pub mod kind {
    /// A disk unit: a load-source unit or a pool's.
    pub const DISK_UNIT: &str = "disk-unit";
    /// A storage pool.
    pub const STORAGE_POOL: &str = "storage-pool";
    /// A virtual disk: its name and its pool.
    pub const VIRTUAL_DISK: &str = "virtual-disk";
    /// A physical network interface.
    pub const NETWORK_INTERFACE: &str = "network-interface";
    /// A virtual switch.
    pub const VIRTUAL_SWITCH: &str = "virtual-switch";
    /// A partition.
    pub const PARTITION: &str = "partition";
    /// A slot of the host's own code.
    pub const CODE_SLOT: &str = "code-slot";
    /// One of the host's supervised services.
    pub const SERVICE: &str = "service";
}

/// How a condition opens a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening {
    /// What describes it: its first-level text is the problem's
    /// description, its second-level text what to do.
    pub message: &'static Message,
    /// The kinds of resource its message names, in order.
    pub resources: &'static [&'static str],
    /// The progress codes whose posting marks it cleared.
    pub cleared_by: &'static [&'static str],
    /// The problem's code is the system's own, which the posting names,
    /// not the host's code that posted it.
    pub system_code: bool,
}

/// One attention or fatal code of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Condition {
    /// The code.
    pub code: &'static str,
    /// How it opens a problem; none when it does not.
    pub opens: Option<Opening>,
    /// Why it opens none: where a person sees it instead.
    pub note: &'static str,
}

const fn opens(
    code: &'static str,
    message: &'static Message,
    resources: &'static [&'static str],
    cleared_by: &'static [&'static str],
) -> Condition {
    Condition {
        code,
        opens: Some(Opening {
            message,
            resources,
            cleared_by,
            system_code: false,
        }),
        note: "",
    }
}

const fn none(code: &'static str, note: &'static str) -> Condition {
    Condition {
        code,
        opens: None,
        note,
    }
}

#[allow(clippy::wildcard_imports)] // the table names the kinds
use kind::*;

const ACTIVATION: &str = "An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history.";
const INSTALLER: &str =
    "The installer has no problem log: its screens and its log say what failed.";

/// Every attention and fatal code of the host, in order.
pub const CONDITIONS: &[Condition] = &[
    opens("A2001030", &PRB_RECORD, &[], &[]),
    opens("A2001035", &PRB_SLOT_FELL_BACK, &[CODE_SLOT], &[]),
    opens("A2001040", &PRB_DATA_NOT_MOUNTED, &[], &["C2001040"]),
    opens("A2001045", &PRB_DATA_FULL, &[], &["C2001045"]),
    opens("A2001050", &PRB_CLOCK, &[], &["C2001050"]),
    none(
        "A2001060",
        "A seed is the operator's: the console says why it was refused, and nothing was written.",
    ),
    opens("A2001070", &PRB_STATE_FILE, &[], &[]),
    opens("A2001072", &PRB_MEMORY_SHORT, &[], &["C2001072"]),
    opens("A2001075", &PRB_IDENTITY, &[], &["C2001075"]),
    opens("A2001080", &PRB_SERVICE_STOPPED, &[SERVICE], &[]),
    opens("A2001095", &PRB_BOOT_ORDER, &[CODE_SLOT], &["C2001095"]),
    opens("A2002005", &PRB_NO_INTERFACE, &[], &["C2002010"]),
    opens(
        "A2002010",
        &PRB_NO_CARRIER,
        &[NETWORK_INTERFACE],
        &["C2002010"],
    ),
    opens(
        "A2002020",
        &PRB_NO_ADDRESS,
        &[NETWORK_INTERFACE],
        &["C2002020"],
    ),
    opens("A2003010", &PRB_SET_NOT_ASSEMBLED, &[], &["C2003010"]),
    opens(
        "A2003020",
        &PRB_UNIT_FAILED,
        &[DISK_UNIT],
        &["C2003025", "C2003035"],
    ),
    opens(
        "A2003030",
        &PRB_REBUILD_FAILED,
        &[DISK_UNIT],
        &["C2003025", "C2003035"],
    ),
    opens("A2003040", &PRB_UNITS_APART, &[], &["C2003050"]),
    none("A2004010", INSTALLER),
    none("A2004015", INSTALLER),
    none("A2004070", INSTALLER),
    none("A2004090", INSTALLER),
    none("A2005000", ACTIVATION),
    none("A2005010", ACTIVATION),
    none("A2005020", ACTIVATION),
    none("A2005021", ACTIVATION),
    none("A2005022", ACTIVATION),
    none("A2005030", ACTIVATION),
    none("A2005031", ACTIVATION),
    none("A2005032", ACTIVATION),
    none("A2005033", ACTIVATION),
    none("A2005034", ACTIVATION),
    none("A2005040", ACTIVATION),
    none("A2005041", ACTIVATION),
    none("A2005050", ACTIVATION),
    opens("A2005060", &PRB_MACHINE_NOT_BUILT, &[PARTITION], &[]),
    none("A2005065", ACTIVATION),
    none(
        "A2005100",
        "The code before it says why the partition did not IPL, and is the one to look at.",
    ),
    opens("A2005120", &PRB_GUEST_FAULT, &[PARTITION], &[]),
    opens("A2005130", &PRB_VMM_ERROR, &[PARTITION], &[]),
    none(
        "A2005140",
        "The host ended the partition, as it was asked to.",
    ),
    opens("A2005150", &PRB_VMM_KILLED, &[PARTITION], &[]),
    opens("A2005160", &PRB_HELD_FOR_SPACE, &[PARTITION], &["C2005160"]),
    opens("A2005170", &PRB_HELD_FAULT, &[PARTITION], &[]),
    opens("A2005180", &PRB_DUMP_NOT_WRITTEN, &[PARTITION], &[]),
    none("A2006010", ACTIVATION),
    opens("A2006020", &PRB_AUTO_START, &[PARTITION], &[]),
    opens("A2006030", &PRB_PARTITION_STATE, &[], &[]),
    opens("A2006040", &PRB_CONTROLS, &[PARTITION], &[]),
    opens("A2006090", &PRB_NOT_ENDED, &[], &[]),
    opens(
        "A2007010",
        &PRB_POOL_NOT_ASSEMBLED,
        &[STORAGE_POOL],
        &["C2007010"],
    ),
    opens(
        "A2007020",
        &PRB_POOL_UNIT_FAILED,
        &[DISK_UNIT, STORAGE_POOL],
        &["C2007025", "C2007035"],
    ),
    opens(
        "A2007025",
        &PRB_POOL_EXPOSED,
        &[STORAGE_POOL],
        &["C2007035", "C2007040"],
    ),
    opens(
        "A2007030",
        &PRB_POOL_REBUILD_FAILED,
        &[STORAGE_POOL],
        &["C2007025", "C2007035"],
    ),
    opens("A2007040", &PRB_POOL_FAILED, &[STORAGE_POOL], &["C2007010"]),
    none(
        "A2007050",
        "A warning ahead of A2007051, which opens the problem.",
    ),
    opens(
        "A2007051",
        &PRB_POOL_NEARLY_FULL,
        &[STORAGE_POOL],
        &["C2007055"],
    ),
    none(
        "A2007060",
        "An import that fails is answered to whoever asked for it, and nothing of it is left.",
    ),
    opens("A2008010", &PRB_SWITCH_NOT_BUILT, &[VIRTUAL_SWITCH], &[]),
    opens(
        "A2008020",
        &PRB_UPLINK_LOST,
        &[VIRTUAL_SWITCH],
        &["C2008020"],
    ),
    opens(
        "A2008025",
        &PRB_BOND_MEMBER_DOWN,
        &[NETWORK_INTERFACE, VIRTUAL_SWITCH],
        &["C2008020"],
    ),
    opens(
        "A2008030",
        &PRB_UPLINK_MISSING,
        &[NETWORK_INTERFACE, VIRTUAL_SWITCH],
        &[],
    ),
    opens("A2009010", &PRB_CHANNEL_CLOSED, &[PARTITION], &["C2009000"]),
    opens("A2009020", &PRB_NOT_RESPONDING, &[PARTITION], &["C2009020"]),
    opens("A2009030", &PRB_IDENTITY_DIFFERS, &[PARTITION], &[]),
    none(
        "A2009040",
        "The shut down asked for was done, immediately at the end of its time.",
    ),
    Condition {
        code: "A2009055",
        opens: Some(Opening {
            message: &PRB_TERMINAL,
            resources: &[PARTITION],
            cleared_by: &[],
            system_code: true,
        }),
        note: "",
    },
    none(
        "A2009080",
        "The switch's own problem, A2008020, is the one to look at.",
    ),
    none(
        "A2010000",
        "Its cause opens the problem: no identity (A2001075), no management address (A2002005 to A2002020) or the processor (A2010010).",
    ),
    opens("A2010010", &PRB_PROCESSOR, &[], &[]),
    none(
        "A2010030",
        "One attempt: attempts that fail again and again open A2010035.",
    ),
    opens("A2010035", &PRB_PAIRING_FAILING, &[], &[]),
    opens("A2011010", &PRB_HOST_DUMP_FAILED, &[], &[]),
    none(
        "A2012010",
        "An image refused is answered to whoever staged it, with why, and is not kept.",
    ),
    opens("A2012030", &PRB_INSTALL_FAILED, &[CODE_SLOT], &["C2012040"]),
    none(
        "A2012070",
        "A2001035, posted by the slot that came back, opens the problem.",
    ),
    opens("B2001000", &PRB_PID1_FAILED, &[], &[]),
    opens("B2003000", &PRB_STORAGE_LOST, &[], &[]),
];

/// The row of `code`.
#[must_use]
pub fn condition(code: &str) -> Option<&'static Condition> {
    CONDITIONS.iter().find(|c| c.code == code)
}

/// Each condition the progress code `code` clears.
pub fn cleared(code: &str) -> impl Iterator<Item = &'static Condition> + use<'_> {
    CONDITIONS.iter().filter(move |c| {
        c.opens
            .as_ref()
            .is_some_and(|o| o.cleared_by.contains(&code))
    })
}

/// The table as `docs/PROBLEMS.md` lists it.
#[must_use]
pub fn table() -> String {
    let mut rows = vec![
        "| Code | Opens | Description | Message | Resources | Cleared by |".to_string(),
        "|---|---|---|---|---|---|".to_string(),
    ];
    for c in CONDITIONS {
        rows.push(match &c.opens {
            Some(o) => format!(
                "| `{}` | yes{} | {} | `{}` | {} | {} |",
                c.code,
                if o.system_code {
                    ", with the system's own code"
                } else {
                    ""
                },
                o.message.text,
                o.message.id,
                if o.resources.is_empty() {
                    "the host".to_string()
                } else {
                    o.resources.join(", ")
                },
                if o.cleared_by.is_empty() {
                    "a person only".to_string()
                } else {
                    o.cleared_by
                        .iter()
                        .map(|c| format!("`{c}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            ),
            None => format!("| `{}` | no | {} | | | |", c.code, c.note),
        });
    }
    rows.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_in_order_and_whole() {
        for (i, c) in CONDITIONS.iter().enumerate() {
            assert!(
                c.code.starts_with("A2") || c.code.starts_with("B2"),
                "{}",
                c.code
            );
            assert!(
                CONDITIONS[..i].iter().all(|o| o.code < c.code),
                "{}",
                c.code
            );
            match &c.opens {
                Some(o) => {
                    assert!(c.note.is_empty(), "{}", c.code);
                    // Its values: one for each kind, then the text.
                    assert!(o.message.values() <= o.resources.len() + 1, "{}", c.code);
                    assert!(o.message.id.starts_with("QVS51"), "{}", c.code);
                    for code in o.cleared_by {
                        assert!(code.starts_with("C2"), "{} {code}", c.code);
                    }
                }
                None => assert!(!c.note.is_empty(), "{}", c.code),
            }
        }
        assert_eq!(cleared("C2007035").count(), 3);
        assert!(condition("C2001000").is_none());
    }

    /// With `QVS_PROBLEMS=update` the document's table is written
    /// instead of compared.
    #[test]
    fn the_document_lists_the_table() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/PROBLEMS.md");
        let doc = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        let (begin, end) = ("<!-- problems -->\n", "<!-- end of problems -->");
        let table = table();
        if std::env::var_os("QVS_PROBLEMS").is_some_and(|v| v == "update") {
            let head = doc.split(begin).next().unwrap_or_default();
            let tail = doc.split(end).nth(1).unwrap_or("\n");
            std::fs::write(path, format!("{head}{begin}{table}{end}{tail}")).unwrap();
            return;
        }
        let listed = doc
            .split(begin)
            .nth(1)
            .and_then(|rest| rest.split(end).next())
            .unwrap_or_default();
        assert!(
            listed == table,
            "docs/PROBLEMS.md is not the table: run this test with QVS_PROBLEMS=update"
        );
    }
}
