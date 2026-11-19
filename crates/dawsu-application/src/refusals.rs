//! The words a mapper reads when Open, New difficulty, Generate, Copy, Import or a write is
//! refused. All of them live here, so no message shows a Rust variant name. Every caller handles
//! the outcome that went through before it asks for words, so those arms are unreachable.

use dawsu_editor::tempo::SnapSpacingError;
use dawsu_hitsounding::Decompiled;
use dawsu_hitsounding::copy::{CopyOutcome, TargetOutcome};
use dawsu_hitsounding::generate::{GenerateOutcome, NewDiff};
use dawsu_osu_format::{OszImport, PathRefusal, Refusal, SaveOutcome};

fn mode_name(mode: i32) -> String {
    match mode {
        0 => "an osu!standard".into(),
        1 => "an osu!taiko".into(),
        2 => "an osu!catch".into(),
        3 => "an osu!mania".into(),
        other => format!("a mode {other}"),
    }
}

pub(crate) fn open(outcome: &Decompiled) -> String {
    match outcome {
        Decompiled::Done(_) => unreachable!("a read difficulty is not a refusal"),
        Decompiled::UnsupportedMode { mode } => format!(
            "This is {} difficulty. dawsu! edits osu!standard difficulties only.",
            mode_name(*mode)
        ),
        Decompiled::Cancelled => "Opening the difficulty was cancelled.".into(),
    }
}

pub(crate) fn new_difficulty(outcome: &NewDiff) -> String {
    match outcome {
        NewDiff::Planned { .. } => unreachable!("a planned difficulty is not a refusal"),
        NewDiff::BlankName => "Type a name for the new difficulty.".into(),
        NewDiff::VersionTaken => {
            "A difficulty with that name already exists in this mapset. Choose another name.".into()
        }
        NewDiff::NameTaken => {
            "A difficulty file with that name is already in the mapset folder. Choose another name."
                .into()
        }
    }
}

pub(crate) fn generate(outcome: &GenerateOutcome) -> String {
    match outcome {
        GenerateOutcome::Planned { .. } => unreachable!("a planned Generate is not a refusal"),
        GenerateOutcome::NameTaken => "A difficulty in this mapset already has the hitsound difficulty's file name, and dawsu! did not make it. Rename that difficulty, then generate again.".into(),
        GenerateOutcome::VersionTaken => "A difficulty in this mapset is already called \"Hitsounds\". Rename it, then generate again.".into(),
        GenerateOutcome::Cancelled => "Generate was cancelled.".into(),
    }
}

pub(crate) fn copy(outcome: &CopyOutcome) -> String {
    match outcome {
        CopyOutcome::Targets(_) => unreachable!("planned Copy targets are not a refusal"),
        CopyOutcome::InvalidOptions(why) => format!("Check the Copy settings: {why}"),
        CopyOutcome::Cancelled => "Copy was cancelled.".into(),
    }
}

/// Follows the target's file name and a colon.
pub(crate) fn copy_target(outcome: &TargetOutcome) -> String {
    match outcome {
        TargetOutcome::Planned(_) => unreachable!("a planned Copy target is not a refusal"),
        TargetOutcome::UnsupportedMode { mode } => format!(
            "this is {} difficulty, and Copy writes to osu!standard difficulties only",
            mode_name(*mode)
        ),
        TargetOutcome::Refused(why) => why.clone(),
    }
}

pub(crate) fn import(outcome: &OszImport) -> String {
    match outcome {
        OszImport::Imported { .. } => unreachable!("an imported .osz is not a refusal"),
        OszImport::NotAnArchive(why) => format!("This .osz file could not be unpacked: {why}"),
        OszImport::TooLarge => "This .osz file is too large to import.".into(),
        OszImport::TooManyEntries => "This .osz file holds too many files to import.".into(),
        OszImport::UnsafeName(name) => {
            format!("This .osz file holds a file name that could leave the mapset folder: {name}")
        }
        OszImport::Cancelled => "Import was cancelled.".into(),
    }
}

/// Follows the written file's path and a colon.
pub(crate) fn write(outcome: &SaveOutcome) -> String {
    match outcome {
        SaveOutcome::Saved { .. } | SaveOutcome::NothingToSave => unreachable!("a finished write is not a refusal"),
        SaveOutcome::ChangedOnDisk => {
            "the file changed on disk after the review, so it was not written. Review again.".into()
        }
        SaveOutcome::AlreadyExists => {
            "a file with this name appeared after the review, so it was not written. Review again."
                .into()
        }
    }
}

/// Why one line of a difficulty could not be changed.
pub(crate) fn line(refusal: &Refusal) -> String {
    match refusal {
        Refusal::NotUtf8 => "the line is not plain UTF-8 text, so dawsu! leaves it as it is",
        Refusal::NotFound => "the line is no longer in the difficulty",
        Refusal::RedLine => "the line is a red line, which dawsu! never removes",
        Refusal::WrongKind => "the line is not the kind this change needs",
        Refusal::CannotFill => "a value before it is missing and cannot be filled in",
        Refusal::LineBreak => "the text holds a line break",
    }
    .into()
}

pub(crate) fn file_name(refusal: &PathRefusal) -> String {
    format!("\"{}\" is not a safe file name: {}", refusal.path, refusal.reason)
}

pub(crate) fn snap(beats: u32, divisions: u32, error: SnapSpacingError) -> String {
    match error {
        SnapSpacingError::Zero => format!("Snap {beats}/{divisions} is not available. Both numbers must be 1 or more."),
        SnapSpacingError::LongAndFraction => format!(
            "Snap {beats}/{divisions} is not available. Use a fraction of a beat, like 1/4, or whole beats, like 2/1."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The variant's name as `{:?}` shows it, without its fields.
    fn variant<T: std::fmt::Debug>(value: &T) -> String {
        format!("{value:?}").chars().take_while(char::is_ascii_alphanumeric).collect()
    }

    fn assert_plain<T: std::fmt::Debug>(values: &[T], message: impl Fn(&T) -> String) {
        for value in values {
            let (name, text) = (variant(value), message(value));
            assert!(!text.contains(&name), "{text:?} shows the variant name {name}");
        }
    }

    #[test]
    #[should_panic]
    fn an_outcome_that_went_through_is_never_worded_as_a_refusal() {
        write(&SaveOutcome::NothingToSave);
    }

    #[test]
    fn no_refusal_shows_a_variant_name() {
        assert_plain(&[Decompiled::UnsupportedMode { mode: 3 }, Decompiled::Cancelled], open);
        assert_plain(&[NewDiff::BlankName, NewDiff::VersionTaken, NewDiff::NameTaken], new_difficulty);
        assert_plain(
            &[GenerateOutcome::NameTaken, GenerateOutcome::VersionTaken, GenerateOutcome::Cancelled],
            generate,
        );
        assert_plain(&[CopyOutcome::InvalidOptions("the leniency must be 0 or more".into()), CopyOutcome::Cancelled], copy);
        assert_plain(
            &[TargetOutcome::UnsupportedMode { mode: 1 }, TargetOutcome::Refused("turn on copying".into())],
            copy_target,
        );
        assert_plain(
            &[
                OszImport::NotAnArchive("invalid Zip archive".into()),
                OszImport::TooLarge,
                OszImport::TooManyEntries,
                OszImport::UnsafeName("../a.osu".into()),
                OszImport::Cancelled,
            ],
            import,
        );
        assert_plain(&[SaveOutcome::ChangedOnDisk, SaveOutcome::AlreadyExists], write);
        assert_plain(
            &[
                Refusal::NotUtf8,
                Refusal::NotFound,
                Refusal::RedLine,
                Refusal::WrongKind,
                Refusal::CannotFill,
                Refusal::LineBreak,
            ],
            line,
        );
        let path = PathRefusal { path: "C:/a.wav".into(), reason: "absolute and network paths are not allowed" };
        assert_plain(&[path], file_name);
        assert_plain(&[SnapSpacingError::Zero, SnapSpacingError::LongAndFraction], |error| snap(2, 3, *error));
        assert_eq!(open(&Decompiled::UnsupportedMode { mode: 3 }), "This is an osu!mania difficulty. dawsu! edits osu!standard difficulties only.");
    }
}
