//! Left outliner and right properties panel.

use egui::RichText;

use mrpc_core::i18n::{t, tf, Locale};
use mrpc_core::model_props::{DisplayOption, PartCondition, PartType, RenderStage};
use mrpc_core::vehicle::TransportMode;

use crate::app::MrpcApp;
use crate::state::Selection;

pub fn outliner_panel(app: &mut MrpcApp, ui: &mut egui::Ui) {
    egui::Panel::left("outliner")
        .resizable(true)
        .default_size(240.0)
        .min_size(180.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("outliner_scroll")
                .show(ui, |ui| outliner(app, ui));
        });
}

pub fn properties_panel(app: &mut MrpcApp, ui: &mut egui::Ui) {
    egui::Panel::right("properties")
        .resizable(true)
        .default_size(320.0)
        .min_size(240.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("properties_scroll")
                .show(ui, |ui| properties(app, ui));
        });
}

pub fn outliner(app: &mut MrpcApp, ui: &mut egui::Ui) {
    let locale = app.state.locale;
    ui.heading(t(locale, "outliner.title"));
    ui.separator();

    ui.set_min_width(ui.available_width());

    // ---- Vehicles ----------------------------------------------------------
    ui.strong(t(locale, "outliner.vehicles"));
    let vehicle_count = app.state.pack.resources.vehicles.len();
    ui.label(tf(locale, "outliner.vehicle_count", &[("n", &vehicle_count.to_string())]))
        .on_disabled_hover_text("");
    let mut select = None;
    let mut delete = None;
    let mut duplicate = None;
    for (i, vehicle) in app.state.pack.resources.vehicles.iter().enumerate() {
        let is_selected = matches!(app.state.selection, Selection::Vehicle(v) if v == i);
        let label = if vehicle.name.is_empty() { vehicle.id.clone() } else { vehicle.name.clone() };
        let text = if is_selected {
            RichText::new(format!("▸ {label}")).strong().underline()
        } else {
            RichText::new(format!("  {label}"))
        };
        let response = ui.selectable_label(is_selected, text);
        if response.clicked() {
            select = Some(i);
        }
        let sub = ui.indent(format!("veh{i}"), |ui| {
            ui.set_min_width(ui.available_width());
            ui.style_mut().visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
            if let Some(model) = vehicle.models.first() {
                let short = model.model_resource.rsplit('/').next().unwrap_or("");
                ui.monospace(format!("model: {short}"));
            }
            ui.monospace(format!("{} {}", vehicle.color, vehicle.id))
                .on_hover_text(&vehicle.id);
            if let Some(script_id) = &vehicle.script_id {
                ui.weak(format!("script: {script_id}"));
            }
            ui.horizontal(|ui| {
                if ui.small_button("⧉").clicked() {
                    duplicate = Some(i);
                }
                if ui.small_button("✕").clicked() {
                    delete = Some(i);
                }
            });
        });
        let _ = sub.response;
    }
    if ui.button(t(locale, "outliner.add_vehicle")).clicked() {
        let mut v = mrpc_core::vehicle::VehicleResource::default();
        v.id = format!("custom_train_{}", vehicle_count + 1);
        app.state.pack.resources.vehicles.push(v);
        app.state.selection = Selection::Vehicle(vehicle_count);
        app.state.dirty = true;
        app.reload_model();
    }
    if let Some(i) = select {
        app.state.selection = Selection::Vehicle(i);
        app.reload_model();
    }
    if let Some(i) = duplicate {
        if let Some(v) = app.state.pack.resources.vehicles.get(i).cloned() {
            let mut copy = v;
            copy.id = format!("{}_copy", copy.id);
            app.state.pack.resources.vehicles.push(copy);
            app.state.dirty = true;
        }
    }
    if let Some(i) = delete {
        if app.state.pack.resources.vehicles.len() > i {
            app.state.pack.resources.vehicles.remove(i);
            app.state.selection = Selection::None;
            app.state.dirty = true;
            app.reload_model();
        }
    }

    ui.add_space(6.0);
    ui.separator();

    // ---- Scripts -----------------------------------------------------------
    ui.strong(t(locale, "outliner.scripts"));
    let script_count = app.state.pack.resources.vehicle_scripts.len();
    ui.label(tf(locale, "outliner.scripts_count", &[("n", &script_count.to_string())]));
    let mut select_script = None;
    for (i, entry) in app.state.pack.resources.vehicle_scripts.iter().enumerate() {
        let is_selected = matches!(app.state.selection, Selection::Script(s) if s == i);
        let response = ui.selectable_label(
            is_selected,
            format!("⚙ {} [{}]", entry.id, entry.language_summary()),
        );
        if response.clicked() {
            select_script = Some(i);
        }
    }
    if let Some(i) = select_script {
        app.state.selection = Selection::Script(i);
        app.load_script_editor();
    }

    ui.add_space(6.0);
    ui.separator();

    // ---- Assets ------------------------------------------------------------
    ui.strong(t(locale, "outliner.assets"));
    let files = app.state.pack.asset_paths();
    ui.label(tf(locale, "outliner.files", &[("n", &files.len().to_string())]));
    egui::ScrollArea::vertical()
        .id_salt("asset_list")
        .max_height(200.0)
        .show(ui, |ui| {
            for path in files.iter().take(300) {
                let is_selected = matches!(&app.state.selection, Selection::Asset(p) if p == path);
                if ui
                    .selectable_label(is_selected, RichText::new(path.to_string()).small())
                    .clicked()
                {
                    app.state.selection = Selection::Asset((*path).to_string());
                }
            }
        });
}

/// Editable text field with a label on the left.
pub fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
    });
}

pub fn num_field(ui: &mut egui::Ui, label: &str, value: &mut f64, unit: String, speed: f64) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).speed(speed));
        if !unit.is_empty() {
            ui.weak(unit);
        }
    });
}

fn combo_enum<T: PartialEq + Copy + Clone + 'static>(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut T,
    variants: &[T],
    name_of: impl Fn(&T) -> String,
) {
    egui::ComboBox::from_label(label)
        .selected_text(name_of(selected))
        .show_ui(ui, |ui| {
            for v in variants {
                ui.selectable_value(selected, *v, name_of(v));
            }
        });
}

pub fn properties(app: &mut MrpcApp, ui: &mut egui::Ui) {
    let locale = app.state.locale;
    ui.heading(t(locale, "props.title"));
    ui.separator();
    ui.set_min_width(ui.available_width());

    match app.state.selection.clone() {
        Selection::None => {
            ui.weak(t(locale, "outliner.no_vehicles"));
        }
        Selection::Vehicle(vi) => vehicle_props(app, ui, vi, locale),
        Selection::Model { vehicle, model } => model_props(app, ui, vehicle, model, locale),
        Selection::Part { vehicle, model, part } => {
            part_props(app, ui, vehicle, model, part, locale)
        }
        Selection::Script(si) => script_props(app, ui, si, locale),
        Selection::Asset(path) => {
            ui.strong(&path);
            ui.separator();
            if let Some(bytes) = app.state.pack.files.get(&path).cloned() {
                ui.label(format!("{} bytes", bytes.len()));
                let lower = path.to_ascii_lowercase();
                if lower.ends_with(".json") || lower.ends_with(".cfg") || lower.ends_with(".csv")
                    || lower.ends_with(".obj") || lower.ends_with(".mtl")
                    || lower.ends_with(".js") || lower.ends_with(".txt")
                {
                    ui.separator();
                    ui.collapsing("preview", |ui| {
                        let text = String::from_utf8_lossy(&bytes);
                        egui::ScrollArea::vertical()
                            .id_salt("asset_preview")
                            .max_height(400.0)
                            .show(ui, |ui| {
                                ui.monospace(text.into_owned());
                            });
                    });
                } else if lower.ends_with(".png") {
                    show_texture_preview(ui, &bytes);
                }
            } else {
                ui.weak("missing");
            }
        }
    }
}

fn show_texture_preview(ui: &mut egui::Ui, bytes: &[u8]) {
    if let Ok(img) = image::load_from_memory(bytes) {
        let size = [img.width() as usize, img.height() as usize];
        let rgba = img.to_rgba8();
        let pixels = rgba.into_raw();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
        let tex = ui
            .ctx()
            .load_texture("asset_preview", color_image, egui::TextureOptions::default());
        ui.image((tex.id(), egui::vec2(256.0, 256.0)));
    } else {
        ui.weak("invalid image");
    }
}

fn transport_name(locale: Locale, mode: TransportMode) -> String {
    match mode {
        TransportMode::Train => t(locale, "transport.TRAIN"),
        TransportMode::Boat => t(locale, "transport.BOAT"),
        TransportMode::CableCar => t(locale, "transport.CABLE_CAR"),
        TransportMode::Airplane => t(locale, "transport.AIRPLANE"),
        TransportMode::Unknown => t(locale, "transport.UNKNOWN"),
    }
}

fn vehicle_props(app: &mut MrpcApp, ui: &mut egui::Ui, vi: usize, locale: Locale) {
    let Some(vehicle) = app.state.pack.resources.vehicles.get_mut(vi) else { return };
    ui.strong(format!("{} · {}", t(locale, "outliner.vehicles"), vehicle.id));
    ui.separator();

    text_field(ui, &t(locale, "props.id"), &mut vehicle.id);
    text_field(ui, &t(locale, "props.name"), &mut vehicle.name);
    ui.horizontal(|ui| {
        ui.label(t(locale, "props.color"));
        let c = vehicle.color_rgb();
        let mut rgb = [(c >> 16 & 0xFF) as u8, (c >> 8 & 0xFF) as u8, (c & 0xFF) as u8];
        ui.color_edit_button_srgb(&mut rgb);
        if ui.small_button("→").clicked() {
            vehicle.color = format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
        }
        ui.monospace(format!("#{}", vehicle.color));
    });

    combo_enum(
        ui,
        &t(locale, "props.transport_mode"),
        &mut vehicle.transport_mode,
        &[
            TransportMode::Train,
            TransportMode::Boat,
            TransportMode::CableCar,
            TransportMode::Airplane,
        ],
        |m| transport_name(locale, *m),
    );
    num_field(ui, &t(locale, "props.length"), &mut vehicle.length, t(locale, "units.blocks"), 0.1);
    num_field(ui, &t(locale, "props.width"), &mut vehicle.width, t(locale, "units.blocks"), 0.05);
    num_field(ui, &t(locale, "props.bogie1"), &mut vehicle.bogie1_position, t(locale, "units.blocks"), 0.1);
    num_field(ui, &t(locale, "props.bogie2"), &mut vehicle.bogie2_position, t(locale, "units.blocks"), 0.1);
    num_field(ui, &t(locale, "props.coupling1"), &mut vehicle.coupling_padding1, t(locale, "units.blocks"), 0.05);
    num_field(ui, &t(locale, "props.coupling2"), &mut vehicle.coupling_padding2, t(locale, "units.blocks"), 0.05);

    ui.separator();
    ui.collapsing(t(locale, "props.description"), |ui| {
        ui.add(egui::TextEdit::multiline(&mut vehicle.description).desired_rows(3).desired_width(f32::INFINITY));
        text_field(ui, &t(locale, "props.wikipedia"), &mut vehicle.wikipedia_article);
        let mut tags = vehicle.tags.join(", ");
        text_field(ui, &t(locale, "props.tags"), &mut tags);
        vehicle.tags = tags
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    });

    ui.collapsing(t(locale, "props.bogie_models"), |ui| {
        ui.weak(format!("{}: {}", t(locale, "outliner.files"), vehicle.bogie1_models.len() + vehicle.bogie2_models.len()));
    });

    ui.separator();
    ui.strong(t(locale, "props.sounds"));
    text_field(ui, &t(locale, "props.bve_sound"), &mut vehicle.bve_sound_base_resource);
    ui.collapsing(t(locale, "props.legacy_speed_sound"), |ui| {
        text_field(ui, &t(locale, "props.legacy_speed_sound"), &mut vehicle.legacy_speed_sound_base_resource);
        let mut count = vehicle.legacy_speed_sound_count;
        ui.horizontal(|ui| {
            ui.label(t(locale, "props.legacy_speed_count"));
            ui.add(egui::DragValue::new(&mut count).speed(1.0));
        });
        vehicle.legacy_speed_sound_count = count;
        ui.checkbox(&mut vehicle.legacy_use_acceleration_sounds_when_coasting, t(locale, "props.legacy_coast_accel"));
        ui.checkbox(&mut vehicle.legacy_constant_playback_speed, t(locale, "props.legacy_const_playback"));
        text_field(ui, &t(locale, "props.legacy_door_sound"), &mut vehicle.legacy_door_sound_base_resource);
        num_field(ui, &t(locale, "props.legacy_door_close_time"), &mut vehicle.legacy_door_close_sound_time, t(locale, "units.ms"), 10.0);
    });

    ui.separator();
    ui.strong(t(locale, "props.script"));
    let script_ids: Vec<String> = app
        .state
        .pack
        .resources
        .vehicle_scripts
        .iter()
        .map(|s| s.id.clone())
        .collect();
    let mut attached = vehicle.script_id.clone().unwrap_or_default();
    egui::ComboBox::from_label(t(locale, "props.script"))
        .selected_text(if attached.is_empty() { t(locale, "props.script_none") } else { attached.clone() })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut attached, String::new(), t(locale, "props.script_none"));
            for id in &script_ids {
                ui.selectable_value(&mut attached, id.clone(), id.clone());
            }
        });
    vehicle.script_id = if attached.is_empty() { None } else { Some(attached) };
    ui.checkbox(&mut vehicle.hide_display_parts, t(locale, "props.hide_display_parts"));

    ui.separator();
    if ui.button(t(locale, "model.title")).clicked() {
        app.state.selection = Selection::Model { vehicle: vi, model: 0 };
    }
}

fn model_props(app: &mut MrpcApp, ui: &mut egui::Ui, vi: usize, mi: usize, locale: Locale) {
    let Some(vehicle) = app.state.pack.resources.vehicles.get_mut(vi) else { return };
    let Some(model) = vehicle.models.get_mut(mi) else { return };
    ui.strong(format!("{} #{}", t(locale, "props.models"), mi + 1));
    ui.separator();
    text_field(ui, &t(locale, "model.model_resource"), &mut model.model_resource);
    text_field(ui, &t(locale, "model.texture_resource"), &mut model.texture_resource);
    text_field(ui, &t(locale, "model.properties_resource"), &mut model.model_properties_resource);
    text_field(ui, &t(locale, "model.positions_resource"), &mut model.position_definitions_resource);
    ui.checkbox(&mut model.flip_texture_v, t(locale, "model.flip_texture_v"));

    ui.separator();
    ui.strong(t(locale, "model.parts"));
    let part_count = app
        .state
        .model
        .as_ref()
        .map(|m| m.properties.parts.len())
        .unwrap_or(0);
    ui.label(tf(locale, "model.part_count", &[("n", &part_count.to_string())]));
    let mut select_part = None;
    if let Some(loaded) = app.state.model.as_ref() {
        for (pi, part) in loaded.properties.parts.iter().enumerate() {
            let label = if part.names.is_empty() {
                "—".to_string()
            } else {
                part.names.join(", ")
            };
            let marker = if part.is_door() { "🚪 " } else { "" };
            let response = ui.selectable_label(
                matches!(app.state.selection, Selection::Part { part: p, .. } if p == pi),
                format!("{marker}{label}"),
            );
            if response.clicked() {
                select_part = Some(pi);
            }
        }
    }
    if let Some(pi) = select_part {
        app.state.selection = Selection::Part { vehicle: vi, model: mi, part: pi };
    }
    if ui.button(t(locale, "part.add")).clicked() {
        if let Some(loaded) = app.state.model.as_mut() {
            loaded.properties.parts.push(mrpc_core::model_props::ModelPropertiesPart::default());
            app.save_properties_back();
        }
    }
    if ui.button(t(locale, "model.auto_parts")).clicked() {
        app.auto_parts_from_model();
    }

    ui.separator();
    if ui.button(format!("← {}", t(locale, "outliner.vehicles"))).clicked() {
        app.state.selection = Selection::Vehicle(vi);
    }
}

fn part_props(app: &mut MrpcApp, ui: &mut egui::Ui, _vi: usize, _mi: usize, pi: usize, locale: Locale) {
    let Some(loaded) = app.state.model.as_ref() else { return };
    let Some(mut part) = loaded.properties.parts.get(pi).cloned() else { return };
    ui.strong(t(locale, "model.parts"));
    ui.separator();

    let mut names = part.names.join(", ");
    text_field(ui, &t(locale, "part.names"), &mut names);
    part.names = names
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let mut defs = part.position_definitions.join(", ");
    text_field(ui, &t(locale, "part.position_definitions"), &mut defs);
    part.position_definitions = defs
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    combo_enum(
        ui,
        &t(locale, "part.condition"),
        &mut part.condition,
        &[
            PartCondition::Normal,
            PartCondition::AtDepot,
            PartCondition::OnRouteForwards,
            PartCondition::OnRouteBackwards,
            PartCondition::DoorsClosed,
            PartCondition::DoorsOpened,
        ],
        |c| condition_name(locale, *c),
    );
    combo_enum(
        ui,
        &t(locale, "part.render_stage"),
        &mut part.render_stage,
        &[
            RenderStage::Exterior,
            RenderStage::Interior,
            RenderStage::InteriorTranslucent,
            RenderStage::Light,
            RenderStage::AlwaysOnLight,
        ],
        |s| render_stage_name(locale, *s),
    );
    combo_enum(
        ui,
        &t(locale, "part.type"),
        &mut part.part_type,
        &[
            PartType::Normal,
            PartType::Display,
            PartType::Floor,
            PartType::Doorway,
            PartType::Seat,
        ],
        |p| part_type_name(locale, *p),
    );

    ui.separator();
    ui.strong(t(locale, "part.is_door"));
    num_field(ui, &t(locale, "part.door_x"), &mut part.door_x_multiplier, String::new(), 0.5);
    num_field(ui, &t(locale, "part.door_z"), &mut part.door_z_multiplier, String::new(), 0.5);
    combo_enum(
        ui,
        &t(locale, "part.door_animation"),
        &mut part.door_animation_type,
        &mrpc_core::door_anim::DoorAnimationType::ALL,
        |d| serde_json::to_string(d).unwrap_or_default().trim_matches('"').to_string(),
    );

    ui.separator();
    if part.part_type == PartType::Display {
        ui.strong("display");
        text_field(ui, &t(locale, "part.display_default_text"), &mut part.display_default_text);
        text_field(ui, &t(locale, "part.display_color"), &mut part.display_color);
        text_field(ui, &t(locale, "part.display_color_cjk"), &mut part.display_color_cjk);
        num_field(ui, &t(locale, "part.display_x_padding"), &mut part.display_x_padding, String::new(), 0.1);
        num_field(ui, &t(locale, "part.display_y_padding"), &mut part.display_y_padding, String::new(), 0.1);
        num_field(ui, &t(locale, "part.display_max_line_height"), &mut part.display_max_line_height, String::new(), 0.1);
        num_field(ui, &t(locale, "part.display_cjk_ratio"), &mut part.display_cjk_size_ratio, String::new(), 0.05);
        let mut pad = part.display_pad_zeros;
        ui.horizontal(|ui| {
            ui.label(t(locale, "part.display_pad_zeros"));
            ui.add(egui::DragValue::new(&mut pad).speed(1.0));
        });
        part.display_pad_zeros = pad;

        ui.separator();
        ui.label(t(locale, "part.display_options"));
        for option in DisplayOption::ALL {
            let key = option.as_str().to_string();
            let mut on = part.display_options.contains(&key);
            ui.checkbox(&mut on, option.as_str());
            if on && !part.display_options.contains(&key) {
                part.display_options.push(key);
            } else if !on {
                part.display_options.retain(|k| k != &key);
            }
        }
    }

    ui.separator();
    ui.collapsing(t(locale, "part.render_window"), |ui| {
        let mut from_o = part.render_from_opening_door_time;
        let mut until_o = part.render_until_opening_door_time;
        let mut from_c = part.render_from_closing_door_time;
        let mut until_c = part.render_until_closing_door_time;
        ui.horizontal(|ui| {
            ui.label(t(locale, "part.render_from_open"));
            ui.add(egui::DragValue::new(&mut from_o).speed(50.0));
        });
        ui.horizontal(|ui| {
            ui.label(t(locale, "part.render_until_open"));
            ui.add(egui::DragValue::new(&mut until_o).speed(50.0));
        });
        ui.horizontal(|ui| {
            ui.label(t(locale, "part.render_from_close"));
            ui.add(egui::DragValue::new(&mut from_c).speed(50.0));
        });
        ui.horizontal(|ui| {
            ui.label(t(locale, "part.render_until_close"));
            ui.add(egui::DragValue::new(&mut until_c).speed(50.0));
        });
        part.render_from_opening_door_time = from_o;
        part.render_until_opening_door_time = until_o;
        part.render_from_closing_door_time = from_c;
        part.render_until_closing_door_time = until_c;
    });

    ui.separator();
    let apply = ui.button(t(locale, "common.apply")).clicked();
    let remove = ui.button(t(locale, "common.remove")).clicked();
    // Write the edited clone back into the model.
    if remove {
        if let Some(loaded) = app.state.model.as_mut() {
            if pi < loaded.properties.parts.len() {
                loaded.properties.parts.remove(pi);
            }
        }
        app.save_properties_back();
        app.state.selection = Selection::Model { vehicle: _vi, model: _mi };
        app.state.dirty = true;
    } else {
        if let Some(loaded) = app.state.model.as_mut() {
            if pi < loaded.properties.parts.len() {
                loaded.properties.parts[pi] = part.clone();
            }
        }
        if apply {
            app.save_properties_back();
            app.state.dirty = true;
        }
    }
}

fn script_props(app: &mut MrpcApp, ui: &mut egui::Ui, si: usize, locale: Locale) {
    let Some(entry) = app.state.pack.resources.vehicle_scripts.get_mut(si) else { return };
    ui.strong(format!("{} · {}", t(locale, "script.title"), entry.id));
    ui.separator();
    text_field(ui, &t(locale, "script.entry_id"), &mut entry.id);

    ui.label(t(locale, "script.locations"));
    let mut remove_at = None;
    let location_count = entry.script_locations.len();
    for li in 0..location_count {
        ui.horizontal(|ui| {
            let mut s = match entry.script_locations.get(li) {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY));
            if ui.small_button("✕").clicked() {
                remove_at = Some(li);
            }
            if let Some(slot) = entry.script_locations.get_mut(li) {
                *slot = serde_json::Value::String(s);
            }
        });
    }
    if let Some(li) = remove_at {
        if li < entry.script_locations.len() {
            entry.script_locations.remove(li);
        }
    }
    if ui.button(t(locale, "script.add_location")).clicked() {
        entry.script_locations
            .push(serde_json::Value::String("mtr:custom/pack/js/main.js".into()));
    }

    ui.separator();
    ui.label(t(locale, "script.prepend"));
    let mut prepend = entry.prepend_expressions.join("\n");
    ui.add(
        egui::TextEdit::multiline(&mut prepend)
            .code_editor()
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    entry.prepend_expressions = prepend
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    ui.label(t(locale, "script.data_fetch_mode"));
    let modes = [
        mrpc_core::script::DataFetchMode::Skip,
        mrpc_core::script::DataFetchMode::Mandatory,
        mrpc_core::script::DataFetchMode::All,
    ];
    let mut mode = entry.data_fetch_mode.unwrap_or(mrpc_core::script::DataFetchMode::Skip);
    egui::ComboBox::from_id_salt("fetch_mode")
        .selected_text(fetch_mode_name(locale, mode))
        .show_ui(ui, |ui| {
            for m in modes {
                ui.selectable_value(&mut mode, m, fetch_mode_name(locale, m));
            }
        });
    entry.data_fetch_mode = Some(mode);
    ui.checkbox(&mut entry.force_load, t(locale, "script.force_load"));

    ui.separator();
    ui.label(t(locale, "script.editor"));
    ui.add(
        egui::TextEdit::multiline(&mut app.state.script.code)
            .code_editor()
            .desired_rows(14)
            .desired_width(f32::INFINITY),
    );
    ui.label(t(locale, "script.input"));
    ui.add(
        egui::TextEdit::multiline(&mut app.state.script.input_json)
            .code_editor()
            .desired_rows(4)
            .desired_width(f32::INFINITY),
    );
    if ui.button(t(locale, "common.apply")).clicked() {
        app.save_script_back();
    }
}

pub fn condition_name(locale: Locale, c: PartCondition) -> String {
    match c {
        PartCondition::Normal => t(locale, "condition.NORMAL"),
        PartCondition::AtDepot => t(locale, "condition.AT_DEPOT"),
        PartCondition::OnRouteForwards => t(locale, "condition.ON_ROUTE_FORWARDS"),
        PartCondition::OnRouteBackwards => t(locale, "condition.ON_ROUTE_BACKWARDS"),
        PartCondition::DoorsClosed => t(locale, "condition.DOORS_CLOSED"),
        PartCondition::DoorsOpened => t(locale, "condition.DOORS_OPENED"),
        _ => "—".into(),
    }
}

pub fn render_stage_name(locale: Locale, s: RenderStage) -> String {
    match s {
        RenderStage::Light => t(locale, "renderstage.LIGHT"),
        RenderStage::AlwaysOnLight => t(locale, "renderstage.ALWAYS_ON_LIGHT"),
        RenderStage::Interior => t(locale, "renderstage.INTERIOR"),
        RenderStage::InteriorTranslucent => t(locale, "renderstage.INTERIOR_TRANSLUCENT"),
        RenderStage::Exterior => t(locale, "renderstage.EXTERIOR"),
        RenderStage::Unknown => "—".into(),
    }
}

pub fn part_type_name(locale: Locale, p: PartType) -> String {
    match p {
        PartType::Normal => t(locale, "parttype.NORMAL"),
        PartType::Display => t(locale, "parttype.DISPLAY"),
        PartType::Floor => t(locale, "parttype.FLOOR"),
        PartType::Doorway => t(locale, "parttype.DOORWAY"),
        PartType::Seat => t(locale, "parttype.SEAT"),
        PartType::Unknown => "—".into(),
    }
}

pub fn fetch_mode_name(locale: Locale, m: mrpc_core::script::DataFetchMode) -> String {
    match m {
        mrpc_core::script::DataFetchMode::Skip => t(locale, "fetchmode.SKIP"),
        mrpc_core::script::DataFetchMode::Mandatory => t(locale, "fetchmode.MANDATORY"),
        mrpc_core::script::DataFetchMode::All => t(locale, "fetchmode.ALL"),
    }
}
