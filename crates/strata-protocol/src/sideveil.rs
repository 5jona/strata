//! SideVeil `hub.sock` plugin wire DTOs and typed `NativeHudScene` scene graph structures.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// SideVeil `hub.sock` plugin backend handshake role literal.
pub const SIDEVEIL_ROLE_PLUGIN_BACKEND: &str = "pluginBackend";
/// Outbound event pushing a `NativeHudScene` payload to SideVeil.
pub const SIDEVEIL_HUD_UPDATE_SCENE: &str = "hud.updateScene";
/// Outbound event dismissing the active Native HUD scene.
pub const SIDEVEIL_HUD_DISMISS: &str = "hud.dismiss";
/// Inbound Hub notification reporting dark/light appearance changes.
pub const HUB_EVENT_APPEARANCE_CHANGED: &str = "appearance.changed";
/// Inbound Hub notification reporting SideVeil panel visibility changes.
pub const HUB_EVENT_PANEL_VISIBILITY_CHANGED: &str = "panel.visibilityChanged";
/// Inbound Hub notification reporting Android network transport changes.
pub const HUB_EVENT_NETWORK_CONTEXT_CHANGED: &str = "network.contextChanged";

/// Initial NDJSON handshake frame sent by a `uds-native` plugin backend to SideVeil `hub.sock`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideVeilPluginHello<'a> {
    pub role: &'a str,
    pub plugin_id: &'a str,
    pub package_name: Option<&'a str>,
}

impl<'a> SideVeilPluginHello<'a> {
    /// Builds a standard `pluginBackend` hello frame for the given plugin identifier.
    #[must_use]
    pub const fn plugin_backend(plugin_id: &'a str) -> Self {
        Self {
            role: SIDEVEIL_ROLE_PLUGIN_BACKEND,
            plugin_id,
            package_name: None,
        }
    }
}

/// Handshake acknowledgement frame returned by SideVeil Hub after `SideVeilPluginHello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SideVeilHandshakeAck {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Outbound NDJSON event frame emitted by a plugin backend over SideVeil `hub.sock`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideVeilBackendEvent<'a> {
    pub event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<&'a Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ephemeral: bool,
}

/// Native HUD surface backdrop style supported by SideVeil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HudBackdrop {
    NativeGlass,
}

/// Text alignment mode for SideVeil Native HUD text nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HudTextAlign {
    Start,
    Center,
    End,
}

/// Geometric drawing primitive rendered on a SideVeil Native HUD scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HudPrimitive {
    #[serde(rename_all = "camelCase")]
    RoundedPill {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        fill_argb: u32,
    },
    #[serde(rename_all = "camelCase")]
    ProgressBar {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        value: f32,
        track_argb: u32,
        fill_start_argb: u32,
        fill_end_argb: u32,
    },
}

/// Positioned text node rendered on a SideVeil Native HUD scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HudText {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub size_sp: f32,
    pub color_argb: u32,
    pub align: HudTextAlign,
    pub bold: bool,
    pub tabular_nums: bool,
    pub max_width_dp: f32,
}

/// Typed SideVeil `NativeHudScene` wire payload pushed via `hud.updateScene`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHudScene {
    pub plugin_id: String,
    pub width_dp: f32,
    pub height_dp: f32,
    pub corner_radius_dp: f32,
    pub backdrop: HudBackdrop,
    pub click_through: bool,
    pub anchor_x_percent: f32,
    pub anchor_y_percent: f32,
    pub primitives: Vec<HudPrimitive>,
    pub texts: Vec<HudText>,
}

impl NativeHudScene {
    /// Creates an empty `NativeHudScene` with `native-glass` backdrop and anchor defaults.
    #[must_use]
    pub fn new(
        plugin_id: impl Into<String>,
        width_dp: f32,
        height_dp: f32,
        corner_radius_dp: f32,
        click_through: bool,
    ) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            width_dp,
            height_dp,
            corner_radius_dp,
            backdrop: HudBackdrop::NativeGlass,
            click_through,
            anchor_x_percent: 4.0,
            anchor_y_percent: 12.0,
            primitives: Vec::new(),
            texts: Vec::new(),
        }
    }

    /// Sets the scene's viewport anchor coordinates in percentage units.
    #[must_use]
    pub fn with_anchor(mut self, x_percent: f32, y_percent: f32) -> Self {
        self.anchor_x_percent = x_percent;
        self.anchor_y_percent = y_percent;
        self
    }

    /// Appends a drawing primitive to the scene.
    #[must_use]
    pub fn with_primitive(mut self, primitive: HudPrimitive) -> Self {
        self.primitives.push(primitive);
        self
    }

    /// Appends a positioned text node to the scene.
    #[must_use]
    pub fn with_text(mut self, text: HudText) -> Self {
        self.texts.push(text);
        self
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn serializes_sideveil_handshake_and_backend_event_wire_frames() {
        let hello = SideVeilPluginHello::plugin_backend("strata.manager");
        assert_eq!(
            serde_json::to_value(hello).expect("serialize hello"),
            json!({
                "role": "pluginBackend",
                "pluginId": "strata.manager",
                "packageName": null
            })
        );

        let ack: SideVeilHandshakeAck =
            serde_json::from_str(r#"{"ok":true}"#).expect("parse ok ack");
        assert_eq!(
            ack,
            SideVeilHandshakeAck {
                ok: true,
                error: None
            }
        );

        let payload = json!({ "taskId": "task-1" });
        let reliable = SideVeilBackendEvent {
            event: "task.exit",
            payload: Some(&payload),
            ephemeral: false,
        };
        assert_eq!(
            serde_json::to_value(reliable).expect("serialize reliable"),
            json!({ "event": "task.exit", "payload": { "taskId": "task-1" } })
        );

        let ephemeral = SideVeilBackendEvent {
            event: "task.output",
            payload: None,
            ephemeral: true,
        };
        assert_eq!(
            serde_json::to_value(ephemeral).expect("serialize ephemeral"),
            json!({ "event": "task.output", "ephemeral": true })
        );
    }

    #[test]
    fn serializes_native_hud_scene_identically_to_sideveil_wire_schema() {
        let ratio = 0.25_f32;
        let scene = NativeHudScene::new("strata.manager", 212.0, 72.0, 16.0, true)
            .with_anchor(4.0, 12.0)
            .with_primitive(HudPrimitive::RoundedPill {
                x: 12.0,
                y: 14.0,
                w: 8.0,
                h: 8.0,
                radius: 4.0,
                fill_argb: 0xFF34_D399,
            })
            .with_primitive(HudPrimitive::ProgressBar {
                x: 12.0,
                y: 54.0,
                w: 188.0,
                h: 6.0,
                radius: 3.0,
                value: ratio,
                track_argb: 0x33FF_FFFF,
                fill_start_argb: 0xFF34_D399,
                fill_end_argb: 0xFF38_BDF8,
            })
            .with_text(HudText {
                x: 26.0,
                y: 22.0,
                text: "Strata  1/1 运行".to_owned(),
                size_sp: 12.0,
                color_argb: 0xFFF1_F5F9,
                align: HudTextAlign::Start,
                bold: true,
                tabular_nums: true,
                max_width_dp: 176.0,
            });

        let expected = json!({
            "pluginId": "strata.manager", "widthDp": 212.0, "heightDp": 72.0,
            "cornerRadiusDp": 16.0, "backdrop": "native-glass", "clickThrough": true,
            "anchorXPercent": 4.0, "anchorYPercent": 12.0,
            "primitives": [
                {
                    "kind": "roundedPill", "x": 12.0, "y": 14.0, "w": 8.0, "h": 8.0,
                    "radius": 4.0, "fillArgb": 0xFF34_D399_u32
                },
                {
                    "kind": "progressBar", "x": 12.0, "y": 54.0, "w": 188.0, "h": 6.0,
                    "radius": 3.0, "value": ratio, "trackArgb": 0x33FF_FFFF_u32,
                    "fillStartArgb": 0xFF34_D399_u32, "fillEndArgb": 0xFF38_BDF8_u32
                }
            ],
            "texts": [{
                "x": 26.0, "y": 22.0, "text": "Strata  1/1 运行", "sizeSp": 12.0,
                "colorArgb": 0xFFF1_F5F9_u32, "align": "start", "bold": true,
                "tabularNums": true, "maxWidthDp": 176.0
            }]
        });
        let encoded = serde_json::to_value(&scene).expect("serialize scene");
        assert_eq!(encoded, expected);
        let decoded: NativeHudScene = serde_json::from_value(encoded).expect("roundtrip scene");
        assert_eq!(decoded, scene);
    }
}
