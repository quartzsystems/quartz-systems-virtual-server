# QS/VS V1R0M0 release notes

QS/VS (Quartz Systems/Virtual Server) V1R0M0, version 1.0.0: the first
release. This is the release candidate; the release is tagged once the
runs on a real machine and the joint gate with QS/MC are done ("What the
tag waits on", below).

## What QS/VS is

A hypervisor appliance whose only job is to run QS/400 systems as
partitions. It is a Linux kernel and a userland written wholly in Rust:
no shell, no web interface, no SSH. It is managed by QS/MC (Quartz
Systems/Management Console) over QSI, the one service it listens on
(TCP 2300 of its management addresses, TLS 1.3 with both ends pinned),
and locally by its Service Tools console on the display and the serial
port.

It gives partitions dedicated or shared processors, memory in 2 MB huge
pages, virtual disks in storage pools (no protection, mirrored, RAID 5
or RAID 6), optical and tape devices over a media library, virtual
Ethernet over VLAN-aware switches, whole PCI devices with their IOMMU
groups, a physical tape drive command by command, a service channel to
each QS/400 system (its codes, its shut down, its console), dumps of a
partition and of the host, problems from the reference codes, and code
update into the slot that does not run, with rollback.

## What it needs

| | |
|---|---|
| Processor | x86_64 with hardware virtualization (Intel VT-x with EPT, or AMD-V with NPT), and AVX2, ADX, AES-NI and PCLMULQDQ: QSI's cryptography needs them, and a host without them posts `A2010010` and serves no console |
| Firmware | UEFI; virtualization on; for physical I/O, VT-d (or AMD-Vi) and interrupt remapping on. Secure Boot may be on or off: the release's UKIs are signed, and the certificate to enrol is on the media (`docs/INSTALL.md`) |
| Memory | the host keeps the larger of 2 GiB and 5% for itself; the rest is partitions' huge pages |
| Disks | one load-source disk, or two mirrored, each of at least the size `docs/INSTALL.md` gives; storage pools on other disks |
| Network | one port for management, not shared with partitions; further ports for switches' uplinks |
| Console | a display and keyboard, or a serial port at 115200 |

A partition's QS/400 needs its handoff 17 or later (V1R1M0) for the
service channel: its codes, its states, an orderly shut down, its
console in a viewer, its own dumps. An older QS/400 runs with no service
channel: it is Running once its kernel is entered and the host has
waited for a channel (`channel_wait_secs`, 30 s), it is shut down
immediately, and its console is the serial port alone
(`docs/SERVICE.md`).

QSI is version 1, with the capabilities `PHYSICAL_IO` and
`PARTITION_V2` (`docs/QSISERVER.md`).

## Installing and updating

Install from the ISO, attended at the display or the serial port, or
unattended from a seed file (`docs/INSTALL.md`). An update is one
signed image, brought by QS/MC over QSI or on a removable volume and
installed into the other slot from Service Tools option 8 or QSI; a
slot that does not confirm in 15 minutes rolls back
(`docs/UPDATE.md`). The host fetches nothing: it makes no outbound
connection but DHCP, DNS and NTP. A console reads the stream's signed
index to offer an update (`docs/RELEASING.md`).

## Known limits

Filled in at the end of Phase 11 (`docs/PHASE11.md`).

## Reporting a problem

Take a host dump (Service Tools option 8, or QSI's base family) and,
for a partition, its dump; send both, with the problem's identifier and
the reference codes from Service Tools option 7, to Quartz Systems. A
host dump holds the logs, the problems, the state files and the
service's view of every partition; it never holds the host's private
key, the users' password hashes or the consoles' pins
(`docs/DUMPS.md`).

## What the tag waits on

Filled in at the end of Phase 11 (`docs/PHASE11.md`, `docs/QUALIFICATION.md`).
