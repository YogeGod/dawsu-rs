use dawsu_application::Application;
use dawsu_test_support::Scratch;
use dawsu_ui::EditorUi;
use egui::{Context, Event, Key, Modifiers, Pos2, RawInput, Rect, Vec2};
use std::time::{Duration, Instant};

fn frame(
    ctx: &Context,
    editor: &mut EditorUi,
    events: Vec<Event>,
    size: [f32; 2],
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(size[0], size[1]))),
            events,
            ..Default::default()
        },
        |ui| editor.ui(ui),
    );
    // This behaviour harness has no GPU backend to consume texture uploads.
    output.textures_delta.clear();
    output
}
#[test]
fn open_shortcut_reaches_the_application_and_empty_screen_draws() {
    let ctx = Context::default();
    EditorUi::configure(&ctx);
    let mut editor = EditorUi::new(Application::headless());
    let output = frame(&ctx, &mut editor, vec![], [1440.0, 900.0]);
    assert!(!output.shapes.is_empty());
    frame(
        &ctx,
        &mut editor,
        vec![Event::Key {
            key: Key::O,
            physical_key: Some(Key::O),
            pressed: true,
            repeat: false,
            modifiers: Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        }],
        [1440.0, 900.0],
    );
    assert_eq!(editor.application.requests.len(), 1);
}
#[test]
fn a_real_loaded_document_draws_at_the_minimum_and_default_window_sizes() {
    let scratch = Scratch::new("ui-map");
    let path = scratch.path("test.osu");
    std::fs::write(&path,b"osu file format v14\n[General]\nMode:0\n[Metadata]\nTitle:Surface test\nVersion:Hard\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n").unwrap();
    let mut editor = EditorUi::new(Application::headless());
    editor.application.request_open(path);
    let deadline = Instant::now() + Duration::from_secs(5);
    while editor.application.busy {
        editor.application.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(editor.application.document().is_some());
    let ctx = Context::default();
    EditorUi::configure(&ctx);
    for size in [[1440.0, 900.0], [1024.0, 720.0]] {
        let output = frame(&ctx, &mut editor, vec![], size);
        assert!(output.shapes.len() > 50);
    }
    assert!(!editor.application.is_dirty());
}

#[test]
fn overview_centres_the_view_and_stops_at_the_edges() {
    use dawsu_ui::overview_start;
    assert_eq!(overview_start(0.0, 10_000.0, 2_000.0), 0.0);
    assert_eq!(overview_start(0.5, 10_000.0, 2_000.0), 4_000.0);
    assert_eq!(overview_start(1.0, 10_000.0, 2_000.0), 8_000.0);
    assert_eq!(overview_start(0.5, 10_000.0, 20_000.0), 0.0);
}
