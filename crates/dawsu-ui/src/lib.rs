//! The native editor surface. Actions go through Application.

use dawsu_application::{Application, PlatformRequest, PointerPhase, lane_name};
use dawsu_editor::keys::{Key, KeyChord, Mods};
use dawsu_editor::tools::{PointerButton, PointerInput, PointerSurface, ToolKind};
use dawsu_foundation::SongTime;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::time::Duration;

const BG: Color32 = Color32::from_rgb(25, 29, 36);
const PANEL: Color32 = Color32::from_rgb(34, 39, 47);
const CYAN: Color32 = Color32::from_rgb(91, 207, 225);
const TEXT: Color32 = Color32::from_rgb(221, 227, 235);
const MUTED: Color32 = Color32::from_rgb(158, 169, 184);
const LABEL: f32 = 188.0;

pub struct EditorUi {
    pub application: Application,
    filter: String,
    show_settings: bool,
    show_help: bool,
    home: bool,
    pointer_down: bool,
    controller_down: bool,
    insert_volume: i32,
    selection_volume: i32,
    show_new_diff: bool,
    new_diff_name: String,
    show_copy: bool,
    copy_targets: Vec<std::path::PathBuf>,
    copy_options: dawsu_hitsounding::copy::CopyOptions,
    pub confirm_close: bool,
    pub allow_close: bool,
}
impl EditorUi {
    pub fn new(application: Application) -> Self {
        Self {
            application,
            filter: String::new(),
            show_settings: false,
            show_help: false,
            home: false,
            pointer_down: false,
            controller_down: false,
            insert_volume: 100,
            selection_volume: 80,
            show_new_diff: false,
            new_diff_name: "Hitsounds".into(),
            show_copy: false,
            copy_targets: Vec::new(),
            copy_options: Default::default(),
            confirm_close: false,
            allow_close: false,
        }
    }
    pub fn configure(ctx: &egui::Context) {
        let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = PANEL;
        style.visuals.window_fill = PANEL;
        style.visuals.extreme_bg_color = BG;
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = Color32::from_rgb(40, 94, 112);
        style.visuals.selection.stroke = Stroke::new(1.0, CYAN);
        style.spacing.item_spacing = Vec2::new(7.0, 6.0);
        style.spacing.button_padding = Vec2::new(9.0, 5.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
        ctx.set_style_of(egui::Theme::Dark, style);
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.application.poll();
        let ctx = ui.ctx().clone();
        if self.application.help_requested {
            self.show_help = true;
            self.application.help_requested = false;
        }
        self.shortcuts(&ctx);
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        for path in dropped {
            self.application.request_open(path);
            self.home = false;
        }
        egui::Panel::top("menu")
            .exact_size(29.0)
            .show(ui, |ui| self.menu(ui));
        egui::Panel::top("ribbon")
            .exact_size(44.0)
            .show(ui, |ui| self.ribbon(ui));
        egui::Panel::bottom("transport")
            .exact_size(56.0)
            .show(ui, |ui| self.transport(ui));
        if self.application.preferences.show_browser {
            egui::Panel::left("browser")
                .default_size(210.0)
                .min_size(150.0)
                .resizable(true)
                .show(ui, |ui| self.browser(ui));
        }
        if self.application.preferences.show_inspector {
            egui::Panel::right("inspector")
                .default_size(245.0)
                .min_size(200.0)
                .resizable(true)
                .show(ui, |ui| self.inspector(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if self.application.document().is_none() || self.home {
                self.welcome(ui);
            } else {
                self.timeline(ui);
            }
        });
        self.overlays(&ctx);
        let cleaning_audio = self.application.collect_retired_audio();
        if self.application.is_playing() {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if self.application.busy || cleaning_audio {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }
    fn menu(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("dawsu!").strong().color(CYAN));
            ui.separator();
            ui.menu_button("File", |ui| {
                if ui.button("Open difficulty…     Ctrl+O").clicked() {
                    self.application.requests.push(PlatformRequest::OpenMap);
                    ui.close();
                }
                if ui.button("Save                       Ctrl+S").clicked() {
                    self.application.save();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.application.is_dirty() && !self.application.busy,
                        egui::Button::new("Revert to last saved…"),
                    )
                    .clicked()
                {
                    self.application.command("revert");
                    ui.close();
                }
                if let Some(d) = self.application.document() {
                    let folder = d.path.parent().unwrap().to_path_buf();
                    if ui.button("Open map folder").clicked() {
                        self.application
                            .requests
                            .push(PlatformRequest::OpenFolder(folder));
                        ui.close();
                    }
                }
                if ui.button("Preferences…").clicked() {
                    self.show_settings = true;
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                for (label, id) in [
                    ("Undo   Ctrl+Z", "undo"),
                    ("Redo   Ctrl+Shift+Z", "redo"),
                    ("Cut   Ctrl+X", "cut"),
                    ("Copy   Ctrl+C", "copy"),
                    ("Paste   Ctrl+V", "paste"),
                    ("Duplicate   Ctrl+D", "duplicate"),
                    ("Delete", "delete"),
                    ("Select all   Ctrl+A", "selectAll"),
                    ("Quantize", "quantize"),
                ] {
                    if ui.button(label).clicked() {
                        self.application.command(id);
                        ui.close();
                    }
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.application.preferences.show_browser, "Browser");
                ui.checkbox(
                    &mut self.application.preferences.show_inspector,
                    "Inspector",
                );
                ui.checkbox(
                    &mut self.application.preferences.show_controller,
                    "Volume controller",
                );
                ui.checkbox(
                    &mut self.application.preferences.used_only,
                    "Used lanes only",
                );
                ui.checkbox(&mut self.application.preferences.lane_names, "Lane names");
                ui.checkbox(&mut self.application.preferences.follow, "Follow playback");
                ui.checkbox(
                    &mut self.application.preferences.object_lines,
                    "Object lines",
                );
                if ui.button("Fit difficulty").clicked() {
                    self.fit(ui.available_width().max(600.0));
                    ui.close();
                }
            });
            if ui.button("Help  F1").clicked() {
                self.show_help = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(d) = self.application.document() {
                    ui.label(
                        RichText::new(format!(
                            "{}{}",
                            if self.application.is_dirty() {
                                "●  "
                            } else {
                                ""
                            },
                            d.path.file_name().unwrap().to_string_lossy()
                        ))
                        .color(MUTED),
                    )
                    .on_hover_text(d.path.display().to_string());
                }
            });
        });
    }
    fn ribbon(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            if ui.button("Home").clicked() {
                self.home = !self.home;
            }
            if ui.button("Open…").clicked() {
                self.application.requests.push(PlatformRequest::OpenMap);
                self.home = false;
            }
            if ui
                .add_enabled(
                    self.application.document().is_some() && !self.application.busy,
                    egui::Button::new("Save").fill(Color32::from_rgb(40, 83, 99)),
                )
                .clicked()
            {
                self.application.save();
            }
            ui.separator();
            if ui.button("Undo").clicked() {
                self.application.command("undo");
            }
            if ui.button("Redo").clicked() {
                self.application.command("redo");
            }
            ui.separator();
            ui.menu_button("Output", |ui| {
                if ui.button("New hitsound difficulty…").clicked() {
                    self.show_new_diff = true;
                    ui.close();
                }
                if ui.button("Generate hitsound difficulty…").clicked() {
                    self.application.generate();
                    ui.close();
                }
                if ui.button("Copy hitsounds…").clicked() {
                    self.show_copy = true;
                    self.copy_targets.clear();
                    ui.close();
                }
            });
            for tool in ToolKind::ALL {
                if ui
                    .selectable_label(self.application.tool() == tool, format!("{tool:?}"))
                    .clicked()
                {
                    self.application.set_tool(tool);
                }
            }
            ui.separator();
            let (mut snap, mut spacing, mut mode) = self
                .application
                .document()
                .map(|d| {
                    (
                        d.composer.snap_enabled,
                        (d.composer.spacing.beats(), d.composer.spacing.divisions()),
                        d.composer.snap_mode,
                    )
                })
                .unwrap_or((true, (1, 4), dawsu_editor::tempo::SnapMode::Grid));
            ui.checkbox(&mut snap, "Snap");
            egui::ComboBox::from_id_salt("snap")
                .selected_text(if spacing.0 > 1 {
                    format!("{}/1", spacing.0)
                } else {
                    format!("1/{}", spacing.1)
                })
                .width(55.0)
                .show_ui(ui, |ui| {
                    for beats in [4, 3, 2] {
                        ui.selectable_value(&mut spacing, (beats, 1), format!("{beats}/1"));
                    }
                    for divisions in [1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 16] {
                        ui.selectable_value(&mut spacing, (1, divisions), format!("1/{divisions}"));
                    }
                });
            egui::ComboBox::from_id_salt("snap-mode")
                .selected_text(match mode {
                    dawsu_editor::tempo::SnapMode::Grid => "Grid",
                    dawsu_editor::tempo::SnapMode::GridRelative => "Grid relative",
                    dawsu_editor::tempo::SnapMode::Objects => "Objects",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut mode, dawsu_editor::tempo::SnapMode::Grid, "Grid");
                    ui.selectable_value(
                        &mut mode,
                        dawsu_editor::tempo::SnapMode::GridRelative,
                        "Grid relative",
                    );
                    ui.selectable_value(
                        &mut mode,
                        dawsu_editor::tempo::SnapMode::Objects,
                        "Objects",
                    );
                });
            self.application
                .set_snap_options(snap, spacing.0, spacing.1, mode);
        });
    }
    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.add_space(65.0);
        ui.horizontal(|ui| {
            ui.add_space(44.0);
            ui.vertical(|ui| {
                ui.heading(RichText::new("Your next hitsound session").size(28.0));
                ui.add_space(10.0);
                ui.label(
                    RichText::new("Open a difficulty, or drop an .osu or .osz file here.")
                        .size(15.0)
                        .color(MUTED),
                );
                ui.add_space(20.0);
                if ui
                    .add_sized(
                        [190.0, 38.0],
                        egui::Button::new("Open difficulty…").fill(Color32::from_rgb(43, 96, 113)),
                    )
                    .clicked()
                {
                    self.application.requests.push(PlatformRequest::OpenMap);
                    self.home = false;
                }
                ui.add_space(28.0);
                ui.label(RichText::new("RECENT MAPS").color(MUTED).size(11.0));
                for path in self.application.preferences.recent.clone() {
                    if ui
                        .button(path.file_name().unwrap_or_default().to_string_lossy())
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        self.application.request_open(path);
                        self.home = false;
                    }
                }
                ui.add_space(25.0);
                ui.label(RichText::new("Codex development build · real map editing").color(MUTED));
            });
        });
    }
    fn browser(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Browser");
            if ui.small_button("Hide").clicked() {
                self.application.preferences.show_browser = false;
            }
        });
        ui.add_space(7.0);
        ui.text_edit_singleline(&mut self.filter)
            .on_hover_text("Filter samples and lanes");
        ui.separator();
        if let Some(d) = self.application.document() {
            let lanes = d.composer.lanes.clone();
            let filter = self.filter.to_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for lane in lanes {
                    let name = lane_name(&lane);
                    if !name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    if ui
                        .selectable_label(false, &name)
                        .on_hover_text("Click to audition this sample")
                        .clicked()
                    {
                        self.application.audition(&lane);
                    }
                }
            });
        } else {
            ui.label(RichText::new("Samples appear when a map opens.").color(MUTED));
        }
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Inspector");
            if ui.small_button("Hide").clicked() {
                self.application.preferences.show_inspector = false;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(d) = self.application.document() {
                let title = d.title.clone();
                let count = d.composer.selected.len();
                let dirty = d.composer.dirty();
                let steps: Vec<_> = d
                    .composer
                    .history
                    .steps()
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                let position = d.composer.history.position();
                let difficulties = d.difficulties.clone();
                let current = d.path.clone();
                let mut hs = d.hs_mode;
                ui.label(RichText::new(title).strong());
                ui.label(
                    RichText::new(if dirty { "Unsaved edits" } else { "Saved" }).color(if dirty {
                        CYAN
                    } else {
                        MUTED
                    }),
                );
                ui.add_space(8.0);
                egui::ComboBox::from_id_salt("difficulty")
                    .selected_text(current.file_name().unwrap().to_string_lossy())
                    .width(ui.available_width() - 10.0)
                    .show_ui(ui, |ui| {
                        for path in difficulties {
                            if ui
                                .selectable_label(
                                    path == current,
                                    path.file_name().unwrap().to_string_lossy(),
                                )
                                .clicked()
                            {
                                self.application.request_open(path);
                            }
                        }
                    });
                if ui
                    .checkbox(&mut hs, "HS difficulty mode")
                    .on_hover_text("Create circles for hits with no gameplay object when saving")
                    .changed()
                {
                    self.application.set_hs_mode(hs);
                }
                ui.add_space(15.0);
                ui.strong(format!("Selection · {count} hits"));
                ui.horizontal(|ui| {
                    ui.label("Volume");
                    if ui
                        .add(
                            egui::DragValue::new(&mut self.selection_volume)
                                .range(0..=100)
                                .suffix(" %"),
                        )
                        .changed()
                    {
                        self.application
                            .selection_volume(self.selection_volume, false);
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("−5").clicked() {
                        self.application.selection_volume(-5, true);
                    }
                    if ui.button("+5").clicked() {
                        self.application.selection_volume(5, true);
                    }
                    if ui.button("Quantize").clicked() {
                        self.application.command("quantize");
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Duplicate").clicked() {
                        self.application.command("duplicate");
                    }
                    if ui.button("Delete").clicked() {
                        self.application.command("delete");
                    }
                });
                ui.add_space(15.0);
                ui.strong("New hits");
                if ui
                    .add(egui::Slider::new(&mut self.insert_volume, 0..=100).text("Volume"))
                    .changed()
                {
                    self.application.set_insert_volume(self.insert_volume);
                }
                ui.checkbox(&mut self.application.preferences.audition, "Audition edits");
                let mut protect = self.application.preferences.protect_hitnormals;
                if ui
                    .checkbox(&mut protect, "Protect hitnormals from deletion")
                    .changed()
                {
                    self.application.set_normal_protection(protect);
                }
                ui.add_space(15.0);
                ui.strong("Song audio");
                if ui
                    .checkbox(&mut self.application.preferences.song_muted, "Mute song")
                    .changed()
                {
                    self.application.gains_changed();
                }
                ui.horizontal(|ui| {
                    ui.label("Song offset");
                    if ui
                        .add(
                            egui::DragValue::new(&mut self.application.song_offset)
                                .speed(1.0)
                                .range(-60_000.0..=60_000.0)
                                .suffix(" ms"),
                        )
                        .on_hover_text("Positive values delay the song without moving any hits")
                        .changed()
                    {
                        self.application.cycle_changed();
                    }
                });
                ui.add_space(15.0);
                ui.strong("History");
                if ui
                    .selectable_label(position == 0, "Opened difficulty")
                    .clicked()
                {
                    self.application.go_to_history(0);
                }
                for (i, label) in steps.iter().enumerate() {
                    if ui.selectable_label(position == i + 1, label).clicked() {
                        self.application.go_to_history(i + 1);
                    }
                }
            } else {
                ui.label("Open a difficulty to edit its hits.");
            }
            if !self.application.warnings.is_empty() {
                ui.add_space(15.0);
                egui::CollapsingHeader::new(format!(
                    "{} warnings",
                    self.application.warnings.len()
                ))
                .show(ui, |ui| {
                    for warning in &self.application.warnings {
                        ui.label(RichText::new(warning).color(MUTED));
                    }
                });
            }
        });
    }
    fn fit(&mut self, width: f32) {
        self.application.zoom =
            ((width - LABEL).max(200.0) as f64 / self.application.duration_ms()).clamp(0.005, 2.0);
        self.application.set_view_start(0.0);
    }
    fn timeline(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let position = self.application.position_ms();
            ui.label(RichText::new(format_time(position)).monospace().color(CYAN));
            if let Some(d) = self.application.document() {
                let time = SongTime::from_ms(position);
                let tempo = d.composer.tempo.section_at(time);
                let beat = d.composer.tempo.bar_beat_at(time);
                let tick = (beat.beat_fraction * 480.0).floor() as u32;
                ui.label(
                    RichText::new(format!(
                        "{}.{}.{:03}  ·  {:.1} BPM  ·  {}/4",
                        beat.bar,
                        beat.beat,
                        tick,
                        60_000.0 / tempo.beat_length_ms,
                        tempo.beats_per_bar
                    ))
                    .monospace()
                    .color(MUTED),
                );
            }
            ui.separator();
            if ui.small_button("−").clicked() {
                self.application.zoom = (self.application.zoom / 1.3).max(0.005);
            }
            if ui.small_button("+").clicked() {
                self.application.zoom = (self.application.zoom * 1.3).min(3.0);
            }
            if ui.small_button("Fit").clicked() {
                self.fit(ui.available_width() + 200.0);
            }
            ui.label("Zoom");
            ui.add(
                egui::Slider::new(&mut self.application.row_height, 20.0..=65.0)
                    .text("Lane height"),
            );
        });
        let Some(d) = self.application.document() else {
            return;
        };
        let start = d.composer.view_start;
        let scale = self.application.zoom;
        let width = ui.available_width();
        let visible = (width - LABEL).max(1.0) as f64 / scale;
        let header = ui.allocate_exact_size(Vec2::new(width, 94.0), Sense::click_and_drag());
        let rect = header.0;
        let grid = Rect::from_min_max(Pos2::new(rect.left() + LABEL, rect.top()), rect.max);
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, BG);
        painter.text(
            rect.left_top() + Vec2::new(12.0, 44.0),
            egui::Align2::LEFT_TOP,
            "Song audio",
            egui::FontId::proportional(13.0),
            MUTED,
        );
        for tick in d
            .composer
            .tempo
            .ruler_ticks(start, start + grid.width() as f64 / scale, scale)
        {
            let time = tick.ms();
            let x = grid.left() + ((time - start) * scale) as f32;
            painter.line_segment(
                [Pos2::new(x, grid.top()), Pos2::new(x, grid.bottom())],
                Stroke::new(1.0, Color32::from_rgb(56, 64, 76)),
            );
            painter.text(
                Pos2::new(x + 4.0, grid.top() + 6.0),
                egui::Align2::LEFT_TOP,
                format!(
                    "{}",
                    d.composer.tempo.bar_beat_at(SongTime::from_ms(time)).bar
                ),
                egui::FontId::monospace(11.0),
                MUTED,
            );
        }
        let duration = self.application.duration_ms();
        let bins = &self.application.waveform;
        for (i, bin) in bins.iter().enumerate() {
            let time = i as f64 * duration / bins.len().max(1) as f64;
            let x = grid.left() + ((time - start) * scale) as f32;
            if x < grid.left() || x > grid.right() {
                continue;
            }
            let y = grid.top() + 60.0;
            let h = bin.peak.min(1.0) * 27.0;
            painter.line_segment(
                [Pos2::new(x, y - h), Pos2::new(x, y + h)],
                Stroke::new(1.0, wave_colour(bin.low, bin.mid, bin.high)),
            );
        }
        for (time, name) in [
            (self.application.locator_left, "L"),
            (self.application.locator_right, "R"),
        ] {
            let x = grid.left() + ((time - start) * scale) as f32;
            if grid.x_range().contains(x) {
                let colour = Color32::from_rgb(208, 170, 96);
                painter.vline(x, grid.y_range(), Stroke::new(1.0, colour));
                painter.text(
                    Pos2::new(x + 3.0, grid.top() + 22.0),
                    egui::Align2::LEFT_TOP,
                    name,
                    egui::FontId::monospace(11.0),
                    colour,
                );
            }
        }
        let play_x = grid.left() + ((self.application.position_ms() - start) * scale) as f32;
        if grid.x_range().contains(play_x) {
            painter.vline(play_x, grid.y_range(), Stroke::new(1.5, CYAN));
        }
        if (header.1.clicked() || header.1.dragged())
            && header
                .1
                .interact_pointer_pos()
                .is_some_and(|p| p.x >= grid.left())
        {
            let p = header.1.interact_pointer_pos().unwrap();
            self.application
                .seek(start + (p.x - grid.left()) as f64 / scale);
        }
        let document = self.application.document().unwrap();
        let range = document.composer.range;
        let object_positions: Vec<_> = if self.application.preferences.object_lines {
            document
                .composer
                .hitsounds()
                .moments()
                .iter()
                .filter(|moment| moment.place() != dawsu_hitsounding::Place::NoObject)
                .map(|moment| ((moment.time().ms() - start) * scale) as f32)
                .filter(|offset| (0.0..=(width - LABEL)).contains(offset))
                .collect()
        } else {
            Vec::new()
        };
        let lanes = self.application.document().unwrap().composer.lanes.clone();
        let available = ui.available_height();
        let controller = if self.application.preferences.show_controller {
            85.0
        } else {
            0.0
        };
        egui::ScrollArea::vertical()
            .id_salt("lanes")
            .max_height((available - controller - 35.0).max(80.0))
            .show(ui, |ui| {
                for (row, lane) in lanes.iter().enumerate() {
                    let Some(d) = self.application.document() else {
                        break;
                    };
                    let used = d
                        .composer
                        .hitsounds()
                        .moments()
                        .iter()
                        .any(|m| m.has_hit(lane));
                    if self.application.preferences.used_only && !used {
                        continue;
                    }
                    if !lane_name(lane)
                        .to_lowercase()
                        .contains(&self.filter.to_lowercase())
                    {
                        continue;
                    }
                    let (r, response) = ui.allocate_exact_size(
                        Vec2::new(width, self.application.row_height),
                        Sense::click_and_drag(),
                    );
                    let timeline = Rect::from_min_max(Pos2::new(r.left() + LABEL, r.top()), r.max);
                    let p = ui.painter();
                    p.rect_filled(
                        r,
                        0.0,
                        if row % 2 == 0 {
                            Color32::from_rgb(29, 34, 42)
                        } else {
                            BG
                        },
                    );
                    p.hline(
                        r.x_range(),
                        r.bottom(),
                        Stroke::new(1.0, Color32::from_rgb(43, 49, 59)),
                    );
                    if self.application.preferences.lane_names {
                        p.text(
                            r.left_center() + Vec2::new(8.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            lane_name(lane),
                            egui::FontId::proportional(12.0),
                            if used { TEXT } else { MUTED },
                        );
                    }
                    let mute_rect = Rect::from_min_size(
                        Pos2::new(r.left() + 143.0, r.top() + 4.0),
                        Vec2::splat(21.0),
                    );
                    let solo_rect = mute_rect.translate(Vec2::new(23.0, 0.0));
                    if ui
                        .put(
                            mute_rect,
                            egui::Button::new("M").selected(self.application.is_muted(lane)),
                        )
                        .clicked()
                    {
                        self.application.mute(lane.clone());
                    }
                    if ui
                        .put(
                            solo_rect,
                            egui::Button::new("S").selected(self.application.is_solo(lane)),
                        )
                        .clicked()
                    {
                        self.application.solo(lane.clone());
                    }
                    let d = self.application.document().unwrap();
                    let p = ui.painter();
                    if let Some(range) = range {
                        let l = timeline.left()
                            + ((range.from.ms().min(range.to.ms()) - start) * scale) as f32;
                        let rr = timeline.left()
                            + ((range.from.ms().max(range.to.ms()) - start) * scale) as f32;
                        p.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(l.max(timeline.left()), r.top()),
                                Pos2::new(rr.min(r.right()), r.bottom()),
                            ),
                            0.0,
                            Color32::from_rgba_unmultiplied(70, 140, 155, 35),
                        );
                    }
                    for offset in &object_positions {
                        p.vline(
                            timeline.left() + offset,
                            r.y_range(),
                            Stroke::new(1.0, Color32::from_gray(65)),
                        );
                    }
                    for (i, m) in d.composer.hitsounds().moments().iter().enumerate() {
                        let x = timeline.left() + ((m.time().ms() - start) * scale) as f32;
                        if x < timeline.left() || x > timeline.right() {
                            continue;
                        }
                        if m.has_hit(lane) {
                            let hit = Rect::from_min_max(
                                Pos2::new(x - 3.0, r.top() + 5.0),
                                Pos2::new(x + 5.0, r.bottom() - 5.0),
                            );
                            let selected = d
                                .composer
                                .selected
                                .items()
                                .iter()
                                .any(|k| k.moment == i && &k.lane == lane);
                            p.rect_filled(hit, 1.0, volume_colour(m.volume()));
                            if selected {
                                p.rect_stroke(
                                    hit.expand(1.0),
                                    1.0,
                                    Stroke::new(1.5, Color32::WHITE),
                                    egui::StrokeKind::Outside,
                                );
                            }
                        }
                    }
                    if timeline.x_range().contains(play_x) {
                        p.vline(play_x, r.y_range(), Stroke::new(1.5, CYAN));
                    }
                    let pointer = ui.ctx().input(|i| i.pointer.interact_pos());
                    if let Some(pos) = pointer {
                        if response.hovered()
                            && pos.x < timeline.left()
                            && ui.ctx().input(|i| i.pointer.primary_clicked())
                        {
                            self.application.active_lane = row;
                            self.application.audition(lane);
                        }
                        if pos.x >= timeline.left()
                            && (response.hovered()
                                || self.pointer_down && r.y_range().contains(pos.y))
                        {
                            let down = ui.ctx().input(|i| i.pointer.primary_pressed());
                            let up = ui.ctx().input(|i| i.pointer.primary_released());
                            let moved = ui.ctx().input(|i| i.pointer.delta() != Vec2::ZERO);
                            let mut time = start + (pos.x - timeline.left()) as f64 / scale;
                            if down {
                                let d = self.application.document().unwrap();
                                if let Some(m) = d
                                    .composer
                                    .hitsounds()
                                    .moments()
                                    .iter()
                                    .filter(|m| {
                                        m.has_hit(lane)
                                            && (m.time().ms() - time).abs() * scale <= 6.0
                                    })
                                    .min_by(|a, b| {
                                        (a.time().ms() - time)
                                            .abs()
                                            .total_cmp(&(b.time().ms() - time).abs())
                                    })
                                {
                                    time = m.time().ms();
                                }
                            }
                            let mods = ui.ctx().input(|i| mods(i.modifiers));
                            let input = PointerInput {
                                time: SongTime::from_ms(time),
                                row: row as i32,
                                row_value: None,
                                surface: PointerSurface::Grid,
                                button: PointerButton::Left,
                                mods,
                                click_count: if response.double_clicked() { 2 } else { 1 },
                                x: pos.x as f64,
                                y: pos.y as f64,
                            };
                            if down {
                                self.pointer_down = true;
                                self.application.pointer(PointerPhase::Down, input);
                            } else if up && self.pointer_down {
                                self.application.pointer(PointerPhase::Up, input);
                                self.pointer_down = false;
                            } else if moved && self.pointer_down {
                                self.application.pointer(PointerPhase::Move, input);
                            }
                        }
                    }
                    response.context_menu(|ui| {
                        for tool in ToolKind::ALL {
                            if ui.button(format!("{tool:?}")).clicked() {
                                self.application.set_tool(tool);
                                ui.close();
                            }
                        }
                    });
                }
            });
        if self.pointer_down && ui.ctx().input(|i| i.pointer.primary_released()) {
            self.application.pointer(
                PointerPhase::Up,
                PointerInput::at(
                    self.application.position_ms(),
                    self.application.active_lane as i32,
                ),
            );
            self.pointer_down = false;
        }
        if self.application.preferences.show_controller {
            self.controller(ui, start, scale, width);
        }
        if self.controller_down && ui.ctx().input(|i| i.pointer.primary_released()) {
            let mut input = PointerInput::at(
                self.application.position_ms(),
                self.application.active_lane as i32,
            );
            input.surface = PointerSurface::ValueLane;
            self.application.pointer(PointerPhase::Up, input);
            self.controller_down = false;
        }
        self.overview(ui, start, visible, width);
        if ui.rect_contains_pointer(ui.max_rect()) {
            let scroll = ui.ctx().input(|i| i.smooth_scroll_delta);
            if ui.ctx().input(|i| i.modifiers.ctrl) && scroll.y != 0.0 {
                self.application.zoom =
                    (self.application.zoom * (scroll.y as f64 * 0.005).exp()).clamp(0.005, 3.0);
            }
        }
        if self.application.preferences.follow && self.application.is_playing() {
            let pos = self.application.position_ms();
            let visible = (width - LABEL) as f64 / self.application.zoom;
            if pos > start + visible * 0.85 {
                self.application
                    .set_view_start((pos - visible * 0.2).max(0.0));
            }
        }
    }
    fn overview(&mut self, ui: &mut egui::Ui, start: f64, visible: f64, width: f32) {
        let (rect, input) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click_and_drag());
        let painter = ui.painter();
        painter.rect_filled(rect, 2.0, BG);
        let duration = self.application.duration_ms().max(1.0);
        let time_x =
            |time: f64| rect.left() + (time / duration).clamp(0.0, 1.0) as f32 * rect.width();
        let waveform = &self.application.waveform;
        let columns = (rect.width() as usize).max(1);
        let chunk_size = waveform.len().div_ceil(columns).max(1);
        for (column, chunk) in waveform.chunks(chunk_size).enumerate() {
            let peak = chunk
                .iter()
                .map(|bin| bin.peak)
                .fold(0.0, f32::max)
                .min(1.0);
            let time = (column * chunk_size) as f64 / waveform.len() as f64 * duration;
            let height = peak * rect.height() * 0.45;
            let x = time_x(time);
            painter.vline(
                x,
                rect.center().y - height..=rect.center().y + height,
                Stroke::new(1.0, MUTED),
            );
        }
        let viewport = Rect::from_min_max(
            Pos2::new(time_x(start), rect.top()),
            Pos2::new(time_x(start + visible), rect.bottom()),
        );
        painter.rect_filled(
            viewport,
            1.0,
            Color32::from_rgba_unmultiplied(91, 207, 225, 30),
        );
        painter.rect_stroke(
            viewport,
            1.0,
            Stroke::new(1.0, CYAN),
            egui::StrokeKind::Inside,
        );
        painter.vline(
            time_x(self.application.position_ms()),
            rect.y_range(),
            Stroke::new(1.0, TEXT),
        );
        if (input.clicked() || input.dragged())
            && let Some(pos) = input.interact_pointer_pos()
        {
            let fraction = (pos.x - rect.left()) as f64 / rect.width().max(1.0) as f64;
            self.application
                .set_view_start(overview_start(fraction, duration, visible));
        }
        input.on_hover_text("Click or drag to navigate the whole difficulty");
    }

    fn controller(&mut self, ui: &mut egui::Ui, start: f64, scale: f64, width: f32) {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, 80.0), Sense::click_and_drag());
        let grid = Rect::from_min_max(Pos2::new(rect.left() + LABEL, rect.top()), rect.max);
        let p = ui.painter();
        p.rect_filled(rect, 0.0, BG);
        p.text(
            rect.left_center() + Vec2::new(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "Volume",
            egui::FontId::proportional(12.0),
            MUTED,
        );
        if let Some(d) = self.application.document() {
            for m in d.composer.hitsounds().moments() {
                let x = grid.left() + ((m.time().ms() - start) * scale) as f32;
                if !grid.x_range().contains(x) {
                    continue;
                }
                let h = m.volume() as f32 / 100.0 * (grid.height() - 12.0);
                p.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(x - 2.0, grid.bottom() - h),
                        Pos2::new(x + 2.0, grid.bottom()),
                    ),
                    0.0,
                    volume_colour(m.volume()),
                );
            }
        }
        if let Some(pos) = response.interact_pointer_pos() {
            if pos.x >= grid.left() {
                let down = ui.ctx().input(|i| i.pointer.primary_pressed());
                let up = ui.ctx().input(|i| i.pointer.primary_released());
                let input = PointerInput {
                    time: SongTime::from_ms(start + (pos.x - grid.left()) as f64 / scale),
                    row: self.application.active_lane as i32,
                    row_value: Some(
                        ((grid.bottom() - pos.y) / grid.height() * 100.0).clamp(0.0, 100.0) as f64,
                    ),
                    surface: PointerSurface::ValueLane,
                    button: PointerButton::Left,
                    mods: ui.ctx().input(|i| mods(i.modifiers)),
                    click_count: 1,
                    x: pos.x as f64,
                    y: pos.y as f64,
                };
                if down {
                    self.controller_down = true;
                    self.application.pointer(PointerPhase::Down, input);
                } else if up {
                    self.application.pointer(PointerPhase::Up, input);
                    self.controller_down = false;
                } else if response.dragged() {
                    self.application.pointer(PointerPhase::Move, input);
                }
            }
        }
    }
    fn transport(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            if ui.button("|◀").on_hover_text("Go to start").clicked() {
                self.application.seek(0.0);
            }
            if ui
                .add_sized(
                    [62.0, 32.0],
                    egui::Button::new(if self.application.is_playing() {
                        "Pause"
                    } else {
                        "Play"
                    })
                    .fill(Color32::from_rgb(39, 87, 100)),
                )
                .clicked()
            {
                self.application.play_pause();
            }
            if ui.button("Stop").clicked() {
                self.application.stop();
            }
            ui.label(
                RichText::new(format_time(self.application.position_ms()))
                    .monospace()
                    .size(19.0),
            );
            ui.separator();
            if ui.checkbox(&mut self.application.cycle, "Cycle").changed() {
                self.application.cycle_changed();
            }
            if ui.small_button("L").clicked() {
                self.application.locator_left = self.application.position_ms();
                self.application.cycle_changed();
            }
            if ui.small_button("R").clicked() {
                self.application.locator_right = self.application.position_ms();
                self.application.cycle_changed();
            }
            ui.separator();
            let mut speed = self.application.playback_speed;
            egui::ComboBox::from_id_salt("speed")
                .selected_text(format!("{:.0}%", speed * 100.0))
                .width(65.0)
                .show_ui(ui, |ui| {
                    for value in [0.25, 0.5, 0.75, 1.0, 1.25, 1.5] {
                        ui.selectable_value(&mut speed, value, format!("{:.0}%", value * 100.0));
                    }
                });
            if speed != self.application.playback_speed {
                self.application.set_speed(speed);
            }
            ui.separator();
            let song = ui
                .add(
                    egui::Slider::new(&mut self.application.preferences.song_gain, 0.0..=1.0)
                        .text("Song")
                        .max_decimals(2),
                )
                .changed();
            let master = ui
                .add(
                    egui::Slider::new(&mut self.application.preferences.master_gain, 0.2..=1.0)
                        .text("Main")
                        .max_decimals(2),
                )
                .changed();
            if song || master {
                self.application.gains_changed();
            }
            if self.application.busy {
                ui.spinner();
                if ui.small_button("Cancel").clicked() {
                    self.application.cancel_job();
                }
            }
            ui.label(
                RichText::new(&self.application.status)
                    .size(11.0)
                    .color(MUTED),
            );
        });
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                repeat: false,
                ..
            } = event
            {
                if key == egui::Key::Escape {
                    self.application.stop();
                    self.application
                        .pointer(PointerPhase::Cancel, PointerInput::at(0.0, 0));
                    self.pointer_down = false;
                    self.controller_down = false;
                }
                if let Some(logical) = kernel_key(key) {
                    let physical = physical_key.and_then(kernel_key).unwrap_or(logical);
                    self.application.key(KeyChord {
                        key: logical,
                        physical,
                        mods: mods(modifiers),
                    });
                }
            }
        }
    }
    fn overlays(&mut self, ctx: &egui::Context) {
        if let Some(error) = self.application.error.clone() {
            egui::Window::new("Action could not complete")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(error);
                    if ui.button("Keep editing").clicked() {
                        self.application.error = None;
                    }
                });
        }
        if self.application.revert_requested {
            egui::Window::new("Revert to last saved?")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Discard local edits and session settings, then reopen the saved difficulty and session from disk.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.application.revert_requested = false;
                        }
                        if ui.button("Revert difficulty").clicked() {
                            self.application.confirm_revert();
                        }
                    });
                });
        }
        if self.application.pending_open.is_some() && !self.application.busy {
            egui::Window::new("Keep your unsaved edits?")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("This difficulty has unsaved changes.");
                    ui.horizontal(|ui| {
                        if ui.button("Save and continue").clicked() {
                            self.application.save();
                        }
                        if ui.button("Discard and continue").clicked() {
                            self.application.discard_and_open();
                        }
                        if ui.button("Cancel").clicked() {
                            self.application.pending_open = None;
                        }
                    });
                });
        }
        if self.confirm_close {
            egui::Window::new("Save before closing?")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Your open difficulty has unsaved edits.");
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            self.application.save();
                        }
                        if !self.application.is_dirty() && !self.application.busy {
                            self.allow_close = true;
                            self.confirm_close = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Close without saving").clicked() {
                            self.allow_close = true;
                            self.confirm_close = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_close = false;
                        }
                    });
                });
        }
        if self.application.first_launch {
            egui::Window::new("Choose your keyboard preset")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("You can change these keys in Preferences later.");
                    ui.horizontal(|ui| {
                        if ui.button("DAW-like").clicked() {
                            self.application.choose_key_preset("DAW");
                        }
                        if ui.button("osu!").clicked() {
                            self.application.choose_key_preset("osu!");
                        }
                    });
                });
        }
        if self.show_new_diff {
            egui::Window::new("New hitsound difficulty")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(
                        "Name the new, empty difficulty. The current difficulty stays unchanged.",
                    );
                    ui.text_edit_singleline(&mut self.new_diff_name);
                    ui.horizontal(|ui| {
                        if ui.button("Review new difficulty").clicked() {
                            self.application.new_diff(self.new_diff_name.clone());
                            self.show_new_diff = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_new_diff = false;
                        }
                    });
                });
        }
        if self.show_copy {
            egui::Window::new("Copy hitsounds")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label("Choose the difficulties this action will write.");
                    if let Some(d) = self.application.document() {
                        let paths = d.difficulties.clone();
                        let current = d.path.clone();
                        for path in paths {
                            if path == current {
                                continue;
                            }
                            let mut selected = self.copy_targets.contains(&path);
                            if ui
                                .checkbox(
                                    &mut selected,
                                    path.file_name().unwrap().to_string_lossy(),
                                )
                                .changed()
                            {
                                if selected {
                                    self.copy_targets.push(path);
                                } else {
                                    self.copy_targets.retain(|p| p != &path);
                                }
                            }
                        }
                    }
                    ui.separator();
                    ui.checkbox(&mut self.copy_options.copy_hitsounds, "Copy hitsounds");
                    ui.checkbox(
                        &mut self.copy_options.copy_body_hitsounds,
                        "Copy slider body sounds",
                    );
                    ui.checkbox(
                        &mut self.copy_options.copy_sample_sets,
                        "Copy sample sets and indexes",
                    );
                    ui.checkbox(&mut self.copy_options.copy_volumes, "Copy volumes");
                    ui.horizontal(|ui| {
                        ui.label("Matching tolerance");
                        ui.add(
                            egui::DragValue::new(&mut self.copy_options.leniency)
                                .range(0.0..=1000.0)
                                .suffix(" ms"),
                        );
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Review Copy").clicked() {
                            self.application.copy_hitsounds(
                                self.copy_targets.clone(),
                                self.copy_options.clone(),
                            );
                            self.show_copy = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_copy = false;
                        }
                    });
                });
        }
        if let Some(plan) = self.application.output_review.as_ref() {
            let title = plan.label.clone();
            let files: Vec<_> = plan
                .files
                .iter()
                .map(|f| {
                    (
                        f.path.display().to_string(),
                        f.expected.is_some(),
                        f.file.change_summary(),
                    )
                })
                .collect();
            let details = plan.details.clone();
            let mixes = plan.mix_paths().map(|path| path.display().to_string()).collect::<Vec<_>>();
            egui::Window::new(title)
                .collapsible(false)
                .default_width(650.0)
                .show(ctx, |ui| {
                    ui.strong("Files this action will write");
                    for (path, overwrite, summary) in &files {
                        ui.label(format!("{} {path}", if *overwrite { "Replace:" } else { "Create:" }));
                        let timing = summary.green_lines_added + summary.green_lines_edited
                            + summary.green_lines_removed + summary.red_lines_changed;
                        ui.label(RichText::new(format!(
                            "{} objects changed · {} added · {} removed · {timing} timing lines affected",
                            summary.hit_objects_changed, summary.hit_objects_added, summary.hit_objects_removed,
                        )).size(11.0).color(MUTED));
                    }
                    if !mixes.is_empty() {
                        ui.strong("Sample mixes");
                        for (index, path) in mixes.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.button("Audition mix").clicked()
                                    && let Err(error) = self.application.audition_output_mix(index)
                                {
                                    self.application.error = Some(error);
                                }
                                ui.label(path);
                            });
                        }
                    }
                    for detail in details {
                        ui.label(detail);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Write these files").clicked() {
                            self.application.apply_output();
                        }
                        if ui.button("Cancel").clicked() {
                            self.application.output_review = None;
                        }
                    });
                });
        }
        if self.show_settings {
            let mut open = true;
            egui::Window::new("Preferences")
                .open(&mut open)
                .default_width(480.0)
                .show(ctx, |ui| {
                    ui.heading("Audio");
                    ui.label(self.application.device_name());
                    let devices = self.application.devices();
                    for device in devices {
                        if ui
                            .selectable_label(device == self.application.device_name(), &device)
                            .clicked()
                        {
                            self.application.select_device(device);
                        }
                    }
                    ui.checkbox(
                        &mut self.application.preferences.return_on_stop,
                        "Return to starting position on Stop",
                    );
                    ui.separator();
                    ui.strong("Folders");
                    ui.label(format!(
                        "Default samples: {}",
                        self.application.preferences.default_samples.display()
                    ));
                    if ui.button("Choose default sample folder…").clicked() {
                        self.application.requests.push(PlatformRequest::OpenSamples);
                    }
                    if ui.button("Choose Songs folder…").clicked() {
                        self.application.requests.push(PlatformRequest::OpenSongs);
                    }
                    ui.separator();
                    if ui.button("Save preferences").clicked() {
                        self.application.save_preferences();
                    }
                });
            self.show_settings = open;
        }
        if self.show_help {
            let mut open = true;
            egui::Window::new("Shortcuts and mouse actions")
                .open(&mut open)
                .default_size([590.0, 500.0])
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for group in self.application.help() {
                            ui.strong(group.title);
                            for entry in group.entries {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(entry.keys).monospace().color(CYAN));
                                    ui.label(entry.description);
                                });
                            }
                            ui.separator();
                        }
                    });
                });
            self.show_help = open;
        }
    }
}
fn mods(m: egui::Modifiers) -> Mods {
    let mut result = Mods::NONE;
    if m.ctrl || m.command {
        result = result.union(Mods::CTRL);
    }
    if m.shift {
        result = result.union(Mods::SHIFT);
    }
    if m.alt {
        result = result.union(Mods::ALT);
    }
    result
}
pub fn format_time(ms: f64) -> String {
    let t = ms.max(0.0) as u64;
    format!("{:02}:{:02}.{:03}", t / 60000, t / 1000 % 60, t % 1000)
}
fn volume_colour(v: i32) -> Color32 {
    let t = v.clamp(0, 100) as f32 / 100.0;
    Color32::from_rgb(
        (78.0 + 171.0 * t) as u8,
        (145.0 - 65.0 * t) as u8,
        (235.0 - 125.0 * t) as u8,
    )
}

fn kernel_key(key: egui::Key) -> Option<Key> {
    Some(match key {
        egui::Key::A => Key::A,
        egui::Key::B => Key::B,
        egui::Key::C => Key::C,
        egui::Key::D => Key::D,
        egui::Key::E => Key::E,
        egui::Key::F => Key::F,
        egui::Key::G => Key::G,
        egui::Key::H => Key::H,
        egui::Key::I => Key::I,
        egui::Key::J => Key::J,
        egui::Key::K => Key::K,
        egui::Key::L => Key::L,
        egui::Key::M => Key::M,
        egui::Key::N => Key::N,
        egui::Key::O => Key::O,
        egui::Key::P => Key::P,
        egui::Key::Q => Key::Q,
        egui::Key::R => Key::R,
        egui::Key::S => Key::S,
        egui::Key::T => Key::T,
        egui::Key::U => Key::U,
        egui::Key::V => Key::V,
        egui::Key::W => Key::W,
        egui::Key::X => Key::X,
        egui::Key::Y => Key::Y,
        egui::Key::Z => Key::Z,
        egui::Key::Num0 => Key::D0,
        egui::Key::Num1 => Key::D1,
        egui::Key::Num2 => Key::D2,
        egui::Key::Num3 => Key::D3,
        egui::Key::Num4 => Key::D4,
        egui::Key::Num5 => Key::D5,
        egui::Key::Num6 => Key::D6,
        egui::Key::Num7 => Key::D7,
        egui::Key::Num8 => Key::D8,
        egui::Key::Num9 => Key::D9,
        egui::Key::ArrowLeft => Key::Left,
        egui::Key::ArrowRight => Key::Right,
        egui::Key::ArrowUp => Key::Up,
        egui::Key::ArrowDown => Key::Down,
        egui::Key::Space => Key::Space,
        egui::Key::Delete => Key::Delete,
        egui::Key::Backspace => Key::Backspace,
        egui::Key::Escape => Key::Escape,
        egui::Key::F1 => Key::F1,
        egui::Key::Home => Key::Home,
        egui::Key::End => Key::End,
        egui::Key::Slash => Key::Slash,
        _ => return None,
    })
}

fn wave_colour(low: f32, mid: f32, high: f32) -> Color32 {
    let sum = (low + mid + high).max(0.0001);
    Color32::from_rgb(
        (65.0 + 165.0 * low / sum) as u8,
        (90.0 + 145.0 * mid / sum) as u8,
        (100.0 + 145.0 * high / sum) as u8,
    )
}

/// View start for a point on the overview strip.
pub fn overview_start(fraction: f64, duration: f64, visible: f64) -> f64 {
    (fraction * duration - visible / 2.0).clamp(0.0, (duration - visible).max(0.0))
}
