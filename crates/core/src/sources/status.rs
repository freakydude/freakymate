//! The whole printer state as the subscription reports it: Klipper's own objects and those of its plugins.

use super::afc::{self, AfcStatus};
use super::happy_hare::{self, MmuStatus};
use super::klipper::{
    Activity, CalibrationState, ExtruderState, Fan, FilamentSensor, Heater, KlipperParts, KlippyState, Led, Motion,
    PrintJob, PrintObject, PrintState, Retraction, TemperatureSensor,
};
use super::led_effect::{self, LedEffect};
use super::mainsail_config::{self, PlannedLayerPause};
use super::objects::ObjectStore;
use super::tool_macros::{self, ToolSpool};
use super::toolchanger::{self, ToolchangerStatus};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PrinterStatus {
    pub klippy_state: KlippyState,
    pub klippy_message: String,
    pub activity: Activity,
    pub job: PrintJob,
    pub motion: Motion,
    pub extruder: ExtruderState,
    /// Extruder first, then bed, then additional heaters in object order.
    pub heaters: Vec<Heater>,
    pub sensors: Vec<TemperatureSensor>,
    /// Part fan first, then the other fans; see [`Fan`].
    pub fans: Vec<Fan>,
    pub leds: Vec<Led>,
    /// Effects of the optional led_effect plugin.
    pub led_effects: Vec<LedEffect>,
    /// `None` without `[firmware_retraction]`.
    pub retraction: Option<Retraction>,
    pub save_config_pending: bool,
    /// `Some(applied)` if quad gantry level or Z tilt is present.
    pub leveling_applied: Option<bool>,
    /// Bed mesh, screw results, interactive modes and what `SAVE_CONFIG` would write.
    pub calibration: CalibrationState,
    pub filament_sensors: Vec<FilamentSensor>,
    /// Objects of the print with their exclude state; empty without the `exclude_object` module.
    pub print_objects: Vec<PrintObject>,
    /// Happy Hare; `None` without the `mmu` object.
    pub mmu: Option<MmuStatus>,
    /// AFC; `None` without the `AFC` object.
    pub afc: Option<AfcStatus>,
    /// The pause planned with Mainsail's layer macros; `None` if the plan is not known.
    pub layer_pause_plan: Option<PlannedLayerPause>,
    /// The tool changer with its heads; `None` without one.
    pub toolchanger: Option<ToolchangerStatus>,
    /// The spools the tool macros `T<n>` hold, for those that declare `spool_id`.
    pub tool_spools: Vec<ToolSpool>,
}

impl PrinterStatus {
    pub fn from_store(store: &ObjectStore) -> Self {
        let klipper = KlipperParts::read(store);
        Self {
            klippy_state: klipper.klippy_state,
            klippy_message: klipper.klippy_message,
            activity: klipper.activity,
            job: klipper.job,
            motion: klipper.motion,
            extruder: klipper.extruder,
            heaters: klipper.heaters,
            sensors: klipper.sensors,
            fans: klipper.fans,
            leds: klipper.leds,
            led_effects: led_effect::read_effects(store),
            retraction: klipper.retraction,
            save_config_pending: klipper.save_config_pending,
            leveling_applied: klipper.leveling_applied,
            calibration: klipper.calibration,
            filament_sensors: klipper.filament_sensors,
            print_objects: klipper.print_objects,
            mmu: happy_hare::read_mmu(store),
            afc: afc::read_afc(store),
            layer_pause_plan: mainsail_config::read_plan(store),
            toolchanger: toolchanger::read_toolchanger(store),
            tool_spools: tool_macros::read_spools(store),
        }
    }

    /// A print is under way. After a shutdown `print_stats` keeps "printing" or "paused" (Klipper's virtual_sdcard
    /// notes a pause), but the print is lost and restarts must stay possible.
    pub fn print_running(&self) -> bool {
        self.klippy_state == KlippyState::Ready && self.job.state.is_active()
    }

    /// The print moves the machine right now; unlike a pause, there is no room for moves of one's own.
    pub fn print_moving(&self) -> bool {
        self.klippy_state == KlippyState::Ready && self.job.state == PrintState::Printing
    }

    pub fn heater(&self, object: &str) -> Option<&Heater> {
        self.heaters.iter().find(|heater| heater.object == object)
    }

    /// Happy Hare while it has the say, see [`MmuStatus::is_in_charge`]; every rule that depends on Happy Hare asks
    /// here.
    pub fn happy_hare(&self) -> Option<&MmuStatus> {
        self.mmu.as_ref().filter(|mmu| mmu.is_in_charge())
    }
}
