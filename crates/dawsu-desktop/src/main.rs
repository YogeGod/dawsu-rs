#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dawsu_application::{Application, PlatformRequest};
use dawsu_ui::EditorUi;
use std::path::PathBuf;

struct Desktop {
    editor: EditorUi,
}
impl eframe::App for Desktop {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        self.editor.ui(ui);
        if ui.ctx().input(|i| i.viewport().close_requested()) && self.editor.application.busy {
            ui.ctx()
                .send_viewport_cmd(eframe::egui::ViewportCommand::CancelClose);
            self.editor.application.cancel_job();
            self.editor.application.error =
                Some("Wait for the current job to finish cancelling before closing.".into());
        } else if ui.ctx().input(|i| i.viewport().close_requested())
            && self.editor.application.is_dirty()
            && !self.editor.allow_close
        {
            ui.ctx()
                .send_viewport_cmd(eframe::egui::ViewportCommand::CancelClose);
            self.editor.confirm_close = true;
        }
        for request in std::mem::take(&mut self.editor.application.requests) {
            match request {
                PlatformRequest::OpenMap => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_parent(frame)
                        .add_filter("osu! difficulty or mapset", &["osu", "osz"])
                        .pick_file()
                    {
                        self.editor.application.request_open(path);
                    }
                }
                PlatformRequest::OpenSamples => {
                    if let Some(path) = rfd::FileDialog::new().set_parent(frame).pick_folder() {
                        self.editor.application.preferences.default_samples = path;
                        self.editor.application.save_preferences();
                    }
                }
                PlatformRequest::OpenSongs => {
                    if let Some(path) = rfd::FileDialog::new().set_parent(frame).pick_folder() {
                        self.editor.application.preferences.songs_folder = path;
                        self.editor.application.save_preferences();
                    }
                }
                PlatformRequest::OpenFolder(path) => {
                    #[cfg(target_os = "windows")]
                    let result = std::process::Command::new("explorer.exe").arg(path).spawn();
                    #[cfg(target_os = "macos")]
                    let result = std::process::Command::new("open").arg(path).spawn();
                    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
                    let result = std::process::Command::new("xdg-open").arg(path).spawn();
                    if let Err(error) = result {
                        self.editor.application.error = Some(format!("Opening folder: {error}"));
                    }
                }
            }
        }
        let title = self
            .editor
            .application
            .document()
            .map(|d| {
                format!(
                    "{}{} · dawsu!",
                    if self.editor.application.is_dirty() {
                        "● "
                    } else {
                        ""
                    },
                    d.title
                )
            })
            .unwrap_or_else(|| "dawsu! · Hitsound editor".into());
        ui.ctx()
            .send_viewport_cmd(eframe::egui::ViewportCommand::Title(title));
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(error) = self.editor.application.save_session() {
            eprintln!("{error}");
        }
        self.editor.application.save_preferences();
    }
}
fn main() -> eframe::Result {
    let path = std::env::args_os()
        .skip(1)
        .find(|s| !s.to_string_lossy().starts_with('-'))
        .map(PathBuf::from);
    eframe::run_native(
        "dawsu!",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_title("dawsu!")
                .with_inner_size([1440.0, 900.0])
                .with_min_inner_size([1024.0, 720.0]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| {
            EditorUi::configure(&cc.egui_ctx);
            let mut app = Application::new();
            if let Some(path) = path {
                app.request_open(path);
            }
            Ok(Box::new(Desktop {
                editor: EditorUi::new(app),
            }))
        }),
    )
}
