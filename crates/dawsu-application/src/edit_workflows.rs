use super::Application;
pub use dawsu_editor_composer::{PatternStampOptions, RangeOperation, VolumeScope};
use std::path::Path;

pub enum TimelineEdit {
    Quantize(f64),
    MoveSelectionStart(f64),
    Repeat {
        copies: u32,
        gap_steps: u32,
    },
    Range {
        operation: RangeOperation,
        offset: f64,
        rows: Option<Vec<usize>>,
    },
    Scale {
        scope: VolumeScope,
        low: i32,
        high: i32,
        percent: f64,
    },
}

impl Application {
    pub fn requested_edit(&self) -> Option<&'static str> {
        self.edit_dialog
    }

    pub fn quantize_strength(&self) -> f64 {
        self.quantize_strength
    }

    pub fn cancel_timeline_edit(&mut self) {
        if self.edit_dialog.take().is_some() {
            self.bump_view_revision();
        }
    }

    pub fn apply_timeline_edit(&mut self, path: &Path, edit: TimelineEdit) {
        self.cancel_timeline_edit();
        if self.busy || self.is_stroke_open() {
            return;
        }
        let Some(document) = self.document.as_mut().filter(|doc| doc.path == path) else {
            return;
        };
        let composer = &mut document.composer;
        let changed = match edit {
            TimelineEdit::Quantize(strength) if strength.is_finite() => {
                self.quantize_strength = strength.clamp(1.0, 100.0);
                composer.quantize_with_strength(self.quantize_strength)
            }
            TimelineEdit::Quantize(_) => false,
            TimelineEdit::MoveSelectionStart(position) => composer.move_selection_start(position),
            TimelineEdit::Repeat { copies, gap_steps } => {
                composer.repeat_selected(copies, gap_steps)
            }
            TimelineEdit::Range {
                operation,
                offset,
                rows,
            } => composer.edit_marked_range(operation, offset, rows.as_deref()),
            TimelineEdit::Scale {
                scope,
                low,
                high,
                percent,
            } => composer.scale_by_volume(scope, low, high, percent),
        };
        if !changed {
            self.status = "No matching hits changed".into();
        }
        self.bump_view_revision();
    }

    pub fn pattern_stamp(&mut self, options: PatternStampOptions) -> usize {
        self.cancel_timeline_edit();
        if self.busy || self.is_stroke_open() {
            return 0;
        }
        let Some(document) = self.document.as_mut() else {
            return 0;
        };
        let count = document.composer.pattern_stamp(&options);
        if count == 0 {
            self.status = "No matching hits changed".into();
        }
        self.bump_view_revision();
        count
    }
}
