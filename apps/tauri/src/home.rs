use super::{Action, ViewSnapshot, map_path};
use dawsu_application::{Application, home_route::HomeRoute};

#[derive(Default)]
pub(super) struct Route(HomeRoute);

impl Route {
    pub(super) fn handle(&mut self, action: Action, app: &mut Application) -> Option<Action> {
        match action {
            Action::Home => self.0.request_home(app),
            Action::HomeChoice { choice } => self.0.resolve_review(app, &choice),
            Action::Resume => self.0.resume(app),
            Action::Open { path } if matches!(self.0, HomeRoute::Parked) => {
                let valid = map_path(&path);
                self.0.open_map(app, path, valid);
            }
            Action::OpenPracticeMapset if matches!(self.0, HomeRoute::Parked) => {
                self.0.open_practice(app);
            }
            action @ Action::Key { .. } if self.visible() || app.document().is_none() => {
                if let Action::Key { key, code, ctrl, shift, alt, .. } = &action
                    && let Some(chord) = super::keyboard::chord(key, code, *ctrl, *shift, *alt)
                    && HomeRoute::allows_home_shortcut(app.keymap().command_for(chord.combination()))
                {
                    return Some(action);
                }
            }
            action => return Some(action),
        }
        None
    }

    pub(super) fn settle(&mut self, app: &mut Application) {
        self.0.settle(app);
    }

    pub(super) fn visible(&self) -> bool {
        self.0.visible()
    }

    pub(super) fn pending(&self) -> bool {
        self.0.pending()
    }

    pub(super) fn add_status(&self, view: &mut ViewSnapshot) {
        view.show_home = Some(self.visible());
        view.pending_home = Some(self.pending());
    }
}
