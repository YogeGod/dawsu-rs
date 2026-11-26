use super::{Application, Document};
use dawsu_editor::transport::{Bookmark, BookmarkList};
use dawsu_foundation::SongTime;
use dawsu_osu_format::OsuFile;

pub(super) fn import(file: &OsuFile) -> BookmarkList {
    let value = file.value("Editor", "Bookmarks").unwrap_or_default();
    BookmarkList::new(value.split(',').filter_map(|item| {
        let ms = item.trim().parse::<i64>().ok()?;
        (ms >= 0).then(|| SongTime::from_ms(ms as f64))
    }))
}

impl Document {
    pub fn bookmarks(&self) -> &[Bookmark] {
        self.bookmarks.items()
    }
}

fn label_text(text: String) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

impl Application {
    fn editable_bookmarks(&mut self) -> Result<&mut Document, String> {
        if self.busy || self.tools.is_stroke_open() {
            return Err("Finish the current edit before changing bookmarks".into());
        }
        self.document.as_mut().ok_or_else(|| "Open a difficulty first".into())
    }

    pub fn add_bookmark(&mut self, label: Option<String>) -> Result<(), String> {
        self.add_bookmark_at(self.position_ms(), label)
    }

    pub fn add_bookmark_at(&mut self, time: f64, label: Option<String>) -> Result<(), String> {
        if !time.is_finite() || time < 0.0 || time > self.duration_ms() {
            return Err("Invalid bookmark time".into());
        }
        let doc = self.editable_bookmarks()?;
        doc.bookmark_override |= doc.bookmarks.add(SongTime::from_ms(time), label.and_then(label_text));
        self.bump_view_revision();
        Ok(())
    }

    pub fn rename_bookmark(&mut self, time: f64, label: String) -> Result<(), String> {
        let doc = self.editable_bookmarks()?;
        let mark = doc.bookmarks.items().iter().find(|mark| mark.time.ms() == time)
            .ok_or("The bookmark is no longer present")?;
        let label = label_text(label);
        if mark.label != label {
            let at = mark.time;
            doc.bookmarks.remove_at(at);
            doc.bookmarks.add(at, label);
            doc.bookmark_override = true;
            self.bump_view_revision();
        }
        Ok(())
    }

    pub fn remove_bookmark(&mut self, time: f64) -> Result<(), String> {
        if !time.is_finite() || time < 0.0 {
            return Err("Invalid bookmark time".into());
        }
        let doc = self.editable_bookmarks()?;
        doc.bookmark_override |= doc.bookmarks.remove_at(SongTime::from_ms(time));
        self.bump_view_revision();
        Ok(())
    }

    pub fn jump_to_bookmark(&mut self, time: f64) {
        if self.document().is_some_and(|d| d.bookmarks().iter().any(|mark| mark.time.ms() == time)) {
            let position = time.min(self.duration_ms());
            self.seek(position);
            self.set_view_start(position - 32.0 / self.zoom);
        }
    }

    pub(super) fn step_bookmark(&mut self, forward: bool) {
        let Some(doc) = &self.document else { return };
        let at = SongTime::from_ms(self.position_ms());
        let next = if forward { doc.bookmarks.next(at) } else { doc.bookmarks.previous(at) };
        if let Some(time) = next {
            self.jump_to_bookmark(time.ms());
            self.bookmark_step = Some(time.ms());
            self.bump_view_revision();
        }
    }
}
