# Service Tools

The console of a QS/VS host, on its display and on its first serial
port: what must work before the network does. It is the only way to
configure a host by hand; partitions are made and run from the
management console, QS/MC, in a later phase.

The panels every Quartz appliance has are `base/panels` (shared with
QS/MC); QS/VS's menu, its system status and its disk units are
`svc/tools`, the service `tools` of PID 1's table. Options 4 and 8 of
the menu come with later phases: 4 with Phase 8, the QSI host server,
and 8 with Phase 9, service and code update. Every panel's screen
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
      3. Configure management network
      4. Management console pairing  (not yet available: comes with Phase 8)
      5. Work with service tools user IDs
      6. Date and time
      7. Display logs
      8. Service functions  (not yet available: comes with Phase 9)
      9. Power off or restart
```

Options 4 and 8 are on the menu and cannot be selected: the message
line says which phase brings each (`It comes with Phase 8, the QSI
host server.`, `It comes with Phase 9, service and code update.`).

### 1. Work with system status

The host as it is now; the display follows it. The host's state (every
service has been ready, or the IPL is in progress); the release, the
build and whether the image is a release or a test image; the slot
this IPL came from, with the A/B record; the reference code posted
last; the management line; what `/data` is mounted from; what the KVM
probe found; the uptime; and each service of the table with its state
and how often it has failed. A last line says that partitions are
managed from QS/MC.

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

Under them, the machine's other disks, as not configured: storage pools
arrive in a later phase.

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

### 3. Configure management network

Everything on the installer's network screen, with the values in use:
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
any more need only wait. A change of the host name is made at once.

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

1 is the reference code history, 2 the system log. Each is shown
newest first, a page at a time: F8 goes to older lines, F7 back to
newer ones. A line is its time in the display zone, a mark (`W` for a
warning or an attention code, `E` for an error or a fatal code) and its
text. The `Show` field filters by severity: 1 everything, 2 warnings
and errors, 3 errors only.

### 9. Power off or restart

1 powers the host off, 2 restarts it; each asks for F10 to confirm.
The shutdown is the orderly one of `docs/PHASE1.md`: the services end
in reverse order, `/data` is unmounted, the storage service marks its
set clean, and the machine stops.

## Who changes what

The console edits no state file itself.

| Change | Asked of | Which |
|---|---|---|
| the management network | the network service | applies it on trial, writes `network.toml` when it is kept |
| the clock set by hand | the network service | sets the clock and the RTC |
| the host name, the display zone, the time servers | PID 1 | writes `host.toml`, takes the name and the zone into use; the network service then reads the time servers again |
| power off, restart | PID 1 | the orderly shutdown |
| a disk added or replaced | the storage service | prepares the disk, keeps the set's label, rebuilds |
| the unit to keep, of two that ran apart | the storage service | writes the kept unit's label to both disks, assembles the set |
| service tools users | the console service's own users' store | `users.db`, written beside and renamed, as `svcusers` does |

The users have no other owner: nothing but the local console signs
anyone on. The logs are read, never written.
