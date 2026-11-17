use super::{Application, Document, PlatformRequest};
use dawsu_editor::commands::{CommandCatalog, CommandDefinition, CommandRegistry, areas};
use dawsu_editor::editing_commands as kernel;
use dawsu_editor::keys::{Key, KeyCombination as K, Mods};
use dawsu_editor::tools::ToolKind;
use dawsu_foundation::SongTime;

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
    doc!(kernel::undo(), d, { d.composer.undo() });
    doc!(kernel::redo(), d, { d.composer.redo() });
    doc!(kernel::copy(), d, { d.composer.copy() });
    doc!(kernel::cut(), d, {
        d.composer.copy();
        d.composer.remove_selected()
    });
    doc!(kernel::delete(), d, { d.composer.remove_selected() });
    doc!(kernel::select_all(), d, { d.composer.select_all() });
    doc!(kernel::deselect(), d, { d.composer.selected.clear() });
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
    doc!(
        CommandDefinition::new("quantize", "Quantize selected hits"),
        d,
        { d.composer.quantize() }
    );
    doc!(
        CommandDefinition::new("volumeUp", "Increase volume by 5")
            .keys(&[K::new(Key::Up, Mods::ALT)]),
        d,
        { d.composer.volume(5, true) }
    );
    doc!(
        CommandDefinition::new("volumeDown", "Decrease volume by 5")
            .keys(&[K::new(Key::Down, Mods::ALT)]),
        d,
        { d.composer.volume(-5, true) }
    );
    doc!(
        CommandDefinition::new("nudgeLeft", "Move hits to the previous grid step")
            .legacy(&["moveLeft"])
            .keys(&[K::new(Key::Left, Mods::CTRL)]),
        d,
        {
            let step = d
                .composer
                .spacing
                .step_ms(d.composer.tempo.beat_length_at(SongTime::from_ms(0.0)));
            d.composer.move_selected(-step, None, 5.0)
        }
    );
    doc!(
        CommandDefinition::new("nudgeRight", "Move hits to the next grid step")
            .legacy(&["moveRight"])
            .keys(&[K::new(Key::Right, Mods::CTRL)]),
        d,
        {
            let step = d
                .composer
                .spacing
                .step_ms(d.composer.tempo.beat_length_at(SongTime::from_ms(0.0)));
            d.composer.move_selected(step, None, 5.0)
        }
    );
    doc!(
        CommandDefinition::new("navUp", "Move selected hits to the lane above")
            .keys(&[K::plain(Key::Up)]),
        d,
        { d.composer.move_rows(-1) }
    );
    doc!(
        CommandDefinition::new("navDown", "Move selected hits to the lane below")
            .keys(&[K::plain(Key::Down)]),
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
            a.zoom = (a.zoom / 1.3).max(0.005);
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
        &CommandDefinition::new("play", "Play or pause").keys(&[K::plain(Key::Space)]),
        areas::HS_PLAYBACK,
        |app| {
            app.play_pause();
            Ok(())
        },
        Some(|app| app.document.is_some()),
    );
    registry.bind(
        &CommandDefinition::new("stop", "Stop playback"),
        areas::HS_PLAYBACK,
        |app| {
            app.stop();
            Ok(())
        },
        None,
    );
    registry.bind(
        &CommandDefinition::new("first", "Jump to the first object").keys(&[K::plain(Key::Z)]),
        areas::HS_TIMELINE,
        |app| {
            let t = app
                .document
                .as_ref()
                .and_then(|d| d.composer.hitsounds().moments().first())
                .map(|m| m.time().ms())
                .unwrap_or(0.0);
            app.seek(t);
            Ok(())
        },
        None,
    );
    registry.bind(
        &CommandDefinition::new("last", "Jump to the last object").keys(&[K::plain(Key::V)]),
        areas::HS_TIMELINE,
        |app| {
            let t = app
                .document
                .as_ref()
                .map(|d| d.duration_ms() - 2000.0)
                .unwrap_or(0.0);
            app.seek(t);
            Ok(())
        },
        None,
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
    CommandCatalog::build(registry)
        .expect("the application's command catalogue must have no clashes or unbound commands")
}
