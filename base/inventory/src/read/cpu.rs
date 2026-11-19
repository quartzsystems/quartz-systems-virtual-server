// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The processors, from `/proc/cpuinfo`: one block of `key : value`
//! lines per thread, blocks parted by an empty line.

use std::collections::BTreeSet;

use state::doc::inventory::Processors;

/// The processors `cpuinfo` describes.
pub(crate) fn parse(cpuinfo: &str) -> Processors {
    let mut threads = 0usize;
    let mut sockets = BTreeSet::new();
    let mut cores = BTreeSet::new();
    let mut model = None;
    let mut virtualization = "none";
    for block in cpuinfo.split("\n\n") {
        let field = |key: &str| {
            block.lines().find_map(|line| {
                let (k, v) = line.split_once(':')?;
                (k.trim() == key).then(|| v.trim())
            })
        };
        let Some(processor) = field("processor") else {
            continue;
        };
        threads += 1;
        // A kernel without topology gives neither: one socket, a core each.
        let socket = field("physical id").unwrap_or("0");
        let core = field("core id").unwrap_or(processor);
        sockets.insert(socket);
        cores.insert((socket, core));
        if model.is_none() {
            model = field("model name");
        }
        if virtualization == "none" {
            let flags = field("flags").unwrap_or_default();
            let has = |flag: &str| flags.split_ascii_whitespace().any(|f| f == flag);
            if has("vmx") {
                virtualization = "VMX";
            } else if has("svm") {
                virtualization = "SVM";
            }
        }
    }
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Processors {
        sockets: count(sockets.len()),
        cores: count(cores.len()),
        threads: count(threads),
        model: model.unwrap_or_default().to_string(),
        virtualization: virtualization.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(processor: u32, socket: u32, core: u32, flags: &str) -> String {
        format!(
            "processor\t: {processor}\nvendor_id\t: GenuineExample\n\
             model name\t: Example CPU @ 2.40GHz\nphysical id\t: {socket}\n\
             siblings\t: 4\ncore id\t\t: {core}\ncpu cores\t: 2\n\
             flags\t\t: fpu vme {flags} sse2\nbogomips\t: 4800.00\n\n"
        )
    }

    #[test]
    fn two_sockets_of_two_cores_of_two_threads_are_counted() {
        let mut text = String::new();
        for processor in 0..8 {
            // Threads of a core are not next to each other.
            text += &thread(processor, processor % 2, (processor / 2) % 2, "vmx");
        }
        let p = parse(&text);
        assert_eq!((p.sockets, p.cores, p.threads), (2, 4, 8));
        assert_eq!(p.model, "Example CPU @ 2.40GHz");
        assert_eq!(p.virtualization, "VMX");
    }

    #[test]
    fn a_single_thread_is_one_socket_one_core_one_thread() {
        let p = parse(&thread(0, 0, 0, "svm"));
        assert_eq!((p.sockets, p.cores, p.threads), (1, 1, 1));
        assert_eq!(p.virtualization, "SVM");
    }

    #[test]
    fn a_processor_with_neither_extension_has_no_virtualization() {
        // A flag that only contains the letters does not count.
        let p = parse(&thread(0, 0, 0, "vmx_like svmx"));
        assert_eq!(p.virtualization, "none");
    }

    #[test]
    fn processors_with_no_topology_are_one_socket_and_a_core_each() {
        let p = parse("processor : 0\nmodel name : Plain\n\nprocessor : 1\n");
        assert_eq!((p.sockets, p.cores, p.threads), (1, 2, 2));
        assert_eq!(p.model, "Plain");
    }

    #[test]
    fn an_empty_cpuinfo_has_no_processors() {
        let p = parse("");
        assert_eq!((p.sockets, p.cores, p.threads), (0, 0, 0));
        assert_eq!(p.model, "");
        assert_eq!(p.virtualization, "none");
    }
}
