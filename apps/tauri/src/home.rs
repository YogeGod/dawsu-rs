use super::{Action, map_path};
use dawsu_application::Application;
use serde_json::Value;

#[derive(Default)]
pub(super) enum Route {
    #[default]
    Editor,
    Review,
    Leaving,
    Parked,
    Opening,
}

impl Route {
    pub(super) fn handle(&mut self, action: Action, app: &mut Application) -> Option<Action> {
        match action {
            Action::Home if matches!(self, Self::Editor) && !app.busy && app.document().is_some() => {
                if app.is_dirty() {
                    *self = Self::Review;
                } else if let Err(error) = app.save_session() {
                    app.error = Some(error);
                } else {
                    self.park(app);
                }
            }
            Action::HomeChoice { choice } if matches!(self, Self::Review) && !app.busy => {
                *self = Self::Editor;
                match choice.as_str() {
                    "save" => app.save(),
                    "discard" => {
                        app.command("revert");
                        app.confirm_revert();
                    }
                    _ => {}
                }
                if app.busy {
                    *self = Self::Leaving;
                }
            }
            Action::Resume if matches!(self, Self::Parked) && !app.busy => {
                *self = Self::Editor;
            }
            Action::Open { path } if matches!(self, Self::Parked) => {
                if map_path(&path) {
                    app.request_open(path);
                    if app.busy {
                        *self = Self::Opening;
                    }
                }
            }
            action @ Action::Key { .. } if self.visible() || app.document().is_none() => {
                if let Action::Key { key, code, ctrl, shift, alt } = &action
                    && let Some(chord) = super::keyboard::chord(key, code, *ctrl, *shift, *alt)
                    && matches!(app.keymap.command_for(chord.combination()), Some("open" | "help"))
                {
                    return Some(action);
                }
            }
            Action::Home | Action::HomeChoice { .. } | Action::Resume => {}
            action => return Some(action),
        }
        None
    }

    pub(super) fn settle(&mut self, app: &mut Application) {
        if app.busy {
            return;
        }
        match self {
            Self::Leaving if app.error.is_none() && !app.is_dirty() && app.pending_open.is_none() => self.park(app),
            Self::Leaving => *self = Self::Editor,
            Self::Opening if app.error.is_none() => *self = Self::Editor,
            Self::Opening => *self = Self::Parked,
            _ => {}
        }
    }

    fn park(&mut self, app: &mut Application) {
        app.stop();
        *self = Self::Parked;
    }

    pub(super) fn visible(&self) -> bool {
        matches!(self, Self::Parked | Self::Opening)
    }

    pub(super) fn add_status(&self, view: &mut Value) {
        view["showHome"] = self.visible().into();
        view["pendingHome"] = matches!(self, Self::Review).into();
    }
}