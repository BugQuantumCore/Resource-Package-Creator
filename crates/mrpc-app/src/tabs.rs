//! Bottom tab bar: door animation timeline, VVVF audition, script testing,
//! export & validation.

use egui::{Color32, Pos2, RichText, Stroke};

use mrpc_core::i18n::{t, tf, Locale};

use crate::app::MrpcApp;
use crate::state::Selection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Anim,
    Vvvf,
    Script,
    Export,
}

pub fn bottom_tabs(app: &mut MrpcApp, ui: &mut egui::Ui) {
    let locale = app.state.locale;
    let tab = app.bottom_tab;
    egui::Panel::bottom("bottom_tabs")
        .resizable(true)
        .default_size(280.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut app.bottom_tab, BottomTab::Anim, t(locale, "anim.title"));
                ui.selectable_value(&mut app.bottom_tab, BottomTab::Vvvf, t(locale, "vvvf.title"));
                ui.selectable_value(&mut app.bottom_tab, BottomTab::Script, t(locale, "script.title"));
                ui.selectable_value(&mut app.bottom_tab, BottomTab::Export, t(locale, "export.title"));
            });
            ui.separator();
            match tab {
                BottomTab::Anim => anim_tab(app, ui, locale),
                BottomTab::Vvvf => vvvf_tab(app, ui, locale),
                BottomTab::Script => script_tab(app, ui, locale),
                BottomTab::Export => export_tab(app, ui, locale),
            }
        });
}

// ---------------------------------------------------------------------------
// Door animation timeline
// ---------------------------------------------------------------------------

fn anim_tab(app: &mut MrpcApp, ui: &mut egui::Ui, locale: Locale) {
    let total = app.state.anim.cycle.total_ms() as f32;
    ui.horizontal(|ui| {
        let playing = app.state.anim.playing;
        if ui
            .button(if playing { t(locale, "anim.pause") } else { t(locale, "anim.play") })
            .clicked()
        {
            app.state.anim.playing = !playing;
            if !app.state.anim.playing {
                app.state.door_manual = app.state.anim.door().0;
            }
        }
        if ui.button(t(locale, "anim.reset")).clicked() {
            app.state.anim.t_ms = 0.0;
            app.state.door_manual = 0.0;
        }
        ui.separator();
        ui.label(t(locale, "anim.scrub"));
        let mut t_pos = app.state.anim.t_ms as f32;
        ui.add(egui::Slider::new(&mut t_pos, 0.0..=total).smallest_positive(1.0));
        app.state.anim.t_ms = t_pos as f64;
        if !app.state.anim.playing {
            app.state.door_manual = app.state.anim.door().0;
        }
        let (value, phase) = app.state.anim.cycle.sample(app.state.anim.t_ms as i64);
        ui.separator();
        ui.label(format!("{}: {:.0}%", t(locale, "anim.door_value"), value * 100.0));
        ui.label(format!("{}: {}", t(locale, "anim.phase"), phase_name(locale, phase)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(t(locale, "viewport.hint"));
        });
    });

    ui.horizontal(|ui| {
        let mut delay = app.state.anim.cycle.delay_ms;
        let mut mv = app.state.anim.cycle.move_ms;
        let mut dwell = app.state.anim.cycle.dwell_ms;
        ui.label(t(locale, "anim.door_delay"));
        ui.add(egui::DragValue::new(&mut delay).speed(100.0));
        ui.label(t(locale, "anim.door_move_time"));
        ui.add(egui::DragValue::new(&mut mv).speed(100.0));
        ui.label(t(locale, "anim.dwell_time"));
        ui.add(egui::DragValue::new(&mut dwell).speed(100.0));
        app.state.anim.cycle.delay_ms = delay.max(0);
        app.state.anim.cycle.move_ms = mv.max(100);
        app.state.anim.cycle.dwell_ms = dwell.max(0);
    });

    // Timeline canvas.
    let size = egui::vec2(ui.available_width(), ui.available_height().max(120.0));
    let (response, painter) = ui.allocate_painter(size, egui::Sense::click_and_drag());
    let rect = response.rect;
    let bg = Color32::from_rgb(24, 24, 26);
    painter.rect_filled(rect, 4.0, bg);

    let cycle = app.state.anim.cycle;
    let delay = cycle.delay_ms as f32;
    let mv = cycle.move_ms as f32;
    let dwell = cycle.dwell_ms as f32;
    let total_f = cycle.total_ms() as f32;

    let x_of = |t_ms: f32| rect.left() + (t_ms / total_f) * rect.width();
    let y_of = |v: f32| rect.bottom() - (v.clamp(0.0, 1.4) / 1.4) * rect.height();

    // Phase bands.
    let band = |from: f32, to: f32, color: Color32| {
        let a = Pos2::new(x_of(from), rect.top());
        let b = Pos2::new(x_of(to), rect.bottom());
        painter.rect_filled(egui::Rect::from_two_pos(a, b), 0.0, color);
    };
    band(0.0, delay, Color32::from_rgb(38, 38, 42));
    band(delay, delay + mv, Color32::from_rgb(45, 62, 90));
    band(delay + mv, delay + mv + dwell, Color32::from_rgb(56, 78, 56));
    band(delay + mv + dwell, delay + 2.0 * mv + dwell, Color32::from_rgb(90, 66, 45));
    band(delay + 2.0 * mv + dwell, total_f, Color32::from_rgb(38, 38, 42));

    // Door value curve (TSC: DOOR_DELAY → DOOR_MOVE_TIME ramp).
    let mut door_curve: Vec<Pos2> = Vec::new();
    let steps = 240;
    for i in 0..=steps {
        let t_ms = total_f * i as f32 / steps as f32;
        let (v, _) = cycle.sample(t_ms as i64);
        door_curve.push(Pos2::new(x_of(t_ms), y_of(v as f32)));
    }
    painter.add(egui::Shape::line(
        door_curve,
        Stroke::new(2.5, Color32::from_rgb(254, 197, 229)),
    ));

    // Selected part displacement curves (model units).
    if let Selection::Part { part, .. } = app.state.selection {
        if let Some(loaded) = app.state.model.as_ref() {
            if let Some(p) = loaded.properties.parts.get(part) {
                if p.is_door() {
                    let mut x_curve: Vec<Pos2> = Vec::new();
                    let mut z_curve: Vec<Pos2> = Vec::new();
                    let max_disp =
                        (p.door_x_multiplier.abs() + p.door_z_multiplier.abs()).max(8.0) as f32;
                    let y_disp = |v: f64| rect.bottom() - ((v as f32) / max_disp * 0.5 + 0.5) * rect.height();
                    for i in 0..=steps {
                        let t_ms = total_f * i as f32 / steps as f32;
                        let (v, phase) = cycle.sample(t_ms as i64);
                        let opening = !matches!(
                            phase,
                            mrpc_core::door_anim::DoorPhase::Closing
                                | mrpc_core::door_anim::DoorPhase::ClosedAfterClose
                        );
                        let dx = p
                            .door_animation_type
                            .door_animation_x(p.door_x_multiplier, false, v);
                        let dz = p
                            .door_animation_type
                            .door_animation_z(p.door_z_multiplier, false, v, opening);
                        x_curve.push(Pos2::new(x_of(t_ms), y_disp(dx)));
                        z_curve.push(Pos2::new(x_of(t_ms), y_disp(dz)));
                    }
                    painter.add(egui::Shape::dashed_line(
                        &x_curve,
                        Stroke::new(1.5, Color32::from_rgb(120, 180, 255)),
                        6.0,
                        6.0,
                    ));
                    painter.add(egui::Shape::line(
                        z_curve,
                        Stroke::new(1.5, Color32::from_rgb(120, 255, 180)),
                    ));
                    let legend = format!(
                        "{} {} · {} {} ({})",
                        t(locale, "anim.curve_x"),
                        p.door_x_multiplier,
                        t(locale, "anim.curve_z"),
                        p.door_z_multiplier,
                        serde_json::to_string(&p.door_animation_type).unwrap_or_default().trim_matches('"').to_string()
                    );
                    painter.text(
                        rect.left_top() + egui::vec2(8.0, 6.0),
                        egui::Align2::LEFT_TOP,
                        legend,
                        egui::FontId::monospace(11.0),
                        Color32::from_rgb(200, 200, 210),
                    );
                }
            } else {
                no_door_parts_note(painter.clone(), rect, locale, false);
            }
        }
    } else {
        no_door_parts_note(painter.clone(), rect, locale, true);
    }

    // Playhead.
    let px = x_of(app.state.anim.t_ms as f32);
    painter.line_segment(
        [Pos2::new(px, rect.top()), Pos2::new(px, rect.bottom())],
        Stroke::new(1.5, Color32::WHITE),
    );
    // Scrub by dragging on the canvas.
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let frac = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            app.state.anim.t_ms = (frac * total_f) as f64;
            app.state.anim.playing = false;
            app.state.door_manual = app.state.anim.door().0;
        }
    }
}

fn no_door_parts_note(painter: egui::Painter, rect: egui::Rect, locale: Locale, generic: bool) {
    let msg = if generic {
        t(locale, "anim.no_door_parts")
    } else {
        t(locale, "anim.no_door_parts")
    };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        msg,
        egui::FontId::proportional(12.0),
        Color32::from_rgb(150, 150, 158),
    );
}

fn phase_name(locale: Locale, phase: mrpc_core::door_anim::DoorPhase) -> String {
    use mrpc_core::door_anim::DoorPhase;
    match phase {
        DoorPhase::ClosedBeforeOpen => t(locale, "anim.phase_closed_before"),
        DoorPhase::Opening => t(locale, "anim.phase_opening"),
        DoorPhase::Open => t(locale, "anim.phase_open"),
        DoorPhase::Closing => t(locale, "anim.phase_closing"),
        DoorPhase::ClosedAfterClose => t(locale, "anim.phase_closed_after"),
    }
}

// ---------------------------------------------------------------------------
// VVVF audition
// ---------------------------------------------------------------------------

fn vvvf_tab(app: &mut MrpcApp, ui: &mut egui::Ui, locale: Locale) {
    let mode = app.state.vvvf.mode;
    ui.horizontal(|ui| {
        egui::ComboBox::from_label(t(locale, "vvvf.mode"))
            .selected_text(vvvf_mode_name(locale, mode))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut app.state.vvvf.mode, 0, vvvf_mode_name(locale, 0));
                ui.selectable_value(&mut app.state.vvvf.mode, 1, vvvf_mode_name(locale, 1));
                ui.selectable_value(&mut app.state.vvvf.mode, 2, vvvf_mode_name(locale, 2));
            });
        ui.separator();
        ui.label(t(locale, "vvvf.speed"));
        let mut speed = app.state.vvvf.speed_kmh as f32;
        ui.add(egui::Slider::new(&mut speed, 0.0..=130.0));
        app.state.vvvf.speed_kmh = speed as f64;
        ui.label(t(locale, "vvvf.notch"));
        let mut notch = app.state.vvvf.notch as f32;
        ui.add(egui::Slider::new(&mut notch, -4.0..=5.0));
        app.state.vvvf.notch = notch as f64;
    });

    ui.horizontal(|ui| {
        let playing = app.state.vvvf.playing;
        if ui
            .button(if playing { t(locale, "vvvf.stop") } else { t(locale, "vvvf.play") })
            .clicked()
        {
            app.state.vvvf.playing = !playing;
            app.set_audio_playing(app.state.vvvf.playing);
        }
        if ui.button(t(locale, "vvvf.render_wav")).clicked() {
            app.render_vvvf_wav();
        }
        let fund = app.state.vvvf.fundamental_hz();
        ui.separator();
        ui.monospace(format!("{} ≈ {:.1} Hz", t(locale, "vvvf.fundamental"), fund));
        #[cfg(not(feature = "audio"))]
        {
            ui.separator();
            ui.colored_label(Color32::from_rgb(200, 160, 60), t(locale, "vvvf.no_audio_build"));
        }
        #[cfg(feature = "audio")]
        {
            if app.audio.is_none() {
                ui.separator();
                ui.weak(t(locale, "vvvf.no_audio_build"));
            }
        }
        if let Some(msg) = app.state.vvvf.message.clone() {
            ui.separator();
            ui.label(&msg);
        }
    });

    ui.separator();

    match mode {
        0 => synth_params(app, ui, locale),
        1 => bve_curves(app, ui, locale),
        _ => legacy_sounds(app, ui, locale),
    }

    ui.separator();

    // Waveform + spectrum side by side.
    let scope_h = ui.available_height().clamp(64.0, 140.0);
    ui.horizontal(|ui| {
        let wave_w = ui.available_width() * 0.6;
        let (r, _resp) = ui.allocate_exact_size(egui::vec2(wave_w, scope_h), egui::Sense::hover());
        draw_waveform(ui, r, &app.state.vvvf.wave, t(locale, "vvvf.waveform"));
        let spec_w = ui.available_width() * 0.9;
        let (r, _resp) = ui.allocate_exact_size(egui::vec2(spec_w, scope_h), egui::Sense::hover());
        draw_spectrum(ui, r, &app.state.vvvf.spectrum, t(locale, "vvvf.spectrum"));
    });
}

fn vvvf_mode_name(locale: Locale, mode: u8) -> String {
    match mode {
        0 => t(locale, "vvvf.mode_synth"),
        1 => t(locale, "vvvf.mode_bve"),
        _ => t(locale, "vvvf.mode_legacy"),
    }
}

fn synth_params(app: &mut MrpcApp, ui: &mut egui::Ui, locale: Locale) {
    ui.horizontal(|ui| {
        ui.label(t(locale, "vvvf.scheme"));
        let mut scheme = app.state.vvvf.params.scheme;
        egui::ComboBox::from_id_salt("scheme")
            .selected_text(if scheme == 0 { t(locale, "vvvf.scheme_spwm") } else { t(locale, "vvvf.scheme_she") })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut scheme, 0, t(locale, "vvvf.scheme_spwm"));
                ui.selectable_value(&mut scheme, 1, t(locale, "vvvf.scheme_she"));
            });
        app.state.vvvf.params.scheme = scheme;

        ui.separator();
        let mut carrier = app.state.vvvf.params.carrier_freq as f32;
        ui.label(t(locale, "vvvf.carrier_freq"));
        ui.add(egui::Slider::new(&mut carrier, 200.0..=3000.0));
        app.state.vvvf.params.carrier_freq = carrier as f64;

        let mut poles = app.state.vvvf.params.pole_pairs as i32;
        ui.label(t(locale, "vvvf.pole_pairs"));
        ui.add(egui::Slider::new(&mut poles, 2..=8));
        app.state.vvvf.params.pole_pairs = poles as f64;
    });
    ui.horizontal(|ui| {
        let mut base = app.state.vvvf.params.base_freq_hz as f32;
        ui.label(t(locale, "vvvf.base_freq"));
        ui.add(egui::Slider::new(&mut base, 30.0..=150.0));
        app.state.vvvf.params.base_freq_hz = base as f64;

        let mut depth = app.state.vvvf.params.max_mod_depth as f32;
        ui.label(t(locale, "vvvf.mod_depth"));
        ui.add(egui::Slider::new(&mut depth, 0.2..=1.15));
        app.state.vvvf.params.max_mod_depth = depth as f64;

        let mut bus = app.state.vvvf.params.dc_bus as f32;
        ui.label(t(locale, "vvvf.dc_bus"));
        ui.add(egui::Slider::new(&mut bus, 600.0..=3000.0));
        app.state.vvvf.params.dc_bus = bus as f64;
    });
    ui.horizontal(|ui| {
        let mut r = app.state.vvvf.params.load_r as f32;
        ui.label(t(locale, "vvvf.load_r"));
        ui.add(egui::Slider::new(&mut r, 0.2..=3.0));
        app.state.vvvf.params.load_r = r as f64;

        let mut l = (app.state.vvvf.params.load_l * 1000.0) as f32;
        ui.label(t(locale, "vvvf.load_l"));
        ui.add(egui::Slider::new(&mut l, 1.0..=20.0));
        app.state.vvvf.params.load_l = l as f64 / 1000.0;

        let mut noise = app.state.vvvf.params.noise_gain as f32;
        ui.label(t(locale, "vvvf.noise_gain"));
        ui.add(egui::Slider::new(&mut noise, 0.0..=1.0));
        app.state.vvvf.params.noise_gain = noise as f64;
    });
    ui.horizontal(|ui| {
        let mut sync_min = app.state.vvvf.params.sync_min_ratio;
        let mut sync_max = app.state.vvvf.params.sync_max_ratio;
        ui.label(t(locale, "vvvf.sync_min"));
        ui.add(egui::Slider::new(&mut sync_min, 3..=15));
        ui.label(t(locale, "vvvf.sync_max"));
        ui.add(egui::Slider::new(&mut sync_max, 3..=21));
        app.state.vvvf.params.sync_min_ratio = sync_min;
        app.state.vvvf.params.sync_max_ratio = sync_max;
    });
}

fn bve_curves(app: &mut MrpcApp, ui: &mut egui::Ui, locale: Locale) {
    // Parse once per vehicle selection.
    if app.state.vvvf.bve.is_none() {
        let bve = app.load_bve_data();
        app.state.vvvf.bve = bve;
        if app.state.vvvf.bve.is_none() {
            ui.weak(t(locale, "vvvf.no_bve_data"));
            return;
        }
    }
    let Some(bve) = app.state.vvvf.bve.as_ref() else { return };
    let sound_count = bve.sound_count();
    ui.label(tf(locale, "vvvf.sound_count", &[("n", &sound_count.to_string())]));

    let speed_ms = (app.state.vvvf.speed_kmh / 3.6) as f32;
    let notch = app.state.vvvf.notch as f32;
    let power = if notch == 0.0 {
        0.0
    } else if notch > 0.0 {
        (notch / 5.0).min(1.0)
    } else {
        (notch / 4.0).max(-1.0)
    };

    // Pitch/volume table for channel 0..min(3, count).
    ui.monospace(format!(
        "speed = {:.1} m/s · power = {:+.2}",
        speed_ms, power
    ));
    egui::Grid::new("bve_table").striped(true).show(ui, |ui| {
        ui.strong("ch");
        ui.strong("pitch");
        ui.strong("volume");
        ui.end_row();
        for ch in 0..sound_count.min(4) {
            ui.monospace(format!("{ch}"));
            ui.monospace(format!("{:.3}", bve.get_pitch(ch, speed_ms, power)));
            ui.monospace(format!("{:.3}", bve.get_volume(ch, speed_ms, power)));
            ui.end_row();
        }
    });

    // Drive the synthesiser with the BVE curve (approximate audition): the
    // fundamental follows the power-frequency table over a base frequency.
    let base_hz = 45.0;
    let pitch = bve.get_pitch(0, speed_ms, power);
    let volume = bve.get_volume(0, speed_ms, power);
    let fundamental = pitch * base_hz;
    app.state.vvvf.params.max_mod_depth = ((volume.max(0.0) * 1.1).clamp(0.1, 1.1)) as f64;
    app.state.vvvf.set_bve_fundamental(fundamental.max(0.0) as f64);
    ui.weak(format!(
        "synth ≈ {:.1} Hz · m ≈ {:.2}",
        fundamental * base_hz.max(1.0) / base_hz * base_hz,
        volume
    ));

    // ---- real-sound-source playback --------------------------------------
    ui.separator();
    ui.strong(t(locale, "vvvf.bve_playback"));
    match app.state.vvvf.bve_engine.as_ref() {
        Some(engine) => {
            ui.weak(tf(
                locale,
                "vvvf.bve_loaded",
                &[("n", &engine.channel_count().to_string())],
            ));
            ui.monospace(format!(
                "loop ≈ {:.2} s · {}",
                engine.loop_duration(),
                if app.state.vvvf.playing {
                    t(locale, "vvvf.play")
                } else {
                    t(locale, "vvvf.stop")
                }
            ));
        }
        None => {
            ui.weak(t(locale, "vvvf.bve_no_source"));
        }
    }
    ui.horizontal(|ui| {
        let mut wav_path = app.state.vvvf.bve_wav_path.clone();
        ui.label(t(locale, "vvvf.bve_load"));
        if ui
            .add(egui::TextEdit::singleline(&mut wav_path).desired_width(240.0).hint_text("motor_loop.wav"))
            .changed()
        {
            app.state.vvvf.bve_wav_path = wav_path;
        }
        if ui.button(t(locale, "common.ok")).clicked() {
            let path = app.state.vvvf.bve_wav_path.clone();
            if !path.is_empty() {
                app.load_manual_bve_wav(&path);
            }
        }
    });
    ui.weak(t(locale, "vvvf.bve_hint"));
}

fn legacy_sounds(app: &mut MrpcApp, ui: &mut egui::Ui, locale: Locale) {
    let Some(vi) = app.state.selected_vehicle() else {
        ui.weak(t(locale, "outliner.no_vehicles"));
        return;
    };
    let vehicle = app.state.pack.resources.vehicles[vi].clone();
    if vehicle.legacy_speed_sound_base_resource.is_empty() && vehicle.legacy_door_sound_base_resource.is_empty() {
        ui.weak(t(locale, "vvvf.no_bve_data"));
        return;
    }

    // MTR3 legacy naming: <base>_acceleration_<g><letter>, groups of 3.
    ui.monospace(format!("base = {}", vehicle.legacy_speed_sound_base_resource));
    ui.monospace(format!("count = {}", vehicle.legacy_speed_sound_count));
    ui.separator();
    ui.monospace(format!(
        "door open  → {}_door_open",
        vehicle.legacy_door_sound_base_resource
    ));
    ui.monospace(format!(
        "door close → {}_door_close (t = {} ms)",
        vehicle.legacy_door_sound_base_resource, vehicle.legacy_door_close_sound_time
    ));
    ui.separator();

    let speed = app.state.vvvf.speed_kmh / 3.6;
    // MTR: floorSpeed = speed / referenceAcceleration / MILLIS_PER_SPEED_SOUND
    // with Siding.ACCELERATION_DEFAULT = 0.5 and MILLIS_PER_SPEED_SOUND = 250.
    let reference_acceleration = 0.5;
    let millis_per_speed_sound = 250.0;
    let floor_speed = (speed / reference_acceleration / (millis_per_speed_sound / 1000.0)).floor();
    let letters = ['a', 'b', 'c'];
    if floor_speed > 0.0 && vehicle.legacy_speed_sound_count > 0 {
        let index = (floor_speed.min(vehicle.legacy_speed_sound_count as f64) as usize) - 1;
        let group = index / 3;
        let letter = letters[index % 3];
        let accel = if app.state.vvvf.notch >= 0.0 { "acceleration" } else { "deceleration" };
        ui.monospace(format!(
            "→ {}_{}_{group}{letter}",
            vehicle.legacy_speed_sound_base_resource, accel
        ));
    } else {
        ui.weak("(stopped — no speed sound)");
    }
}

fn draw_waveform(ui: &mut egui::Ui, rect: egui::Rect, wave: &[f32], title: String) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(24, 24, 26));
    let mid = (rect.top() + rect.bottom()) / 2.0;
    painter.line_segment(
        [Pos2::new(rect.left(), mid), Pos2::new(rect.right(), mid)],
        Stroke::new(1.0, Color32::from_rgb(60, 60, 66)),
    );
    if wave.len() > 1 {
        let mut points: Vec<Pos2> = Vec::with_capacity(wave.len());
        for (i, s) in wave.iter().enumerate() {
            let x = rect.left() + (i as f32 / wave.len() as f32) * rect.width();
            let y = mid + (s.clamp(-1.0, 1.0) * rect.height() / 2.2);
            points.push(Pos2::new(x, y));
        }
        painter.add(egui::Shape::line(points, Stroke::new(1.2, Color32::from_rgb(120, 200, 255))));
    }
    painter.text(
        rect.left_top() + egui::vec2(6.0, 4.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(11.0),
        Color32::from_rgb(170, 170, 178),
    );
}

fn draw_spectrum(ui: &mut egui::Ui, rect: egui::Rect, bars: &[(f64, f32)], title: String) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(24, 24, 26));
    let max_val = bars.iter().map(|b| b.1).fold(0.2f32, f32::max);
    for (i, (_freq, v)) in bars.iter().enumerate() {
        let x = rect.left() + (i as f32 / bars.len() as f32) * rect.width();
        let h = (v / max_val) * rect.height() * 0.9;
        let color = Color32::from_rgb(120, 220, 160);
        painter.rect_filled(
            egui::Rect::from_min_size(
                Pos2::new(x, rect.bottom() - h),
                egui::vec2(rect.width() / bars.len() as f32 - 1.0, h),
            ),
            0.0,
            color,
        );
    }
    painter.text(
        rect.left_top() + egui::vec2(6.0, 4.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(11.0),
        Color32::from_rgb(170, 170, 178),
    );
    if let Some((freq, _)) = bars.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
        painter.text(
            rect.right_bottom() + egui::vec2(-6.0, -4.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{freq:.0} Hz"),
            egui::FontId::monospace(10.0),
            Color32::from_rgb(190, 190, 200),
        );
    }
}

// ---------------------------------------------------------------------------
// Script testing
// ---------------------------------------------------------------------------

fn script_tab(app: &mut MrpcApp, ui: &mut egui::Ui, locale: Locale) {
    ui.horizontal(|ui| {
        if ui.button(t(locale, "script.template_js")).clicked() {
            app.new_script_entry(true);
        }
        if ui.button(t(locale, "script.template_cpp")).clicked() {
            app.new_script_entry(false);
        }
        if ui.button(t(locale, "script.run")).clicked() {
            app.run_script_test();
        }
        ui.separator();
        ui.weak(t(locale, "script.native_note"));
    });

    ui.separator();
    let remaining = ui.available_size();
    ui.horizontal(|ui| {
        // Editor.
        ui.vertical(|ui| {
            ui.strong(t(locale, "script.editor"));
            ui.add(
                egui::TextEdit::multiline(&mut app.state.script.code)
                    .code_editor()
                    .desired_rows(12)
                    .desired_width(remaining.x * 0.55),
            );
        });
        ui.vertical(|ui| {
            ui.strong(t(locale, "script.api_reference"));
            egui::ScrollArea::vertical()
                .id_salt("api_ref")
                .max_height(remaining.y - 40.0)
                .show(ui, |ui| {
                    for (name, sig) in VEHICLE_API {
                        ui.monospace(RichText::new(*sig).small().color(egui::Color32::from_rgb(150, 200, 255)))
                            .on_hover_text(*name);
                    }
                });
        });
    });

    // Native module testing.
    ui.horizontal(|ui| {
        ui.label("C++:");
        ui.add(
            egui::TextEdit::singleline(&mut app.state.script.native_path)
                .desired_width(240.0)
                .hint_text("./my_lcd.so"),
        );
        if ui.button(t(locale, "script.run")).clicked() {
            app.run_native_test();
        }
        if !app.state.script.native_display.is_empty() {
            ui.separator();
            ui.monospace(format!("display: {}", app.state.script.native_display));
        }
        if let Some(msg) = app.state.script.native_message.clone() {
            ui.separator();
            ui.colored_label(egui::Color32::from_rgb(220, 120, 120), msg);
        }
    });

    ui.separator();
    ui.strong(t(locale, "script.console"));
    egui::ScrollArea::vertical()
        .id_salt("script_console")
        .stick_to_bottom(true)
        .max_height(90.0)
        .show(ui, |ui| {
            for line in &app.state.script.console {
                ui.monospace(RichText::new(line).small());
            }
        });
}

/// JCM `VehicleWrapper` API surface (reference panel).
const VEHICLE_API: &[(&str, &str)] = &[
    ("Speed in kilometres per hour.", "getSpeedKmh()"),
    ("Speed in metres per second.", "getSpeedMs()"),
    ("Door state, 0 closed … 1 fully open.", "getDoorValue()"),
    ("True while the doors are opening.", "isDoorOpening()"),
    ("Traction notch, negative = braking.", "getNotchLevel()"),
    ("Number of cars in the consist.", "getCarCount()"),
    ("Length of a car (blocks).", "getLength(car)"),
    ("Width of a car (blocks).", "getWidth(car)"),
    ("Transport type string, e.g. \"train\".", "getTransportType()"),
    ("not | left | right | all", "getDirectionOfDoorOpening()"),
    ("Travel direction is reversed.", "isReversed()"),
    ("Currently on a route.", "isOnRoute()"),
    ("Total dwell time at the platform (ms).", "getTotalDwellTime()"),
    ("Elapsed dwell time (ms).", "getElapsedDwellTime()"),
    ("All stops of the run.", "getStops() → [{name, getName()}]"),
    ("Stops of the current route.", "getThisRouteStops()"),
    ("Stops of the next route.", "getNextRouteStops()"),
    ("Vehicle id (64-bit).", "getId()"),
    ("Lifecycle: called once after load.", "function create(ctx, state, vehicle)"),
    ("Lifecycle: called every frame.", "function render(ctx, state, vehicle)"),
    ("Lifecycle: called before unload.", "function dispose(ctx, state, vehicle)"),
    ("Evaluate another script file.", "include(Resources.id(\"mtr:pack/js/lib.js\"))"),
    ("Entry input JSON.", "SCRIPT_INPUT"),
    ("Console output.", "print(...) / console.log(...)"),
    ("Simulated clock.", "Timing.millis() / Timing.delta()"),
    ("Cycle \"A|B\" strings over time.", "TextUtil.cycleString(\"A|B\", ms?)"),
];

// ---------------------------------------------------------------------------
// Export & validation
// ---------------------------------------------------------------------------

fn export_tab(app: &mut MrpcApp, ui: &mut egui::Ui, locale: Locale) {
    ui.horizontal(|ui| {
        ui.label(t(locale, "export.path"));
        let mut path = app
            .state
            .config
            .last_export
            .clone()
            .unwrap_or_else(|| "my-pack.zip".into());
        ui.add(egui::TextEdit::singleline(&mut path).desired_width(f32::INFINITY));
        app.state.config.last_export = Some(path.clone());
        if ui.button(t(locale, "export.export")).clicked() {
            match app.state.pack.export_zip(std::path::Path::new(&path)) {
                Ok(()) => {
                    app.state.export_message =
                        Some(tf(locale, "export.done", &[("path", &path)]));
                    app.state.config.save();
                }
                Err(e) => {
                    app.state.export_message = Some(format!("{}: {e}", t(locale, "error.io")));
                }
            }
        }
        if ui.button(t(locale, "common.refresh")).clicked() {
            app.state.revalidate();
        }
    });

    if let Some(msg) = app.state.export_message.clone() {
        ui.label(RichText::new(msg).color(egui::Color32::from_rgb(120, 220, 160)));
    }

    ui.separator();
    let (errors, warnings, hints) = mrpc_core::validate::summarize(&app.state.validation);
    ui.label(tf(
        locale,
        "export.validation_summary",
        &[
            ("e", &errors.to_string()),
            ("w", &warnings.to_string()),
            ("h", &hints.to_string()),
        ],
    ));
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("validation")
        .show(ui, |ui| {
            if app.state.validation.is_empty() {
                ui.label(RichText::new(t(locale, "validation.no_issues")).color(egui::Color32::from_rgb(120, 220, 160)));
            }
            for item in &app.state.validation {
                let (color, sev) = match item.severity {
                    mrpc_core::validate::Severity::Error => {
                        (egui::Color32::from_rgb(235, 100, 100), t(locale, "validation.severity_error"))
                    }
                    mrpc_core::validate::Severity::Warning => {
                        (egui::Color32::from_rgb(230, 180, 70), t(locale, "validation.severity_warning"))
                    }
                    mrpc_core::validate::Severity::Hint => {
                        (egui::Color32::from_rgb(120, 180, 235), t(locale, "validation.severity_hint"))
                    }
                };
                ui.label(
                    RichText::new(format!("[{sev}] {} {}", t(locale, &item.key), item.detail))
                        .small()
                        .color(color),
                );
            }
        });
}
