# Service Tools

The console of a QS/VS host, on its display and on its first serial
port: what must work before the network does. It is the only way to
configure a host by hand; partitions are made and run from the
management console, QS/MC, over QSI (`docs/QSISERVER.md`), which
option 4 pairs.

The panels every Quartz appliance has are `base/panels` (shared with
QS/MC); QS/VS's menu, its system status and its disk units are
`svc/tools`, the service `tools` of PID 1's table. Option 8 of the
menu comes with Phase 9, service and code update. Every panel's screen
is a golden file under `tools/conformance/golden/`.

## The two sessions

The display and the serial port each have a session of their own; what
one shows the other does not.

**The display** shows the status screen while nobody is signed on: the
release, the slot with the A/B record, the host name, the management
address or the reason there is none, the date and time, the reference
code posted last, and a line a service. Enter leads to sign-on.

**The serial port** (115200 8N1) is PID 1's while nobody is signed on:
it prints each reference code as it is posted, the status screen as
lines whenever it changes, and a heartbeat every five seconds, as in
Phase 1. Enter leads to sign-on; from then until sign-off the port
carries the session's screens, drawn with ANSI sequences, and PID 1
prints nothing there. What it would have printed is on the display and
in the logs. At sign-off the terminal is cleared and PID 1 has the
port again.

## Keys

| Key | On the display | On a serial terminal |
|---|---|---|
| Enter | Enter | Enter |
| next field, previous field | Tab, Shift+Tab | Tab, Shift+Tab |
| clear the field from the cursor and go on | keypad + | Ctrl+K |
| F1 to F12 | F1 to F12 | the terminal's own, or Esc and `1` to `9`, `0`, `-`, `=` |

F1 is help on every panel. F12 goes back one panel, F3 to the main
menu. On the main menu either signs off.

## Sign-on

A service tools user ID and its password. User IDs are not case
sensitive; passwords are. The password is not shown.

- A wrong password and an unknown user ID are answered alike.
- Five wrong passwords in a row disable a user ID for five minutes.
  The right password does not help while that lasts. A new password
  given from another user's session enables the ID at once.
- Every refused sign-on is in the system log, with the user ID and
  where it was typed, and never with what was typed as the password.
- A session with no key for ten minutes is signed off. A sign-on
  display with no key for a minute is left.

The count of wrong passwords is kept with the users; the five minutes
are counted in memory, so a restart of the console service ends them.

## Before any session: the unit to keep

One panel is shown without a sign-on, in place of everything above,
and only when the two units of a mirrored load source each ran without
the other (`A2003040`, `docs/REFCODES.md`). The host has then mounted
no `/data` and started no service but the storage service, so there
are no users to sign on as: the users are on `/data`, and which `/data`
is the question. Being at the machine's display or serial port is the
authority, as it is for the installer. PID 1 runs the console for this
one panel, on the display and the serial port at once, with the same
screen on both.

```
                        Select Load-Source Unit to Keep
 Each unit ran without the other. Type 1 by the unit to keep.

 Opt Unit  Location                Serial              Last written
        1  P0000-B00-D10-F0        QSVSHD0             2026-11-12 10:22 UTC
        2  P0000-B00-D11-F0        QSVSHD1             2026-11-12 10:31 UTC

 The host has started nothing and waits for this choice. The unit
 that is not kept is written again from the one that is: whatever
 was written to it while it ran alone is lost.
```

Each unit has its location code, its disk's serial number and the time
its disk was last written, in UTC (the display zone is on `/data`
too); `not known` for a disk last written by a build before the label
held the time. Type 1 by a unit and press Enter; `Confirm Unit to
Keep` names the unit kept and the one that is written again, and F10
does it. F3 and F12 do nothing here: the host cannot go on without a
choice, and waits as long as it takes.

The console asks the storage service, which writes the kept unit's
label to both disks with the other unit stale, posts `C2003050` and
assembles the set. The IPL then goes on as any other, and the other
unit is written from the one kept while the host runs (`C2003020`,
`C2003025`). To look at what a unit holds before choosing, power the
machine off and start it with that disk alone.

## Service tools user IDs and passwords

The users are kept in `/data/state/users.db`, through QS/400's
`svcusers`: each user ID with a salted Argon2id hash of its password.
No password is kept, logged or shown anywhere. Every user can do
everything in Service Tools; there is one kind of user in this release.

| Rule | |
|---|---|
| User ID | 1 to 10 letters and digits, kept in upper case |
| Password | 8 to 128 characters, at least one letter and one digit, not containing the user ID, no control characters |

## The main menu

```
                              QS/VS Service Tools
 Select one of the following:

      1. Work with system status
      2. Work with disk units
      3. Work with network
      4. Management console pairing
      5. Work with service tools user IDs
      6. Date and time
      7. Display logs
      8. Service functions
      9. Power off or restart
     10. Work with I/O for partitions
```

### 1. Work with system status

The host as it is now; the display follows it. The host's state (every
service has been ready, or the IPL is in progress); the release, the
build and whether the image is a release or a test image; the slot
this IPL came from, with the A/B record; the reference code posted
last; the management line; what `/data` is mounted from; what the KVM
probe found; the uptime; the partitions, counted: how many are defined
and how many are running, starting, shutting down or in error; and
each service of the table with its state and how often it has failed,
two to a line, or three when two would not all fit on the screen.

**F11, Display Partitions.** Every partition of the host, a row each,
as the partition manager says: its ID, its name, its state (Not
activated, Starting, Running, Shutting down, Error), its current
reference code, live (the system's own while it posts them on its
service channel, else the host's: `docs/SERVICE.md`), the system's
release as its service channel said it (`-` with none), and what it
holds now: its processors (`2 dedicated`, or its processing units and
virtual processors, `0.50 on 1 shr`) and its memory. A partition that is not
activated holds nothing and shows `-`. The display follows the
partitions; F7 and F8 page a long list. **Nothing here acts on a
partition**: there is no option to type, and a partition is created,
changed, activated and shut down from QS/MC. A partition manager that
does not answer is said so in place of the list.

### 2. Work with disk units

The load source: its protection, and each unit with its location code,
its disk's serial number, its size and its state.

| State | The unit |
|---|---|
| Unprotected | is the one disk of a load source that is not mirrored |
| Active | is in step with the other |
| Resuming | is being written from the other: a resynchronization or a rebuild, with its percent |
| Failed | is there and failed I/O; the host runs on the other |
| Missing | was not found; the host runs on the other. Its location code and serial number are the ones it had |

Under them, how many other disks the machine has; option 6 shows each.

**1. Start mirrored protection** adds a disk to a load source of one
disk. Choose the disk from the list of those not configured and confirm
that it is erased. The storage service gives it the partitions, a copy
of the ESP with both slots, the A/B record and the set's label, and the
firmware its boot entries, while the host runs (`C2003040`). The
protection itself starts at the next IPL, which assembles the set and
writes the new unit from the first; restart the host when it suits. The
disk must be at least as large as the first one's data partition
needs. A load source that was not written by the installer (a built
disk image) cannot be mirrored: its file system does not have the
sectors the set's device has.

**2. Replace a failed unit** puts a disk in the place of a unit that is
missing or failed. Choose the disk and confirm; it is prepared the same
way and then rebuilt from the other unit while the host runs
(`C2003030`, `C2003035`). The display follows the rebuild; Enter leaves
it and the work goes on. When it says both units are in step the load
source is protected again, and the host starts from either disk.

**5. Display the units again** reads the states anew.

**6. Display disk configuration** shows every disk of the host by its
location code, serial number and size, and what it is: the load
source; a unit of one of this host's storage pools, with the pool and
the unit's state; a hot spare of a pool; not configured; or foreign,
with what is on it (another host's pool, a partition table, a file
system, another volume manager's label). A foreign disk is taken only
when its name is typed to confirm that it is erased.

**7. Work with storage pools** lists the pools: each with its
protection, its state (with the work's percent while it rebuilds or
resynchronizes), its size and how much of it is allocated; `*` marks
the one that holds the media library. Type an option by a pool:

| Option | Does |
|---|---|
| 5 | displays the pool: its size, allocated and provisioned bytes, each unit by location code, serial, size and state, and the rebuild or resynchronization with its progress. It follows the work |
| 6 | puts a disk in the place of a unit that failed or is missing: choose the disk, type its name to confirm. It is rebuilt while the host runs |
| 7 | adds a hot spare: choose the disk, type its name to confirm |
| 8 | takes a hot spare away: choose it, type its name. It is not configured afterwards |
| 2 | renames the pool, while no partition uses it |
| 4 | deletes the pool, when it holds no virtual disk and no media: type its name to confirm. Its disks are not configured afterwards |
| 9 | makes the pool the media repository: type its name to confirm |

**F6 creates a pool**: its name and protection (`none`, `mirrored`,
`raid5`, `raid6`), then the disks: `1` by each member, `2` by each
hot spare, from those not configured and the foreign ones. The next
display shows the pool's capacity and lists the disks that hold what
is not this host's; typing the pool's name confirms, and every disk
chosen is erased. **F7 imports an image** into the media library from
a device of the host: an optical drive's whole disc (`sr0`, the file
left empty), or a file of a removable volume (`sdb1` and its path).
The import runs while the host does; the history says when it ends.

**8. Display virtual disks** and **9. Display media library** show the
virtual disks of every pool (size, allocated, thin or thick, which
partition and slot hold it) and the library's optical images and tape
volumes, with the line that they are managed from QS/MC. Nothing of
them is changed here.

### 3. Work with network

A menu of four:

```
 1. Configure management network
 2. Display network hardware
 3. Work with virtual switches
 4. Display partition adapters
```

**1. Configure management network.** Everything on the installer's network screen, with the values in use:
the host name, the interface, DHCP or a static address with its prefix
length and gateway, the VLAN ID, the DNS servers. Enter applies; nothing
restarts.

A change of the network is tried before it is kept. The network service
puts it on the interface and shows the trial:

- as soon as the interface has carrier and an address, the change is
  kept and written to `network.toml`;
- if it has neither after 30 seconds, the configuration of before is
  put back on the interface and nothing was written;
- F10 keeps the change at once, whatever the interface says: for a
  network that is not connected yet.

So a mistyped address or a wrong VLAN cannot cut off a host that is
managed over that network: the operator who cannot reach the console
any more need only wait. A change of the host name is made at once. A
kept change returns to this menu with the line that it changed.

The management interface is never a switch's uplink, and an interface
that is on a switch is never taken for management (`docs/NETWORK.md`).

**2. Display network hardware.** Every network interface of the host:
its location code, MAC address, driver, speed, whether it has a link,
and what it is: `Management`, `Uplink of <switch>`, `Bond member of
<switch>`, or `Not used`.

**3. Work with virtual switches.** The switches, each with its VLANs,
MTU, its uplink's state (`None` for an internal switch, `Up`,
`Degraded`, `Down`, `Missing`) and how many partitions' ports are on
it. F6 makes a switch: its name, its VLANs (apart by spaces) and its
MTU. By a switch:

| Option | What it does |
|---|---|
| 2 | change its VLANs and MTU; partitions on it go on running |
| 4 | delete it, once its name is typed in full; one a profile names or a partition is on is refused |
| 5 | display it: its VLANs, MTU, uplink with each interface by location code and its link, native VLAN, ports |
| 7 | set its uplink, or move it: 1 by one interface, or by several to bond them, then the bond mode (`active-backup` or `802.3ad`) and the native VLAN, or none |
| 8 | clear its uplink: it is internal from then on |
| 9 | display its forwarding table: each address, its VLAN, where it is (a partition and slot, the uplink, the switch) and how it came to be there |

**4. Display partition adapters.** The running partitions' adapters:
partition, slot, switch, port VLAN, MAC address, link, and the frames
each sent and was sent; with the line that adapters are managed from
QS/MC. Nothing of them is changed here.

### 4. Management console pairing

The QSI host server's consoles, over its local socket
(`docs/QSISERVER.md`). Nothing here keeps a pin or a password: the
server does.

```
                          Management Console Pairing
 Select one of the following:

      1. Work with paired consoles
      2. Start pairing
      3. Display management interface
```

On a processor that lacks what QSI's cryptography needs (AVX2, ADX, AES,
PCLMULQDQ; `A2010010`) the menu says so above the options, `The
processor lacks AVX2, ADX: the service interface is down.` and
`Everything at this console works.`, and starting pairing is refused
with the same words.

**1. Work with paired consoles** lists the consoles paired, at most two:
the name each gave when it paired, the start of its certificate's
fingerprint, when it last had a session, and `connected` while it has
one. 5 by a console shows it whole: its name, its whole fingerprint in
groups of four, when it paired, its last contact and the address it
came from, and whether it has a session now. 4 removes it after its name
is typed in full (`Confirm Remove of Console`): it can connect no more,
and a session it has ends at once (`C2010050`).

**2. Start pairing** makes a password and shows it in groups of four,
with when it expires and the attempts it has left, and the host's
certificate fingerprint for the operator to compare with what the
console shows. The password is shown on this panel only: leaving it,
or F12, ends pairing and the password is gone (`C2010040`). F5 shows the
attempts left, or that the password is gone because a console paired
with it, it expired, or its attempts ran out. With two consoles paired
no password is made: `Two consoles are paired with this host: remove
one first.`

**3. Display management interface** shows where the server listens
(TCP 2300 on each management address, or why it listens on nothing),
QSI's version and TLS 1.3, the host's certificate fingerprint, and the
sessions now: each one's address and its console's name, or `pairing`.

### 5. Work with service tools user IDs

The user IDs. F6 adds one, with its password typed twice. Option 2 by
a user ID gives it a new password, which also enables an ID that wrong
passwords have disabled. Option 4 removes it, after a confirmation.
The last user ID cannot be removed.

### 6. Date and time

The UTC offset dates and times are displayed in (`+01:00`, `-05:30`;
blank is UTC), and the time servers the clock is kept in step with.
With no time servers the clock may be set by hand: the date and the
time as they are now, in the offset shown. The host keeps its clock in
UTC and writes the RTC from it.

Only fixed offsets are known in this release: there is no time zone
database and no change for summer time (`docs/OPEN_QUESTIONS.md`
item 9).

### 7. Display logs

1 is the reference code history, 2 the system log, 3 the problem log.
The first two are shown newest first, a page at a time: F8 goes to
older lines, F7 back to newer ones. A line is its time in the display
zone, a mark (`W` for a warning or an attention code, `E` for an error
or a fatal code) and its text. The `Show` field filters by severity: 1
everything, 2 warnings and errors, 3 errors only.

**3. Problem log** (`Display Problem Log`) lists every problem the log
keeps, open or closed, newest first, two lines each: its number, when
it was opened and last seen, how often, its status (`Open`, `Cleared`,
`Closed`) and its code; then its description. It is read only: option 8,
1 works with them (`docs/PROBLEMS.md`).

### 8. Service functions

```
                               Service Functions
 Select one of the following:

      1. Work with problems
      2. Take a host dump
      3. Work with dumps
      4. Install an update from a removable volume
      5. Display code levels
      6. Shut down all partitions
```

What a host with no management console needs (`docs/PROBLEMS.md`,
`docs/DUMPS.md`, `docs/UPDATE.md`).

**1. Work with problems** (`tools-problems`) lists the open problems,
newest first (F11 shows the closed ones too): the number, `Open` or
`Cleared` (its condition no longer holds), when last seen, the code and
the description. 5 shows one whole (`Display Problem`): its code and
text, whose it is, first and last seen and how often, the message and
what to do (its second-level text), each resource with its location
code and serial number, and the analysis log; F7 and F8 page, F10
closes it. 4 closes one (`Close Problem`): Enter when its condition no
longer holds; `Y` to close it anyway while it holds. The close is the
signed-on user's, kept in the problem.

**2. Take a host dump** (`Take Host Dump`) says what a dump holds and
that no secret goes in; Enter takes one, by the signed-on user at this
console, and the message line names it.

**3. Work with dumps** (`tools-dumps`) lists the host's dumps and each
partition's: whose, the name, the size and when it was taken. 6 copies
one to a removable volume (`Copy Dump to Removable Volume`): the device's
kernel name (`sdb1`); a FAT, exFAT, ext4 or xfs volume is mounted, the
dump written to its top directory (a partition's named after the
partition), and the volume unmounted. 4 deletes one after Enter.

**4. Install an update from a removable volume** asks for the device
(`sdb1`), mounts it to read only and lists the update images on it
(`.upd`, at the top or one directory down). 1 by one stages it, 2 stages
it allowing an older release; it is copied to the host and verified,
and a refusal says which check failed. `Update Image` then shows what it
is (product, release, build, fixes, size) beside the release that runs
and the slot it is written to: F10 installs it, F12 discards it.
`Installing Update` shows the progress (F5); once the slot is pending,
1 restarts now and 2 leaves it for later. `Confirm Restart into Update`
lists the partitions that run, which are shut down through their
operating systems first, with the seconds each is given (300), and
takes `RESTART` typed.

**5. Display code levels** (`Display Code Levels`) shows each slot: the
release and build it holds, its fixes, whether the host runs from it and
whether it is the default, and how its last activation ended (never,
pending, confirmed, rolled back with why); then the slot pending and
what is staged.

**6. Shut down all partitions** lists the partitions that run and takes
`SHUTDOWN` typed: each is shut down through its operating system, in
the reverse of the order they were activated in.

### 9. Power off or restart

1 powers the host off, 2 restarts it; each asks for F10 to confirm.
The shutdown is the orderly one of `docs/PHASE1.md`: the services end
in reverse order, `/data` is unmounted, the storage service marks its
set clean, and the machine stops.

### 10. Work with I/O for partitions

Every PCI device and physical tape drive of the host, by location code,
14 to a page (F7 and F8), with what the host uses each for, whether it
is released for partitions and which partition holds it
(`docs/PHYSICALIO.md`):

```
                          Work with I/O for Partitions
 2=Release   4=Take back   5=Display

 Opt Location                  Description              Use
  _  P0000-B00-D02-F0          NVMe controller          host: load-source di
  _  P0000-B00-D1A-F0          Ethernet controller      released
  _  P0000-B00-D1B-F0          NVMe controller          held by IO
  _  NOPCI-ST0                 tape drive Linux scsi_de not released
```

- **2, Release.** `Confirm Release for Partitions` lists every device of
  the isolation group that goes with it and asks for the device's
  location code typed in full; Enter releases the group, F12 goes back
  with nothing released. A device the host uses, or whose group holds
  one, cannot be released; the message says why.
- **4, Take back.** `Confirm Take Back`: Enter gives the group back to
  the host's drivers, while no partition holds it and no profile names
  it.
- **5, Display.** `Display I/O for Partitions`: the device whole, its
  PCI address, isolation group, the devices that go with it, how it
  resets, its interrupts and its use, and every reason it cannot be
  released. For a drive, its serial number and whether a cartridge is
  loaded.

A management console sees the same list over QSI and names released
devices in profiles; only this option releases one. The golden screens
are `tools-io`, `tools-io-display`, `tools-io-release` and
`tools-io-released`.

## Who changes what

The console edits no state file itself.

| Change | Asked of | Which |
|---|---|---|
| the management network | the network service | applies it on trial, writes `network.toml` when it is kept |
| the clock set by hand | the network service | sets the clock and the RTC |
| the host name, the display zone, the time servers | PID 1 | writes `host.toml`, takes the name and the zone into use; the network service then reads the time servers again |
| power off, restart | PID 1 | the orderly shutdown |
| a disk added or replaced | the storage service | prepares the disk, keeps the set's label, rebuilds |
| a storage pool made, a unit replaced, a hot spare added or taken away, a pool renamed or deleted, the media repository designated, an image imported | the pool service | `docs/POOLD.md` |
| the unit to keep, of two that ran apart | the storage service | writes the kept unit's label to both disks, assembles the set |
| service tools users | the console service's own users' store | `users.db`, written beside and renamed, as `svcusers` does |
| a problem closed | the problem service | `docs/PROBLEMS.md` |
| a host dump taken | PID 1 | `docs/DUMPS.md` |
| a host dump deleted | the console itself, in `/data/dumps/host` | the dump's file; `C2011020` |
| a partition's dump deleted or copied | the partition manager | `docs/DUMPS.md` |
| an update staged, installed, activated | the update service | `docs/UPDATE.md` |
| every partition shut down | the partition manager | its `EndAll` |
| a device or drive released or taken back | the partition manager | binds it to `vfio-pci` or back, keeps `physio.toml` (`docs/PHYSICALIO.md`) |

The users have no other owner: nothing but the local console signs
anyone on. The logs are read, never written. The partitions are read
too, from the partition manager, and nothing of them is changed here;
so are the virtual disks and the media library, from the pool service.
