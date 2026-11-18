# Problems

A problem is something a person should look at: one record for each
condition, not one for each reference code posted. The host opens one
when a condition begins, counts it each time the condition is seen
again, notes when it clears, and keeps it until a person closes it. A
management console reads and closes problems over QSI's `service`
family; the local console at Service Tools option 8, and its log at
option 7.

## Where problems come from

Every reference code the host posts goes to the history
(`<data>/log/refcodes.ndjson`, `docs/REFCODES.md`), with what it is about
when whoever posted it said: each resource's kind, the name a person
finds it by, its location code and its serial number. The problem
service reads the history on from where it had got to and takes each
code by the table below:

- A code that **opens** a problem opens one for its condition: the code,
  whose it is (the host's, or a partition's) and what it is about. When
  a problem of that condition is open, it is counted instead: its count
  and last seen go up, it holds again, and its analysis log gains a
  line. A disk pulled twice is two conditions (its serial number
  differs); the same disk seen missing at each IPL is one.
- A code that **clears** a condition marks each open problem of it that
  still holds and that the code names (its partition, and each resource
  it names of a kind the problem lists) as cleared. The problem stays
  open.
- A person **closes** a problem, at a console or at Service Tools; one
  whose condition still holds only when told to close it anyway. A
  closed problem stays closed: the condition seen again opens a new
  one.

The table is the message catalogue's (`lic/qvsmsg/src/problems.rs`);
it lists every attention and fatal code of the host, and a unit test
holds it to the codes (`problem::tests`) and to this document
(`QVS_PROBLEMS=update cargo test -p qvsmsg` writes it). A message's
values are the names of the resources the posting names, one for each
kind the row lists and in that order, then the text the code was
posted with.

<!-- problems -->
| Code | Opens | Description | Message | Resources | Cleared by |
|---|---|---|---|---|---|
| `A2001005` | yes | The host restarted after its kernel panicked: &1. | `QVS5156` | the host | a person only |
| `A2001030` | yes | The A/B record of the load source was not valid: &1. | `QVS5101` | the host | a person only |
| `A2001035` | yes | Code slot &1 did not confirm, and the host went back to the other slot: &2. | `QVS5102` | code-slot | a person only |
| `A2001040` | yes | The data file system was not mounted: &1. | `QVS5103` | the host | `C2001040` |
| `A2001045` | yes | The data file system is nearly full: &1. | `QVS5104` | the host | `C2001045` |
| `A2001050` | yes | The clock could not be set from the hardware clock: &1. | `QVS5105` | the host | `C2001050` |
| `A2001060` | no | A seed is the operator's: the console says why it was refused, and nothing was written. | | | |
| `A2001070` | yes | A state file was not valid, and its defaults are in use: &1. | `QVS5106` | the host | a person only |
| `A2001072` | yes | The memory for partitions was not reserved in full: &1. | `QVS5107` | the host | `C2001072` |
| `A2001075` | yes | The host's identity could not be made: &1. | `QVS5108` | the host | `C2001075` |
| `A2001080` | yes | Service &1 failed five times in ten minutes and is not started again. | `QVS5109` | service | a person only |
| `A2001095` | yes | The firmware's boot order was not written for code slot &1: &2. | `QVS5110` | code-slot | `C2001095` |
| `A2002005` | yes | No management interface was found: &1. | `QVS5111` | the host | `C2002010` |
| `A2002010` | yes | Management interface &1 has no carrier. | `QVS5112` | network-interface | `C2002010` |
| `A2002020` | yes | Management interface &1 has no address. | `QVS5113` | network-interface | `C2002020` |
| `A2003010` | yes | The mirrored load source was not assembled: &1. | `QVS5114` | the host | `C2003010` |
| `A2003020` | yes | Load-source unit &1 is missing or failed. | `QVS5115` | disk-unit | `C2003025`, `C2003035` |
| `A2003030` | yes | The rebuild of load-source unit &1 did not finish: &2. | `QVS5116` | disk-unit | `C2003025`, `C2003035` |
| `A2003040` | yes | The load-source units each ran without the other: &1. | `QVS5117` | the host | `C2003050` |
| `A2004010` | no | The installer has no problem log: its screens and its log say what failed. | | | |
| `A2004015` | no | The installer has no problem log: its screens and its log say what failed. | | | |
| `A2004070` | no | The installer has no problem log: its screens and its log say what failed. | | | |
| `A2004090` | no | The installer has no problem log: its screens and its log say what failed. | | | |
| `A2005000` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005010` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005020` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005021` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005022` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005030` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005031` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005032` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005033` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005034` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005040` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005041` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005050` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005060` | yes | The machine of partition &1 could not be built: &2. | `QVS5118` | partition | a person only |
| `A2005065` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2005100` | no | The code before it says why the partition did not IPL, and is the one to look at. | | | |
| `A2005120` | yes | Partition &1 ended with a guest fault: &2. | `QVS5119` | partition | a person only |
| `A2005130` | yes | The VMM of partition &1 failed while the partition ran: &2. | `QVS5120` | partition | a person only |
| `A2005140` | no | The host ended the partition, as it was asked to. | | | |
| `A2005150` | yes | The VMM of partition &1 was killed: &2. | `QVS5121` | partition | a person only |
| `A2005160` | yes | Partition &1 is held: a storage pool its virtual disks are in is full. | `QVS5122` | partition | `C2005160` |
| `A2005170` | yes | Partition &1 is held after a guest fault: &2. | `QVS5123` | partition | a person only |
| `A2005180` | yes | A dump of partition &1 could not be written: &2. | `QVS5124` | partition | a person only |
| `A2005190` | yes | The VMM of partition &1 was ended by its confinement: &2. | `QVS5155` | partition | a person only |
| `A2006010` | no | An activation that fails is answered to whoever asked for it, with its messages, and is in the partition's history. | | | |
| `A2006020` | yes | Partition &1 was not started automatically: &2. | `QVS5125` | partition | a person only |
| `A2006030` | yes | A partition's definition or profile was not valid: &1. | `QVS5126` | the host | a person only |
| `A2006040` | yes | The resource controls of partition &1 could not be set: &2. | `QVS5127` | partition | a person only |
| `A2006090` | yes | Partitions had not ended when the host shut down: &1. | `QVS5128` | the host | a person only |
| `A2007010` | yes | Storage pool &1 was not assembled: &2. | `QVS5129` | storage-pool | `C2007010` |
| `A2007020` | yes | Disk unit &1 of storage pool &2 failed or is missing. | `QVS5130` | disk-unit, storage-pool | `C2007025`, `C2007035` |
| `A2007025` | yes | Storage pool &1 is degraded and has no hot spare to rebuild onto. | `QVS5131` | storage-pool | `C2007035`, `C2007040` |
| `A2007030` | yes | The rebuild of storage pool &1 did not finish: &2. | `QVS5132` | storage-pool | `C2007025`, `C2007035` |
| `A2007040` | yes | Storage pool &1 failed: &2. | `QVS5133` | storage-pool | `C2007010` |
| `A2007050` | no | A warning ahead of A2007051, which opens the problem. | | | |
| `A2007051` | yes | Storage pool &1 is 90% allocated: &2. | `QVS5134` | storage-pool | `C2007055` |
| `A2007060` | no | An import that fails is answered to whoever asked for it, and nothing of it is left. | | | |
| `A2008010` | yes | Virtual switch &1 was not built: &2. | `QVS5135` | virtual-switch | a person only |
| `A2008020` | yes | Every interface of the uplink of virtual switch &1 lost carrier. | `QVS5136` | virtual-switch | `C2008020` |
| `A2008025` | yes | Interface &1 of the uplink of virtual switch &2 lost carrier. | `QVS5137` | network-interface, virtual-switch | `C2008020` |
| `A2008030` | yes | Interface &1, in the uplink of virtual switch &2, is not on the host. | `QVS5138` | network-interface, virtual-switch | a person only |
| `A2009010` | yes | The service channel of partition &1 was closed: &2. | `QVS5139` | partition | `C2009000` |
| `A2009020` | yes | Partition &1 is not responding on its service channel. | `QVS5140` | partition | `C2009020` |
| `A2009030` | yes | The system in partition &1 names another identity than the host gave it: &2. | `QVS5141` | partition | a person only |
| `A2009040` | no | The shut down asked for was done, immediately at the end of its time. | | | |
| `A2009055` | yes, with the system's own code | Partition &1 is held after its system posted a terminal code: &2. | `QVS5142` | partition | a person only |
| `A2009080` | no | The switch's own problem, A2008020, is the one to look at. | | | |
| `A2010000` | no | Its cause opens the problem: no identity (A2001075), no management address (A2002005 to A2002020) or the processor (A2010010). | | | |
| `A2010010` | yes | The processor lacks what the service interface needs: &1. | `QVS5143` | the host | a person only |
| `A2010030` | no | One attempt: attempts that fail again and again open A2010035. | | | |
| `A2010035` | yes | Attempts to pair a management console failed again and again: &1. | `QVS5144` | the host | a person only |
| `A2011010` | yes | A host dump could not be written: &1. | `QVS5146` | the host | a person only |
| `A2012010` | no | An image refused is answered to whoever staged it, with why, and is not kept. | | | |
| `A2012030` | yes | An update could not be installed into code slot &1: &2. | `QVS5147` | code-slot | `C2012040` |
| `A2012070` | no | A2001035, posted by the slot that came back, opens the problem. | | | |
| `A2013010` | yes | The host's IOMMU or interrupt remapping is off: &1. | `QVS5150` | the host | `C2013000` |
| `A2013030` | yes | Released device &1 was not found at the IPL. | `QVS5151` | device | `C2013020`, `C2013015` |
| `A2013035` | yes | Released device &1 was not given to the pass-through driver: its isolation group changed. | `QVS5152` | device | `C2013020`, `C2013015` |
| `A2013050` | yes | Device &1 did not reset and is fenced. | `QVS5153` | device | `C2013020` |
| `A2013080` | yes | Physical tape drive &1 went away. | `QVS5154` | tape-drive | `C2013050` |
| `B2001000` | yes | The host's first process failed: &1. | `QVS5148` | the host | a person only |
| `B2003000` | yes | The storage service of the mirrored load source stopped: &1. | `QVS5149` | the host | a person only |
<!-- end of problems -->

## A problem

As QSI's `service` family carries it (QS/400's `docs/QSI.md`):

| Field | What |
|---|---|
| number | ascending from 1, never used again |
| status | open or closed |
| first seen, last seen | seconds since the epoch |
| count | how many times its condition was seen |
| reference code | the code as last posted for it, with its place in the history; for `A2009055`, the system's own terminal code |
| source | the host, or a partition (its ID and name) |
| description | the message's first-level text with its values |
| message | its identifier, `QVS51xx`, whose second-level text says what to do |
| holds | whether the condition still holds |
| closed by, closed at | who closed it (the console's user, or the service tools user) and when |
| resources | each disk unit, pool, virtual disk, interface, switch, partition, code slot or service it is about, with its location code and serial number where it has them; a virtual resource is named as one (`virtual disk VD03 in pool POOL1`) |
| analysis log | a line for its opening, each resource, each time it is seen again, its clearing and its close; the first and the newest 49 are kept |

## The service

The problem service (`svc/problem`, run as `lic/qsvsprobd`) is a
supervised service of its own, after the log service, so problems are
recorded whether or not the QSI server or any console is up.

- **On disk.** `/data/problems/problems.json`: the problems, the next
  number, and the place in the history it has read to, written whole
  and atomically after each change and before anyone is told. A service
  started again reads on from that place: no code is lost and none is
  taken twice. A file that does not read is set aside as
  `problems.json.bad` and a new log begun.
- **Bounded.** 500 problems: past that the oldest closed go first, and
  only then the oldest open. An analysis log keeps 50 lines.
- **The data file system.** Every minute the service looks at how full
  `/data` is: at 90% it posts `A2001045`, which opens a problem; back
  under 85% it posts `C2001045`, which clears it.
- **Its socket.** `/run/qsvsprobd.sock`, root's and the QSI server's
  group's: list (a filter of status, partition and last seen, a page
  newest first), get, close (with who closed it and whether anyway),
  and a subscription whose events are each problem opened, seen again,
  cleared and closed. A refusal is a `QVS50xx` message.

| Message | When |
|---|---|
| `QVS5001` | no problem of that number |
| `QVS5002` | closed already, by whom |
| `QVS5003` | a close while the condition holds, not anyway |
| `QVS5004` | the log could not be written; nothing changed |

## Over QSI

The QSI host server maps the `service` family onto the socket and sets
its capability bit (`docs/QSISERVER.md`): `List`, `Get` and `Close`
are the socket's requests, the close's user is the request's, and the
four events are QSI's `Opened`, `Seen`, `Cleared` and `Closed`. A
refusal leads with QSI's identifier (`QSI1001` the condition holds,
`QSI1002` no such problem, `QSI1003` closed already) and then the
host's message.

## The host's certificate

The host's identity (`base/identity`) is a self-signed certificate
made once at the first IPL, and it has no end date: its validity ends at
9999-12-31, RFC 5280's value for a certificate with no well-defined
expiration. A certificate with an end date would open a problem 30 days
before it; QS/VS's has none, so nothing looks for one and no code
exists for it.
