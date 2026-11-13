# Installing QS/VS

How a machine becomes a QS/VS host: what the media is, the attended
installation screen by screen, the unattended one, and what is on the
disks afterwards. The code is `base/install` (the screens, shared with
QS/MC) and `lic/qsvsinstall` (the media, the disks, the state).

## The media

`cargo run -p build -- iso` writes `QSVS-V1R0M0-<hash>.iso`. It is a
UEFI-bootable ISO 9660 image and, the same bytes, a disk image with a
GPT: it starts a machine from a CD-ROM drive, from a BMC's virtual media
(which is a USB CD-ROM to the machine), or from a USB stick it was
written to whole (`dd if=QSVS-....iso of=/dev/sdX bs=4M`).

On it are the boot image with the media UKI, and the payload under
`QSVS/`:

| File | What |
|---|---|
| `SLOTA.EFI`, `SLOTB.EFI` | the UKIs of slot A and slot B, as the host will boot them |
| `PAYLOAD.TXT` | the manifest: the product, the release, the build, and each UKI's length and SHA-256 |
| `PAYLOAD.SIG` | the manifest's ed25519 signature, by the build's signing key |

The media UKI's initramfs holds the installer and the public keys it
trusts (`/qsvs/keys/system.pub`: the build's key and those of
`QSVS_TRUSTED_KEYS`). Before anything else the installer finds the
media among the block devices by its volume identifier
(`QSVS_V1R0M0`), mounts it read-only, checks the manifest's signature
against those keys and each UKI against the manifest. Media that fails
any of it is refused on a screen that says why, and nothing is written.
A media and an installer of different releases refuse each other.

The manifest on the media is not the build's artifact manifest
(`manifest.json`): that one lists the ISO, so it cannot be on it.

## What the installer has

The machine's display and its first serial port (115200 8N1), both at
once: every screen is on both and keys are taken from both. On the
display the function keys are the keyboard's. On a serial terminal they
are the terminal's own, or Esc and a key of the top row: Esc 1 for F1,
Esc 3 for F3, Esc 0 for F10, Esc = for F12. Tab goes to the next field;
Ctrl+K clears a field from the cursor on and goes to the next.

F1 is help on every screen. F12 goes back one screen, F3 to the first.
Each field is checked when Enter is pressed on its screen; what is
wrong is on the message line at the bottom. Nothing is written to any
disk before F10 on the summary.

## The attended installation

1. **Welcome.** The product and its release, and what will happen.
   Enter goes on; F3 leaves, and the machine restarts with nothing
   written.
2. **Select load-source disks.** Every disk of the machine with its
   location code (`docs/LOCATION.md`), model, serial number and size.
   Type 1 by one disk for an unprotected load source, or by two for
   mirrored protection. A disk smaller than a load source may be (some
   560 MB) is listed, marked, and cannot be chosen. Two disks of
   different size are mirrored to the size of the smaller. The disk the
   media is on, and a seed volume, are not listed.
3. **Confirm erase.** The disks again, and what they become. Type
   `ERASE` to go on. Nothing is erased yet.
4. **Host name and management network.** The host name; the interface,
   by its number in a list that shows each with its location code, its
   hardware address and whether its link has carrier now (blank takes
   the first that has carrier at each IPL); DHCP, or a static IP
   address with its prefix length and an optional gateway; an optional
   VLAN ID; DNS servers.
5. **Time.** The offset from UTC that dates and times are displayed in
   (fixed offsets only; the clock itself keeps UTC); time servers; and,
   when there are none, the date and time to set the clock to. The
   screen shows what the clock says now; blank leaves it.
6. **Service tools user.** The first user ID that can sign on to
   Service Tools, and its password, typed twice and never shown. The
   rules are on the screen: a user ID is 1 to 10 letters and digits; a
   password is 8 to 128 characters with at least one letter and one
   digit, and does not contain the user ID.
7. **Summary.** Everything chosen. F10 installs; F12 goes back.
8. **Progress.** The steps, each with the reference code posted when it
   is done (`docs/REFCODES.md`):

   | Code | Step |
   |---|---|
   | `C2004020` | Partitioning the disks: each gets its table, an ESP and cleared record and data partitions |
   | `C2004030` | Setting up protection: with two disks, the mirrored set's label on both |
   | `C2004040` | Making the data file system |
   | `C2004050`, `C2004055` | Copying slot A and slot B to every disk's ESP, each read back |
   | `C2004060` | Writing the A/B record, on every disk |
   | `C2004070` | Writing boot entries: one for each slot of each disk, slot A first |
   | `C2004080` | Writing state: the host's settings, the network's, the user |

   A step that fails is named, with `A2004090` and the reason, and
   the installation stops there. Enter starts the installer over. It
   never goes on from what a failed or interrupted attempt left: the
   first step writes every disk's table again and clears the heads of
   its partitions.
9. **Complete.** Remove the media, or detach the virtual media, and
   press Enter. The machine restarts and the host comes up from its
   load source.

On a machine whose firmware lets no boot entry be written the step is
not failed: `A2004070` is logged, and the disks still start by the
default path every UEFI firmware tries.

## The first IPL

The host IPLs as `docs/PHASE1.md` describes, configured: it does not
look for a seed. Once, at this IPL, it makes its identity: an Ed25519
key pair and a self-signed certificate that names the host and carries
its serial number, `identity.key` and `identity.crt` under
`/data/state/`, with `C2001075`. The management console will pair with
the host by that certificate. It is not made again, whatever changes.

With mirrored protection the second unit is written from the first at
this IPL (`C2003020`, then `C2003025`): the installer formats one disk
and does not claim the other holds the same bytes. The host runs
meanwhile. It is protected once that is finished; Service Tools option
2 shows where it stands.

Press Enter on the display or at the serial port and sign on with the
user made in step 6 (`docs/SERVICETOOLS.md`).

## The unattended installation

Put an answers file on a FAT volume labelled `QSVSSEED` and attach it
with the media: a second USB stick, or a second virtual disk. The file
is `seed.toml` with an `[install]` table (`docs/SEED.md`):

```toml
schema_version = 2

[install]
disks = ["S4EVNX0M702341", "S4EVNX0M702377"]
user_id = "ADMIN"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$..."

[host]
name = "vs-01"
time_zone = "UTC+01:00"
ntp_servers = ["time1.example.net"]

[network]
device = "auto"
mode = "static"
address = "192.0.2.10/24"
gateway = "192.0.2.1"
```

With answers it can use, the installer shows the progress display at
once, asks nothing, and restarts the machine by itself when it is done.
Take the media out of the boot order first, or the machine installs
again: the firmware's boot entries written by the installation come
first only where the firmware honors them.

With answers it cannot use, it writes nothing and stops on a screen
that says what is wrong: a file that does not parse, a key it does not
know, a disk the machine does not have or that is too small, a user ID
or a hash that is not one. From there Enter leads to the attended
installation and F3 leaves.

## What is on the disks

Each load-source disk, alike:

| Partition | Size | What |
|---|---|---|
| `QSVS ESP` | 256 MiB | FAT32: slot A at `EFI/BOOT/BOOTX64.EFI`, slot B at `EFI/QSVS/B.EFI` |
| `QSVS-AB` | 1 MiB | the A/B record, the mirrored set's label, and the map of a resynchronization under way |
| `QSVSDATA` | the rest | the host's own xfs, mounted at `/data`: unprotected, the partition itself; mirrored, one member of the set the file system is on |

`docs/ARCHITECTURE.md` says why each disk has its own ESP and record.
The file system has 4096-byte sectors on either kind of load source, so
a second disk can be added to an unprotected one later.

Every other disk of the machine is left alone. Storage pools go on
them, in a later release.

## An installation that was cut off

Power lost during an installation leaves disks that are partly
written. Start the media again: the installer starts over from its
first step, whatever is on the disks. A disk that got as far as its
ESP may start by itself in the meantime; it comes up as a host with no
state, no user and nothing to lose. Install again.
