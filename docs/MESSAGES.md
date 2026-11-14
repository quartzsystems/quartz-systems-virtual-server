# Messages

QS/VS says what went wrong in messages. A message has

- an identifier: `QVS` and four digits;
- first-level text: one line that says what happened;
- second-level text: why, and what to do about it.

`&1` to `&9` in either text stand for the values the message is issued
with: a name, an amount, a slot. A request to the partition manager
that fails answers with messages (`docs/PARTD.md`); validation of a
profile gives every message at once, not the first.

Reference codes (`docs/REFCODES.md`) are a different thing: a code
says where the host or a partition is, on the console and in the
history; a message answers someone who asked for something.

The catalogue is the crate `lic/qvsmsg`. The table below is written
from it (`QVS_MESSAGES=update cargo test -p qvsmsg`), and a unit test
fails when the two differ. No message borrows another vendor's
wording.

## Ranges

| Range | About |
|---|---|
| `QVS10xx` | a partition's definition |
| `QVS11xx` | a profile: processors and memory |
| `QVS12xx` | a profile: storage, boot and the sections later phases bring |
| `QVS20xx` | requests to the partition manager |
| `QVS21xx` | activation and resources |
| `QVS22xx` | shared processor pools and host settings |

## The catalogue

<!-- messages -->
| Message | First-level text | Second-level text |
|---|---|---|
| `QVS1001` | Partition name &1 is not valid. | A partition's name is 1 to 10 characters: the letters A to Z, the digits 0 to 9 and the underscore, with a letter first. Choose a name of that form. |
| `QVS1002` | Partition ID &1 is not valid. | A partition's ID is a number from 1 to &2. The partition manager assigns it when the partition is created. |
| `QVS1003` | System serial number &1 is not valid. | A partition's serial number is seven characters, digits and capital letters without I and O. It is made when the partition is created and is not changed afterwards. |
| `QVS1004` | Auto start delay &1 is not valid. | The delay before a partition is started automatically is 0 to &2 seconds. |
| `QVS1005` | Partition name &1 is already in use. | Partition &2 has that name. No two partitions of a host have one name. Choose another name. |
| `QVS1006` | The &1 of partition &2 cannot be changed. | A partition's ID, its serial number, its environment and the time it was created are set when it is created and stay as they are for as long as its definition exists. |
| `QVS1007` | Partition &1 has no profile &2 to be its default. | The default profile of a partition is one of its own profiles. Create the profile first, or name one the partition has. |
| `QVS1101` | Profile name &1 is not valid. | A profile's name is 1 to 10 characters: the letters A to Z, the digits 0 to 9 and the underscore, with a letter first. Choose a name of that form. |
| `QVS1102` | The number of &1 is out of range: minimum &2, maximum &3. | A profile names at least 1 and at most &4 &1. Correct the minimum or the maximum. |
| `QVS1103` | Minimum &2, desired &3 and maximum &4 &1 are not in order. | The minimum is not more than the desired value, and the desired value is not more than the maximum. Correct the three values. |
| `QVS1104` | &1 is for shared processors only. | The profile's processors are dedicated. Processing units, capped or uncapped sharing, the uncapped weight and the pool belong to shared processors. Remove the value, or make the processors shared. |
| `QVS1105` | Shared processors need minimum, desired and maximum processing units. | A profile with shared processors gives all three of minimum_units, desired_units and maximum_units. One unit is one whole core's worth of processor time. |
| `QVS1106` | Minimum &1, desired &2 and maximum &3 processing units are not in order. | The minimum is not more than the desired value, and the desired value is not more than the maximum. Correct the three values. |
| `QVS1107` | The &1 of &2 processing units is too few for &3 virtual processors: at least &4. | Each virtual processor needs at least 0.05 processing units. Give more units, or fewer virtual processors, at that level. |
| `QVS1108` | The &1 of &2 processing units is too many for &3 virtual processors: at most &4. | A virtual processor cannot use more than one whole core, which is 1.00 processing unit. Give fewer units, or more virtual processors, at that level. |
| `QVS1109` | Uncapped weight &1 is not valid. | The uncapped weight is 0 to &2. A weight of 0 keeps the partition to its processing units, as if it were capped. |
| `QVS1110` | Shared processor pool name &1 is not valid. | A pool's name is 1 to 10 characters: the letters A to Z, the digits 0 to 9 and the underscore, with a letter first. The default pool is DEFAULT. |
| `QVS1111` | Minimum &1 MB, desired &2 MB and maximum &3 MB of memory are not in order. | The minimum is not more than the desired value, and the desired value is not more than the maximum. Correct the three values. |
| `QVS1112` | The &1 memory of &2 MB is not a multiple of the memory block size, &3 MB. | Memory is given to partitions in whole blocks. The block size is a setting of the host. Give an amount that is a multiple of it. |
| `QVS1113` | The &1 memory of &2 MB is out of range. | A partition has at least &3 MB and at most &4 MB of memory. |
| `QVS1201` | Virtual slot &2 of a &1 adapter is not valid. | A profile's adapters use the virtual slots &3 to &4. Slots 0 and 1 are the partition's console and its service channel. |
| `QVS1202` | Virtual slot &1 is used twice: by a &2 adapter and a &3 adapter. | Each adapter of a profile has a virtual slot of its own, whatever its kind. Give one of the two another slot. |
| `QVS1203` | The profile has &1 virtual SCSI adapters; the most is &2. | Put the units on fewer adapters. One adapter takes many units. |
| `QVS1204` | The profile has no load source. | A partition IPLs from the first unit of its first virtual SCSI adapter: the lowest unit number in the lowest slot. Give the profile a virtual SCSI adapter with at least one unit. |
| `QVS1205` | The virtual SCSI adapter in slot &1 has &2 units; the most is &3. | Put the other units on a second virtual SCSI adapter. |
| `QVS1206` | Unit &2 of the virtual SCSI adapter in slot &1 is not valid. | An adapter's unit numbers are 0 to &3. |
| `QVS1207` | The virtual SCSI adapter in slot &1 has unit &2 twice. | Each unit of an adapter has a number of its own. |
| `QVS1208` | The backing store of unit &2 in slot &1 is not valid: &3 | A unit's backing store is the absolute path of a file or of a block device on the host, with no .. in it. |
| `QVS1209` | The backing store of unit &2 in slot &1 is another unit's too: &3 | Two units of a partition cannot be kept in one file or one device. Give each unit a backing store of its own. |
| `QVS1210` | The serial number of unit &2 in slot &1 is not valid: &3 | A unit's serial number is 1 to 20 letters, digits, hyphens and underscores. It is how the partition knows the disk again, so it should stay the same. |
| `QVS1211` | The serial number of unit &2 in slot &1 is another unit's too: &3 | Each unit of a partition has a serial number of its own. Change one of the two, or leave it out: the host then makes one from the partition's serial number, the slot and the unit. |
| `QVS1212` | VLAN &2 of the virtual Ethernet adapter in slot &1 is not valid. | A port's VLAN is 1 to 4094. |
| `QVS1213` | MAC address &2 of the virtual Ethernet adapter in slot &1 is not valid. | A MAC address is six pairs of hexadecimal digits with colons between them, and is not a group address: the first pair is an even number. Leave it out to have the host choose one. |
| `QVS1214` | The media of the virtual &1 device in slot &2 is not valid: &3 | The media loaded into a virtual device is the absolute path of a file on the host, with no .. in it. Leave it out for an empty device. |
| `QVS1215` | Location code &1 is not valid. | A physical I/O device is named by its location code, as the host's inventory shows it: letters, digits, hyphens and full stops, 64 characters at most. |
| `QVS1216` | Location code &1 is listed twice. | A physical I/O device is given to a partition once. |
| `QVS1217` | The units of the virtual SCSI adapter in slot &1 are not numbered from 0 without a gap. | An adapter with &2 units has the unit numbers 0 to &3. Number the units in order from 0. |
| `QVS2001` | Partition &1 does not exist. | No partition of this host has that ID. List the partitions to see the IDs there are. |
| `QVS2002` | Partition &1 is activated. | A partition is changed, deleted or activated only while it is not activated. Its state is &2. Shut it down first. |
| `QVS2003` | Partition &1 is not activated. | Only an activated partition can be shut down or restarted. Its state is &2. |
| `QVS2004` | Partition &1 has no profile &2. | List the partition's profiles to see the names there are, or create the profile. |
| `QVS2005` | Partition &1 has no default profile. | An activation that names no profile uses the partition's default profile. Create a profile for the partition, which becomes its default, or name one. |
| `QVS2006` | The host already has &1 partitions, the most it is set to have. | The number of partitions a host may have defined is a setting of the host. Delete a partition, or raise the setting. |
| `QVS2007` | Profile &2 is the default profile of partition &1. | The default profile cannot be deleted. Make another profile the partition's default first, or delete the partition. |
| `QVS2008` | &1 could not be kept: &2 | The host could not write or read the file in its state directory. Nothing was changed. The host's log has more; a load source that is failing is the likely cause. |
| `QVS2009` | Partition &1 is already shutting down. | The partition was asked to shut down and its VMM has not ended yet. A list shows when it is not activated. |
| `QVS2010` | Partition &1 cannot be activated: the host is shutting down. | The host is ending its partitions before it powers off or restarts. Activate the partition when the host has started again. |
| `QVS2011` | Partition &1 already has &2 profiles, the most a partition has. | Delete a profile the partition no longer needs, or replace one by giving the new profile its name. |
| `QVS2101` | Partition &1 needs &2 dedicated processors; &3 are available. | The profile's minimum number of processors is more than the cores that no running partition holds and that the shared partitions of the default pool do not need. Shut down a partition, lower the profile's minimum, or use shared processors. |
| `QVS2102` | Partition &1 needs &2 processing units in pool &3; &4 are available. | The profile's minimum processing units are more than the pool has left: its running partitions are entitled to the rest. Shut down a partition of the pool, lower the profile's minimum, or use another pool. |
| `QVS2103` | Partition &1 needs &2 virtual processors; pool &3 has &4 cores. | Each virtual processor of a partition runs on a core of its pool, so a partition cannot have more virtual processors than its pool has cores. Lower the profile's minimum, or use a pool with more cores. |
| `QVS2104` | Partition &1 needs &2 MB of memory; &3 MB are available. | The profile's minimum memory is more than what no running partition holds. Memory is not overcommitted. Shut down a partition, or lower the profile's minimum. |
| `QVS2105` | Shared processor pool &1 does not exist. | Create the pool, or name a pool the host has. The default pool is DEFAULT. |
| `QVS2106` | &1 is not available in this release: it comes with Phase &2. | The profile is valid and is kept, but a partition cannot be activated with it yet. Activate the partition with a profile that does not use this. |
| `QVS2107` | Partition &1 could not be started: &2 | The host could not set up the partition's resource controls or start its virtual machine monitor. Nothing is held for the partition. The host's log has more. |
| `QVS2108` | Partition &1 did not IPL: reference code &2. | The partition's virtual machine monitor ended before the partition ran. The reference code says what was missing: a backing store that cannot be opened, a load source with nothing to IPL from, memory that was not there. The partition's history has the codes before it. |
| `QVS2109` | Partition &1 ended in error: reference code &2, &3. | The partition's virtual machine monitor failed, or was killed, while the partition was activated. The partition holds nothing now and is not started again by itself. Activate it when the cause is known. |
| `QVS2110` | Partition &1 cannot be activated: the backing store of unit &3 in slot &2 is not there: &4 | The file or block device the profile names for the unit does not exist on the host. Nothing was started. Correct the profile, or bring the disk back. |
| `QVS2201` | Setting &1 cannot be &2. | The value allowed is &3. |
| `QVS2202` | Shared processor pool &1 already exists. | No two pools of a host have one name. Choose another name, or change the pool that is there. |
| `QVS2203` | Core &2 cannot be in shared processor pool &1. | A pool's cores are cores of this host that are not kept back for the host itself. The host resources show which cores those are. |
| `QVS2204` | Core &2 of shared processor pool &1 is listed twice or is in another pool. | A core is in one pool. Cores in no named pool are in the default pool. |
| `QVS2205` | Core &2 cannot be in shared processor pool &1: a running partition has it as a dedicated processor. | Shut down the partition that holds the core, or choose another core. |
| `QVS2206` | Shared processor pool &1 has no core. | A pool has at least one core. |
| `QVS2207` | The maximum of &2 processing units is not valid for shared processor pool &1. | A pool's maximum is more than 0.00 and not more than its cores' worth, &3 processing units. Leave it out for the cores' worth. |
| `QVS2208` | Shared processor pool &1 is in use by &2 running partitions. | A pool is changed or deleted while none of its partitions is activated. Shut the partitions down first. |
| `QVS2209` | The default shared processor pool cannot be created, changed or deleted. | The default pool always holds every core that is not dedicated to a partition, kept back for the host or in a named pool. |
| `QVS2210` | The default pool's running partitions need its cores: &1 processing units and &2 virtual processors on &3 cores. | The change would take cores out of the default pool that the partitions running in it are entitled to, or need to have a core for each virtual processor. Choose other cores, or shut a partition down first. |
<!-- end of messages -->
