// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Every message, in the order of its identifier.
//!
//! | Range | About |
//! |---|---|
//! | `QVS10xx` | a partition's definition |
//! | `QVS11xx` | a profile: processors and memory |
//! | `QVS12xx` | a profile: storage, boot and the later sections |
//! | `QVS20xx` | requests to the partition manager |
//! | `QVS21xx` | activation and resources |
//! | `QVS22xx` | shared processor pools and host settings |
//! | `QVS23xx` | the service channel: shut down, the panel, dumps, the console |
//! | `QVS24xx` | physical I/O: devices released, given and reset |
//! | `QVS25xx` | a running partition's entitlement |
//! | `QVS30xx` | storage pools and their disks |
//! | `QVS31xx` | virtual disks |
//! | `QVS32xx` | the media library |
//! | `QVS33xx` | physical tape drives |
//! | `QVS40xx` | virtual switches |
//! | `QVS41xx` | uplinks and the host's interfaces |
//! | `QVS42xx` | partitions' ports |
//! | `QVS50xx` | the problem service |
//! | `QVS51xx` | problems: what each condition is |
//! | `QVS52xx` | the host's dumps |
//! | `QVS53xx` | code update |

use crate::Message;

macro_rules! catalogue {
    ($( $(#[$doc:meta])* $name:ident = $id:literal, $text:literal, $help:literal; )*) => {
        $(
            $(#[$doc])*
            pub const $name: Message = Message { id: $id, text: $text, help: $help };
        )*
        /// Every message.
        pub const ALL: &[&Message] = &[ $( &$name ),* ];
    };
}

catalogue! {
    /// A partition's name breaks the naming rule.
    NAME_INVALID = "QVS1001",
        "Partition name &1 is not valid.",
        "A partition's name is 1 to 10 characters: the letters A to Z, the digits 0 to 9 and the underscore, with a letter first. Choose a name of that form.";
    /// A partition ID out of range.
    ID_INVALID = "QVS1002",
        "Partition ID &1 is not valid.",
        "A partition's ID is a number from 1 to &2. The partition manager assigns it when the partition is created.";
    /// A serial number that is not one.
    SERIAL_INVALID = "QVS1003",
        "System serial number &1 is not valid.",
        "A partition's serial number is seven characters, digits and capital letters without I and O. It is made when the partition is created and is not changed afterwards.";
    /// An auto start delay out of range.
    DELAY_INVALID = "QVS1004",
        "Auto start delay &1 is not valid.",
        "The delay before a partition is started automatically, and the longest wait for its IPL to complete, are 0 to &2 seconds; the wait is 1 second at least.";
    /// A second partition of one name.
    NAME_IN_USE = "QVS1005",
        "Partition name &1 is already in use.",
        "Partition &2 has that name. No two partitions of a host have one name. Choose another name.";
    /// A change to a field that is never changed.
    FIELD_FIXED = "QVS1006",
        "The &1 of partition &2 cannot be changed.",
        "A partition's ID, its serial number, its environment and the time it was created are set when it is created and stay as they are for as long as its definition exists.";
    /// The default profile named is not one of the partition's.
    DEFAULT_PROFILE_MISSING = "QVS1007",
        "Partition &1 has no profile &2 to be its default.",
        "The default profile of a partition is one of its own profiles. Create the profile first, or name one the partition has.";
    /// A description too long, or with control characters.
    DESCRIPTION_INVALID = "QVS1008",
        "A partition's description of &1 bytes is not valid.",
        "A description is one line of at most &2 bytes, without control characters.";

    /// A profile's name breaks the naming rule.
    PROFILE_NAME_INVALID = "QVS1101",
        "Profile name &1 is not valid.",
        "A profile's name is 1 to 10 characters: the letters A to Z, the digits 0 to 9 and the underscore, with a letter first. Choose a name of that form.";
    /// Processor counts out of range.
    PROCESSORS_RANGE = "QVS1102",
        "The number of &1 is out of range: minimum &2, maximum &3.",
        "A profile names at least 1 and at most &4 &1. Correct the minimum or the maximum.";
    /// Minimum, desired and maximum processors out of order.
    PROCESSORS_ORDER = "QVS1103",
        "Minimum &2, desired &3 and maximum &4 &1 are not in order.",
        "The minimum is not more than the desired value, and the desired value is not more than the maximum. Correct the three values.";
    /// A field of shared processors in a dedicated profile.
    SHARED_ONLY = "QVS1104",
        "&1 is for shared processors only.",
        "The profile's processors are dedicated. Processing units, capped or uncapped sharing, the uncapped weight and the pool belong to shared processors. Remove the value, or make the processors shared.";
    /// A shared profile without all three unit values.
    UNITS_MISSING = "QVS1105",
        "Shared processors need minimum, desired and maximum processing units.",
        "A profile with shared processors gives all three of minimum_units, desired_units and maximum_units. One unit is one whole core's worth of processor time.";
    /// Minimum, desired and maximum units out of order.
    UNITS_ORDER = "QVS1106",
        "Minimum &1, desired &2 and maximum &3 processing units are not in order.",
        "The minimum is not more than the desired value, and the desired value is not more than the maximum. Correct the three values.";
    /// Fewer than 0.05 units for each virtual processor.
    UNITS_TOO_FEW = "QVS1107",
        "The &1 of &2 processing units is too few for &3 virtual processors: at least &4.",
        "Each virtual processor needs at least 0.05 processing units. Give more units, or fewer virtual processors, at that level.";
    /// More than 1.00 unit for each virtual processor.
    UNITS_TOO_MANY = "QVS1108",
        "The &1 of &2 processing units is too many for &3 virtual processors: at most &4.",
        "A virtual processor cannot use more than one whole core, which is 1.00 processing unit. Give fewer units, or more virtual processors, at that level.";
    /// An uncapped weight above 255.
    WEIGHT_INVALID = "QVS1109",
        "Uncapped weight &1 is not valid.",
        "The uncapped weight is 0 to &2. A weight of 0 keeps the partition to its processing units, as if it were capped.";
    /// A pool's name breaks the naming rule.
    POOL_NAME_INVALID = "QVS1110",
        "Shared processor pool name &1 is not valid.",
        "A pool's name is 1 to 10 characters: the letters A to Z, the digits 0 to 9 and the underscore, with a letter first. The default pool is DEFAULT.";
    /// Minimum, desired and maximum memory out of order.
    MEMORY_ORDER = "QVS1111",
        "Minimum &1 MB, desired &2 MB and maximum &3 MB of memory are not in order.",
        "The minimum is not more than the desired value, and the desired value is not more than the maximum. Correct the three values.";
    /// Memory that is no multiple of the block size.
    MEMORY_BLOCK = "QVS1112",
        "The &1 memory of &2 MB is not a multiple of the memory block size, &3 MB.",
        "Memory is given to partitions in whole blocks. The block size is a setting of the host. Give an amount that is a multiple of it.";
    /// Memory out of range.
    MEMORY_RANGE = "QVS1113",
        "The &1 memory of &2 MB is out of range.",
        "A partition has at least &3 MB and at most &4 MB of memory.";

    /// A virtual slot out of range.
    SLOT_INVALID = "QVS1201",
        "Virtual slot &2 of a &1 adapter is not valid.",
        "A profile's adapters use the virtual slots &3 to &4. Slots 0 and 1 are the partition's console and its service channel.";
    /// One slot used by two adapters.
    SLOT_TWICE = "QVS1202",
        "Virtual slot &1 is used twice: by a &2 adapter and a &3 adapter.",
        "Each adapter of a profile has a virtual slot of its own, whatever its kind. Give one of the two another slot.";
    /// More SCSI adapters than a partition may have.
    TOO_MANY_ADAPTERS = "QVS1203",
        "The profile has &1 virtual SCSI adapters; the most is &2.",
        "Put the units on fewer adapters. One adapter takes many units.";
    /// A profile with no disk unit.
    NO_LOAD_SOURCE = "QVS1204",
        "The profile has no load source.",
        "A partition IPLs from the first unit of its first virtual SCSI adapter: the lowest unit number in the lowest slot. Give the profile a virtual SCSI adapter with at least one unit.";
    /// More units on one adapter than it takes.
    TOO_MANY_UNITS = "QVS1205",
        "The virtual SCSI adapter in slot &1 has &2 units; the most is &3.",
        "Put the other units on a second virtual SCSI adapter.";
    /// A unit number out of range.
    LUN_INVALID = "QVS1206",
        "Unit &2 of the virtual SCSI adapter in slot &1 is not valid.",
        "An adapter's unit numbers are 0 to &3.";
    /// One unit number twice on an adapter.
    LUN_TWICE = "QVS1207",
        "The virtual SCSI adapter in slot &1 has unit &2 twice.",
        "Each unit of an adapter has a number of its own.";
    /// A backing store that is no absolute path.
    BACKING_INVALID = "QVS1208",
        "The backing store of unit &2 in slot &1 is not valid: &3",
        "A unit's backing store is a virtual disk of a storage pool, written as the pool's name and the disk's name: POOL1/PRODLS. In a test image it may also be the absolute path of a file or a block device on the host, with no .. in it.";
    /// One backing store for two units.
    BACKING_TWICE = "QVS1209",
        "The backing store of unit &2 in slot &1 is another unit's too: &3",
        "Two units of a partition cannot be kept in one file or one device. Give each unit a backing store of its own.";
    /// A unit's serial number that is not one.
    UNIT_SERIAL_INVALID = "QVS1210",
        "The serial number of unit &2 in slot &1 is not valid: &3",
        "A unit's serial number is 1 to 20 letters, digits, hyphens and underscores. It is how the partition knows the disk again, so it should stay the same.";
    /// One serial number for two units.
    UNIT_SERIAL_TWICE = "QVS1211",
        "The serial number of unit &2 in slot &1 is another unit's too: &3",
        "Each unit of a partition has a serial number of its own. Change one of the two, or leave it out: the host then makes one from the partition's serial number, the slot and the unit.";
    /// A VLAN out of range.
    VLAN_INVALID = "QVS1212",
        "VLAN &2 of the virtual Ethernet adapter in slot &1 is not valid.",
        "A port's VLAN is 1 to 4094.";
    /// A MAC address that is not one.
    MAC_INVALID = "QVS1213",
        "MAC address &2 of the virtual Ethernet adapter in slot &1 is not valid.",
        "A MAC address is six pairs of hexadecimal digits with colons between them, and is not a group address: the first pair is an even number. Leave it out to have the host choose one.";
    /// Media that is no absolute path.
    MEDIA_INVALID = "QVS1214",
        "The media of the virtual &1 device in slot &2 is not valid: &3",
        "The medium loaded into a virtual device at activation is an image (optical) or a volume (tape) of the media library, by its name: 1 to 32 letters, digits, - and _, a letter first. Leave it out for an empty device.";
    /// A location code that is not shaped like one.
    LOCATION_INVALID = "QVS1215",
        "Location code &1 is not valid.",
        "A physical I/O device is named by its location code, as the host's inventory shows it: letters, digits, hyphens and full stops, 64 characters at most.";
    /// One device listed twice.
    LOCATION_TWICE = "QVS1216",
        "Location code &1 is listed twice.",
        "A physical I/O device is given to a partition once.";
    /// Unit numbers with a gap.
    LUN_GAP = "QVS1217",
        "The units of the virtual SCSI adapter in slot &1 are not numbered from 0 without a gap.",
        "An adapter with &2 units has the unit numbers 0 to &3. Number the units in order from 0.";

    /// A unit backed by a host path in a release image.
    BACKING_NOT_VDISK = "QVS1218",
        "The backing store of unit &2 in slot &1 is not a virtual disk: &3.",
        "A unit's backing store is a virtual disk of a storage pool, written as the pool's name and the disk's name: POOL1/PRODLS. Make a virtual disk with the pool service and name it here.";
    /// IPL source D with no optical device.
    NO_OPTICAL = "QVS1219",
        "The profile's IPL source is D and it has no optical device.",
        "IPL source D reads the installation media in the partition's first optical device. Give the profile an [[optical]] device with the media, or choose another source.";
    /// A tagged VLAN of an adapter that is not one.
    TAGGED_INVALID = "QVS1220",
        "Tagged VLAN &2 of the virtual Ethernet adapter in slot &1 is not valid.",
        "An adapter's tagged VLANs are 1 to 4094, each listed once, none its port VLAN, 32 at most. The port VLAN is the one the guest has untagged.";
    /// An adapter's switch that is not a name.
    ADAPTER_SWITCH_INVALID = "QVS1221",
        "The virtual Ethernet adapter in slot &1 names switch &2, which is not a switch's name.",
        "Every adapter names the virtual switch it is on: 1 to 10 of A to Z, 0 to 9 and _, with a letter first.";
    /// Too many Ethernet adapters.
    ETHERNET_TOO_MANY = "QVS1222",
        "The profile has &1 virtual Ethernet adapters; a partition may have &2.",
        "The host's switch settings say how many adapters one partition may have, 8 at most.";
    /// An adapter on a switch that is not there.
    ADAPTER_SWITCH_MISSING = "QVS1223",
        "The virtual Ethernet adapter in slot &1 names switch &2, which does not exist.",
        "Make the switch in Work with virtual switches, or name one of the host's switches. An adapter that is not required comes up without link when its switch is missing at activation.";
    /// An adapter's VLAN its switch does not carry.
    ADAPTER_VLAN_MISSING = "QVS1224",
        "The virtual Ethernet adapter in slot &1 uses VLAN &3, which switch &2 does not carry.",
        "Add the VLAN to the switch first, or choose one of the VLANs it carries.";
    /// A MAC address another partition has.
    MAC_IN_USE = "QVS1225",
        "MAC address &2 of the virtual Ethernet adapter in slot &1 is also &3's.",
        "No two partitions of a host have one MAC address. Leave the address out to have the host make one from the partition and the slot.";
    /// An adapter with no switch.
    ADAPTER_SWITCH_NEEDED = "QVS1226",
        "The virtual Ethernet adapter in slot &1 names no switch.",
        "Every adapter names the virtual switch it is on, with the key switch.";
    /// A request names a partition that does not exist.
    PARTITION_MISSING = "QVS2001",
        "Partition &1 does not exist.",
        "No partition of this host has that ID. List the partitions to see the IDs there are.";
    /// A request that needs the partition not activated.
    PARTITION_ACTIVE = "QVS2002",
        "Partition &1 is activated.",
        "A partition is changed, deleted or activated only while it is not activated. Its state is &2. Shut it down first.";
    /// A request that needs the partition activated.
    PARTITION_NOT_ACTIVE = "QVS2003",
        "Partition &1 is not activated.",
        "Only an activated partition can be shut down or restarted. Its state is &2.";
    /// A request names a profile the partition does not have.
    PROFILE_MISSING = "QVS2004",
        "Partition &1 has no profile &2.",
        "List the partition's profiles to see the names there are, or create the profile.";
    /// An activation without a profile for a partition with no default.
    NO_DEFAULT_PROFILE = "QVS2005",
        "Partition &1 has no default profile.",
        "An activation that names no profile uses the partition's default profile. Create a profile for the partition, which becomes its default, or name one.";
    /// A create beyond the limit.
    TOO_MANY_PARTITIONS = "QVS2006",
        "The host already has &1 partitions, the most it is set to have.",
        "The number of partitions a host may have defined is a setting of the host. Delete a partition, or raise the setting.";
    /// A delete of the default profile.
    PROFILE_IS_DEFAULT = "QVS2007",
        "Profile &2 is the default profile of partition &1.",
        "The default profile cannot be deleted. Make another profile the partition's default first, or delete the partition.";
    /// A state file could not be written or read.
    STATE_NOT_KEPT = "QVS2008",
        "&1 could not be kept: &2",
        "The host could not write or read the file in its state directory. Nothing was changed. The host's log has more; a load source that is failing is the likely cause.";
    /// A request for a partition that is already ending.
    PARTITION_ENDING = "QVS2009",
        "Partition &1 is already shutting down.",
        "The partition was asked to shut down and its VMM has not ended yet. A list shows when it is not activated.";

    /// An activation while the host is stopping.
    HOST_STOPPING = "QVS2010",
        "Partition &1 cannot be activated: the host is shutting down.",
        "The host is ending its partitions before it powers off or restarts. Activate the partition when the host has started again.";
    /// More profiles than a partition may have.
    TOO_MANY_PROFILES = "QVS2011",
        "Partition &1 already has &2 profiles, the most a partition has.",
        "Delete a profile the partition no longer needs, or replace one by giving the new profile its name.";
    /// A new profile names one the partition has.
    PROFILE_EXISTS = "QVS2012",
        "Partition &1 has a profile &2.",
        "A new profile takes a name the partition has no profile of. To replace the one there, read it and give the revision it was read at.";
    /// A profile changed since the revision a request was made at.
    PROFILE_CHANGED = "QVS2013",
        "Partition &1's profile &2 is not at revision &3.",
        "It was changed or deleted since it was read, by another console or at the host. Read it again and make the change on what it is now.";
    /// A definition changed since the revision a request was made at.
    DEFINITION_CHANGED = "QVS2014",
        "Partition &1's definition is not at revision &2.",
        "It was changed since it was read, by another console or at the host. Read it again and make the change on what it is now.";
    /// Save current of a partition that holds nothing.
    NOTHING_TO_SAVE = "QVS2015",
        "Partition &1 is not activated: it has no current configuration to save.",
        "Its current configuration is what its activation gave it. Activate it first, or put a profile.";
    /// QSI0362: a partition created at an ID another has.
    ID_IN_USE = "QVS2016",
        "Partition ID &1 is in use.",
        "Partition &2 has that ID. Choose another, or leave the ID out for the partition manager to assign the lowest one free.";
    /// QSI0363: an ID past the host's limit of partitions.
    ID_PAST_LIMIT = "QVS2017",
        "Partition ID &1 is past this host's limit of &2 partitions.",
        "A partition created at an ID of its own is given one from 1 to the host's most partitions. Choose a lower ID, or raise the limit in the host's settings.";

    /// Too few cores for a dedicated partition's minimum.
    PROCESSORS_NOT_AVAILABLE = "QVS2101",
        "Partition &1 needs &2 dedicated processors; &3 are available.",
        "The profile's minimum number of processors is more than the cores that no running partition holds and that the shared partitions of the default pool do not need. Shut down a partition, lower the profile's minimum, or use shared processors.";
    /// Too few units in the pool for a shared partition's minimum.
    UNITS_NOT_AVAILABLE = "QVS2102",
        "Partition &1 needs &2 processing units in pool &3; &4 are available.",
        "The profile's minimum processing units are more than the pool has left: its running partitions are entitled to the rest. Shut down a partition of the pool, lower the profile's minimum, or use another pool.";
    /// Fewer cores in the pool than the minimum virtual processors.
    VIRTUALS_NOT_AVAILABLE = "QVS2103",
        "Partition &1 needs &2 virtual processors; pool &3 has &4 cores.",
        "Each virtual processor of a partition runs on a core of its pool, so a partition cannot have more virtual processors than its pool has cores. Lower the profile's minimum, or use a pool with more cores.";
    /// Too little memory for a partition's minimum.
    MEMORY_NOT_AVAILABLE = "QVS2104",
        "Partition &1 needs &2 MB of memory; &3 MB are available.",
        "The profile's minimum memory is more than what no running partition holds. Memory is not overcommitted. Shut down a partition, or lower the profile's minimum.";
    /// A profile names a pool that does not exist.
    POOL_MISSING = "QVS2105",
        "Shared processor pool &1 does not exist.",
        "Create the pool, or name a pool the host has. The default pool is DEFAULT.";
    /// A profile uses something a later phase brings.
    NOT_YET = "QVS2106",
        "&1 is not available in this release: it comes with Phase &2.",
        "The profile is valid and is kept, but a partition cannot be activated with it yet. Activate the partition with a profile that does not use this.";

    /// The VMM could not be started.
    VMM_NOT_STARTED = "QVS2107",
        "Partition &1 could not be started: &2",
        "The host could not set up the partition's resource controls or start its virtual machine monitor. Nothing is held for the partition. The host's log has more.";
    /// The VMM ended before the guest ran.
    DID_NOT_IPL = "QVS2108",
        "Partition &1 did not IPL: reference code &2.",
        "The partition's virtual machine monitor ended before the partition ran. The reference code says what was missing: a backing store that cannot be opened, a load source with nothing to IPL from, memory that was not there. The partition's history has the codes before it.";
    /// The VMM failed or was killed while the partition was activated.
    VMM_FAILED = "QVS2109",
        "Partition &1 ended in error: reference code &2, &3.",
        "The partition's virtual machine monitor failed, or was killed, while the partition was activated. The partition holds nothing now and is not started again by itself. Activate it when the cause is known.";

    /// A unit's backing store is not there.
    BACKING_MISSING = "QVS2110",
        "Partition &1 cannot be activated: the backing store of unit &3 in slot &2 is not there: &4",
        "The file or block device the profile names for the unit does not exist on the host. Nothing was started. Correct the profile, or bring the disk back.";

    /// A storage pool a partition needs is not usable.
    POOL_UNAVAILABLE = "QVS2111",
        "Partition &1 cannot be activated: storage pool &2 is &3.",
        "The partition's virtual disks are in a pool that is failed or has not been assembled. Look at the pool in Work with storage pools; a failed pool's units are to be replaced, or it is to be made again from a saved copy.";
    /// The pool service did not answer.
    POOLS_UNREACHABLE = "QVS2112",
        "Partition &1 cannot be activated: the pool service did not answer: &2.",
        "The partition's virtual disks and media are handed out by the pool service. Look at its state in the host's status; it is started again by itself after a fault.";
    /// A request for a slot that has no optical or tape device.
    MEDIA_SLOT_MISSING = "QVS2113",
        "Partition &1 has no optical or tape device in slot &2.",
        "Media are loaded into the optical and tape devices the partition's profile gave it at its activation.";
    /// The guest prevents a medium's removal.
    MEDIA_PREVENTED = "QVS2114",
        "Partition &1 has prevented the removal of the medium in slot &2.",
        "The partition is using the medium. End its use in the partition, or unload it with force, which the partition sees as the medium taken out.";
    /// A media request the VMM refused.
    MEDIA_REFUSED = "QVS2115",
        "The medium in slot &2 of partition &1 was not changed: &3.",
        "The partition's VMM could not do what was asked of the device. The device is as it was.";
    /// The switch service did not answer.
    SWITCHES_UNREACHABLE = "QVS2116",
        "Partition &1 cannot be activated: the switch service did not answer: &2.",
        "A partition's virtual Ethernet ports are made by the switch service. Look at its state in the host's status; it is started again by itself after a fault.";
    /// A required adapter's switch is missing at activation.
    PORT_NOT_GIVEN = "QVS2117",
        "Partition &1 cannot be activated: the virtual Ethernet adapter in slot &2 has no port: &3.",
        "The adapter is required, so the partition is not activated without it. Mark it not required to have it come up without link instead.";
    /// A host setting out of range.
    SETTING_INVALID = "QVS2201",
        "Setting &1 cannot be &2.",
        "The value allowed is &3.";
    /// A second pool of one name.
    POOL_EXISTS = "QVS2202",
        "Shared processor pool &1 already exists.",
        "No two pools of a host have one name. Choose another name, or change the pool that is there.";
    /// A pool core that is not there or is the host's.
    POOL_CORE_INVALID = "QVS2203",
        "Core &2 cannot be in shared processor pool &1.",
        "A pool's cores are cores of this host that are not kept back for the host itself. The host resources show which cores those are.";
    /// A core in two pools.
    POOL_CORE_TWICE = "QVS2204",
        "Core &2 of shared processor pool &1 is listed twice or is in another pool.",
        "A core is in one pool. Cores in no named pool are in the default pool.";
    /// A pool core that a running dedicated partition holds.
    POOL_CORE_BUSY = "QVS2205",
        "Core &2 cannot be in shared processor pool &1: a running partition has it as a dedicated processor.",
        "Shut down the partition that holds the core, or choose another core.";
    /// A pool with no core.
    POOL_EMPTY = "QVS2206",
        "Shared processor pool &1 has no core.",
        "A pool has at least one core.";
    /// A pool's maximum beyond its cores.
    POOL_MAXIMUM_INVALID = "QVS2207",
        "The maximum of &2 processing units is not valid for shared processor pool &1.",
        "A pool's maximum is more than 0.00 and not more than its cores' worth, &3 processing units. Leave it out for the cores' worth.";
    /// A change to a pool while partitions run in it.
    POOL_IN_USE = "QVS2208",
        "Shared processor pool &1 is in use by &2 running partitions.",
        "A pool is changed or deleted while none of its partitions is activated. Shut the partitions down first.";
    /// A request to change or delete the default pool.
    POOL_DEFAULT = "QVS2209",
        "The default shared processor pool cannot be created, changed or deleted.",
        "The default pool always holds every core that is not dedicated to a partition, kept back for the host or in a named pool.";
    /// A pool change that would leave the default pool too small.
    POOL_CORES_NEEDED = "QVS2210",
        "The default pool's running partitions need its cores: &1 processing units and &2 virtual processors on &3 cores.",
        "The change would take cores out of the default pool that the partitions running in it are entitled to, or need to have a core for each virtual processor. Choose other cores, or shut a partition down first.";
    /// A shut down that needs the system, of a partition without the
    /// service channel.
    NO_CHANNEL = "QVS2301",
        "Partition &1 has no service channel: a &2 shut down cannot be asked of its system.",
        "The system in the partition has not opened its service channel, or its release has none, or the channel is down. A delayed or operating-system shut down is carried out by the system itself and needs the channel. Shut the partition down immediately instead, or wait for the channel to come up.";
    /// A delay out of the range the system takes.
    DELAY_OUT_OF_RANGE = "QVS2302",
        "Shut down delay &1 is not valid.",
        "A delayed shut down's delay is 0 to &2 seconds. The system ends its jobs within the delay and then powers down.";
    /// The system refused a request, with its own message.
    SYSTEM_REFUSED = "QVS2303",
        "Partition &1 refused the request: &2 &3",
        "The system in the partition did not carry the request out and said why with its own message, given here with its identifier. Look at the system's own messages for more, then ask again.";
    /// The system did not answer a request in time.
    SYSTEM_SILENT = "QVS2304",
        "Partition &1 did not answer the request in &2 seconds.",
        "The request was sent on the service channel and neither an acceptance nor a refusal came back. The system may be busy or not responding. Look at its current reference code; shut it down immediately if it does not recover.";
    /// A shut down asked of the system did not end in time.
    SHUTDOWN_FORCED = "QVS2305",
        "Partition &1 was shut down immediately: its &2 shut down had not ended after &3 seconds.",
        "The system accepted the shut down but its virtual machine had not ended when the time allowed had passed, so the host stopped it. The system's next IPL is an abnormal one. The time allowed is a setting of partitions.toml.";
    /// A control panel function the host does not offer.
    PANEL_FUNCTION_INVALID = "QVS2306",
        "Control panel function &1 is not one of a partition's.",
        "A partition's functions are 01, 02, 03, 08, 11 to 19, 20, 21, 22 and 34 (docs/PANEL.md). Choose one of them.";
    /// A function that needs the keylock in manual.
    PANEL_NOT_MANUAL = "QVS2307",
        "Control panel function &1 needs the keylock in manual; partition &2's is &3.",
        "A function that changes or interrupts the system is carried out only with the keylock in the manual position. The keylock is the host's: change it in the definition, or for one activation, then activate the partition again.";
    /// A function's argument that is not one.
    PANEL_ARGUMENT_INVALID = "QVS2308",
        "Control panel function &1: &2 is not valid.",
        "Function 02 selects the IPL source (A, B or D), the IPL type (attended or unattended) and the keylock (normal, manual, auto or secure) of the next activation. Give values of those.";
    /// Function 34 of a partition that is not held.
    NOT_HELD = "QVS2309",
        "Partition &1 is not held: its memory is not kept.",
        "Function 34 takes a dump again from the memory of a partition held in Error after a terminal reference code or a guest fault. Use function 22 or ask for a dump of a running partition instead.";
    /// No pool holds the media repository, where dumps are kept.
    DUMPS_NO_REPOSITORY = "QVS2310",
        "No storage pool holds the media repository: partition &1's dump cannot be kept.",
        "Dumps are kept in the dumps directory of the storage pool designated as the media repository. Designate a pool with Work with storage pools, then ask again.";
    /// A dump that was not written whole.
    DUMP_NOT_WRITTEN = "QVS2311",
        "Partition &1's dump was not written: &2.",
        "Nothing of the dump is kept. When the pool was full, delete dumps or virtual disks the host no longer needs, or lower the number of dumps kept, and take the dump again.";
    /// A dump that is not there.
    DUMP_MISSING = "QVS2312",
        "Partition &1 has no dump &2.",
        "List the partition's dumps to see their names. A dump may have been removed to keep the partition within its limits.";
    /// A second dump while one is being taken.
    DUMP_BUSY = "QVS2313",
        "A dump of partition &1 is being taken.",
        "One dump of a partition is taken at a time. Wait for it to end, then ask again.";
    /// A second viewer of a partition's console.
    CONSOLE_IN_USE = "QVS2314",
        "Partition &1's console has a viewer.",
        "One client at a time has a partition's console. Ask to take it over to detach the other viewer, or wait for it to detach.";
    /// The console has shown nothing yet.
    CONSOLE_EMPTY = "QVS2315",
        "Partition &1's console has shown nothing yet.",
        "The system has not sent its console's screen. It sends it once its IPL has started; ask again in a moment.";
    /// The system says it is other than its definition.
    IDENTITY_DIFFERS = "QVS2316",
        "Partition &1's system says its &2 is &3; the definition has &4.",
        "The system read its identity from its own LIC record or its system values, and it differs from what the host gives the partition. The partition runs on. When the disk is another partition's, correct the profile; otherwise IPL the system again so it reads what the host wrote.";
    /// The service channel was closed for what the system sent.
    CHANNEL_CLOSED = "QVS2317",
        "Partition &1's service channel was closed: &2.",
        "The system sent what is not the partition service protocol, or more than it allows. The channel is closed and the partition runs on; it is opened again when the system's service daemon says hello again.";
    /// A console's panel function for a partition that does not allow them.
    PANEL_NOT_ALLOWED = "QVS2318",
        "Partition &1 does not allow control panel functions from a management console.",
        "Its settings keep its control panel to the host: Service Tools and the host's own requests. Change the partition's settings to allow them.";

    /// QSI1301: a location no device of the host has.
    DEVICE_UNKNOWN = "QVS2401",
        "No device of the host is at location &1.",
        "A profile's physical I/O names the host's devices by location code, as Service Tools option 10 lists them. Check the code, or whether the device was removed from the machine.";
    /// QSI1302.
    DEVICE_NOT_RELEASED = "QVS2402",
        "Device &1 is not released for partitions.",
        "A device is given to a partition only after an operator releases it at the host, at Service Tools option 10. Release it there, or take it out of the profile.";
    /// QSI1303: another partition holds the device or drive.
    DEVICE_HELD = "QVS2403",
        "Device &1 is held by partition &2.",
        "A device is one partition's at a time. It is given back when that partition ends.";
    /// QSI1304: a profile names part of an isolation group.
    DEVICE_GROUP_SPLIT = "QVS2404",
        "Device &1 cannot be given without &2.",
        "The host's IOMMU cannot tell the devices of one isolation group apart, so they go to one partition together or not at all. Name every device of the group in the profile, or none of them.";
    /// QSI1305: it cannot be given now; the reason follows.
    DEVICE_NOT_ASSIGNABLE = "QVS2405",
        "Device &1 cannot be given now: &2",
        "Service Tools option 10 shows why a device cannot be given. A device fenced after a failed reset is given again once the host has been restarted.";
    /// A device the host uses.
    DEVICE_HOST_USES = "QVS2406",
        "Device &1 is the host's: &2.",
        "The controllers of the load source and of every storage pool's disks, the management interface, every uplink, the console's display adapter and the PCI bridges stay the host's. They cannot be released.";
    /// A device whose isolation group has one the host uses.
    DEVICE_GROUP_HOST = "QVS2407",
        "Device &1 shares its isolation group with &2, which is the host's: &3.",
        "A group goes to a partition whole, and the host keeps that device, so no device of the group can be released. Another slot may put the device in a group of its own.";
    /// No reset.
    DEVICE_NO_RESET = "QVS2408",
        "Device &1 cannot be reset alone.",
        "A device is reset before a partition is given it and after the partition ends, so that nothing of one partition reaches the next. One with no function-level, power-management or bus reset cannot be released.";
    /// No message-signalled interrupts.
    DEVICE_NO_MSI = "QVS2409",
        "Device &1 cannot signal message interrupts.",
        "A partition's device interrupts by MSI or MSI-X only, which the host's interrupt remapping checks. A device with line interrupts alone cannot be released.";
    /// The IOMMU is off.
    NO_IOMMU = "QVS2410",
        "The host's IOMMU is not on.",
        "No device can be released without DMA remapping: a device given to a partition could reach the whole machine's memory. Turn VT-d or AMD-Vi on in the firmware's setup and restart the host.";
    /// Interrupt remapping is off.
    NO_REMAPPING = "QVS2411",
        "The host's interrupts are not remapped.",
        "No device can be released without interrupt remapping: a device given to a partition could send any interrupt to the host. Turn interrupt remapping on in the firmware's setup (with VT-d or AMD-Vi) and restart the host.";
    /// A device fenced after a failed reset.
    DEVICE_FENCED = "QVS2412",
        "Device &1 did not reset and is fenced until the host is restarted.",
        "A device that does not reset could carry one partition's data to the next, so it is not given again. Restart the host; if it fails again, service the device.";
    /// A released device not on the host.
    DEVICE_GONE = "QVS2413",
        "Released device &1 is not on the host.",
        "It was released for partitions and was not found at this IPL. Put it back, or take it back at Service Tools option 10 so that it is released no more.";
    /// A released device whose group changed.
    DEVICE_GROUP_CHANGED = "QVS2414",
        "The isolation group of released device &1 is not what it was: &2.",
        "The group was checked when the device was released; it changed since, so the device was not given to the pass-through driver. Take it back and release it again at Service Tools option 10.";
    /// A device a profile names cannot be taken back.
    DEVICE_NAMED = "QVS2415",
        "Device &1 is named by &2.",
        "A device a profile names stays released. Take it out of every profile first.";
    /// Already released.
    DEVICE_RELEASED_ALREADY = "QVS2416",
        "Device &1 is released already.",
        "Nothing was changed.";
    /// The device could not be bound or unbound.
    DEVICE_NOT_BOUND = "QVS2417",
        "Device &1 could not be given to &2: &3",
        "The kernel refused the driver. The device is as it was. The host's system log has the kernel's message.";
    /// A device that is not required was left out of an activation.
    DEVICE_LEFT_OUT = "QVS2418",
        "Device &1 was left out of partition &2: &3",
        "The device is not required, so the partition was activated without it. Mark it required to have the activation fail instead.";
    /// The name typed to release a device is not its location code.
    DEVICE_CONFIRM = "QVS2419",
        "&1 is not the location code of the device to be released.",
        "A release is confirmed by typing the device's location code in full, as the panel shows it.";
    /// QSI1306.
    NOT_A_DRIVE = "QVS2420",
        "Location &1 is not a tape drive of the host.",
        "A tape device names a volume of the media library, or a physical tape drive as @ and its location code. Service Tools option 10 lists the host's tape drives.";
    /// QSI1307.
    DRIVE_SLOT_PHYSICAL = "QVS2421",
        "The tape device in slot &1 of partition &2 is the physical drive &3.",
        "A physical drive's cartridge is changed by hand at the drive; only a virtual drive's volume is mounted and unmounted from a console.";
    /// The device could not be handed to the VMM.
    DEVICE_NOT_OPENED = "QVS2422",
        "Device &1 could not be opened for partition &2: &3",
        "The device stays released and is not held. The host's system log has the kernel's message.";

    /// QSI1401.
    ENTITLEMENT_NOT_RUNNING = "QVS2501",
        "Partition &1 is not running.",
        "A partition's entitlement changes while it runs; one that is not activated takes its profile's at its activation.";
    /// QSI1402.
    ENTITLEMENT_DEDICATED = "QVS2502",
        "Partition &1 has dedicated processors.",
        "Only a shared-processor partition's processing units, capping and weight change while it runs. Dedicated processors, virtual processors and memory change at activation.";
    /// QSI1403.
    ENTITLEMENT_OUTSIDE = "QVS2503",
        "&1 processing units are outside partition &2's profile: &3 to &4.",
        "A running partition's units stay within its profile's minimum and maximum units. Change the profile, then activate the partition again, for more or fewer.";
    /// QSI1404.
    ENTITLEMENT_VIRTUALS = "QVS2504",
        "&1 processing units do not suit partition &2's &3 virtual processors: &4 to &5.",
        "Each virtual processor needs 0.05 units at least and is worth 1.00 at most. Its virtual processors change at activation only.";
    /// QSI1405.
    ENTITLEMENT_POOL_SHORT = "QVS2505",
        "Pool &1 has not &2 processing units for partition &3: &4 are available to it.",
        "What the partition holds now and what no partition of the pool holds are all it can have. Lower another partition's units first, or ask for less.";
    /// Nothing to change, or a weight out of range.
    ENTITLEMENT_INVALID = "QVS2506",
        "The entitlement change is not valid: &1.",
        "A change names at least one of the units, capping and weight; a weight is 0 to 255.";
    /// The kernel's controls refused the change.
    ENTITLEMENT_NOT_APPLIED = "QVS2507",
        "Partition &1's entitlement was not changed: &2",
        "The partition keeps what it had. The host's system log has the reason.";
    /// A storage pool's name breaks the naming rule.
    SPOOL_NAME_INVALID = "QVS3001",
        "Storage pool name &1 is not valid.",
        "A storage pool's name is 1 to 10 characters: the letters A to Z, the digits 0 to 9 and the underscore, with a letter first.";
    /// A second storage pool of one name.
    SPOOL_EXISTS = "QVS3002",
        "A storage pool named &1 exists.",
        "Every storage pool of a host has a name of its own. Choose another.";
    /// No such storage pool.
    SPOOL_MISSING = "QVS3003",
        "There is no storage pool &1.",
        "The host's storage pools are listed in Work with storage pools.";
    /// The wrong number of disks for a protection.
    SPOOL_MEMBERS = "QVS3004",
        "Protection &1 takes &2 to &3 disks; &4 were chosen.",
        "No protection is one disk, mirrored two, RAID 5 three or more and RAID 6 four or more, 18 at most. Hot spares are chosen apart from these.";
    /// A disk the host does not have.
    DISK_MISSING = "QVS3005",
        "There is no disk &1.",
        "The host's disks are listed in Display disk configuration.";
    /// A disk that is the host's for something else.
    DISK_TAKEN = "QVS3006",
        "Disk &1 cannot be used: &2.",
        "A disk of a storage pool is one the host has nothing else on: not the load source, not another pool's unit or hot spare, not in use.";
    /// A disk with what is not this host's on it.
    DISK_FOREIGN = "QVS3007",
        "Disk &1 holds what is not this host's: &2.",
        "The disk has data the host did not put there, or belongs to another host's pool. It is taken only when the request confirms that what is on it may be erased: type the disk's name to confirm.";
    /// A disk too small for a pool.
    DISK_TOO_SMALL = "QVS3008",
        "Disk &1 is too small for a storage pool.",
        "A pool disk holds an 8 MiB record partition and a member partition of 256 MiB at least; a replacement or a hot spare is at least as large as the pool's units.";
    /// Disks of different logical block sizes.
    BLOCK_SIZES = "QVS3009",
        "The disks chosen have different logical block sizes: &1.",
        "A pool's disks have one logical block size. Choose disks that do.";
    /// A hot spare smaller than the units.
    SPARE_TOO_SMALL = "QVS3010",
        "Disk &1 is smaller than the pool's units and cannot be its hot spare.",
        "A hot spare takes a failed unit's place whole: it is at least as large as the pool's smallest unit.";
    /// A pool larger than a file system is made.
    SPOOL_TOO_LARGE = "QVS3011",
        "The pool would be &1 bytes; a storage pool is at most &2 bytes.",
        "A pool's file system is made at 64 TiB at most. Choose fewer or smaller disks, and make another pool of the rest.";
    /// A pool that is not empty.
    SPOOL_NOT_EMPTY = "QVS3012",
        "Storage pool &1 holds &2 virtual disks and &3 media, and cannot be deleted.",
        "A pool is deleted when it holds nothing. Delete its virtual disks and its media first.";
    /// A pool a partition uses.
    SPOOL_IN_USE = "QVS3013",
        "Storage pool &1 is in use by partition &2.",
        "Shut the partition down first.";
    /// A pool that cannot be used.
    SPOOL_NOT_ACTIVE = "QVS3014",
        "Storage pool &1 is &2.",
        "A pool is used while it is Active, Degraded, Rebuilding or Resynchronizing. A failed pool has lost more units than its protection covers.";
    /// A unit that cannot be replaced.
    UNIT_NOT_REPLACEABLE = "QVS3015",
        "Unit &2 of storage pool &1 cannot be replaced: &3.",
        "A unit is replaced when it has failed or is missing, by a disk at least as large that the host has nothing else on.";
    /// A pool that could not be made.
    SPOOL_NOT_MADE = "QVS3016",
        "Storage pool &1 was not made: &2.",
        "The disks chosen may hold part of the pool. They can be chosen again: a pool is made over what is on them.";
    /// A hot spare that cannot be added or removed.
    SPOOL_SPARES = "QVS3017",
        "Hot spare &1: &2.",
        "A pool with protection has up to 8 hot spares, each at least as large as its units.";
    /// The pool service's file system or devices failed.
    STORAGE_FAILED = "QVS3018",
        "The pool service could not &1: &2.",
        "The host's log has more. Try again; if it fails again, a disk may be failing.";
    /// QSI1616: a pool's size is fixed.
    SPOOL_FIXED_SIZE = "QVS3019",
        "Storage pool &1 cannot be extended: a pool's size is fixed when it is made.",
        "A pool's protected set and its file system are made once, of the disks chosen then. Make another pool of the new disks, or move virtual disks to a larger pool.";
    /// A virtual disk's name breaks the rule.
    VDISK_NAME_INVALID = "QVS3101",
        "Virtual disk name &1 is not valid.",
        "A virtual disk's name is 1 to 32 letters, digits, - and _, with a letter first.";
    /// A virtual disk's size out of range.
    VDISK_SIZE_INVALID = "QVS3102",
        "A virtual disk's size is &1 to &2 MiB, not &3.",
        "Sizes are whole megabytes.";
    /// A second virtual disk of one name.
    VDISK_EXISTS = "QVS3103",
        "Storage pool &1 has a virtual disk named &2.",
        "Choose another name.";
    /// No such virtual disk.
    VDISK_MISSING = "QVS3104",
        "There is no virtual disk &2 in storage pool &1.",
        "The pool's virtual disks are listed by the pool service.";
    /// A virtual disk another partition holds.
    VDISK_HELD = "QVS3105",
        "Virtual disk &1 is held by partition &2.",
        "A virtual disk is given to one partition at a time, and is grown or deleted while no partition holds it.";
    /// A virtual disk made smaller.
    VDISK_SHRINK = "QVS3106",
        "Virtual disk &1 cannot be made &2 MiB: a virtual disk only grows.",
        "A guest's data would be lost. Make a new, smaller disk and copy to it from the partition.";
    /// No room for a thick disk.
    SPOOL_NO_ROOM = "QVS3107",
        "Storage pool &1 has no room for &2 MiB.",
        "A thick disk's space is allocated when it is made. Make it thin, smaller, or in another pool.";
    /// A virtual disk a profile names.
    VDISK_NAMED = "QVS3108",
        "Virtual disk &1 is named by &2.",
        "A disk a partition's profile names is deleted only when the request says so, and the profile then names a disk that is not there.";
    /// No pool is the media repository.
    NO_REPOSITORY = "QVS3201",
        "No storage pool is the media repository.",
        "Optical images and tape volumes are kept in the pool the operator designates in Work with storage pools.";
    /// A media name breaks the rule.
    MEDIA_NAME_INVALID = "QVS3202",
        "Media name &1 is not valid.",
        "A name of the media library is 1 to 32 letters, digits, - and _, with a letter first.";
    /// A second medium of one name.
    MEDIA_EXISTS = "QVS3203",
        "The media library has a &1 named &2.",
        "Choose another name.";
    /// No such medium.
    MEDIA_MISSING = "QVS3204",
        "There is no &1 &2 in the media library.",
        "The library's images and volumes are listed by the pool service.";
    /// A medium in use.
    MEDIA_HELD = "QVS3205",
        "&1 is in use by partition &2.",
        "Unload it from the partition's device first.";
    /// Not an ISO 9660 image.
    NOT_ISO = "QVS3206",
        "&1 is not an ISO 9660 image.",
        "An optical image has an ISO 9660 primary volume descriptor at block 16 and 2048-byte blocks. Nothing of it was kept.";
    /// An upload of the wrong size.
    UPLOAD_SIZE = "QVS3207",
        "The image &1 is not the size it was said to be, or is not whole 2048-byte blocks.",
        "An image is uploaded whole, in one request that says its size; one that is cut off leaves nothing.";
    /// A tape volume's description that is not one.
    VOLUME_INVALID = "QVS3208",
        "Tape volume &1 cannot be made: &2.",
        "A volume identifier is 1 to 6 of A to Z and 0 to 9; a volume's capacity is 16 MiB to 16 TiB.";
    /// An import already under way.
    IMPORT_BUSY = "QVS3209",
        "An import into the media library is under way.",
        "One import from a device of the host runs at a time. The history says when it ends.";

    /// A physical tape drive went away.
    DRIVE_GONE = "QVS3301",
        "Physical tape drive &1 is not on the host.",
        "Its cable or power may be off. A partition that held it gets an error for each command until it comes back; it is given again at the partition's next activation.";
    /// A command outside the allow-list.
    DRIVE_COMMAND_REFUSED = "QVS3302",
        "Command &1 was not sent to physical tape drive &2.",
        "A partition's commands reach a physical drive through an allow-list (docs/STORAGE.md): firmware downloads and commands addressed past the drive are never passed.";
    /// A switch's name that is not one.
    SWITCH_NAME_INVALID = "QVS4001",
        "Switch name &1 is not valid.",
        "A virtual switch's name is 1 to 10 characters: the letters A to Z, the digits 0 to 9 and the underscore, with a letter first.";
    /// A second switch of one name.
    SWITCH_EXISTS = "QVS4002",
        "There is already a switch named &1.",
        "No two switches of a host have one name. Choose another name.";
    /// A switch that is not there.
    SWITCH_MISSING = "QVS4003",
        "There is no switch named &1.",
        "The host's switches are listed in Work with virtual switches.";
    /// A switch's VLAN that is not one.
    SWITCH_VLAN_INVALID = "QVS4004",
        "VLAN &1 of switch &2 is not valid.",
        "A switch's VLANs are 1 to 4094, each listed once.";
    /// Too many VLANs on a switch.
    SWITCH_VLANS_TOO_MANY = "QVS4005",
        "Switch &1 would carry &2 VLANs; a switch may carry &3.",
        "The host's switch settings say how many VLANs one switch may carry.";
    /// A switch's MTU out of range.
    SWITCH_MTU_INVALID = "QVS4006",
        "MTU &1 of switch &2 is not valid.",
        "A switch's MTU is 1280 to 9000 bytes. Its uplink and every partition's port on it are given that MTU.";
    /// Too many switches.
    SWITCHES_TOO_MANY = "QVS4007",
        "The host would have &1 switches; it may have &2.",
        "The host's switch settings say how many switches it may have.";
    /// A switch a profile names.
    SWITCH_NAMED = "QVS4008",
        "Switch &1 is named by partition &2, profile &3, slot &4.",
        "A switch that a profile's adapter names is not deleted or renamed. Change or delete the adapter in the profile first.";
    /// A VLAN a profile uses.
    SWITCH_VLAN_NAMED = "QVS4009",
        "VLAN &1 of switch &2 is used by partition &3, profile &4, slot &5.",
        "A VLAN that a profile's adapter uses stays on its switch. Change the adapter in the profile first.";
    /// A native VLAN the switch does not carry.
    NATIVE_VLAN_INVALID = "QVS4010",
        "Native VLAN &1 is not a VLAN switch &2 carries.",
        "The native VLAN is the one carried untagged on the uplink. It is one of the switch's VLANs, or none.";
    /// A limit out of range.
    LIMIT_INVALID = "QVS4011",
        "Switch setting &1 = &2 is not valid: it is &3.",
        "The limits of the switch service are each a number in the range given.";
    /// The kernel refused a step.
    SWITCH_NOT_CHANGED = "QVS4012",
        "The switch service could not &1: &2.",
        "The host's log has more. What was there before the request is as it was.";
    /// Too many ports on a switch.
    SWITCH_PORTS_TOO_MANY = "QVS4013",
        "Switch &1 has &2 partition ports; it may have &3.",
        "The host's switch settings say how many partitions' ports one switch may have.";
    /// Two switches of one number in the state file.
    SWITCH_ID_TWICE = "QVS4014",
        "Switch number &1 is not valid or is used twice.",
        "A switch's number is given by the switch service when the switch is made. The state file was changed by something else.";
    /// A switch with a running partition's port on it.
    SWITCH_IN_USE = "QVS4015",
        "Switch &1 has the port of partition &2, slot &3.",
        "A switch, or a VLAN of it, that a running partition is on is not deleted or taken away. End the partition first.";
    /// No interface at a location.
    INTERFACE_MISSING = "QVS4101",
        "There is no network interface at &1.",
        "Display network hardware lists the host's interfaces with their location codes.";
    /// The management interface named as an uplink.
    INTERFACE_MANAGEMENT = "QVS4102",
        "Interface &1 is the management interface.",
        "The interface the host is managed through is never a switch's uplink. Choose another interface, or move the management network to another interface first.";
    /// An interface another switch has.
    INTERFACE_TAKEN = "QVS4103",
        "Interface &1 is in the uplink of switch &2.",
        "An interface belongs to one switch at a time. Clear that switch's uplink first.";
    /// Several interfaces with no bond mode.
    BOND_MODE_NEEDED = "QVS4104",
        "An uplink of &1 interfaces needs a bond mode.",
        "Several interfaces are bonded: active-backup, which needs nothing of the physical switch, or 802.3ad, which needs LACP on its ports. One interface takes no bond mode.";
    /// An MTU the uplink cannot carry.
    MTU_TOO_LARGE = "QVS4105",
        "Interface &1 cannot carry switch &2's MTU of &3: it carries &4 at most.",
        "A switch's uplink is given the switch's MTU. Choose a smaller MTU, or an interface that carries it.";
    /// An uplink of no interface or too many.
    UPLINK_INTERFACES = "QVS4106",
        "An uplink has 1 to &1 interfaces, not &2.",
        "Name one interface, or several to be bonded.";
    /// An interface named twice.
    INTERFACE_TWICE = "QVS4107",
        "Interface &1 is named twice.",
        "Each interface of an uplink is named once.";
    /// A port that cannot be made.
    PORT_NOT_MADE = "QVS4201",
        "The port of partition &1, slot &2, on switch &3 was not made: &4.",
        "Nothing of it was left behind. The host's log has more.";
    /// A port's address another port has.
    PORT_MAC_IN_USE = "QVS4202",
        "MAC address &1 is the port of partition &2, slot &3.",
        "No two ports of the host have one MAC address.";
    /// A problem number that is not in the log.
    NO_PROBLEM = "QVS5001",
        "There is no problem &1.",
        "Problems are numbered from 1 and no number is used twice. The log keeps 500: past that, the oldest closed problems are removed first.";
    /// A close of a problem closed already.
    PROBLEM_CLOSED = "QVS5002",
        "Problem &1 was closed already, by &2.",
        "A closed problem stays closed. When its condition is seen again a new problem is opened.";
    /// A close while the condition holds.
    PROBLEM_HOLDS = "QVS5003",
        "The condition of problem &1 still holds.",
        "A problem is closed once its condition no longer holds, which its analysis log says. Close it anyway when the condition is understood and will be dealt with later.";
    /// The log could not be written.
    PROBLEMS_NOT_KEPT = "QVS5004",
        "The problem log could not be written.",
        "Nothing was changed. The data file system may be full or failing: look at the problems about it, then try again.";
    /// A note that is empty, too long or more than a line.
    NOTE_INVALID = "QVS5005",
        "A note of &1 bytes is not valid.",
        "A note is one line of 1 to &2 bytes, without control characters. It is added to the problem's analysis log with its author's name.";
    /// A2001030.
    PRB_RECORD = "QVS5101",
        "The A/B record of the load source was not valid: &1.",
        "No copy of the record read, and a new one was written whose default is the slot the host started from; an update that was pending is forgotten. Display the code levels (Service Tools option 8, 5). If this is seen again the load-source disk may be failing.";
    /// A2001035.
    PRB_SLOT_FELL_BACK = "QVS5102",
        "Code slot &1 did not confirm, and the host went back to the other slot: &2.",
        "An update was activated and the slot it was installed in did not become ready at its IPL. The host runs the release it ran before, with its state as it was before the restart. A host dump was taken: take it to the release's supplier.";
    /// A2001040.
    PRB_DATA_NOT_MOUNTED = "QVS5103",
        "The data file system was not mounted: &1.",
        "The host runs with its state in memory only, and keeps nothing it is told. Nothing was reformatted. Look at the load-source units, and restart the host when they are in order.";
    /// A2001045.
    PRB_DATA_FULL = "QVS5104",
        "The data file system is nearly full: &1.",
        "Logs, dumps, a staged update and the state are kept on it. Delete the dumps no longer needed (Service Tools option 8, 3) and a staged update that will not be installed. The problem is cleared when less than 85% is used.";
    /// A2001050.
    PRB_CLOCK = "QVS5105",
        "The clock could not be set from the hardware clock: &1.",
        "The host's time is wrong until a time server sets it. The hardware clock's battery may be spent. Check the time servers and the hardware.";
    /// A2001070.
    PRB_STATE_FILE = "QVS5106",
        "A state file was not valid, and its defaults are in use: &1.",
        "The file named was refused at the IPL and is left as it was; what it held is not in use. Set again what it held, at Service Tools or a management console, which writes it whole.";
    /// A2001072.
    PRB_MEMORY_SHORT = "QVS5107",
        "The memory for partitions was not reserved in full: &1.",
        "The kernel set aside fewer huge pages than the settings ask, so partitions can be given less memory. Give partitions less memory in the host's settings and restart the host, or add memory.";
    /// A2001075.
    PRB_IDENTITY = "QVS5108",
        "The host's identity could not be made: &1.",
        "Without its key and certificate the host cannot be managed over QSI. Look at the data file system; the identity is made at the next IPL.";
    /// A2001080.
    PRB_SERVICE_STOPPED = "QVS5109",
        "Service &1 failed five times in ten minutes and is not started again.",
        "What the service does is not done until the host is restarted. A host dump was taken when it stopped: take it to the release's supplier, and restart the host when you can.";
    /// A2001095.
    PRB_BOOT_ORDER = "QVS5110",
        "The firmware's boot order was not written for code slot &1: &2.",
        "The slot is confirmed in the A/B record, but the firmware may start the other slot first. Look at the firmware's boot settings; the order is written again at the next confirmation.";
    /// A2002005.
    PRB_NO_INTERFACE = "QVS5111",
        "No management interface was found: &1.",
        "No network link matches the management network's settings. Check the cable, and the interface at Service Tools option 3.";
    /// A2002010.
    PRB_NO_CARRIER = "QVS5112",
        "Management interface &1 has no carrier.",
        "Check its cable and the switch port it is connected to. The host cannot be managed over QSI until it has carrier.";
    /// A2002020.
    PRB_NO_ADDRESS = "QVS5113",
        "Management interface &1 has no address.",
        "With DHCP, look at the DHCP server; with a static address, at the settings at Service Tools option 3.";
    /// A2003010.
    PRB_SET_NOT_ASSEMBLED = "QVS5114",
        "The mirrored load source was not assembled: &1.",
        "No unit in step was found, and the data file system is not mounted. Check that the load-source disks are connected and working.";
    /// A2003020.
    PRB_UNIT_FAILED = "QVS5115",
        "Load-source unit &1 is missing or failed.",
        "The other unit carries the load source alone, without protection. Replace the disk: the storage service rebuilds onto the new one, and the problem is cleared when it is in step.";
    /// A2003030.
    PRB_REBUILD_FAILED = "QVS5116",
        "The rebuild of load-source unit &1 did not finish: &2.",
        "The unit is not in step, and the load source is still without protection. Look at the disk named, and replace it if it is failing.";
    /// A2003040.
    PRB_UNITS_APART = "QVS5117",
        "The load-source units each ran without the other: &1.",
        "The host waits at the console for the unit to keep to be chosen; the other is written again from it. Choose the unit that holds the data wanted.";
    /// A2005060.
    PRB_MACHINE_NOT_BUILT = "QVS5118",
        "The machine of partition &1 could not be built: &2.",
        "KVM, memory, a device or the privilege drop failed when the partition was activated. Deal with the reason, then activate the partition again.";
    /// A2005120.
    PRB_GUEST_FAULT = "QVS5119",
        "Partition &1 ended with a guest fault: &2.",
        "The system in the partition faulted in a way its processors could not go on from. Look at the system's own codes in the partition's history, and take its dump to the system's supplier.";
    /// A2005130.
    PRB_VMM_ERROR = "QVS5120",
        "The VMM of partition &1 failed while the partition ran: &2.",
        "The partition ended without its system being asked. Take a host dump to the release's supplier, and activate the partition again.";
    /// A2005150.
    PRB_VMM_KILLED = "QVS5121",
        "The VMM of partition &1 was killed: &2.",
        "The partition ended at once. If the host ran short of memory, give its partitions less.";
    /// A2005160.
    PRB_HELD_FOR_SPACE = "QVS5122",
        "Partition &1 is held: a storage pool its virtual disks are in is full.",
        "Its processors are stopped until the pool has room; nothing reached its system as an error. Delete what the pool does not need. The partition goes on by itself, and the problem is cleared.";
    /// A2005170.
    PRB_HELD_FAULT = "QVS5123",
        "Partition &1 is held after a guest fault: &2.",
        "Its processors are stopped and its memory kept. Dump the partition, then shut it down or restart it.";
    /// A2005180.
    PRB_DUMP_NOT_WRITTEN = "QVS5124",
        "A dump of partition &1 could not be written: &2.",
        "Nothing of it was kept. Make room in the media repository's pool and dump the partition again while it is held.";
    /// A2006020.
    PRB_AUTO_START = "QVS5125",
        "Partition &1 was not started automatically: &2.",
        "The other partitions were tried. Deal with the reason, then activate the partition.";
    /// A2006030.
    PRB_PARTITION_STATE = "QVS5126",
        "A partition's definition or profile was not valid: &1.",
        "The file named was refused when the partition manager started. Correct it, or delete the partition and create it again.";
    /// A2006040.
    PRB_CONTROLS = "QVS5127",
        "The resource controls of partition &1 could not be set: &2.",
        "Its processors or memory may not be limited as its profile asks. Shut the partition down and activate it again.";
    /// A2006090.
    PRB_NOT_ENDED = "QVS5128",
        "Partitions had not ended when the host shut down: &1.",
        "The host went down with partitions still running, and their systems may not have ended cleanly. Look at each at its next IPL.";
    /// A2007010.
    PRB_POOL_NOT_ASSEMBLED = "QVS5129",
        "Storage pool &1 was not assembled: &2.",
        "Its units were found, but its set could not be opened or its file system mounted; its partitions cannot be activated. Check its disks.";
    /// A2007020.
    PRB_POOL_UNIT_FAILED = "QVS5130",
        "Disk unit &1 of storage pool &2 failed or is missing.",
        "The pool runs on the units left, with less protection or none. Replace the disk: the pool rebuilds onto the new one or a hot spare, and the problem is cleared when the rebuild ends.";
    /// A2007025.
    PRB_POOL_EXPOSED = "QVS5131",
        "Storage pool &1 is degraded and has no hot spare to rebuild onto.",
        "Another failure may lose the pool. Replace the failed disk, or add a hot spare, as soon as you can.";
    /// A2007030.
    PRB_POOL_REBUILD_FAILED = "QVS5132",
        "The rebuild of storage pool &1 did not finish: &2.",
        "The pool is still without its full protection. Look at the disk named, and replace it if it is failing.";
    /// A2007040.
    PRB_POOL_FAILED = "QVS5133",
        "Storage pool &1 failed: &2.",
        "More units are lost than its protection covers. Nothing more is written to it and its partitions are in Error. Bring its units back, or make the pool again and restore its virtual disks.";
    /// A2007051.
    PRB_POOL_NEARLY_FULL = "QVS5134",
        "Storage pool &1 is 90% allocated: &2.",
        "A thin virtual disk that cannot grow holds its partition. Delete what the pool does not need, or move virtual disks to another pool. The problem is cleared when less than 80% is allocated.";
    /// A2008010.
    PRB_SWITCH_NOT_BUILT = "QVS5135",
        "Virtual switch &1 was not built: &2.",
        "Its partitions' adapters cannot be given ports. Deal with the reason, then change the switch at Service Tools option 3 or from a console.";
    /// A2008020.
    PRB_UPLINK_LOST = "QVS5136",
        "Every interface of the uplink of virtual switch &1 lost carrier.",
        "Traffic between its partitions goes on; traffic to and from outside the host has stopped. Check the cables and the physical switch. The problem is cleared when carrier is back.";
    /// A2008025.
    PRB_BOND_MEMBER_DOWN = "QVS5137",
        "Interface &1 of the uplink of virtual switch &2 lost carrier.",
        "The bond's other interfaces carry the traffic, without redundancy. Check the cable and the physical switch's port. The problem is cleared when carrier is back.";
    /// A2008030.
    PRB_UPLINK_MISSING = "QVS5138",
        "Interface &1, in the uplink of virtual switch &2, is not on the host.",
        "A card may have been moved or removed. Put it back, or change the switch's uplink.";
    /// A2009010.
    PRB_CHANNEL_CLOSED = "QVS5139",
        "The service channel of partition &1 was closed: &2.",
        "The system sent what is not the protocol. The partition runs on, and the channel opens again when the system says hello; the problem is then cleared. If this is seen again, take it to the system's supplier.";
    /// A2009020.
    PRB_NOT_RESPONDING = "QVS5140",
        "Partition &1 is not responding on its service channel.",
        "Its system has not answered pings for three intervals; nothing was done to it. Look at its console. The problem is cleared when it answers again.";
    /// A2009030.
    PRB_IDENTITY_DIFFERS = "QVS5141",
        "The system in partition &1 names another identity than the host gave it: &2.",
        "Its serial number, side or keylock differs from the partition's. Check that the partition's virtual disks are its own.";
    /// A2009055.
    PRB_TERMINAL = "QVS5142",
        "Partition &1 is held after its system posted a terminal code: &2.",
        "Its processors are stopped and its memory kept. The problem's reference code is the system's own: look it up in the system's documentation. Dump the partition, then shut it down or restart it.";
    /// A2010010.
    PRB_PROCESSOR = "QVS5143",
        "The processor lacks what the service interface needs: &1.",
        "QSI's cryptography needs AVX2, ADX, AES and PCLMULQDQ. The host cannot be managed over QSI on this processor; everything at its console works. Move the host to a processor that has them.";
    /// A2010035.
    PRB_PAIRING_FAILING = "QVS5144",
        "Attempts to pair a management console failed again and again: &1.",
        "A pairing password's attempts were spent, or an address was turned away for its failures. If nobody was pairing a console, someone may be trying to: look at the addresses in the system log.";
    /// A2011010.
    PRB_HOST_DUMP_FAILED = "QVS5146",
        "A host dump could not be written: &1.",
        "Nothing of it was kept. Make room on the data file system, then take the dump again (Service Tools option 8, 2).";
    /// A2012030.
    PRB_INSTALL_FAILED = "QVS5147",
        "An update could not be installed into code slot &1: &2.",
        "The slot is not pending, and the host runs on as before. Install the update again; if it fails again the load-source disk may be failing. The problem is cleared by an install that succeeds.";
    /// B2001000.
    PRB_PID1_FAILED = "QVS5148",
        "The host's first process failed: &1.",
        "The host stopped. Take the system log to the release's supplier.";
    /// B2003000.
    PRB_STORAGE_LOST = "QVS5149",
        "The storage service of the mirrored load source stopped: &1.",
        "The host stopped, its data file system without a server. Restart the host.";

    /// A2013010.
    PRB_NO_IOMMU = "QVS5150",
        "The host's IOMMU or interrupt remapping is off: &1.",
        "No device can be released for partitions, and none is given to one. Everything else works. Turn VT-d or AMD-Vi and interrupt remapping on in the firmware's setup and restart the host.";
    /// A2013030.
    PRB_DEVICE_GONE = "QVS5151",
        "Released device &1 was not found at the IPL.",
        "Partitions that require it are not activated; the others are. Put the device back and restart the host, or take it back at Service Tools option 10.";
    /// A2013035.
    PRB_DEVICE_GROUP_CHANGED = "QVS5152",
        "Released device &1 was not given to the pass-through driver: its isolation group changed.",
        "A firmware update or a card moved can change a group. Take the device back and release it again at Service Tools option 10, where the new group is shown.";
    /// A2013050.
    PRB_DEVICE_FENCED = "QVS5153",
        "Device &1 did not reset and is fenced.",
        "It is not given to any partition until the host is restarted. If it fails to reset again, service the device or take it back.";
    /// A2013080.
    PRB_DRIVE_GONE = "QVS5154",
        "Physical tape drive &1 went away.",
        "Check its cable and power. The partition that held it gets an error for each command until it comes back.";
    /// A2005190.
    PRB_VMM_FORBIDDEN = "QVS5155",
        "The VMM of partition &1 was ended by its confinement: &2.",
        "It made a system call a VMM never needs, which is what a system that took over its VMM would do; nothing outside the partition was reached. Keep the partition's dump and a host dump for the release's supplier before activating it again.";
    /// A2001005.
    PRB_HOST_PANICKED = "QVS5156",
        "The host restarted after its kernel panicked: &1.",
        "The kernel's last messages were kept in the firmware and moved to the data file system; the next host dump holds them. Take a host dump to the release's supplier.";
    /// No host dump of that name.
    HOST_DUMP_MISSING = "QVS5201",
        "There is no host dump &1.",
        "A host dump's name is HOSTDUMP- and the time it was taken. The list shows the dumps kept: at most 8, the oldest put out first.";
    /// One is being taken.
    HOST_DUMP_TAKING = "QVS5202",
        "A host dump is being taken: &1.",
        "One host dump is taken at a time. Wait for it to be written, then ask again.";
    /// QSI1101.
    UPDATE_SIGNATURE = "QVS5301",
        "Update image &1 is not signed by a key this host trusts.",
        "An image is installed only if its manifest is signed by a key the running release trusts, and nothing turns that off. Get the image from the release's supplier again.";
    /// QSI1102.
    UPDATE_DAMAGED = "QVS5302",
        "Update image &1 is damaged: &2.",
        "The image does not read, or a size or a hash in it is not its manifest's. Copy or upload it again; if it is refused again, get it from the release's supplier again.";
    /// QSI1103.
    UPDATE_PRODUCT = "QVS5303",
        "Update image &1 is for &2, and this host is &3.",
        "An update image updates the product it was built for. Use an image of this host's product.";
    /// QSI1104.
    UPDATE_OLDER = "QVS5304",
        "Update image &1 holds &2, which is older than &3, which runs.",
        "An older release is installed only when the stage allows it. Stage the image again, allowing an older release, if that is what is wanted.";
    /// QSI1105.
    UPDATE_STATE = "QVS5305",
        "Update image &1 holds &2, which cannot read the host's state: &3.",
        "The release in the image reads an older form of that file than the host has. It is never installed, whatever the stage allowed: the host would not start with its state.";
    /// QSI1106.
    UPDATE_NOTHING_STAGED = "QVS5306",
        "No update image is staged.",
        "Stage an image first: upload it from a management console, or install it from a removable volume at Service Tools option 8, 4.";
    /// QSI1107.
    UPDATE_STAGED_ALREADY = "QVS5307",
        "Update image &1 is staged already.",
        "One image is staged at a time. Install it, or discard it first.";
    /// QSI1108.
    UPDATE_NOT_VERIFIED = "QVS5308",
        "Update image &1 has not verified: &2.",
        "Only an image that passed every check is installed. Discard it and stage another.";
    /// QSI1109.
    UPDATE_TOO_LONG = "QVS5309",
        "Update image &1 sent more than the &2 bytes it named.",
        "Nothing of it was kept. Stage it again with its size.";
    /// QSI1110.
    UPDATE_CUT = "QVS5310",
        "Update image &1 ended after &2 of its &3 bytes.",
        "Nothing of it was kept. Stage it again.";
    /// QSI1111.
    UPDATE_BUSY = "QVS5311",
        "An update is &1.",
        "One install or activation at a time. Wait for it to end.";
    /// QSI1112.
    UPDATE_NOTHING_PENDING = "QVS5312",
        "No code slot is pending.",
        "Install a verified image first: the slot it is written to is then pending.";
    /// QSI1113.
    UPDATE_PARTITIONS_RUN = "QVS5313",
        "Partitions are running: &1.",
        "An activation restarts the host. Shut the partitions down first, or activate saying that they are to be shut down through their operating systems.";
    /// QSI1114.
    UPDATE_PENDING = "QVS5314",
        "Code slot &1 is pending already.",
        "Activate the pending slot, or cancel it, before another image is installed.";
    /// An install failed.
    UPDATE_NOT_WRITTEN = "QVS5315",
        "Code slot &1 could not be written: &2.",
        "The slot is not pending, and the slot that runs is as it was. Install the image again; if it fails again the load-source disk named may be failing.";
    /// PID 1 refused.
    UPDATE_RECORD = "QVS5316",
        "The A/B record could not be changed: &1.",
        "The slots are as they were. Display the code levels (Service Tools option 8, 5) and try again.";
    /// Staging failed.
    UPDATE_NOT_KEPT = "QVS5317",
        "Update image &1 could not be kept on the data file system: &2.",
        "Nothing of it was kept. Make room on the data file system, then stage it again.";
    /// A stage's name or size.
    UPDATE_NAME = "QVS5318",
        "&1 is not a name and size an update image is staged by.",
        "A stage names the image in 1 to 64 printable characters and gives its size, more than 0 and at most 300 MiB.";
}
