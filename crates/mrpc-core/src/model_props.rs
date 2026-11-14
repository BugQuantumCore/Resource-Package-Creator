//! Model properties JSON (`modelPropertiesResource`) — `parts` + gangway/barrier.
//!
//! Mirrors `org.mtr.mod.resource.ModelProperties` / `ModelPropertiesPart`.

use serde::{Deserialize, Serialize};
use serde_json::Map;

/// Render condition of a part (`PartCondition` in MTR source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PartCondition {
    #[default]
    #[serde(rename = "NORMAL")]
    Normal,
    #[serde(rename = "AT_DEPOT")]
    AtDepot,
    #[serde(rename = "ON_ROUTE_FORWARDS")]
    OnRouteForwards,
    #[serde(rename = "ON_ROUTE_BACKWARDS")]
    OnRouteBackwards,
    #[serde(rename = "DOORS_CLOSED")]
    DoorsClosed,
    #[serde(rename = "DOORS_OPENED")]
    DoorsOpened,
    #[serde(rename = "CHRISTMAS_LIGHT_RED")]
    ChristmasLightRed,
    #[serde(rename = "CHRISTMAS_LIGHT_YELLOW")]
    ChristmasLightYellow,
    #[serde(rename = "CHRISTMAS_LIGHT_GREEN")]
    ChristmasLightGreen,
    #[serde(rename = "CHRISTMAS_LIGHT_BLUE")]
    ChristmasLightBlue,
    #[serde(other, rename = "UNKNOWN")]
    Unknown,
}

/// Shader stage a part renders in (`RenderStage` in MTR source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderStage {
    #[serde(rename = "LIGHT")]
    Light,
    #[serde(rename = "ALWAYS_ON_LIGHT")]
    AlwaysOnLight,
    #[serde(rename = "INTERIOR")]
    Interior,
    #[serde(rename = "INTERIOR_TRANSLUCENT")]
    InteriorTranslucent,
    #[serde(rename = "EXTERIOR")]
    Exterior,
    #[serde(other, rename = "UNKNOWN")]
    Unknown,
}

impl Default for RenderStage {
    fn default() -> Self {
        Self::Exterior
    }
}

/// Logical type of a part (`PartType` in MTR source). `DOORWAY` parts are doors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PartType {
    #[default]
    #[serde(rename = "NORMAL")]
    Normal,
    #[serde(rename = "DISPLAY")]
    Display,
    #[serde(rename = "FLOOR")]
    Floor,
    #[serde(rename = "DOORWAY")]
    Doorway,
    #[serde(rename = "SEAT")]
    Seat,
    #[serde(other, rename = "UNKNOWN")]
    Unknown,
}

/// Content shown on DISPLAY parts (`DisplayType` in MTR source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DisplayType {
    #[serde(rename = "DESTINATION")]
    #[default]
    Destination,
    #[serde(rename = "ROUTE_NUMBER")]
    RouteNumber,
    #[serde(rename = "DEPARTURE_INDEX")]
    DepartureIndex,
    #[serde(rename = "NEXT_STATION")]
    NextStation,
    #[serde(rename = "NEXT_STATION_KCR")]
    NextStationKcr,
    #[serde(rename = "NEXT_STATION_MTR")]
    NextStationMtr,
    #[serde(rename = "NEXT_STATION_UK")]
    NextStationUk,
    #[serde(rename = "ROUTE_COLOR")]
    RouteColor,
    #[serde(rename = "ROUTE_COLOR_ROUNDED")]
    RouteColorRounded,
    #[serde(other, rename = "UNKNOWN")]
    Unknown,
}

/// Display option flags (`displayOptions` in the part JSON; kept as strings so
/// unknown options from future MTR versions survive a round-trip).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayOption {
    SingleLine,
    UpperCase,
    SpaceCjk,
    ScrollNormal,
    ScrollLightRail,
    SevenSegment,
    AlignLeftCjk,
    AlignRightCjk,
    AlignLeft,
    AlignRight,
    AlignTop,
    AlignBottom,
    CycleLanguages,
}

impl DisplayOption {
    pub const ALL: [DisplayOption; 13] = [
        DisplayOption::SingleLine,
        DisplayOption::UpperCase,
        DisplayOption::SpaceCjk,
        DisplayOption::ScrollNormal,
        DisplayOption::ScrollLightRail,
        DisplayOption::SevenSegment,
        DisplayOption::AlignLeftCjk,
        DisplayOption::AlignRightCjk,
        DisplayOption::AlignLeft,
        DisplayOption::AlignRight,
        DisplayOption::AlignTop,
        DisplayOption::AlignBottom,
        DisplayOption::CycleLanguages,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            DisplayOption::SingleLine => "SINGLE_LINE",
            DisplayOption::UpperCase => "UPPER_CASE",
            DisplayOption::SpaceCjk => "SPACE_CJK",
            DisplayOption::ScrollNormal => "SCROLL_NORMAL",
            DisplayOption::ScrollLightRail => "SCROLL_LIGHT_RAIL",
            DisplayOption::SevenSegment => "SEVEN_SEGMENT",
            DisplayOption::AlignLeftCjk => "ALIGN_LEFT_CJK",
            DisplayOption::AlignRightCjk => "ALIGN_RIGHT_CJK",
            DisplayOption::AlignLeft => "ALIGN_LEFT",
            DisplayOption::AlignRight => "ALIGN_RIGHT",
            DisplayOption::AlignTop => "ALIGN_TOP",
            DisplayOption::AlignBottom => "ALIGN_BOTTOM",
            DisplayOption::CycleLanguages => "CYCLE_LANGUAGES",
        }
    }
}

/// One part of the model properties `parts` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPropertiesPart {
    /// OBJ / BBModel part names this entry applies to.
    #[serde(default)]
    pub names: Vec<String>,
    /// Names from the position definitions JSON; each placement × name becomes a draw.
    #[serde(rename = "positionDefinitions", default)]
    pub position_definitions: Vec<String>,
    #[serde(default)]
    pub condition: PartCondition,
    #[serde(default)]
    pub render_stage: RenderStage,
    #[serde(default)]
    #[serde(rename = "type")]
    pub part_type: PartType,
    #[serde(rename = "displayXPadding", default)]
    pub display_x_padding: f64,
    #[serde(rename = "displayYPadding", default)]
    pub display_y_padding: f64,
    /// Hex colour for CJK text, without `#`.
    #[serde(rename = "displayColorCjk", default)]
    pub display_color_cjk: String,
    /// Hex colour for Latin text, without `#`.
    #[serde(rename = "displayColor", default)]
    pub display_color: String,
    #[serde(rename = "displayMaxLineHeight", default)]
    pub display_max_line_height: f64,
    #[serde(rename = "displayCjkSizeRatio", default)]
    pub display_cjk_size_ratio: f64,
    #[serde(rename = "displayPadZeros", default)]
    pub display_pad_zeros: i64,
    #[serde(rename = "displayType", default)]
    pub display_type: DisplayType,
    #[serde(rename = "displayDefaultText", default)]
    pub display_default_text: String,
    /// Door slide amount along the car axis, in model units (1/16 block).
    #[serde(rename = "doorXMultiplier", default)]
    pub door_x_multiplier: f64,
    /// Door slide amount across the car axis, in model units (1/16 block).
    #[serde(rename = "doorZMultiplier", default)]
    pub door_z_multiplier: f64,
    #[serde(rename = "doorAnimationType", default)]
    pub door_animation_type: crate::door_anim::DoorAnimationType,
    /// Render window (ms from door-open start) — 0/0 means always visible.
    #[serde(rename = "renderFromOpeningDoorTime", default)]
    pub render_from_opening_door_time: i64,
    #[serde(rename = "renderUntilOpeningDoorTime", default)]
    pub render_until_opening_door_time: i64,
    #[serde(rename = "renderFromClosingDoorTime", default)]
    pub render_from_closing_door_time: i64,
    #[serde(rename = "renderUntilClosingDoorTime", default)]
    pub render_until_closing_door_time: i64,
    #[serde(rename = "flashOnTime", default)]
    pub flash_on_time: i64,
    #[serde(rename = "flashOffTime", default)]
    pub flash_off_time: i64,
    /// Display option flags as raw strings (see [`DisplayOption`]).
    #[serde(rename = "displayOptions", default)]
    pub display_options: Vec<String>,
    /// Unknown keys, preserved on round-trip.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, serde_json::Value>,
}

impl Default for ModelPropertiesPart {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            position_definitions: Vec::new(),
            condition: PartCondition::Normal,
            render_stage: RenderStage::Exterior,
            part_type: PartType::Normal,
            display_x_padding: 0.0,
            display_y_padding: 0.0,
            display_color_cjk: String::new(),
            display_color: String::new(),
            display_max_line_height: 0.0,
            display_cjk_size_ratio: 0.0,
            display_pad_zeros: 0,
            display_type: DisplayType::Destination,
            display_default_text: String::new(),
            door_x_multiplier: 0.0,
            door_z_multiplier: 0.0,
            door_animation_type: crate::door_anim::DoorAnimationType::Standard,
            render_from_opening_door_time: 0,
            render_until_opening_door_time: 0,
            render_from_closing_door_time: 0,
            render_until_closing_door_time: 0,
            flash_on_time: 0,
            flash_off_time: 0,
            display_options: Vec::new(),
            extra: Map::new(),
        }
    }
}

impl ModelPropertiesPart {
    /// Whether this part is animated with the doors.
    pub fn is_door(&self) -> bool {
        self.door_x_multiplier != 0.0 || self.door_z_multiplier != 0.0
    }

    /// Whether the part carries dynamic text (MTR DISPLAY type).
    pub fn is_display(&self) -> bool {
        self.part_type == PartType::Display
    }
}

/// Root of a `modelPropertiesResource` JSON file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelProperties {
    #[serde(default)]
    pub parts: Vec<ModelPropertiesPart>,
    #[serde(rename = "modelYOffset", default)]
    pub model_y_offset: f64,
    #[serde(rename = "gangwayInnerSideResource", default, skip_serializing_if = "String::is_empty")]
    pub gangway_inner_side_resource: String,
    #[serde(rename = "gangwayInnerTopResource", default, skip_serializing_if = "String::is_empty")]
    pub gangway_inner_top_resource: String,
    #[serde(rename = "gangwayInnerBottomResource", default, skip_serializing_if = "String::is_empty")]
    pub gangway_inner_bottom_resource: String,
    #[serde(rename = "gangwayOuterSideResource", default, skip_serializing_if = "String::is_empty")]
    pub gangway_outer_side_resource: String,
    #[serde(rename = "gangwayOuterTopResource", default, skip_serializing_if = "String::is_empty")]
    pub gangway_outer_top_resource: String,
    #[serde(rename = "gangwayOuterBottomResource", default, skip_serializing_if = "String::is_empty")]
    pub gangway_outer_bottom_resource: String,
    #[serde(rename = "gangwayWidth", default)]
    pub gangway_width: f64,
    #[serde(rename = "gangwayHeight", default)]
    pub gangway_height: f64,
    #[serde(rename = "gangwayYOffset", default)]
    pub gangway_y_offset: f64,
    #[serde(rename = "gangwayZOffset", default)]
    pub gangway_z_offset: f64,
    #[serde(rename = "barrierInnerSideResource", default, skip_serializing_if = "String::is_empty")]
    pub barrier_inner_side_resource: String,
    #[serde(rename = "barrierInnerTopResource", default, skip_serializing_if = "String::is_empty")]
    pub barrier_inner_top_resource: String,
    #[serde(rename = "barrierInnerBottomResource", default, skip_serializing_if = "String::is_empty")]
    pub barrier_inner_bottom_resource: String,
    #[serde(rename = "barrierOuterSideResource", default, skip_serializing_if = "String::is_empty")]
    pub barrier_outer_side_resource: String,
    #[serde(rename = "barrierOuterTopResource", default, skip_serializing_if = "String::is_empty")]
    pub barrier_outer_top_resource: String,
    #[serde(rename = "barrierOuterBottomResource", default, skip_serializing_if = "String::is_empty")]
    pub barrier_outer_bottom_resource: String,
    #[serde(rename = "barrierWidth", default)]
    pub barrier_width: f64,
    #[serde(rename = "barrierHeight", default)]
    pub barrier_height: f64,
    #[serde(rename = "barrierYOffset", default)]
    pub barrier_y_offset: f64,
    #[serde(rename = "barrierZOffset", default)]
    pub barrier_z_offset: f64,
    /// Unknown keys, preserved on round-trip.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, serde_json::Value>,
}

impl ModelProperties {
    /// If the file has no parts, synthesise one part from the model's part names,
    /// exactly like `ModelProperties.addPartsIfEmpty` in MTR.
    pub fn add_parts_if_empty(&mut self, part_names: &mut Vec<String>) {
        if self.parts.is_empty() && !part_names.is_empty() {
            self.parts.push(ModelPropertiesPart {
                names: std::mem::take(part_names),
                ..Default::default()
            });
        }
    }
}
