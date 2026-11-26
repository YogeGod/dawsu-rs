use crate::Application;
use std::path::PathBuf;

/// UI-independent workflow state machine tracking transitions between the Editor and Home screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HomeRoute {
    #[default]
    Editor,
    Review,
    Leaving,
    Parked,
    Opening,
}

impl HomeRoute {
    pub fn request_home(&mut self, app: &mut Application) {
        if !matches!(self, Self::Editor) || app.is_busy() || app.document().is_none() {
            return;
        }
        if app.is_dirty() {
            *self = Self::Review;
        } else if let Err(error) = app.save_session() {
            app.set_error(Some(error));
        } else {
            self.park(app);
        }
    }

    pub fn resolve_review(&mut self, app: &mut Application, choice: &str) {
        if !matches!(self, Self::Review) || app.is_busy() {
            return;
        }
        *self = Self::Editor;
        match choice {
            "save" => app.save(),
            "discard" => {
                app.command("revert");
                app.confirm_revert();
            }
            _ => {}
        }
        if app.is_busy() {
            *self = Self::Leaving;
        }
    }

    pub fn resume(&mut self, app: &Application) {
        if matches!(self, Self::Parked) && !app.is_busy() {
            *self = Self::Editor;
        }
    }

    pub fn open_map(&mut self, app: &mut Application, path: PathBuf, is_valid_map: bool) {
        if matches!(self, Self::Parked) && is_valid_map {
            app.request_open(path);
            if app.is_busy() {
                *self = Self::Opening;
            }
        }
    }

    pub fn open_practice(&mut self, app: &mut Application) {
        if matches!(self, Self::Parked) {
            app.request_open_practice_mapset();
            if app.is_busy() {
                *self = Self::Opening;
            }
        }
    }

    pub fn allows_home_shortcut(command_id: Option<&str>) -> bool {
        matches!(
            command_id,
            Some("open" | "help" | "importOsz" | "openHitsoundFolder")
        )
    }

    pub fn settle(&mut self, app: &mut Application) {
        if app.is_busy() {
            return;
        }
        match self {
            Self::Leaving
                if app.error().is_none() && !app.is_dirty() && !app.has_pending_open() =>
            {
                self.park(app)
            }
            // The Save hitsounds summary is open: its Save goes on Home, its Cancel stays here.
            Self::Leaving if app.error().is_none() && app.save_review_pending() => {}
            Self::Leaving => *self = Self::Editor,
            Self::Opening if app.error().is_none() => *self = Self::Editor,
            Self::Opening => *self = Self::Parked,
            _ => {}
        }
    }

    pub fn park(&mut self, app: &mut Application) {
        app.stop();
        *self = Self::Parked;
    }

    pub fn visible(self) -> bool {
        matches!(self, Self::Parked | Self::Opening)
    }

    pub fn pending(self) -> bool {
        matches!(self, Self::Review)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn park_resume_and_shortcut_allowlist_behave_as_expected() {
        let mut app = Application::headless();
        let mut route = HomeRoute::default();
        assert!(!route.visible());
        assert!(!route.pending());

        route.park(&mut app);
        assert!(route.visible());
        assert!(!route.pending());

        route.resume(&app);
        assert_eq!(route, HomeRoute::Editor);

        assert!(HomeRoute::allows_home_shortcut(Some("open")));
        assert!(HomeRoute::allows_home_shortcut(Some("help")));
        assert!(!HomeRoute::allows_home_shortcut(Some("save")));
        assert!(!HomeRoute::allows_home_shortcut(None));
    }
}
