//! AFC commands as the printer applies them: the plugin's state lives in [`Afc`], the printer supplies what AFC asks
//! Klipper (homed axes, print state) and follows the head on the shuttle of a tool changer.

use super::*;

impl Printer {
    pub(super) fn apply_afc(&mut self, command: AfcCommand) -> Result<(), CommandError> {
        let (homed, printing) = (self.all_axes_homed(), self.print_state == PrintState::Printing);
        // Without AFC these commands are unknown to Klipper and the fake ignores them.
        let Some(afc) = self.afc.as_mut() else { return Ok(()) };
        let result = afc.apply(command, homed, printing);
        self.follow_afc_shuttle();
        result
    }

    /// `T<n>` of AFC; `None` if the printer has no AFC.
    pub(super) fn afc_change_tool(&mut self, tool: usize) -> Option<Result<(), CommandError>> {
        let homed = self.all_axes_homed();
        let result = self.afc.as_mut()?.change_tool_by_map(tool, homed);
        self.follow_afc_shuttle();
        Some(result)
    }

    /// Klipper's `toolhead.extruder` is the extruder of the head AFC has on the shuttle.
    pub(super) fn follow_afc_shuttle(&mut self) {
        let (Some(afc), Some(toolchanger)) = (self.afc.as_ref(), self.toolchanger.as_mut()) else { return };
        if let Some(extruder) = afc.shuttle_extruder() {
            toolchanger.activate_extruder(extruder);
        }
    }

    fn all_axes_homed(&self) -> bool {
        self.homed.iter().all(|homed| *homed)
    }

    /// AFC's objects.
    pub(super) fn add_afc_status(&self, objects: &mut Map<String, Value>) {
        if let Some(afc) = &self.afc {
            objects.extend(afc.status());
        }
    }

    pub(super) fn toolchanger_has_tool(&self, tool: usize) -> bool {
        self.toolchanger.as_ref().is_some_and(|toolchanger| toolchanger.has_tool(tool))
    }
}
