//! Procedurally generated demonstration resource pack.
//!
//! Everything (OBJ model, MTL, texture PNG, properties/positions JSON, BVE
//! motor data, vehicle script) is generated in code — no external assets —
//! and written into a [`ResourcePack`] that can be previewed, edited and
//! exported as-is. It doubles as an executable format reference.

use crate::format::CustomResources;
use crate::model_props::{
    DisplayType, ModelProperties, ModelPropertiesPart, PartType, RenderStage,
};
use crate::pack::ResourcePack;
use crate::positions::{PartPosition, PositionDefinition, PositionDefinitions};
use crate::script::VehicleScriptEntry;
use crate::vehicle::{VehicleModel, VehicleResource};

/// Namespace prefix for all demo assets.
pub const DEMO_NS: &str = "mtr:custom/demo";

/// Build the demo pack in memory.
pub fn create_demo_pack() -> ResourcePack {
    let mut pack = ResourcePack::new("MRPC demo pack — procedurally generated example vehicle");

    // ------------------------------------------------------------------ model
    pack.set_resource(&format!("{DEMO_NS}/car.obj"), "obj", demo_car_obj().into_bytes());
    pack.set_resource(&format!("{DEMO_NS}/car.mtl"), "mtl", DEMO_CAR_MTL.as_bytes().to_vec());
    pack.set_resource(&format!("{DEMO_NS}/car.png"), "png", demo_car_texture());

    // ------------------------------------------------------------ properties
    let properties = ModelProperties {
        parts: vec![
            // Static shell at the origin — without a part entry the OBJ's
            // body/roof/window_band are simply never rendered (MTR parity:
            // only referenced parts draw), which made the demo car appear
            // as doors and a display floating in mid-air.
            ModelPropertiesPart {
                names: vec!["body".into(), "roof".into(), "window_band".into()],
                // No positionDefinitions → one placement at the origin.
                render_stage: RenderStage::Exterior,
                ..Default::default()
            },
            ModelPropertiesPart {
                names: vec!["door".into()],
                position_definitions: vec!["doorLeft".into(), "doorRight".into()],
                render_stage: RenderStage::Exterior,
                door_z_multiplier: 30.0,
                door_animation_type: crate::door_anim::DoorAnimationType::Standard,
                ..Default::default()
            },
            ModelPropertiesPart {
                names: vec!["door_interior".into()],
                position_definitions: vec!["doorLeft".into(), "doorRight".into()],
                render_stage: RenderStage::Interior,
                door_z_multiplier: 30.0,
                door_animation_type: crate::door_anim::DoorAnimationType::Standard,
                ..Default::default()
            },
            ModelPropertiesPart {
                names: vec!["display".into()],
                position_definitions: vec!["frontDisplay".into()],
                part_type: PartType::Display,
                display_type: DisplayType::Destination,
                display_default_text: "Demo Line|示範綫".into(),
                render_stage: RenderStage::Interior,
                ..Default::default()
            },
        ],
        model_y_offset: 0.5,
        gangway_width: 1.6,
        gangway_height: 2.4,
        gangway_y_offset: 1.4,
        ..Default::default()
    };
    pack.set_resource(
        &format!("{DEMO_NS}/car_properties.json"),
        "json",
        serde_json::to_vec_pretty(&properties).unwrap(),
    );

    // -------------------------------------------------------------- positions
    // Car axis is Z (model units, 1/16 block); doors sit every 64 units.
    let door_zs = [-128.0, -64.0, 0.0, 64.0, 128.0];
    let positions = PositionDefinitions {
        position_definitions: vec![
            PositionDefinition {
                name: "doorLeft".into(),
                positions: door_zs
                    .iter()
                    .map(|&z| PartPosition { x: -20.0, z, ..Default::default() })
                    .collect(),
                ..Default::default()
            },
            PositionDefinition {
                name: "doorRight".into(),
                positions: door_zs
                    .iter()
                    .map(|&z| PartPosition { x: 20.0, z, ..Default::default() })
                    .collect(),
                ..Default::default()
            },
            PositionDefinition {
                name: "frontDisplay".into(),
                positions: vec![PartPosition { y: 26.0, z: 152.0, ..Default::default() }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    pack.set_resource(
        &format!("{DEMO_NS}/car_positions.json"),
        "json",
        serde_json::to_vec_pretty(&positions).unwrap(),
    );

    // ------------------------------------------------------------ BVE sounds
    // assets/mtr/sounds/custom/demo/sound/{sound.cfg,powervol.csv,...}
    let sound_base = "mtr:custom/demo/sound";
    pack.set_resource(&format!("mtr:sounds/custom/demo/sound/sound.cfg"), "cfg", DEMO_SOUND_CFG.as_bytes().to_vec());
    pack.set_resource(&format!("mtr:sounds/custom/demo/sound/powervol.csv"), "csv", DEMO_POWERVOL.as_bytes().to_vec());
    pack.set_resource(&format!("mtr:sounds/custom/demo/sound/powerfreq.csv"), "csv", DEMO_POWERFREQ.as_bytes().to_vec());
    pack.set_resource(&format!("mtr:sounds/custom/demo/sound/brakevol.csv"), "csv", DEMO_BRAKEVOL.as_bytes().to_vec());
    pack.set_resource(&format!("mtr:sounds/custom/demo/sound/brakefreq.csv"), "csv", DEMO_BRAKEFREQ.as_bytes().to_vec());
    let _ = sound_base;

    // ----------------------------------------------------------- JS  script
    pack.set_resource(
        &format!("{DEMO_NS}/js/main.js"),
        "js",
        script_demo_source().as_bytes().to_vec(),
    );

    // -------------------------------------------------- mtr_custom_resources
    let vehicle = VehicleResource {
        id: "demo_metro_car".into(),
        name: "Demo Metro Car".into(),
        color: "FEC5E5".into(),
        description: "Procedurally generated demonstration car for the MTR Resource Package Creator.".into(),
        wikipedia_article: "https://github.com/BugQuantumCore/Resource-Package-Creator".into(),
        tags: vec!["demo".into()],
        length: 20.0,
        width: 2.5,
        bogie1_position: -6.0,
        bogie2_position: 6.0,
        has_gangway1: true,
        has_gangway2: true,
        has_barrier1: true,
        has_barrier2: true,
        bve_sound_base_resource: format!("{DEMO_NS}/sound"),
        script_id: Some("demo_lcd".into()),
        models: vec![VehicleModel {
            model_resource: format!("{DEMO_NS}/car.obj"),
            texture_resource: format!("{DEMO_NS}/car.png"),
            model_properties_resource: format!("{DEMO_NS}/car_properties.json"),
            position_definitions_resource: format!("{DEMO_NS}/car_positions.json"),
            flip_texture_v: false,
            ..Default::default()
        }],
        ..Default::default()
    };
    let script_entry = VehicleScriptEntry {
        id: "demo_lcd".into(),
        script_locations: vec![serde_json::json!(format!("{DEMO_NS}/js/main.js"))],
        input: Some(serde_json::json!({ "routeName": "Demo Line", "yearEra": 2026 })),
        data_fetch_mode: Some(crate::script::DataFetchMode::All),
        ..Default::default()
    };
    pack.resources = CustomResources {
        vehicles: vec![vehicle],
        vehicle_scripts: vec![script_entry],
        ..Default::default()
    };
    pack
}

/// The demo vehicle script (a friendlier variant of the JS template).
pub fn script_demo_source() -> String {
    r#"// Demo vehicle script for Joban Client Mod (MTR Scripting).
// SCRIPT_INPUT comes from the vehicleScripts entry's "input" JSON.
const ROUTE = SCRIPT_INPUT && SCRIPT_INPUT.routeName ? SCRIPT_INPUT.routeName : "Demo Line";

function create(ctx, state, vehicle) {
    state.frames = 0;
    state.lastDoor = 0;
    state.displays = 0;
    print("[" + ROUTE + "] created; cars=" + vehicle.getCarCount()
        + " doorSide=" + vehicle.getDirectionOfDoorOpening());
}

function render(ctx, state, vehicle) {
    state.frames++;
    const door = vehicle.getDoorValue();
    if (door > 0 && door !== state.lastDoor) {
        print("doors " + (vehicle.isDoorOpening() ? "opening" : "at " + Math.round(door * 100) + "%"));
    }
    state.lastDoor = door;
    if (state.frames % 90 === 0) {
        print("speed=" + vehicle.getSpeedKmh().toFixed(1) + " km/h notch=" + vehicle.getNotchLevel()
            + " next=" + (vehicle.getThisRouteStops()[0] ? vehicle.getThisRouteStops()[0].getName() : "?"));
    }
}

function dispose(ctx, state, vehicle) {
    print("disposed after " + state.frames + " frames");
}
"#
    .to_string()
}

// ---------------------------------------------------------------------------
// OBJ generation
// ---------------------------------------------------------------------------

struct MeshBuilder {
    out: String,
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    normals: Vec<[f32; 3]>,
    current_part: Option<String>,
}

impl MeshBuilder {
    fn new() -> Self {
        Self {
            out: String::from("# Demo metro car generated by mrpc-core\nmtllib car.mtl\n"),
            positions: Vec::new(),
            uvs: Vec::new(),
            normals: Vec::new(),
            current_part: None,
        }
    }

    fn part(&mut self, name: &str) {
        if self.current_part.as_deref() != Some(name) {
            self.out.push_str(&format!("o {name}\n"));
            self.current_part = Some(name.to_string());
        }
    }

    fn usemtl(&mut self, name: &str) {
        self.out.push_str(&format!("usemtl {name}\n"));
    }

    /// Add an axis-aligned quad between corners `a` and `b` on plane `axis`
    /// facing `dir`, with UV region `uv` (x1, y1, x2, y2 in 0..1).
    fn quad(&mut self, axis: u8, dir: i8, a: [f32; 3], b: [f32; 3], uv: [f32; 4]) {
        let (x1, y1, z1) = (a[0], a[1], a[2]);
        let (x2, y2, z2) = (b[0], b[1], b[2]);
        let normal = match (axis, dir) {
            (0, 1) => [1.0, 0.0, 0.0],
            (0, _) => [-1.0, 0.0, 0.0],
            (1, 1) => [0.0, 1.0, 0.0],
            (1, _) => [0.0, -1.0, 0.0],
            (2, 1) => [0.0, 0.0, 1.0],
            _ => [0.0, 0.0, -1.0],
        };
        let corners: Vec<([f32; 3], [f32; 2])> = match axis {
            0 => vec![
                ([x1, y1, z1], [uv[0], uv[1]]),
                ([x1, y2, z1], [uv[0], uv[3]]),
                ([x1, y2, z2], [uv[2], uv[3]]),
                ([x1, y1, z2], [uv[2], uv[1]]),
            ],
            1 => vec![
                ([x1, y1, z1], [uv[0], uv[1]]),
                ([x2, y1, z1], [uv[2], uv[1]]),
                ([x2, y1, z2], [uv[2], uv[3]]),
                ([x1, y1, z2], [uv[0], uv[3]]),
            ],
            _ => vec![
                ([x1, y1, z1], [uv[0], uv[1]]),
                ([x2, y1, z1], [uv[2], uv[1]]),
                ([x2, y2, z1], [uv[2], uv[3]]),
                ([x1, y2, z1], [uv[0], uv[3]]),
            ],
        };
        self.emit_face(&corners, normal);
        // Winding: emit the same quad reversed on the back side so the mesh is
        // double-sided, like MTR's cutout rendering of thin panels.
        let mut flipped = corners.clone();
        flipped.reverse();
        let neg = [-normal[0], -normal[1], -normal[2]];
        self.emit_face(&flipped, neg);
    }

    fn emit_face(&mut self, corners: &[([f32; 3], [f32; 2])], normal: [f32; 3]) {
        if corners.len() != 4 {
            return;
        }
        let mut indices = String::new();
        for (p, t) in corners {
            self.positions.push(*p);
            self.uvs.push(*t);
            self.normals.push(normal);
            let vi = self.positions.len();
            indices.push_str(&format!("{vi}/{vi}/{vi} "));
        }
        self.out.push_str(&format!("f {}\n", indices.trim_end()));
    }

    /// A closed box (six quads) from min to max with a UV window.
    fn add_box(&mut self, min: [f32; 3], max: [f32; 3], uv: [f32; 4]) {
        self.quad(2, 1, [min[0], min[1], max[2]], [max[0], max[1], max[2]], uv); // +z
        self.quad(2, -1, [min[0], min[1], min[2]], [max[0], max[1], min[2]], uv); // -z
        self.quad(0, 1, [max[0], min[1], min[2]], [max[0], max[1], max[2]], uv); // +x
        self.quad(0, -1, [min[0], min[1], min[2]], [min[0], max[1], max[2]], uv); // -x
        self.quad(1, 1, [min[0], max[1], min[2]], [max[0], max[1], max[2]], uv); // +y
        self.quad(1, -1, [min[0], min[1], min[2]], [max[0], min[1], max[2]], uv); // -y
    }

    fn finish(mut self) -> String {
        // Prepend v/vt/vn declarations (parts reference them by absolute index).
        // Vertices are authored in PIXELS (1/16 block, matching the position
        // definitions) but MTR OBJ resources must be in BLOCKS ("each
        // unit's length is 1m") — emit ÷16 so the generated pack is valid
        // in-game. The app re-scales ×16 on load, so the preview is unchanged.
        let mut header = String::new();
        for p in &self.positions {
            header.push_str(&format!("v {} {} {}\n", p[0] / 16.0, p[1] / 16.0, p[2] / 16.0));
        }
        for t in &self.uvs {
            header.push_str(&format!("vt {} {}\n", t[0], t[1]));
        }
        for n in &self.normals {
            header.push_str(&format!("vn {} {} {}\n", n[0], n[1], n[2]));
        }
        // Insert after the newline preceding the first `o` statement.
        let idx = self.out.find("\no ").map(|i| i + 1).unwrap_or(self.out.len());
        self.out.insert_str(idx, &header);
        self.out
    }
}

/// The demo car OBJ. Coordinates are authored in PIXELS (1 unit = 1/16
/// block, like the position definitions); the writer converts them to
/// MTR-standard BLOCK units (÷16). X across the car (±20 px = width 2.5
/// blocks), Y up (0 = rail top), Z along the car (±160 px = length 20
/// blocks). Parts: body, roof, window_band, door ×2 sides, door_interior,
/// display, end caps.
fn demo_car_obj() -> String {
    let mut m = MeshBuilder::new();

    // Body shell: a tube with open door gaps simulated by separate panels.
    m.part("body");
    m.usemtl("body_mat");
    // Floor.
    m.add_box([-20.0, 4.0, -160.0], [20.0, 6.0, 160.0], [0.0, 0.0, 0.25, 0.06]);
    // Side walls between doors: 6 segments per side.
    let wall_spans: [(f32, f32); 6] = [(-160.0, -142.0), (-114.0, -50.0), (-22.0, 22.0), (50.0, 114.0), (142.0, 160.0), (-2.0, 2.0)];
    let wall_uv = [0.0, 0.08, 0.5, 0.4];
    for &(z0, z1) in &wall_spans {
        m.quad(0, 1, [20.0, 6.0, z0], [20.0, 44.0, z1], wall_uv);
        m.quad(0, -1, [-20.0, 6.0, z0], [-20.0, 44.0, z1], wall_uv);
    }
    // End caps (cab fronts).
    m.usemtl("front_mat");
    m.add_box([-20.0, 4.0, -160.0], [20.0, 50.0, -156.0], [0.5, 0.08, 1.0, 0.5]);
    m.add_box([-20.0, 4.0, 156.0], [20.0, 50.0, 160.0], [0.5, 0.08, 1.0, 0.5]);

    // Roof.
    m.part("roof");
    m.usemtl("roof_mat");
    m.add_box([-20.0, 44.0, -158.0], [20.0, 50.0, 158.0], [0.0, 0.5, 1.0, 0.56]);

    // Window band above the walls.
    m.part("window_band");
    m.usemtl("window_mat");
    let window_spans: [(f32, f32); 4] = [(-140.0, -116.0), (-52.0, -24.0), (24.0, 52.0), (116.0, 140.0)];
    let win_uv = [0.0, 0.62, 0.5, 0.75];
    for &(z0, z1) in &window_spans {
        m.quad(0, 1, [20.0, 24.0, z0], [20.0, 42.0, z1], win_uv);
        m.quad(0, -1, [-20.0, 24.0, z0], [-20.0, 42.0, z1], win_uv);
    }
    // Cab windscreens.
    m.quad(2, -1, [-14.0, 28.0, -160.0], [14.0, 42.0, -159.0], win_uv);
    m.quad(2, 1, [-14.0, 28.0, 159.0], [14.0, 42.0, 160.0], win_uv);

    // Door leaf (one geometry, replicated by position definitions on both sides).
    m.part("door");
    m.usemtl("door_mat");
    // Centered at local origin so position definitions place it per door bay.
    m.add_box([-16.0, 6.0, -15.0], [20.0, 44.0, 15.0], [0.55, 0.08, 0.95, 0.5]);
    m.part("door_interior");
    m.usemtl("door_mat");
    m.add_box([-16.0, 6.0, -15.0], [19.0, 44.0, 15.0], [0.55, 0.08, 0.95, 0.5]);

    // Destination display at the cab front.
    m.part("display");
    m.usemtl("display_mat");
    m.add_box([-12.0, 30.0, 150.0], [12.0, 38.0, 155.0], [0.0, 0.8, 1.0, 0.95]);

    m.finish()
}

const DEMO_CAR_MTL: &str = "\
# Demo metro car materials
newmtl body_mat
Kd 0.85 0.85 0.88
Ka 0.10 0.10 0.10
Ks 0.25 0.25 0.30
Ns 48
map_Kd default.png

newmtl front_mat
Kd 0.90 0.90 0.92
map_Kd default.png

newmtl roof_mat
Kd 0.55 0.55 0.58
map_Kd default.png

newmtl window_mat
Kd 0.25 0.35 0.45
d 0.65
map_Kd default.png

newmtl door_mat
Kd 0.88 0.88 0.90
map_Kd default.png

newmtl display_mat
Kd 1.00 0.70 0.00
map_Kd default.png
";

// ---------------------------------------------------------------------------
// Texture generation (256×256 livery)
// ---------------------------------------------------------------------------

fn demo_car_texture() -> Vec<u8> {
    use image::{ImageBuffer, Rgb};
    const W: u32 = 256;
    const H: u32 = 256;
    let mut img = ImageBuffer::from_fn(W, H, |_x, y| {
        // Silver body with subtle vertical shading.
        let shade = 200 + ((y / 8) % 2) as i32 * 4 - (y as i32 / 64) * 6;
        Rgb([shade as u8, shade as u8, (shade + 6) as u8])
    });
    // Livery stripe (demo pink).
    for y in 150..170 {
        for x in 0..W {
            img.put_pixel(x, y, Rgb([254, 197, 229]));
        }
    }
    // Window band (dark blue glass).
    for y in 20..80 {
        for x in 0..W {
            let frame = y < 24 || y >= 76 || (x % 64) < 3;
            let c = if frame { [120, 124, 130] } else { [40, 70, 110] };
            img.put_pixel(x, y, Rgb(c));
        }
    }
    // Door panels: five bays outlined along the lower half.
    for bay in 0..5 {
        let x0 = 20 + bay * 44;
        for y in 84..148 {
            for x in x0..x0 + 36 {
                let edge = x == x0 || x == x0 + 35 || y == 84 || y == 147;
                let c = if edge { [90, 90, 96] } else { [186, 186, 190] };
                img.put_pixel(x, y, Rgb(c));
            }
        }
        // Door number.
        let digit = match bay {
            0 => "1",
            1 => "2",
            2 => "3",
            3 => "4",
            _ => "5",
        };
        draw_text_5x7(&mut img, x0 + 15, 100, digit);
    }
    // Yellow display strip.
    for y in 200..220 {
        for x in 0..W {
            img.put_pixel(x, y, Rgb([255, 180, 40]));
        }
    }
    let mut out = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut out);
    image::ImageEncoder::write_image(enc, img.as_raw(), W, H, image::ExtendedColorType::Rgb8)
        .expect("encode demo texture");
    out
}

/// Tiny 5×7 bitmap digit renderer (0-9) for the demo texture.
fn draw_text_5x7(img: &mut image::ImageBuffer<image::Rgb<u8>, Vec<u8>>, x0: u32, y0: u32, text: &str) {
    const DIGITS: [&str; 10] = [
        "01110 10001 10011 10101 11001 10001 01110",
        "00100 01100 00100 00100 00100 00100 01110",
        "01110 10001 00001 00010 00100 01000 11111",
        "11111 00010 00100 00010 00001 10001 01110",
        "00010 00110 01010 10010 11111 00010 00010",
        "11111 10000 11110 00001 00001 10001 01110",
        "00110 01000 10000 11110 10001 10001 01110",
        "11111 00001 00010 00100 01000 01000 01000",
        "01110 10001 10001 01110 10001 10001 01110",
        "01110 10001 10001 01111 00001 00010 01100",
    ];
    let mut x = x0;
    for ch in text.chars() {
        let digit = ch.to_digit(10).unwrap_or(0) as usize;
        for (row, line) in DIGITS[digit].split_whitespace().enumerate() {
            for (col, bit) in line.chars().enumerate() {
                if bit == '1' {
                    img.put_pixel(x + col as u32, y0 + row as u32, image::Rgb([40, 40, 48]));
                }
            }
        }
        x += 6;
    }
}

// ---------------------------------------------------------------------------
// BVE motor data (type 5) — a plausible VVVF curve set
// ---------------------------------------------------------------------------

const DEMO_SOUND_CFG: &str = "\
BveTs Vehicle Sound Data 2.00
MotorNoiseDataType=5
";

/// speed(m/s), channel0 volume, channel1 volume
const DEMO_POWERVOL: &str = "\
0,0.05,0.00
3,0.55,0.10
6,0.75,0.30
9,0.85,0.55
12,0.90,0.75
15,0.85,0.90
18,0.70,0.95
22,0.55,0.85
26,0.40,0.70
";

const DEMO_POWERFREQ: &str = "\
0,0.20,0.00
3,0.80,0.35
6,1.30,0.60
9,1.70,0.85
12,1.00,1.10
15,0.80,1.30
18,0.65,1.05
22,0.55,0.80
26,0.45,0.60
";

const DEMO_BRAKEVOL: &str = "\
0,0.00,0.00
3,0.30,0.10
6,0.55,0.30
9,0.75,0.55
12,0.85,0.75
15,0.80,0.90
18,0.65,0.85
22,0.45,0.70
26,0.30,0.50
";

const DEMO_BRAKEFREQ: &str = "\
0,0.15,0.00
3,0.70,0.30
6,1.10,0.55
9,1.50,0.80
12,1.80,1.00
15,1.40,1.20
18,1.10,1.00
22,0.90,0.80
26,0.70,0.60
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_pack_is_valid() {
        let pack = create_demo_pack();
        // Model & dependencies exist.
        let v = &pack.resources.vehicles[0];
        assert_eq!(v.id, "demo_metro_car");
        assert!(pack.resource(&v.models[0].model_resource, "obj").is_some());
        assert!(pack.resource(&v.models[0].texture_resource, "png").is_some());
        assert!(pack.resource(&v.models[0].model_properties_resource, "json").is_some());
        assert!(pack.resource(&v.models[0].position_definitions_resource, "json").is_some());
        // BVE files exist at the path MTR derives from bveSoundBaseResource.
        assert!(pack.files.contains_key("assets/mtr/sounds/custom/demo/sound/sound.cfg"));
        // Script exists and is referenced.
        assert_eq!(v.script_id.as_deref(), Some("demo_lcd"));
        assert!(pack.files.contains_key("assets/mtr/custom/demo/js/main.js"));
        // Validation passes without errors.
        let items = crate::validate::validate_pack(&pack);
        let errors: Vec<_> = items
            .iter()
            .filter(|i| i.severity == crate::validate::Severity::Error)
            .collect();
        assert!(errors.is_empty(), "validation errors: {errors:?}");
    }

    #[test]
    fn demo_model_parses() {
        let pack = create_demo_pack();
        let obj_text = String::from_utf8(
            pack.resource(&format!("{DEMO_NS}/car.obj"), "obj").unwrap().to_vec(),
        )
        .unwrap();
        let model = crate::obj::ObjModel::parse(&obj_text).unwrap();
        let names: Vec<&str> = model.parts.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"body"));
        assert!(names.contains(&"door"));
        assert!(names.contains(&"display"));
        assert!(model.parts.iter().all(|p| !p.meshes.is_empty()));
        assert!(model.bounds().is_some());
    }

    #[test]
    fn demo_properties_parse() {
        let pack = create_demo_pack();
        let props_text = String::from_utf8(
            pack.resource(&format!("{DEMO_NS}/car_properties.json"), "json").unwrap().to_vec(),
        )
        .unwrap();
        let props: ModelProperties = serde_json::from_str(&props_text).unwrap();
        assert!(props.parts.iter().any(|p| p.is_door()));
        let positions_text = String::from_utf8(
            pack.resource(&format!("{DEMO_NS}/car_positions.json"), "json").unwrap().to_vec(),
        )
        .unwrap();
        let positions: PositionDefinitions = serde_json::from_str(&positions_text).unwrap();
        let door_left = positions.get("doorLeft").unwrap();
        assert_eq!(door_left.positions.len(), 5);
    }
}
