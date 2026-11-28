use tauri::{Emitter, Manager, State};

use crate::{Editor, Request, ask};

/// Opens the Export window beside the editor, or brings it forward when it is open.
/// Async, so building the window never runs on the main thread that drives it.
#[tauri::command]
pub(crate) async fn open_export_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("export") {
        let _ = window.unminimize();
        return window.set_focus().map_err(|error| error.to_string());
    }
    // A plain top-level window, not a child of main, so Show in editor can bring main forward.
    // Its own drop handler is off: a file dropped on Export never reaches main's drop listener.
    tauri::WebviewWindowBuilder::new(&app, "export", tauri::WebviewUrl::App("export.html".into()))
        .title("Export")
        .inner_size(1440.0, 880.0)
        .min_inner_size(1100.0, 720.0)
        .disable_drag_drop_handler()
        .build()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Closes the Export window the way its title bar does, so its preview is cancelled too.
/// With no Export window it is the Export modal inside main that closed: its preview is cancelled here.
#[tauri::command]
pub(crate) fn close_export_window(app: tauri::AppHandle, state: State<'_, Editor>) -> Result<(), String> {
    let window = app.get_webview_window("export");
    close_export(window.map(|window| move || window.close().map_err(|error| error.to_string())), &state)
}

pub(crate) fn close_export(close_window: Option<impl FnOnce() -> Result<(), String>>, editor: &Editor) -> Result<(), String> {
    match close_window {
        // The window's close handler (close_requested) sends ExportClose.
        Some(close) => close(),
        None => {
            let _ = editor.0.send(Request::ExportClose);
            Ok(())
        }
    }
}

/// Brings the editor window forward.
#[tauri::command]
pub(crate) fn show_in_editor(app: tauri::AppHandle) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("The editor window is closed.")?;
    let _ = main.unminimize();
    main.set_focus().map_err(|error| error.to_string())
}

/// Opens the drum detection window beside the editor, or brings it forward when it is open.
/// Async, so building the window never runs on the main thread that drives it.
#[tauri::command]
pub(crate) async fn drum_window_open(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("drum") {
        let _ = window.unminimize();
        return window.set_focus().map_err(|error| error.to_string());
    }
    let built = tauri::WebviewWindowBuilder::new(&app, "drum", tauri::WebviewUrl::App("drum.html".into()))
        .title("Drum hit detection")
        .inner_size(1200.0, 760.0)
        .min_inner_size(960.0, 600.0)
        .disable_drag_drop_handler()
        .build();
    match (built, app.get_webview_window("drum")) {
        (Ok(_), _) => Ok(()),
        // A second open raced the first one: the window exists now, so bring it forward.
        (Err(_), Some(window)) => window.set_focus().map_err(|error| error.to_string()),
        (Err(error), None) => Err(error.to_string()),
    }
}

/// Opens the map preview in its own window, where it last was, or brings it forward when open.
/// Async, so building the window never runs on the main thread that drives it.
#[tauri::command]
pub(crate) async fn preview_window_open(app: tauri::AppHandle, state: State<'_, Editor>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("preview") {
        let _ = window.unminimize();
        // A reloaded editor page learns again that the window is open.
        let _ = app.emit_to("main", "preview-window", true);
        return window.set_focus().map_err(|error| error.to_string());
    }
    let place = ask(&state, Request::PreviewPlaceGet)?;
    // dawsu! 3.1.4's map preview window: 480 × 420 at first, 300 × 240 at least (PlayfieldWindow.cs).
    let mut builder = tauri::WebviewWindowBuilder::new(&app, "preview", tauri::WebviewUrl::App("preview.html".into()))
        .title("Map preview")
        .inner_size(place.map_or(480.0, |p| p.width.max(300.0)), place.map_or(420.0, |p| p.height.max(240.0)))
        .min_inner_size(300.0, 240.0)
        .skip_taskbar(true)
        .disable_drag_drop_handler();
    // Owned by the editor, as 3.1.4's was, so a click on the editor never hides it behind
    // (owned on Windows, a child window on macOS, transient on Linux).
    if let Some(main) = app.get_webview_window("main") {
        builder = builder.parent(&main).map_err(|error| error.to_string())?;
    }
    match place.filter(|place| on_a_monitor(&app, place)) {
        Some(place) => builder = builder.position(place.x, place.y),
        // The first time, in the middle of the screen rather than its top-left corner.
        None => builder = builder.center(),
    }
    let built = builder.build();
    let result = match (built, app.get_webview_window("preview")) {
        (Ok(_), _) => Ok(()),
        (Err(_), Some(window)) => window.set_focus().map_err(|error| error.to_string()),
        (Err(error), None) => Err(error.to_string()),
    };
    if result.is_ok() {
        remember_preview_place(&app, &state, true, false);
        let _ = app.emit_to("main", "preview-window", true);
    }
    result
}

/// Whether a saved place still shows at least 40 × 40 px of the window on a monitor that is there.
/// (Windows puts a window snapped to the left edge a few pixels left of the screen.)
pub(crate) fn on_a_monitor(app: &tauri::AppHandle, place: &dawsu_application::WindowPlace) -> bool {
    app.available_monitors().unwrap_or_default().iter().any(|monitor| {
        let scale = monitor.scale_factor();
        let (at, size) = (monitor.position().to_logical::<f64>(scale), monitor.size().to_logical::<f64>(scale));
        let across = (place.x + place.width).min(at.x + size.width) - place.x.max(at.x);
        let down = (place.y + place.height).min(at.y + size.height) - place.y.max(at.y);
        across >= 40.0 && down >= 40.0
    })
}

/// Hands the preview window's place to the editor, if the window is there.
pub(crate) fn remember_preview_place(app: &tauri::AppHandle, editor: &Editor, open: bool, save: bool) {
    let Some(window) = app.get_webview_window("preview") else { return };
    // Minimised (Windows reports −32000 and 0 × 0) or maximised, the normal place is the last one kept;
    // only whether it is open changes.
    if window.is_minimized().unwrap_or(false) || window.is_maximized().unwrap_or(false) {
        let _ = editor.0.send(Request::PreviewOpen(open, save));
        return;
    }
    let (Ok(scale), Ok(at), Ok(size)) = (window.scale_factor(), window.outer_position(), window.inner_size()) else { return };
    let (at, size) = (at.to_logical::<f64>(scale), size.to_logical::<f64>(scale));
    let place = dawsu_application::WindowPlace { x: at.x, y: at.y, width: size.width, height: size.height, open };
    let _ = editor.0.send(Request::PreviewPlace(place, save));
}

/// Closes the map preview window the way its title bar does.
#[tauri::command]
pub(crate) fn close_preview_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("preview") {
        Some(window) => window.close().map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// Closes the drum detection window the way its title bar does, so its run and Listen stop too.
#[tauri::command]
pub(crate) fn close_drum_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("drum") {
        Some(window) => window.close().map_err(|error| error.to_string()),
        None => Ok(()),
    }
}


/// Opens the node graph window beside the editor, or brings it forward when it is open.
#[tauri::command]
pub(crate) async fn open_node_graph_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("node-graph") {
        let _ = window.unminimize();
        return window.set_focus().map_err(|error| error.to_string());
    }
    let built = tauri::WebviewWindowBuilder::new(&app, "node-graph", tauri::WebviewUrl::App("node-graph.html".into()))
        .title("Node graph")
        .inner_size(1280.0, 800.0)
        .min_inner_size(960.0, 620.0)
        .disable_drag_drop_handler()
        .build();
    match (built, app.get_webview_window("node-graph")) {
        (Ok(_), _) => Ok(()),
        (Err(_), Some(window)) => window.set_focus().map_err(|error| error.to_string()),
        (Err(error), None) => Err(error.to_string()),
    }
}

#[tauri::command]
pub(crate) fn close_node_graph_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("node-graph") {
        Some(window) => window.close().map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

