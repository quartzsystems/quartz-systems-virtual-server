# Security

What a QS/VS host trusts and what it does not, the boundaries between
them, and for each what enforces it and which suite shows it. Phase 11
reviewed every one (`docs/PHASE11.md`).

## What is trusted, and what is not

| Party | Trusted with | Not trusted | Enforced by | Shown by |
|---|---|---|---|---|
| A partition's system (the guest) | its own memory, disks, adapters and devices | anything else: every byte it controls is hostile (register writes, virtqueues, SCSI commands, its disks, its service channel, its console) | `qsvmm` reads guest bytes through bounded readers that never panic or allocate by a guest's size (`docs/VMM.md`); the VMM runs confined (below); the partition manager reads the channel as hostile (`docs/SERVICE.md`); ports pass only the adapter's address on its own VLANs (`docs/NETWORK.md`); a passed device is given with its whole IOMMU group and only the partition's memory is mapped for its DMA (`docs/PHYSICALIO.md`) | `partition-damaged`, `service-hostile`, `pio-hostile`, `pio-isolation`, `net-may-not`, `vmm-confined`, every fuzz target |
| A paired console (QS/MC) | the requests its user may make | its bytes before and after the handshake | QSI's TLS 1.3 with both ends pinned; the session's bounds; every request checked and translated to the service that owns the state (`docs/QSISERVER.md`) | `qsi-*`, `fuzz --target qsi`, `--target qsi-pairing` |
| A console that is not paired | pairing, while a password is out | everything else | the server turns it away unless a pairing password is out, then admits pairing alone; five attempts a password, ten failures a minute from an address and it is turned away for a minute | `qsi-pairing`, `qsi-hostile` |
| The network | carrying frames | anything it says | the host listens on TCP 2300 of its management addresses alone; it has no address on a switch, bridge, tap or bond and forwards nothing; it makes no outbound connection but DHCP, DNS and NTP; no packet filter is needed since nothing else listens | `sockets`, `net-host-unreachable`, `net-internal` |
| Update images | being installed, when a key the running image trusts signed their manifest | anything unsigned or signed otherwise | the update service verifies before a byte is written, and nothing turns that off (`docs/UPDATE.md`) | `svc-update-refused`, `update-keys`, `fuzz --target update-image` |
| Installation media | being installed by its own installer | another build's payload | the installer verifies the payload's manifest against the key in its own initramfs | `media-refused` |
| Media in the library (ISO images, tape volumes) | being read by a partition's optical and tape devices | their contents | the IPL loader and the devices read them bounded | `pool-media-damaged`, `fuzz --target iso`, `--target tape` |
| Removable volumes (seed, update) | a seed's settings once, an update image | their files' contents | the seed and the answers are checked by every rule of the state files before anything is applied; an update image as above | `answers-invalid`, `fuzz --target seed` |
| Disks the host did not make | nothing: they are another host's or blank | their labels | the host takes only its own load source's disks and pools whose labels name it (`docs/STORAGE.md`); labels are read bounded | `pool-foreign`, `fuzz --target pool-label`, `--target ls-label` |
| The firmware | starting the host's own UKIs | nothing after | Secure Boot when the operator enrols the db certificate (`docs/INSTALL.md`) | `secure-boot` |

## The VMM confined

Each partition's `qsvmm` (`vmm/qsvmm/src/confine`):

1. As root, with every descriptor it needs already open: a mount,
   network, IPC and UTS namespace of its own; its root an empty,
   read-only `tmpfs`, moved over `/` and entered with `chroot` as
   `switch_root` leaves an initramfs (no path names anything of the host's: no `/proc`,
   `/dev`, `/data` or another partition's files; its own log directory
   is bound at `/log` in it, below); a network with nothing but its own
   loopback.
2. The drop: the capability bounding set emptied, supplementary groups
   cleared, the partition's own user and group (23000 plus its ID), no
   new privileges; a return to root is checked to fail.
3. A seccomp filter of every call any of its threads makes, built by
   hand as classic BPF (`confine/bpf.rs`), installed before any thread
   starts. Then each thread narrows it to its kind's calls as it
   starts. Filters stack, so a thread can only lose calls. A call of
   another ABI (32-bit, x32) is refused whatever it names.

| Kind | Its own calls, past every thread's (memory, futexes, time, `read`, `write`, `close`, thread exit) |
|---|---|
| processor | `ioctl` for KVM's processor requests (running it, its state, its clock, interrupt routing, ioeventfds, irqfds, MSI), vhost-net's (its owner and memory table at an adapter's first start, its features and rings) and the tap's offloads, VFIO's interrupts; `eventfd2`, `epoll_create1`, `epoll_ctl`, `pread64`/`pwrite64` (a passed function's regions); `clone` of a thread (a device's worker) and `seccomp` to add a filter; a signal to a thread of its own process |
| device worker | `epoll_wait`, `pread64`/`pwrite64`/`preadv`/`pwritev`, `fsync`/`fdatasync`, `fallocate`, `ftruncate`, `sendmsg`/`recvmsg` (the tape relay); a signal to a thread of its own process (a disk's write holds the processors when its pool is full) |
| serial log | `openat` and `renameat` in the log directory's descriptor alone, `fdatasync`, `sendto` |
| console, console input, channel ports | `accept4`, `recvfrom`/`recvmsg`, `sendto`/`sendmsg`, `setsockopt`, `shutdown`; the console's listener may start its input threads |
| control | the listeners' calls, a dump written to the file it was handed, KVM's state requests for the dump, a signal to its own threads |
| drives, signals, main | sleeping; `rt_sigtimedwait`; the end's `fsync` |

`ioctl` is allowed only for the requests named, `clone` only with the
exact flags of a thread, `prctl` only to name a thread or set no new
privileges, `tgkill` only to its own process, and `tkill` only with the
signal that brings a processor out of the guest (its target cannot be
checked by a filter; any other process is not the VMM's user, and the
kernel refuses it). Any other call ends the whole process with `SIGSYS`;
the partition manager posts `A2005190` (its problem opens), and the
partition is in Error holding nothing. The other partitions and every
service carry on. A unit test runs every kind's filter against a list
of calls a VMM never needs (`open`, `execve`, `fork`, `socket`,
`ptrace`, `setuid`, `capset`, `mount`, `pivot_root`, `unshare`, `setns`,
`kill`, `bpf`, `perf_event_open`, `process_vm_readv`, `keyctl`,
`io_uring_setup`, `userfaultfd`, `memfd_create` and others) through an
interpreter of the program.

The policy was taken from `strace` of a QS/400 partition's whole life
(its IPL, the console, the service channel's ports, a tape unloaded and
loaded, a hold and a dump) and from the learning mode, which saw neither
an adapter's first start nor a full pool: those two were added on
2026-11-18, when the network and pool suites first ran confined; a test image's
hook (`confinelearn`) in which a refused call is reported and fails
instead of ending the VMM: a QS/400 partition's run in it refused
nothing. `vmm-confined` shows the running VMM as the kernel sees it
(seccomp mode 2, no new privileges, no capability in any set, its own
user, `lo` alone, one empty read-only `tmpfs` as root), and a VMM made to
try each of twelve forbidden calls, from its control thread and from a
processor, ended with `A2005190` each time while the other partition ran
on in the VMM it had.

What it keeps, and why it is accepted:

- **Its log directory.** The serial log thread may `openat` and
  `renameat` relative to the log directory's descriptor and nothing
  else, to start the log's next generation. The directory
  (`/data/log/partitions/<id>`, its own user's, mode 0700) is bound at
  `/log` inside the empty root, with no set-user-ID, no device and
  nothing executable, and the descriptor is opened again there before
  the drop: a name resolved against it, `..` included, stays in the
  empty root (`vmm-confined` lists what is above it). A VMM taken over
  could create, rename over or fill files in its own log directory, and
  nothing else; the filling is bounded by `/data`'s room like any log's.
  It was found in Phase 11's review: until then the descriptor was the
  host's own, opened before the `chroot`, and `..` from it led out to
  whatever the partition's user could read.
- **KVM's requests.** The whole surface of KVM's ioctls a VMM needs
  stays. KVM is the host kernel's and is hardened there; what is shut is
  every other way into the kernel.
- **Its own memory and its devices' files.** By design: they are the
  partition's.

## The services

Every service but the QSI server runs as root, and PID 1 starts each
with only the capabilities it keeps: every other is taken from its
bounding set before `exec`, and it is given no new privileges, so it and
everything it starts hold its set and no more
(`base/supervise/src/table/caps.rs`, `lic/qsvsinit/src/host/table.rs`).

| Service | User | Capabilities kept, and why | What it writes |
|---|---|---|---|
| `qsvsinit` (PID 1) | root | all: it mounts, reserves huge pages, binds devices to `vfio-pci`, writes the A/B record and the firmware's variables, powers off | the A/B record and the set's label, the firmware's boot entries, `/data/state` (host settings), the inventory, host dumps, `/data/log/panics` |
| `stgd` (load source) | root | `SYS_ADMIN` (ublk), `SYS_RAWIO`, `IPC_LOCK` and `SYS_RESOURCE` (io_uring's buffers), `DAC_OVERRIDE`, `FOWNER` | the load-source disks' data partitions, the set's label and resume map |
| `logd` | root | `SYSLOG` (the kernel's log, root's alone) | `/data/log` |
| `probd` | root | `CHOWN` (its socket shared with the QSI server) | `/data/problems` |
| `netd` | root | `NET_ADMIN`, `NET_RAW` and `NET_BIND_SERVICE` (DHCP), `SYS_TIME` (NTP sets the clock) | `network.toml`, the management interface's configuration, the clock |
| `poold` | root | `SYS_ADMIN` (mounts, ublk), `SYS_RAWIO`, `IPC_LOCK`, `SYS_RESOURCE`, `DAC_OVERRIDE`, `FOWNER`, `CHOWN` | pool disks, `pools.toml`, the pools' file systems (virtual disks, the library) |
| `vswitchd` | root | `NET_ADMIN`, `NET_RAW`, `CHOWN` | `vswitch.toml`, bridges, bonds, taps |
| `partd` | root | `CHOWN`, `DAC_OVERRIDE`, `FOWNER` (the VMMs' directories), `KILL` (VMMs run as their own users), `SETUID`, `SETGID`, `SETPCAP`, `SYS_ADMIN` and `SYS_CHROOT` (what a VMM needs as root for its drop, its namespaces and its empty root; it inherits these), `IPC_LOCK` (a VMM pins memory for a device's DMA), `SYS_RAWIO` (`SG_IO` of a tape drive), `SYS_NICE`, `SYS_RESOURCE` | definitions and profiles, control groups, devices' driver bindings, partitions' dumps |
| `qsvmm` | root at start, then 23000 + ID | none after its drop | its serial log; a dump into a file handed to it |
| `tools` (Service Tools) | root | `SYS_TTY_CONFIG` (the display's console) | the service tools users' hashes; everything else is asked of the service that owns it |
| `updd` | root | `SYS_ADMIN`, `SYS_RAWIO`, `CHOWN` | the slot that does not run, on every load-source disk; staged images |
| `qsid` (QSI server) | root to read its identity and own its directory, then 22990 | at start `SETUID`, `SETGID`, `SETPCAP` (its drop), `CHOWN`, `FOWNER`, `DAC_OVERRIDE` (its directory); none after | its pins and its event log |

## The host

### Kernel

`kernel/config/qsvs.config`'s hardening section, checked by the build
(`REQUIRED_Y`, `REQUIRED_N`). The command line is the locked one
(`console=ttyS0,115200 console=tty0 panic=-1 quiet`): everything is
the configuration's.

| Setting | Why |
|---|---|
| `SECURITY_DMESG_RESTRICT` | the kernel's log is root's alone |
| `HARDENED_USERCOPY`, `FORTIFY_SOURCE` | copies to and from user memory, and string calls, checked |
| `SLAB_FREELIST_RANDOM`, `SLAB_FREELIST_HARDENED`, `RANDOM_KMALLOC_CACHES`, `SLAB_MERGE_DEFAULT` off | the allocators' layouts not to be relied on, freelists checked, caches not merged |
| `LIST_HARDENED`, `BUG_ON_DATA_CORRUPTION`, `SCHED_STACK_END_CHECK` | corruption found is a stop, not a continuation |
| `INIT_ON_ALLOC_DEFAULT_ON` | every allocation zeroed |
| `RANDOMIZE_KSTACK_OFFSET_DEFAULT`, `ZERO_CALL_USED_REGS` | the stack's place random on each call; registers zeroed on return |
| `PANIC_ON_OOPS`, `PSTORE`, `EFI_VARS_PSTORE` | an oops is a panic, a panic restarts the host, and its last messages are kept in the firmware's variables (below) |
| `SECURITY_YAMA` | ptrace only of a process's own descendants |
| `STRICT_KERNEL_RWX`, `STACKPROTECTOR_STRONG`, `VMAP_STACK`, `RANDOMIZE_BASE`, `RANDOMIZE_MEMORY`, `CPU_MITIGATIONS` | the baseline's, held |
| `MODULES` off | no module exists and none can be loaded |
| `KEXEC`, `CRASH_DUMP`, `HIBERNATION` off | no other kernel loaded, no image of this one written |
| `DEVMEM`, `DEVPORT`, `PROC_KCORE` off | no way to physical memory or ports from user space |
| `X86_IOPL_IOPERM`, `IA32_EMULATION`, `LEGACY_VSYSCALL_NONE` | no I/O ports from user space, no 32-bit programs, no vsyscall page |
| `LEGACY_TIOCSTI`, `LDISC_AUTOLOAD` off | no typing into another's terminal, no line discipline loaded on demand |
| `BINFMT_MISC`, `KPROBES`, `UPROBES`, `UPROBE_EVENTS`, `EFI_CUSTOM_SSDT_OVERLAYS`, `MAGIC_SYSRQ`, `DEBUG_FS` off, and `BLK_DEV_IO_TRACE`, which `x86_64_defconfig` turns on and which selects `DEBUG_FS` | nothing a hypervisor host uses (a test image's kernel has debugfs). Every symbol the fragment turns off is checked off in the built `.config`, so one that a default selects back on fails the build (`tools/build/src/product/kernel.rs`); until 2026-11-18 debugfs was on despite the fragment |
| `USER_NS`, `BPF_SYSCALL`, `USERFAULTFD`, `SECURITY_SELINUX` off | no user namespaces, no BPF programs from user space, no userfaultfd; no policy |
| `IPV6_SIT` off | no fallback tunnel device in every new network namespace: a VMM's has its loopback alone (found by `vmm-confined`) |

`MODIFY_LDT_SYSCALL` cannot be left out without `EXPERT`; no VMM's
filter allows it, and the services are trusted code. The command-line
options a hardened host often has were considered and left out because
the command line is locked: `slab_nomerge` and `init_on_alloc=1` are the
configuration's defaults above; `page_alloc.shuffle=1` and `vsyscall=none`
likewise; `lockdown=integrity` would refuse the test image's debugfs
and was not taken. The cost of the hardening on the measured baselines
is in `docs/PHASE11.md` ("Baselines").

### File systems

`/data` and every pool's file system are mounted `nodev,nosuid,noexec`,
and so are `/run`, `/dev` (`nosuid,noexec`), `/proc`, `/sys`, the
control groups, efivarfs and pstore. Once every mount point exists and
before any service starts, PID 1 mounts a small `tmpfs` at `/pools` (where
pools are mounted) and one at `/etc` (the network service's resolver
file, until 2026-11-18 left unwritable by the read-only root) and
remounts the root, the initramfs where every
program is, read-only: nothing on a file system that can be written is
ever executed.

### A host panic

An oops is a panic, and with `panic=-1` the host restarts at once. As it
panics the kernel writes its last messages into the firmware's
variables (`efi-pstore`). At the next IPL, once `/data` is mounted, PID 1
moves them from `/sys/fs/pstore` to `/data/log/panics` (the last eight
panics are kept), frees the firmware's variables, and posts `A2001005`
with why the kernel panicked, which opens its problem; the next host
dump holds the messages. `host-panic` shows it with LKDTM's panic, a
test image's kernel's alone.

### The service interface

The QSI server's limits are `docs/QSISERVER.md`'s: six sessions, four
handshakes at once, ten seconds a handshake, ten failed handshakes or
pairings from one address a minute and then a minute turned away, and
the session's own bounds. TLS: 1.3 only, graviola's cryptography
(TLS13_AES_128_GCM_SHA256, TLS13_AES_256_GCM_SHA384,
TLS13_CHACHA20_POLY1305_SHA256; X25519MLKEM768 and X25519), Ed25519
certificates pinned at both ends at every handshake, nothing resumed,
no tickets, no SNI. A pairing password is the host's, shown once at
Service Tools, kept in memory only, good for one pairing, fifteen
minutes and five attempts. The request log names the console and its
user and never a password, a key or a screen; `qsi-log` checks the log
after a pairing and every family's requests for any of them.

### A partition's memory

Huge pages given to a new partition are clean: the kernel zeroes a huge
page when it is first mapped into a process, and the VMM maps the
partition's memory anew at each activation. `memory-clean` shows it: on
a host whose huge pages are exactly one partition's memory, the first
partition fills its memory with a marker its own dump shows, ends, and
the next, given the same pages, holds none of the marker anywhere.

### Keys and signing

`docs/RELEASING.md`: the release signing key, the Secure Boot db key,
the trusted keys and their rotation, and what a development key does.
