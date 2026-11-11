//! Which optional modules the simulated printer has.

/// One Happy Hare unit of a profile: a `[mmu_unit <name>]` section (`config/base/mmu_hardware.cfg` L137-155 of
/// moggieuk/Happy-Hare at 7df830f). Gates are numbered globally, in the order of the units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmuUnitSpec {
    /// Section name and `units` entry (`mmu_unit <name>`).
    pub name: &'static str,
    /// `display_name` option.
    pub display_name: &'static str,
    /// `vendor` option; picks the unit profile (selector, bypass, buffer) like `mmu_unit.py` L105-125.
    pub vendor: &'static str,
    /// `version` option.
    pub version: &'static str,
    /// `num_gates` option.
    pub gates: usize,
    /// `extruder_name` option: the Klipper extruder the unit feeds.
    pub extruder: &'static str,
}

/// Two units on one extruder, as `happy-hare-units` has them (`extruder_name` defaults to `extruder`).
pub const MMU_UNITS_ONE_EXTRUDER: [MmuUnitSpec; 2] = [
    MmuUnitSpec {
        name: "ercf",
        display_name: "ERCF",
        vendor: "ERCF",
        version: "1.1sb",
        gates: 4,
        extruder: "extruder",
    },
    MmuUnitSpec {
        name: "tradrack",
        display_name: "Tradrack",
        vendor: "Tradrack",
        version: "1.0",
        gates: 4,
        extruder: "extruder",
    },
];

/// One unit per head of a two-head tool changer (`toolchanger-mmu`).
pub const MMU_UNITS_ON_HEADS: [MmuUnitSpec; 2] = [
    MmuUnitSpec {
        name: "ercf",
        display_name: "ERCF",
        vendor: "ERCF",
        version: "1.1sb",
        gates: 4,
        extruder: "extruder",
    },
    MmuUnitSpec {
        name: "tradrack",
        display_name: "Tradrack",
        vendor: "Tradrack",
        version: "1.0",
        gates: 4,
        extruder: "extruder1",
    },
];

/// Which filament changer of AFC (AFC-Klipper-Add-On) the printer has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfcLayout {
    /// One `AFC_BoxTurtle Turtle_1` with four lanes on `extruder`.
    BoxTurtle,
    /// A Snapmaker U1 style tool changer: `AFC_Toolchanger Tools` with four heads, each with its own lane.
    Toolchanger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachineProfile {
    pub quad_gantry_level: bool,
    pub chamber_heater: bool,
    pub mmu_gates: Option<usize>,
    /// Happy Hare v4 units; `mmu_gates` is then their total. `None` is one unit as the older profiles show it.
    pub mmu_units: Option<&'static [MmuUnitSpec]>,
    /// AFC instead of Happy Hare.
    pub afc: Option<AfcLayout>,
    /// Klipper `exclude_object` module with the objects of the print.
    pub exclude_object: bool,
    /// Moonraker's `spoolman` component with a few spools.
    pub spoolman: bool,
    /// Extra fans, LEDs with the led_effect plugin, a WLED strip of Moonraker, more temperature sensors and firmware
    /// retraction.
    pub devices: bool,
    /// Number of filament sensors, taken from the front of the list of the profile (`FILAMENT_SENSORS` or `MMU_FILAMENT_SENSORS`).
    pub filament_sensors: usize,
    /// A probe (`[probe]`, simulated like a BLTouch).
    pub probe: bool,
    /// `[bed_mesh]` with the profile `default`.
    pub bed_mesh: bool,
    /// `[bed_screws]` and `[screws_tilt_adjust]` with four screws, as on a bed slinger.
    pub bed_screws: bool,
    /// Tools of a `klipper-toolchanger` tool changer, each with its own extruder; 0 means no tool changer. The heads
    /// of the AFC tool changer are not counted here (see `afc`).
    pub tools: usize,
}

/// Klipper object names of the simulated filament sensors; the second reports a runout.
pub const FILAMENT_SENSORS: [&str; 2] =
    ["filament_switch_sensor toolhead_runout", "filament_motion_sensor extruder_encoder"];

/// Happy Hare sensors as Klipper objects, as listed by a real single-lane type-B machine with a sync-feedback buffer
/// (Happy Hare 4.1.0, `objects.list`, 2026-10-25). Names come from `mmu_constants.py`; the gate suffix is kept.
/// The machine has no shared exit sensor and no encoder.
pub const MMU_FILAMENT_SENSORS: [&str; 6] = [
    "filament_switch_sensor mmu_entry_0",
    "filament_switch_sensor mmu_exit_0",
    "filament_switch_sensor toolhead",
    "filament_switch_sensor extruder",
    "filament_switch_sensor filament_compression",
    "filament_switch_sensor filament_tension",
];

impl MachineProfile {
    pub const MAX_MMU_GATES: usize = 16;
    pub const TOOLCHANGER_TOOLS: usize = 3;

    pub fn basic() -> Self {
        Self {
            quad_gantry_level: false,
            chamber_heater: false,
            mmu_gates: None,
            mmu_units: None,
            afc: None,
            exclude_object: false,
            spoolman: false,
            devices: false,
            filament_sensors: 0,
            probe: true,
            bed_mesh: true,
            bed_screws: true,
            tools: 0,
        }
    }

    pub fn voron() -> Self {
        Self {
            quad_gantry_level: true,
            chamber_heater: true,
            mmu_gates: None,
            mmu_units: None,
            afc: None,
            exclude_object: true,
            spoolman: true,
            devices: true,
            filament_sensors: 2,
            probe: true,
            bed_mesh: true,
            bed_screws: false,
            tools: 0,
        }
    }

    pub fn happy_hare(gates: usize) -> Self {
        Self {
            quad_gantry_level: true,
            chamber_heater: true,
            mmu_gates: Some(gates),
            mmu_units: None,
            afc: None,
            exclude_object: true,
            spoolman: true,
            devices: true,
            filament_sensors: 5,
            probe: true,
            bed_mesh: true,
            bed_screws: false,
            tools: 0,
        }
    }

    /// A voron with a tool changer of three tools and no MMU.
    pub fn toolchanger() -> Self {
        Self { tools: Self::TOOLCHANGER_TOOLS, ..Self::voron() }
    }

    /// Happy Hare v4 with two units (ERCF and Tradrack, four gates each) on the one extruder.
    pub fn happy_hare_units() -> Self {
        Self { mmu_units: Some(&MMU_UNITS_ONE_EXTRUDER), ..Self::happy_hare(8) }
    }

    /// A tool changer of two heads plus Happy Hare with one unit per head: the ERCF feeds `extruder`, the Tradrack
    /// `extruder1`.
    pub fn toolchanger_mmu() -> Self {
        Self { tools: 2, mmu_units: Some(&MMU_UNITS_ON_HEADS), ..Self::happy_hare(8) }
    }

    /// A voron with one Box Turtle of AFC.
    pub fn afc() -> Self {
        Self { afc: Some(AfcLayout::BoxTurtle), ..Self::voron() }
    }

    /// A tool changer of four heads with AFC, after the template for the Snapmaker U1.
    pub fn afc_toolchanger() -> Self {
        Self { afc: Some(AfcLayout::Toolchanger), ..Self::voron() }
    }

    /// Happy Hare without Moonraker's Spoolman: material and colour of the gates live in Happy Hare's gate map.
    pub fn ercf(gates: usize) -> Self {
        Self { spoolman: false, ..Self::happy_hare(gates) }
    }
}

impl Default for MachineProfile {
    fn default() -> Self {
        Self::voron()
    }
}
