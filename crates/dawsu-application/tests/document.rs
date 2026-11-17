use dawsu_application::{Document, save_document};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_test_support::Scratch;

const MAP: &[u8] = b"osu file format v14\r\n// untouched comment\r\n[General]\r\nMode:0\r\n[Metadata]\r\nTitle:Test\r\nVersion:Hard\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n[TimingPoints]\r\n0,500,4,1,0,100,1,0\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n";
fn clap() -> SampleIdentity {
    SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Clap),
        index: CustomIndex::new(0),
    }
}
#[test]
fn copied_map_edit_save_reopen_keeps_untouched_bytes_and_history() {
    let scratch = Scratch::new("application-save");
    let path = scratch.path("test.osu");
    std::fs::write(&path, MAP).unwrap();
    let mut d = Document::read(&path).unwrap();
    d.composer.add(1000.0, clap(), 80, 5.0);
    let result = save_document(&d, scratch.path("backups")).unwrap();
    assert!(result.problems.is_empty());
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"osu file format v14\r\n// untouched comment\r\n"));
    let reopened = Document::read(&path).unwrap();
    assert!(reopened.composer.hitsounds().moments()[0].has_hit(&clap()));
    assert!(d.composer.history.can_undo());
}
#[test]
fn external_edit_is_preserved_and_not_reported_as_a_successful_save() {
    let scratch = Scratch::new("application-conflict");
    let path = scratch.path("test.osu");
    std::fs::write(&path, MAP).unwrap();
    let mut d = Document::read(&path).unwrap();
    d.composer.add(1000.0, clap(), 80, 5.0);
    std::fs::write(&path, b"external editor wrote this").unwrap();
    assert!(save_document(&d, scratch.path("backups")).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"external editor wrote this");
    assert!(d.composer.dirty());
}
#[test]
fn undo_after_save_can_be_saved_again_without_losing_the_undo() {
    let scratch = Scratch::new("application-save-undo-save");
    let path = scratch.path("test.osu");
    std::fs::write(&path, MAP).unwrap();
    let mut d = Document::read(&path).unwrap();
    d.composer.add(1000.0, clap(), 80, 5.0);
    let first = save_document(&d, scratch.path("backups")).unwrap();
    d.file = first.file;
    d.composer.mark_saved();
    d.composer.undo();
    save_document(&d, scratch.path("backups")).unwrap();
    let reopened = Document::read(&path).unwrap();
    assert!(!reopened.composer.hitsounds().moments()[0].has_hit(&clap()));
    assert_eq!(std::fs::read(&path).unwrap(), MAP);
}
