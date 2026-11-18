# Releasing QS/VS

How a release of QS/VS is signed, built and published, after QS/400's
`docs/releasing.md` with QS/VS's own keys and names. `VERSION` holds
the version (`1.0.0` is V1R0M0); `lic/release` reads it at build time.

## The keys

A release is signed with two keys. Neither is in the repository or in
a log. The keys themselves are Cody's to make and keep (`NEEDS-CODY`,
`docs/QUALIFICATION.md`).

| Key | Variable | Signs | If it leaks / is lost |
|---|---|---|---|
| Release signing key, ed25519: a file of 64 hexadecimal characters (the 32-byte seed) | `QSVS_SIGNING_KEY` | the image's manifest, the installation media's payload, the update image, `SHA256SUMS` and the stream's index; every image trusts its public key | anyone can make update images and media that hosts accept / no update can reach installed hosts until a later release, trusting a new key, is installed from media |
| Secure Boot db key, RSA-2048, PKCS#8, and its X.509 certificate | `QSVS_SB_KEY`, `QSVS_SB_CERT` | the three UKIs (Authenticode, QS/400's `pesign`); `qsvs-db.cer` is on the media and in the release for enrolment | anyone can boot code on a machine that enrolled it / every machine that enrolled it must enrol a new certificate |

Make them once, on a trusted machine, into a directory outside the
checkout (an encrypted volume):

```sh
cargo run -p build -- keys /media/keys
```

It writes `qsvs-release.ed25519` (mode 0600) and its public half
`qsvs-release.pub` (what `QSVS_TRUSTED_KEYS` lists), `qsvs-db.key` (0600),
`qsvs-db.cer` and `qsvs-db.esl` (the certificate as an EFI signature
list), prints the release key's public half and the certificate's
SHA-256, and never writes over a key that is there. Keep two offline
copies of the directory. A key that exists in one place only is lost
with that place, and with the release key lost no installed host can be
updated again.

### Development keys

Without `QSVS_SIGNING_KEY`, a build signs with a development key it
makes under `<target>/keys/dev.ed25519`, and says so. An image signed
with one says so too: `A2001002` at every IPL, "development key" on
Service Tools' status screen (Work with System Status, the Release
line) and in the build `identify` returns over QSI. `build release`
refuses a development key: without `QSVS_SIGNING_KEY`, with it naming
the development key, without both `QSVS_SB_KEY` and `QSVS_SB_CERT`, or
with QS/400's development certificate, it stops before it builds.

### The trusted keys and rotating them

An image trusts a list of keys (`/qsvs/keys/system.pub` in its
initramfs): the key that signed it, then each key `QSVS_TRUSTED_KEYS`
names (a file of 64-hexadecimal-character lines). The update service
and the installer accept an update image or media signed by any of
them, and nothing else (`docs/UPDATE.md`).

To retire key K1 for K2:

1. Make K2 (`build keys`), and keep it offline beside K1.
2. Release N, signed by K1, with `QSVS_TRUSTED_KEYS` naming K2's
   public half: hosts that install N trust K1 and K2.
3. Release N+1 and later are signed by K2 (and name K3 when it exists).
   Their stream entries say they update from N or later
   (`updates_from`).
4. K1 is kept until no host older than N is supported, then destroyed.

A host older than N trusts K1 alone: it refuses N+1 with `QVS5301`
(the image's signer is not one it trusts) and is updated to N first, or
installed from N+1's media. Phase 11's Section 6 shows both
(`docs/PHASE11.md`).

## Cutting a release

From a clean tree at the commit to be released, with the release keys:

```sh
QSVS_SIGNING_KEY=/media/keys/qsvs-release.ed25519 \
QSVS_SB_KEY=/media/keys/qsvs-db.key QSVS_SB_CERT=/media/keys/qsvs-db.cer \
QSVS_TRUSTED_KEYS=/media/keys/next.pub \
  cargo run -p build -- release 1.0.0
```

`build release VERSION` refuses a version that is not `VERSION`'s, a
tree with any change or untracked file, and the keys above; it needs
`docs/RELEASE_NOTES_V<x>R<y>M<z>.md`. It then builds the release image
with the UKIs signed for Secure Boot before the disk and the ISO are
made of them, the ISO (with the certificate, the notices and the
release notes at its root), the update image and the manifest, and
writes `target/release/V1R0M0/`:

| File | What |
|---|---|
| `QSVS-V1R0M0-<hash>.iso` | the installation media |
| `QSVS-V1R0M0-update-<hash>.upd` | the update image |
| `MANIFEST.json`, `MANIFEST.sig` | every artifact's SHA-256, signed |
| `NOTICES.txt` | third-party notices |
| `RELEASE_NOTES_V1R0M0.md` | the release notes |
| `qsvs-db.cer` | the Secure Boot db certificate |
| `catalogue.json` | every reference code with its meaning, cause and recovery, every message (`docs/catalogue.schema.json`) |
| `stream/index.toml` | the stream's index of this release |
| `SHA256SUMS`, `SHA256SUMS.sig` | the above, and their ed25519 signature with the key's public half |

Then it builds the whole release again from clean at the same target
path and compares every file byte for byte; a difference stops it
(`--no-compare` builds once, for a trial). The tag, `v1.0.0`, is
Cody's, and is pushed with the release's files published.

## The stream

Releases of QS/VS are published as the `qsvs` stream: one directory
with every release's update image and ISO, and `index.toml`, the
signed index of them (`base/stream`).

The index has the form of QS/400's signed catalog
(`docs/UPDATE_PROTOCOL.md` section 3, in its repository): UTF-8 TOML in
a fixed order, then one line `# signature <128 hex digits>`, the ed25519
signature of every byte before it by the release signing key. A reader
verifies the bytes as received against the keys it trusts and parses
only what verified; `sequence` only grows, and a reader that has seen
one refuses a lower one.

```toml
format = 1
stream = "qsvs"
sequence = 2
generated = 1767225600

[[release]]
release = "V1R0M0"
build = "a1b2c3d"
fixes = []
updates_from = "V1R0M0"
published = 1767225600
update = "QSVS-V1R0M0-update-0123456789ab.upd"
update_bytes = 32127562
update_sha256 = "…"
iso = "QSVS-V1R0M0-0123456789ab.iso"
iso_bytes = 228644864
iso_sha256 = "…"
```

`updates_from` is the least release an entry updates from: the first
of its version and release by default (V1R0M1 from V1R0M0), and the
release before a key's rotation after one. A console offers a host at
release R the newest entry past R whose `updates_from` is not past R
(`stream::newest_for`): that is all it needs to say "update available".
It downloads the update image, checks its size and SHA-256 against the
index, and brings it to the host over QSI, or an operator copies it to
a removable volume; the host verifies the image itself against its own
trusted keys before a byte is written.

```sh
cargo run -p build -- stream add target/release/V1R0M1 /srv/qsvs/v1
cargo run -p build -- stream verify /srv/qsvs/v1
```

`stream add` copies a release's update image and ISO into the stream's
directory and joins its entry to the index there, one sequence further,
signed again. `stream verify` checks the index against the release key
and `QSVS_TRUSTED_KEYS`, and every file it names against its size and
SHA-256.

### What a shared service would serve

Phase 11 hosts no stream. QS/400's update service (`qs400-updated`)
mirrors a directory and serves it under `/qs400`: `GET /v1/catalog`,
`GET /v1/ptf/QSnnnnn`, `GET /v1/group/NAME`, and the order and service
requests. Serving QS/VS from the same service is a second directory
under `/qsvs`, static files only:

| Path | Answer |
|---|---|
| `GET /qsvs/v1/index` | `index.toml` |
| `GET /qsvs/v1/file/NAME` | an update image or an ISO the index names |

QS/VS has no orders, groups or service requests: a release is whole,
and a host is updated by a person. The service needs no key of
QS/VS's; it serves what the release key signed.

## When a key leaks or is lost

| | Leaked | Lost |
|---|---|---|
| Release signing key | Make a new key. Publish at once a release signed by the old key that trusts only the new one (`QSVS_TRUSTED_KEYS` names it, and the build is given the new key's file as `QSVS_SIGNING_KEY` only for the release after it); tell every operator to install it; take the old key out of the trusted list of every later release. Until a host has that release, an update signed by the leaked key would be accepted by it: say so to every operator, and that they install nothing that did not come from the stream. | No update can reach a host that trusts only the lost key. A new key is made; the next release is installed from its media on every host. |
| Secure Boot db key | Make a new pair, sign the next release with it, and tell every operator who enrolled the old certificate to remove it from db (or add it to dbx) and enrol the new one. | Make a new pair; the next release's UKIs are signed with it, and every machine that enrolled the old certificate enrols the new one before installing it. |

Each event is written into the release notes of the release that
follows it.
