# Operations

What an operator of a QS/VS host does, from the day it is installed:
each task once, with the panel or the request it takes and the
document that says the rest. Installing is `docs/INSTALL.md`; every
panel of the local console is `docs/SERVICETOOLS.md`; every code a
host posts is `docs/REFCODES.md`.

A host is run from two places. **Service Tools**, on the host's display
and its first serial port, is for what must work before the network
does: the host's settings, its network, disks and pools, pairing,
service and power. **QS/MC**, the management console, does everything
else over QSI: partitions, their profiles and their devices, virtual
disks, the media library, virtual switches, and the same service
functions. A host with no console paired can be serviced at Service
Tools, but no partition can be made there.

## After installing

1. Sign on at Service Tools with the user ID and password the
   installation made. Option 5 makes a second user ID: a host with one
   is locked out by one forgotten password.
2. Option 1: the host's state says every service has been ready, the
   slot is the default with nothing pending, and the release is the
   one installed. A release image says neither `test image` nor
   `development key`.
3. Option 6: the date, time and time servers. A host with no time
   server keeps the clock it is set to.
4. Option 3: the management address. The host makes no connection out
   but DHCP, DNS and NTP; it is managed through its one listener, TCP
   2300 (`docs/QSISERVER.md`).
5. Option 2: the load source. With one disk, 1 starts mirrored
   protection on a second (`Start mirrored protection`, then a
   restart).
6. Option 2, 7: the storage pools, F6 to create one, and one of them
   made the media repository (9). Partitions' disks, their dumps and
   the media library are in pools.
7. Option 3: the virtual switches the partitions' adapters will use,
   and each one's uplink.
8. Option 4, 2: pair QS/MC. Compare the fingerprint the console shows
   with the one on this panel before it is accepted. Two consoles at
   most.
9. Option 10, only when a partition is to own a physical device: release
   the device (`docs/PHYSICALIO.md`).

From QS/MC: the partitions, their profiles, virtual disks, media and
devices; a partition flagged to start with the host does so in its
order after every IPL.

## Every day

Nothing. The host keeps its own logs, rotates them and starts what
ends. What wants attention is a **problem**: a condition the host
posted an attention or fatal code for, opened once however often it
recurs (`docs/PROBLEMS.md`). QS/MC lists them; at the host, option 7
shows the problem log and option 8, 1 the problems themselves, each
with what to do. A problem whose condition no longer holds says
`Cleared`; it is closed by a person, at 8, 1, 4, or from QS/MC.

## Starting and stopping

- **A partition** is activated, shut down and restarted from QS/MC.
  Shut down delayed, through its operating system or immediately
  (`docs/SERVICE.md`). Service Tools only lists them (1, F11).
- **Every partition at once**: option 8, 6, `SHUTDOWN` typed.
- **The host**: option 9 (or the power button, or QS/MC). The host
  first shuts down each running partition through its operating
  system, in the reverse of the order they started in, then its
  services, then itself. Never cut the power of a host that runs
  partitions: what a partition had not written is lost, as it would be
  on a machine of its own.
- **After a power loss** the host starts by itself when the firmware
  is set to, takes up its partitions flagged to start, and has posted
  what it found: a pool or load source resynchronized from its
  write-intent map, a partition whose last run did not end.

## Disks

- **A load-source unit fails** (`A2003020` and its problem): the host
  runs on the other unit. Replace the disk, then option 2, 2: the new
  one is written from the other while the host runs.
- **Both units of a mirrored load source ran without each other**
  (`A2003040`): the host starts nothing and asks, at the console,
  which unit to keep. Keep the one that ran last with what matters;
  the other is written again from it.
- **A pool's unit fails**: the pool runs degraded (mirrored, RAID 5:
  one unit; RAID 6: two) and takes a hot spare when it has one. Option
  2, 7, 6 by the pool puts a new disk in its place.
- **A pool fills** (`A2005160`): every partition writing to it is held,
  not failed. Free room in it (a virtual disk or a dump deleted, from
  QS/MC) and the partitions go on.

## Service data

- **A partition's dump** is taken from QS/MC, or by the host when the
  partition ends in error with its memory held; it is in the media
  repository's pool (`docs/DUMPS.md`).
- **A host dump** (option 8, 2, or QS/MC) is service data in one ZIP:
  logs, reference codes, state, problems, the kernel's messages and
  any panic kept. It never holds a key, a password's hash or a
  console's pin. Option 8, 3, 6 copies it to a removable volume.
- **A kernel panic** restarts the host. Its last messages are kept and
  posted at the next IPL (`A2001005`); send the next host dump.

## Code update

An update image of a later release, signed by a key the running image
trusts, is installed into the slot that does not run, from QS/MC or
from a removable volume (option 8, 4), and activated by a restart that
first shuts the partitions down (`docs/UPDATE.md`). The first IPL of the
new slot is a trial: if the host is not ready in 15 minutes it returns
to the old slot by itself, and the old slot's state with it. Option 8, 5
shows both slots, which one runs and how the last trial ended. Read the
release's notes first: one that needs a new Secure Boot certificate
enrolled says so (`docs/INSTALL.md`).

## Secure Boot

A host runs with Secure Boot on or off; option 1 says which. To turn it
on, enrol the release's `qsvs-db.cer` in the firmware's db
(`docs/INSTALL.md`, Secure Boot) before turning it on.

## What a person never does

- Edit a file on the host: there is no shell, and the console edits no
  state itself.
- Give the host's management address to anything but the management
  network. The partitions' traffic is on the switches' uplinks.
- Share a service tools password, or keep it where QS/MC's users can
  read it.
- Pair a console whose fingerprint was not compared.

## Limits

| What | At most | Past it |
|---|---|---|
| partitions defined | 64 | `QVS2006` |
| virtual SCSI adapters a profile | 8 | `QVS1203` |
| units an adapter | 64 | `QVS1205` |
| virtual Ethernet adapters a profile | 8 | `QVS1222` |
| tagged VLANs an adapter | 32 | `QVS1220` |
| virtual switches | 16 | `QVS4007` |
| VLANs a switch | 256 | `QVS4005` |
| paired consoles | 2 | refused at pairing |
| problems kept | 500 | the oldest closed go first |
| host dumps kept | 8 | the oldest goes |

`docs/PHASE11.md` says which suite shows each.
