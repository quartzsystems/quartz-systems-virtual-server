# Host reference codes

A QS/VS host posts reference codes the way a QS/400 system posts its
own: eight characters, a two-character family and six digits. The table
is `lic/refcode/src/lib.rs`; nothing else spells a code.

A code is posted as one line, `REF <code> <text>`:

```
REF C2001000 Host IPL started
```

## The host range

**NEEDS-CODY: the choice of range is Claude's proposal and needs
sign-off before Phase 1 adds codes to it.**

QS/400's families (`lic/slic/src/src.rs` in its repository, at the
pinned commit) are `C1` (LIC initialization), `C6` (OS initialization),
`A6` (attention), `B1` (fatal) and `D1` (power down). A partition's
console and a host's serial log end up side by side in front of the same
operator, so a host code must never read as a partition's.

The host takes the families whose second character is `2`. No QS/400
family ends in `2`, and the first character keeps the meaning it has
there:

| Family | Meaning | Used in Phase 0 |
|---|---|---|
| `C2` | host IPL and progress | yes |
| `A2` | attention; the host continues | reserved |
| `B2` | fatal; the host parks | yes |
| `D2` | power down | reserved |

A unit test (`refcode::tests::no_code_is_in_a_qs400_family`) fails if a
host code lands in a QS/400 family. The word before the code is `REF`,
not QS/400's `SRC`, so a line also says which kind of system posted it.

## The codes

| Code | Text | Posted |
|---|---|---|
| `C2001000` | Host IPL started | by PID 1 at IPL start, on the display and the serial port |
| `C2001005` | Host running | every 5 seconds, on the serial port only: the heartbeat |
| `B2001000` | Host PID 1 failed | by PID 1's panic handler, with the panic message, before it parks |

Within a family, `C2001xxx` is PID 1's own IPL. Later phases take further
blocks (services, storage, networking, the VMM) and record them here.
