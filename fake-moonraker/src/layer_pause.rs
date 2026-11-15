//! Mainsail's layer pause macros (mainsail-config `client.cfg`): `SET_PAUSE_NEXT_LAYER`, `SET_PAUSE_AT_LAYER` and
//! the override of `SET_PRINT_STATS_INFO` that pauses on a layer change. The fake has to know what the macros do;
//! FreakyMate does not and only calls them.

use serde_json::{Value, json};

/// `SET_PAUSE_NEXT_LAYER [ENABLE=0|1]` and `SET_PAUSE_AT_LAYER [ENABLE=0|1] [LAYER=<number>]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerPauseCommand {
    NextLayer {
        enable: bool,
    },
    /// Without `ENABLE`, Mainsail enables the pause exactly when a layer is given.
    AtLayer {
        enable: Option<bool>,
        layer: Option<u32>,
    },
}

/// The variables `pause_next_layer` and `pause_at_layer` of `gcode_macro SET_PRINT_STATS_INFO`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayerPause {
    next_layer: bool,
    at_layer_enabled: bool,
    at_layer: u32,
}

impl LayerPause {
    pub fn apply(&mut self, command: LayerPauseCommand) {
        match command {
            LayerPauseCommand::NextLayer { enable } => self.next_layer = enable,
            LayerPauseCommand::AtLayer { enable, layer } => {
                self.at_layer_enabled = enable.unwrap_or(layer.is_some());
                self.at_layer = layer.unwrap_or(self.at_layer);
            }
        }
    }

    /// A new layer began; true if the print pauses now. A plan that fired is switched off, as in Mainsail.
    pub fn on_layer_change(&mut self, layer: u32) -> bool {
        if self.next_layer {
            self.next_layer = false;
            true
        } else if self.at_layer_enabled && self.at_layer == layer {
            self.at_layer_enabled = false;
            true
        } else {
            false
        }
    }

    /// Mainsail's `CANCEL_PRINT` switches both plans off; the fake does that at the end of every job.
    pub fn clear(&mut self) {
        self.next_layer = false;
        self.at_layer_enabled = false;
        self.at_layer = 0;
    }

    /// Status of `gcode_macro SET_PRINT_STATS_INFO` as on a printer with mainsail-config.
    pub fn macro_status(&self) -> Value {
        json!({
            "pause_next_layer": { "enable": self.next_layer, "call": "PAUSE" },
            "pause_at_layer": { "enable": self.at_layer_enabled, "layer": self.at_layer, "call": "PAUSE" },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_layer_pauses_once() {
        let mut pause = LayerPause::default();
        pause.apply(LayerPauseCommand::NextLayer { enable: true });

        assert!(pause.on_layer_change(5));
        assert!(!pause.on_layer_change(6));
    }

    #[test]
    fn at_layer_pauses_at_that_layer_only() {
        let mut pause = LayerPause::default();
        pause.apply(LayerPauseCommand::AtLayer { enable: None, layer: Some(7) });

        assert!(!pause.on_layer_change(6));
        assert!(pause.on_layer_change(7));
        assert_eq!(pause.macro_status()["pause_at_layer"]["enable"], false);
    }

    #[test]
    fn enable_0_takes_a_plan_back() {
        let mut pause = LayerPause::default();
        pause.apply(LayerPauseCommand::AtLayer { enable: None, layer: Some(7) });
        pause.apply(LayerPauseCommand::AtLayer { enable: Some(false), layer: None });

        assert!(!pause.on_layer_change(7));
        assert_eq!(pause.macro_status()["pause_at_layer"]["layer"], 7);
    }
}
