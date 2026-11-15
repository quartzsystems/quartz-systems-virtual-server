# A partition's control panel

A partition has the control panel functions a QS/400 system's operator
expects, on the partition manager's socket (`docs/PARTD.md`, request
27: the partition, the function's number and its arguments). Each
answers with the text it displays, or with messages. QS/MC's panel page
and QSI's `panel` family in Phase 8 map onto it one to one.

The host carries out every function but 21, which the system carries
out over its service channel (`docs/SERVICE.md`). A function that
changes or interrupts the system needs the **keylock in manual**. The
keylock is the host's (Phase 4): the definition's, or the one an
activation was given, which function 02 can set for the next. A
function the host does not offer, or whose keylock rule is not met, is
refused with a message; nothing changes.

| Function | What | Carried out by | Keylock |
|---|---|---|---|
| 01 | display the IPL source, the IPL type and the keylock | host | any |
| 02 | select the IPL source, IPL type and keylock for the next activation | host | any |
| 03 | restart the partition's IPL now | host | manual |
| 08 | power the partition off now | host | manual |
| 11 to 19 | display a word of the current reference code | host | any |
| 20 | display the partition's type, its serial number and the host's release | host | any |
| 21 | dedicated service tools on the partition's console | the system | manual |
| 22 | dump the partition's memory, then restart its IPL | host | manual |
| 34 | take the dump again from the memory still held | host | manual |

## Each function

**01.** The partition's IPL as it is: for a running partition, this
activation's source, type and keylock; for one that is not, what its
next activation will use: function 02's selection, else its default
profile's and its definition's. `01  IPL source A, IPL type
unattended, keylock normal`.

**02.** Its arguments are an IPL source (A, B or D), an IPL type
(attended or unattended) and a keylock position (normal, manual, auto,
secure), each optional: what is left out stays as it is. The selection
is for the next activation only, in place of what it would otherwise
use when the activation itself names none, and is forgotten after it.
`the side the partition asked for` is not one to select: it is what a
profile says by default. Displays the next activation's IPL.

**03.** The partition's VMM is stopped at once, without the operating
system's leave, and the partition activated again with the profile it
had, this activation's keylock and IPL type. Its system sees an
abnormal end.

**08.** The partition's VMM is stopped at once and the partition is Not
activated.

**11 to 19.** Function 11 is the current reference code itself (word
1); 12 to 19 are its words 2 to 9, as the system gave them with the
code. A host code, or a code the system gave fewer words for, has
`00000000` there. `11  C6001090`.

**20.** `20  type QS/400 partition, serial number K2M4101, host QS/VS
V1R0M0, system V1R2M0`: the system's release when its channel has said
it.

**21.** Asked of the system, which shows dedicated service tools on the
console port over its console session and gives the console back when
they end. It needs the channel up with the panel capability (`QVS2301`
otherwise). The system refuses it before its IPL is complete and while
its service tools are on the console (`QSI0022`), at a keylock it read
as normal (`QSI0021`): the refusal carries its message (`QVS2303`).

**22.** A dump (`docs/DUMPS.md`) of the running or held partition,
then, when it is written, the IPL restarted as 03 does. Answers when
the dump has begun, with its name; `C2005185` and an event say when it
is written. A dump that cannot be written leaves the partition as it
was.

**34.** A dump of a partition held after a terminal code or a guest
fault, from the memory it kept: refused for one that is not held
(`QVS2309`). It may be taken more than once.

A function that acts (02, 03, 08, 21, 22, 34) is in the partition's
history as `C2009070` with its number.

## Messages

| Message | When |
|---|---|
| `QVS2306` | a function that is not one of the table's |
| `QVS2307` | a function that needs the keylock in manual, with the keylock it is |
| `QVS2308` | function 02 with an argument that is not one |
| `QVS2309` | function 34 of a partition that is not held |
| `QVS2003` | 03, 08, 21, 22 or 34 of a partition that is not activated |
| `QVS2301` | 21 of a partition with no service channel |
| `QVS2303`, `QVS2304` | 21 refused by the system, or not answered |
| `QVS2310`, `QVS2311`, `QVS2313` | 22 or 34 with no media repository, a dump that could not begin, one being taken |
