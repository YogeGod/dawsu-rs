use super::{Application, Document, PlatformRequest};
use dawsu_editor::commands::{CommandCatalog, CommandDefinition, CommandRegistry, MouseAction, areas};
use dawsu_editor::editing_commands as kernel;
use dawsu_editor::keys::{Key, KeyCombination as K, Mods};
use dawsu_editor::{tempo::SnapMode, tools::ToolKind};
use dawsu_foundation::SongTime;
use dawsu_hitsounding::Place;
use std::time::{Duration, Instant};

fn can_edit(app: &Application) -> bool {
    app.document().is_some() && !app.busy && !app.is_stroke_open()
}

fn can_edit_selection(app: &Application) -> bool {
    can_edit(app) && app.document().is_some_and(|d| !d.composer.selected.is_empty())
}

fn select_across_lanes(app: &mut Application, direction: isize) -> Result<(), String> {
    if let Some((path, rows, span)) = app.key_lane_view.take()
        && let Some(document) = app.document.as_mut()
        && document.path == path
        && span.is_finite()
        && span > 0.0
    {
        document.composer.select_across_lanes(&rows, direction, span / 2.0);
    }
    Ok(())
}

fn selection_start(document: &Document) -> Option<f64> {
    let composer = &document.composer;
    let moments = composer.hitsounds().moments();
    composer.selected.items().iter().filter_map(|hit| {
        let moment = moments.get(hit.moment)?;
        moment.has_hit(&hit.lane).then(|| moment.time().ms())
    }).min_by(f64::total_cmp)
}

fn align_selection(app: &mut Application, direction: i32) -> Result<(), String> {
    let document = app.document.as_mut().ok_or("Open a difficulty first")?;
    let Some(start) = selection_start(document) else {
        return Ok(());
    };
    let composer = &mut document.composer;
    let moments = composer.hitsounds().moments();
    let target = if composer.snap_mode == SnapMode::Objects {
        let object_times = moments.iter().filter(|moment| moment.place() != Place::NoObject)
            .map(|moment| moment.time().ms());
        if direction < 0 {
            object_times.filter(|&time| time < start).max_by(f64::total_cmp)
        } else {
            object_times.filter(|&time| time > start).min_by(f64::total_cmp)
        }
    } else {
        Some(composer.tempo.grid_step(SongTime::from_ms(start), direction, composer.spacing).ms())
    };
    if let Some(target) = target.filter(|&time| time != start) {
        // The first hit determines the shift. A nearby second object must not alter that shift.
        composer.move_selected(target - start, None, 0.0);
    }
    Ok(())
}

fn jump_to_endpoint(app: &mut Application, first: bool) -> Result<(), String> {
    let now = Instant::now();
    let previous = app.last_endpoint_jump.replace((first, now));
    let boundary = if first { 0.0 } else { app.duration_ms() };
    if previous.is_some_and(|(direction, at)| {
        direction == first && now.duration_since(at) <= Duration::from_millis(400)
    }) {
        app.last_endpoint_jump = None;
        app.seek(boundary);
        return Ok(());
    }

    let document = app.document().ok_or("Open a difficulty first")?;
    let times = document.composer.hitsounds().moments().iter()
        .filter(|moment| moment.place() != Place::NoObject || moment.hits().next().is_some())
        .map(|moment| moment.time().ms());
    let endpoint = if first { times.min_by(f64::total_cmp) } else { times.max_by(f64::total_cmp) };
    app.seek(endpoint.unwrap_or(boundary));
    Ok(())
}

fn request_edit(app: &mut Application, id: &'static str) -> Result<(), String> {
    app.edit_dialog = Some(id);
    Ok(())
}

pub fn catalogue() -> CommandCatalog<Application> {
    let mut registry = CommandRegistry::<Application>::new();
    macro_rules! doc {
        ($definition:expr,$d:ident,$body:block) => {
            registry.bind(
                &$definition,
                areas::HS_HIT_EDITING,
                |app: &mut Application| {
                    let $d: &mut Document =
                        app.document.as_mut().ok_or("Open a difficulty first")?;
                    $body;
                    Ok(())
                },
                Some(|app| app.document.is_some() && !app.busy),
            );
        };
    }
    registry.bind(
        &kernel::undo(),
        areas::HS_HIT_EDITING,
        |app| {
            let slot = app.active_lane_slot();
            app.document.as_mut().ok_or("Open a difficulty first")?.composer.undo();
            app.restore_active_lane(slot);
            Ok(())
        },
        Some(|app| app.document.is_some() && !app.busy),
    );
    registry.bind(
        &kernel::redo(),
        areas::HS_HIT_EDITING,
        |app| {
            let slot = app.active_lane_slot();
            app.document.as_mut().ok_or("Open a difficulty first")?.composer.redo();
            app.restore_active_lane(slot);
            Ok(())
        },
        Some(|app| app.document.is_some() && !app.busy),
    );
    doc!(kernel::copy(), d, { d.composer.copy() });
    doc!(kernel::cut(), d, {
        d.composer.copy();
        d.composer.remove_selected()
    });
    doc!(kernel::delete(), d, { d.composer.remove_selected() });
    doc!(kernel::select_all(), d, { d.composer.select_all() });
    doc!(kernel::deselect(), d, {
        d.composer.selected.clear();
        d.composer.range = None;
    });
    registry.bind(
        &kernel::paste(),
        areas::HS_HIT_EDITING,
        |app| {
            let at = app.position_ms();
            if let Some(d) = app.document.as_mut() {
                d.composer.paste(at);
            }
            Ok(())
        },
        Some(|app| app.document.is_some() && !app.busy),
    );
    doc!(kernel::duplicate(), d, { d.composer.duplicate() });
    registry.bind(
        &CommandDefinition::new("quantize", "Quantize selected hits").keys(&[K::plain(Key::Q)]),
        areas::HS_HIT_EDITING,
        |app| {
            if let Some(doc) = app.document.as_mut()
                && !doc.composer.quantize_with_strength(app.quantize_strength)
            {
                app.status = "Select off-grid loose hits to quantize".into();
            }
            Ok(())
        },
        Some(|app| app.document.is_some() && !app.busy && !app.is_stroke_open()),
    );
    registry.bind(
        &CommandDefinition::new("quantizeDialog", "Quantize with a chosen strength"),
        areas::HS_HIT_EDITING,
        |app| request_edit(app, "quantize"),
        Some(can_edit_selection),
    );
    registry.bind(
        &CommandDefinition::new("repeat", "Repeat selected hits").keys(&[K::new(Key::R, Mods::CTRL)]),
        areas::HS_HIT_EDITING,
        |app| request_edit(app, "repeat"),
        Some(can_edit_selection),
    );
    registry.bind(
        &CommandDefinition::new("editRange", "Copy, move or delete hits in the marked range"),
        areas::HS_HIT_EDITING,
        |app| request_edit(app, "range"),
        Some(|app| can_edit(app) && app.document().is_some_and(|d| d.composer.range.is_some())),
    );
    registry.bind(
        &CommandDefinition::new("scaleByVolume", "Scale hits in a volume range"),
        areas::HS_HIT_EDITING,
        |app| request_edit(app, "scale"),
        Some(can_edit),
    );
    registry.bind(
        &CommandDefinition::new("volumeUp", "Increase volume by 5")
            .keys(&[K::new(Key::Up, Mods::ALT)]),
        areas::HS_HIT_EDITING,
        |app| {
            app.selection_volume(5, true);
            Ok(())
        },
        Some(can_edit),
    );
    registry.bind(
        &CommandDefinition::new("volumeDown", "Decrease volume by 5")
            .keys(&[K::new(Key::Down, Mods::ALT)]),
        areas::HS_HIT_EDITING,
        |app| {
            app.selection_volume(-5, true);
            Ok(())
        },
        Some(can_edit),
    );
    let fine_volume = Mods::ALT.union(Mods::SHIFT);
    registry.bind(
        &CommandDefinition::new("volumeUpFine", "Increase selected volume by 1%")
            .keys(&[K::new(Key::Up, fine_volume)]),
        areas::HS_VOLUME,
        |app| {
            app.selection_volume(1, true);
            Ok(())
        },
        Some(can_edit_selection),
    );
    registry.bind(
        &CommandDefinition::new("volumeDownFine", "Decrease selected volume by 1%")
            .keys(&[K::new(Key::Down, fine_volume)]),
        areas::HS_VOLUME,
        |app| {
            app.selection_volume(-1, true);
            Ok(())
        },
        Some(can_edit_selection),
    );
    registry.bind(
        &CommandDefinition::new("jumpToSelection", "Move playhead to the start of the selection")
            .keys(&[K::new(Key::Space, Mods::ALT)]),
        areas::HS_TIMELINE,
        |app| {
            if let Some(start) = app.document().and_then(selection_start) {
                app.seek(start);
            }
            Ok(())
        },
        Some(can_edit_selection),
    );
    doc!(
        CommandDefinition::new("nudgeLeft", "Move hits to the previous grid step")
            .legacy(&["moveLeft"])
            .keys(&[K::new(Key::Left, Mods::CTRL)]),
        d,
        { d.composer.nudge(-1) }
    );
    doc!(
        CommandDefinition::new("nudgeRight", "Move hits to the next grid step")
            .legacy(&["moveRight"])
            .keys(&[K::new(Key::Right, Mods::CTRL)]),
        d,
        { d.composer.nudge(1) }
    );
    let align_mods = Mods::CTRL.union(Mods::ALT);
    registry.bind(
        &CommandDefinition::new("gridLeft", "Align selection to the previous grid line")
            .keys(&[K::new(Key::Left, align_mods)]),
        areas::HS_HIT_EDITING,
        |app| align_selection(app, -1),
        Some(can_edit_selection),
    );
    registry.bind(
        &CommandDefinition::new("gridRight", "Align selection to the next grid line")
            .keys(&[K::new(Key::Right, align_mods)]),
        areas::HS_HIT_EDITING,
        |app| align_selection(app, 1),
        Some(can_edit_selection),
    );
    doc!(
        CommandDefinition::new("navUp", "Move selected hits to the lane above")
            .legacy(&["laneUp"])
            .keys(&[K::plain(Key::Up), K::new(Key::Up, align_mods)]),
        d,
        { d.composer.move_rows(-1) }
    );
    doc!(
        CommandDefinition::new("navDown", "Move selected hits to the lane below")
            .legacy(&["laneDown"])
            .keys(&[K::plain(Key::Down), K::new(Key::Down, align_mods)]),
        d,
        { d.composer.move_rows(1) }
    );
    doc!(
        CommandDefinition::new("selectPrevious", "Select the previous hit")
            .keys(&[K::plain(Key::Left)])
            .osu(&[]),
        d,
        { d.composer.select_neighbour(-1, false) }
    );
    doc!(
        CommandDefinition::new("selectNext", "Select the next hit")
            .keys(&[K::plain(Key::Right)])
            .osu(&[]),
        d,
        { d.composer.select_neighbour(1, false) }
    );
    doc!(
        CommandDefinition::new("addPrevious", "Add the previous hit to the selection")
            .keys(&[K::new(Key::Left, Mods::SHIFT)])
            .osu(&[]),
        d,
        { d.composer.select_neighbour(-1, true) }
    );
    doc!(
        CommandDefinition::new("addNext", "Add the next hit to the selection")
            .keys(&[K::new(Key::Right, Mods::SHIFT)])
            .osu(&[]),
        d,
        { d.composer.select_neighbour(1, true) }
    );
    registry.bind(
        &CommandDefinition::new("navAddUp", "Add the nearest hit in the lane above")
            .keys(&[K::new(Key::Up, Mods::SHIFT)]),
        areas::HS_HIT_EDITING,
        |app| select_across_lanes(app, -1),
        Some(can_edit_selection),
    );
    registry.bind(
        &CommandDefinition::new("navAddDown", "Add the nearest hit in the lane below")
            .keys(&[K::new(Key::Down, Mods::SHIFT)]),
        areas::HS_HIT_EDITING,
        |app| select_across_lanes(app, 1),
        Some(can_edit_selection),
    );
    registry.bind(
        &CommandDefinition::new("addNumberedLayer", "Add a numbered sample layer"),
        areas::HS_LANES,
        Application::add_numbered_layer,
        Some(|app| app.document.is_some() && !app.busy),
    );
    registry.bind(
        &CommandDefinition::new("rowsOut", "Smaller lanes").keys(&[K::new(Key::Up, Mods::CTRL)]),
        areas::HS_LANES,
        |a| {
            a.row_height = (a.row_height - 2.0).max(20.0);
            Ok(())
        },
        None,
    );
    registry.bind(
        &CommandDefinition::new("rowsIn", "Larger lanes").keys(&[K::new(Key::Down, Mods::CTRL)]),
        areas::HS_LANES,
        |a| {
            a.row_height = (a.row_height + 2.0).min(65.0);
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::zoom_in(),
        areas::HS_TIMELINE,
        |a| {
            a.zoom = (a.zoom * 1.3).min(3.0);
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::zoom_out(),
        areas::HS_TIMELINE,
        |a| {
            a.zoom = (a.zoom / 1.3).max(crate::MIN_ZOOM);
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::seek_back(),
        areas::HS_TIMELINE,
        |a| {
            a.grid_seek(-1);
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::seek_forward(),
        areas::HS_TIMELINE,
        |a| {
            a.grid_seek(1);
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::seek_back_far(),
        areas::HS_TIMELINE,
        |app| {
            app.grid_seek_steps(-1, 4);
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::seek_forward_far(),
        areas::HS_TIMELINE,
        |app| {
            app.grid_seek_steps(1, 4);
            Ok(())
        },
        None,
    );
    let can_bookmark: Option<fn(&Application) -> bool> = Some(|app| app.document.is_some() && !app.busy);
    registry.bind(
        &kernel::add_bookmark(),
        areas::HS_TIMELINE,
        |app| app.add_bookmark(None),
        can_bookmark,
    );
    registry.bind(
        &kernel::remove_bookmark(),
        areas::HS_TIMELINE,
        |app| app.remove_bookmark(app.position_ms()),
        can_bookmark,
    );
    registry.bind(
        &kernel::previous_bookmark(),
        areas::HS_TIMELINE,
        |app| {
            app.step_bookmark(false);
            Ok(())
        },
        can_bookmark,
    );
    registry.bind(
        &kernel::next_bookmark(),
        areas::HS_TIMELINE,
        |app| {
            app.step_bookmark(true);
            Ok(())
        },
        can_bookmark,
    );
    registry.bind_family(
        &kernel::SNAP_TO,
        areas::HS_TIMELINE,
        |app, division| {
            let enabled = app.document().is_some_and(|d| d.composer.snap_enabled);
            app.set_snap(enabled, division);
            Ok(())
        },
        Some(|app, _| app.document().is_some() && !app.busy),
    );
    doc!(kernel::snap_toggle(), d, {
        d.composer.snap_enabled = !d.composer.snap_enabled
    });
    registry.bind(
        &CommandDefinition::new("open", "Open difficulty or mapset")
            .keys(&[K::new(Key::O, Mods::CTRL)]),
        areas::HS_MAP_FILES,
        |app| {
            app.requests.push(PlatformRequest::OpenMap);
            Ok(())
        },
        None,
    );
    registry.bind(
        &CommandDefinition::new("importOsz", "Import an .osz archive"),
        areas::HS_MAP_FILES,
        |app| request_edit(app, "importOsz"),
        Some(|app| !app.busy && !app.is_stroke_open()),
    );
    registry.bind(
        &CommandDefinition::new("openHitsoundFolder", "Open the hitsound library folder"),
        areas::HS_MAP_FILES,
        |app| request_edit(app, "openHitsoundFolder"),
        Some(|app| !app.busy),
    );
    registry.bind(
        &CommandDefinition::new("reviewOutput", "Open export settings"),
        areas::HS_MAP_OUTPUT,
        |app| request_edit(app, "reviewOutput"),
        Some(can_edit),
    );
    registry.bind(
        &CommandDefinition::new("filter", "Filter lanes by name")
            .keys(&[K::new(Key::F, Mods::CTRL)]),
        areas::HS_LANES,
        |app| request_edit(app, "filter"),
        Some(can_edit),
    );
    registry.bind(
        &CommandDefinition::new("newDiff", "New hitsound difficulty")
            .keys(&[K::new(Key::N, Mods::CTRL)]),
        areas::HS_MAP_FILES,
        |app| request_edit(app, "newDiff"),
        Some(can_edit),
    );
    registry.bind(
        &CommandDefinition::new("openFolder", "Open the map folder")
            .keys(&[K::new(Key::O, Mods::CTRL | Mods::SHIFT)]),
        areas::HS_MAP_FILES,
        |app| request_edit(app, "openFolder"),
        Some(can_edit),
    );
    registry.bind(
        &CommandDefinition::new("revert", "Revert to the last saved difficulty"),
        areas::HS_MAP_FILES,
        |app| {
            app.revert_requested = true;
            Ok(())
        },
        Some(|app| app.document.is_some() && !app.busy && app.is_dirty()),
    );
    registry.bind(
        &CommandDefinition::new("save", "Save with a backup").keys(&[K::new(Key::S, Mods::CTRL)]),
        areas::HS_MAP_FILES,
        |app| {
            app.save();
            Ok(())
        },
        Some(|app| app.document.is_some() && !app.busy),
    );
    registry.bind(
        &CommandDefinition::new("play", "Play or pause")
            .keys(&[K::plain(Key::Space)])
            .osu(&[K::plain(Key::Space), K::plain(Key::X), K::plain(Key::C)]),
        areas::HS_PLAYBACK,
        |app| {
            app.play_pause();
            Ok(())
        },
        Some(|app| app.document.is_some()),
    );
    registry.bind(
        &CommandDefinition::new("stop", "Stop playback, or go to start when stopped")
            .keys(&[K::plain(Key::NumPad0)]),
        areas::HS_PLAYBACK,
        |app| {
            if app.is_playing() {
                app.stop();
            } else {
                app.seek(0.0);
            }
            Ok(())
        },
        Some(|app| app.document.is_some()),
    );
    registry.bind(
        &CommandDefinition::new("start", "Go to start").keys(&[K::plain(Key::Home)]),
        areas::HS_PLAYBACK,
        |app| {
            app.seek(0.0);
            Ok(())
        },
        Some(|app| app.document.is_some()),
    );
    registry.bind(
        &CommandDefinition::new("first", "Jump to the first object or hit (twice: song start)")
            .keys(&[K::plain(Key::Z)]),
        areas::HS_TIMELINE,
        |app| jump_to_endpoint(app, true),
        Some(|app| app.document.is_some()),
    );
    registry.bind(
        &CommandDefinition::new("last", "Jump to the last object or hit (twice: song end)")
            .keys(&[K::plain(Key::V)]),
        areas::HS_TIMELINE,
        |app| jump_to_endpoint(app, false),
        Some(|app| app.document.is_some()),
    );
    registry.bind(
        &kernel::cycle(),
        areas::HS_PLAYBACK,
        |app| {
            app.cycle = !app.cycle;
            app.cycle_changed();
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::jump_left(),
        areas::HS_TIMELINE,
        |app| {
            app.seek(app.locator_left);
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::jump_right(),
        areas::HS_TIMELINE,
        |app| {
            app.seek(app.locator_right);
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::set_left(),
        areas::HS_TIMELINE,
        |app| {
            app.locator_left = app.position_ms();
            app.cycle_changed();
            Ok(())
        },
        None,
    );
    registry.bind(
        &kernel::set_right(),
        areas::HS_TIMELINE,
        |app| {
            app.locator_right = app.position_ms();
            app.cycle_changed();
            Ok(())
        },
        None,
    );
    registry.bind(
        &CommandDefinition::new("help", "Show shortcuts and mouse actions")
            .keys(&[K::plain(Key::F1)]),
        areas::HELP,
        |app| {
            app.help_requested = true;
            Ok(())
        },
        None,
    );
    registry.bind(
        &CommandDefinition::new("generate", "Generate hitsound difficulty"),
        areas::HS_MAP_FILES,
        |app| {
            app.generate();
            Ok(())
        },
        Some(|app| app.document.is_some() && !app.busy),
    );
    for tool in ToolKind::ALL {
        let handler: dawsu_editor::commands::RunFn<Application> = match tool {
            ToolKind::Select => |a| {
                a.set_tool(ToolKind::Select);
                Ok(())
            },
            ToolKind::Grab => |a| {
                a.set_tool(ToolKind::Grab);
                Ok(())
            },
            ToolKind::Range => |a| {
                a.set_tool(ToolKind::Range);
                Ok(())
            },
            ToolKind::Draw => |a| {
                a.set_tool(ToolKind::Draw);
                Ok(())
            },
            ToolKind::Erase => |a| {
                a.set_tool(ToolKind::Erase);
                Ok(())
            },
            ToolKind::Mute => |a| {
                a.set_tool(ToolKind::Mute);
                Ok(())
            },
            ToolKind::Ramp => |a| {
                a.set_tool(ToolKind::Ramp);
                Ok(())
            },
        };
        registry.bind(&tool.command(), areas::HS_HIT_EDITING, handler, None);
        registry.mouse(areas::HS_HIT_EDITING, tool.mouse_actions());
    }
    registry.mouse(areas::HS_TIMELINE, &[
        MouseAction { gesture: "Click or drag the overview", description: "Center the timeline on that part of the song." },
        MouseAction { gesture: "Wheel without modifiers", description: "Scroll lanes in DAW-like, or seek one grid line over the timeline in osu!. The lane header area scrolls lanes in either preset." },
        MouseAction { gesture: "Sideways wheel or trackpad swipe", description: "Scroll horizontally through the timeline." },
        MouseAction { gesture: "Click the ruler or waveform", description: "Move the playhead. Click near a bookmark flag to land exactly on it." },
        MouseAction { gesture: "Ctrl+click / Alt+click the ruler", description: "Place the left / right locator at a snapped time." },
        MouseAction { gesture: "Drag the upper ruler", description: "Draw a cycle range outside the band. Drag within it to move it or drag an end to resize." },
        MouseAction { gesture: "Click inside the cycle band", description: "Toggle cycling." },
        MouseAction { gesture: "Drag / double-click under the waveform", description: "Resize the waveform or reset its height." },
        MouseAction { gesture: "Click / double-click a gameplay object on the waveform", description: "Select the object's hits and move the playhead. Double-click also auditions those hits." },
        MouseAction { gesture: "Ctrl+wheel (DAW-like) / Alt+wheel (osu!)", description: "Zoom time around the pointer." },
        MouseAction { gesture: "Alt+wheel (DAW-like) / Ctrl+wheel (osu!)", description: "Change snap spacing." },
        MouseAction { gesture: "Shift+wheel", description: "Scroll in time in DAW-like, or seek four grid lines in osu!." },
    ]);
    registry.mouse(areas::HS_LANES, &[
        MouseAction { gesture: "Drag a sample-set folder header", description: "Reorder the Normal, Soft and Drum folders." },
        MouseAction { gesture: "Click a collapsed hit preview", description: "Expand the folder or numbered sample family and reveal that hit's lane." },
        MouseAction { gesture: "Ctrl+Shift+wheel", description: "Change lane height around the pointer." },
        MouseAction { gesture: "Right-click a lane name", description: "Rename, move, replace, colour or delete a lane." },
        MouseAction { gesture: "Click / Ctrl+click / Shift+click a lane name", description: "Select and audition a lane, toggle it in the lane selection, or select a range of lanes. Mute and Solo apply to the selected lanes when you click one of them." },
    ]);
    registry.mouse(areas::HS_VOLUME, &[
        MouseAction { gesture: "Drag a bar", description: "Change its volume. Selected bars move together, keeping their differences." },
        MouseAction { gesture: "Ctrl+drag a selected bar", description: "Drag up to spread the selected gains or down to bring them toward their average." },
        MouseAction { gesture: "Alt+drag in the volume lane", description: "Paint volumes." },
        MouseAction { gesture: "Shift+drag in the volume lane", description: "Draw a volume ramp." },
    ]);
    CommandCatalog::build(registry)
        .expect("the application's command catalogue must have no clashes or unbound commands")
}
