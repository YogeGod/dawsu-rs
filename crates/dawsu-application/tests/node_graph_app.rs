//! Full Application-layer integration test for the M4 Node Graph workflow.

use dawsu_application::{NodeGraphAction, ViewerOverlay, Application, lane_layout_key};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_test_support::{Scratch, workspace_root};
use std::path::Path;
use std::time::{Duration, Instant};

fn write_wav_16bit_mono(path: &Path, sample_rate: u32, samples: &[f32]) {
    let data_bytes = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let q = (clamped * 32767.0).round() as i16;
        out.extend_from_slice(&q.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

fn pattern_samples() -> Vec<f32> {
    let path = workspace_root().join("crates/dawsu-graph-audio/tests/fixtures/classic-3.1.4/pattern.f32");
    std::fs::read(path)
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

fn wait_until(app: &mut Application, mut pred: impl FnMut(&Application) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !pred(app) {
        app.poll();
        assert!(Instant::now() < deadline, "timed out waiting for Application state");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn open_fixture(scratch: &Scratch) -> (Application, std::path::PathBuf) {
    let wav_path = scratch.path("song.wav");
    write_wav_16bit_mono(&wav_path, 44_100, &pattern_samples());
    let osu_path = scratch.path("Map.osu");
    let osu = "\
osu file format v14
[General]
AudioFilename: song.wav
SampleSet: Soft
Mode: 0

[Metadata]
Title: NodalAppTest
Version: Hard

[TimingPoints]
0,500,4,2,0,45,1,0

[HitObjects]
100,100,0,1,2,0:0:0:0:
200,100,500,1,0,0:0:0:0:
300,100,1000,1,0,0:0:0:0:
400,100,1500,1,0,0:0:0:0:
100,100,2000,1,0,0:0:0:0:
200,100,2500,1,0,0:0:0:0:
";
    std::fs::write(&osu_path, osu).unwrap();
    let mut app = Application::headless_with_projects(scratch.path("data"));
    app.request_open(osu_path.clone());
    wait_until(&mut app, |a| !a.busy && !a.song_loading && a.document().is_some());
    assert!(app.error.is_none(), "{:?}", app.error);
    (app, osu_path)
}

fn lane_key(sound: Hitsound) -> String {
    lane_layout_key(&SampleIdentity::Standard {
        set: SampleSet::Soft,
        sound: Sound::Hit(sound),
        index: CustomIndex::new(0),
    })
}

#[test]
fn node_graph_lifecycle_run_viewer_cancel_stale_apply_undo_and_json_round_trip() {
    let scratch = Scratch::new("node-graph-app");
    let (mut app, osu_path) = open_fixture(&scratch);

    let view0 = app.node_graph_view();
    assert!(view0.has_document);
    assert!(view0.has_song);
    assert_eq!(view0.catalog.len(), 18);
    assert_eq!(view0.nodes.len(), 21);
    assert!(view0.validation_issues.is_empty(), "{:?}", view0.validation_issues);

    // Lower thresholds slightly on the classic fixture just to ensure hits appear cleanly.
    app.node_graph_action(NodeGraphAction::SetParam {
        node_id: "peaks_kick".into(),
        param: "threshold".into(),
        value: serde_json::json!(1.0),
    })
    .unwrap();
    app.node_graph_action(NodeGraphAction::SetParam {
        node_id: "peaks_snare".into(),
        param: "threshold".into(),
        value: serde_json::json!(1.0),
    })
    .unwrap();

    // Bind kick and snare slots + reject slider-body lane bindings.
    app.node_graph_action(NodeGraphAction::SetBinding {
        slot: "kick".into(),
        lane_key: Some(lane_key(Hitsound::Finish)),
    })
    .unwrap();
    app.node_graph_action(NodeGraphAction::SetBinding {
        slot: "snare".into(),
        lane_key: Some(lane_key(Hitsound::Clap)),
    })
    .unwrap();
    let slide_key = lane_layout_key(&SampleIdentity::Standard {
        set: SampleSet::Soft,
        sound: Sound::Slider(dawsu_foundation::SliderSound::Slide),
        index: CustomIndex::new(0),
    });
    assert!(
        app.node_graph_action(NodeGraphAction::SetBinding {
            slot: "kick".into(),
            lane_key: Some(slide_key),
        })
        .is_err()
    );

    // Preview curve socket during run + verify Run never sets `app.busy`.
    app.node_graph_action(NodeGraphAction::SetViewerTarget {
        node: Some("flux_kick".into()),
        socket: Some("curve".into()),
    })
    .unwrap();
    app.node_graph_action(NodeGraphAction::Run).unwrap();
    assert!(!app.busy);
    wait_until(&mut app, |a| a.node_graph_view().status == "ready");

    let ready_view = app.node_graph_view();
    assert!(ready_view.error.is_none(), "{:?}", ready_view.error);
    let batch = ready_view.batch.as_ref().expect("ready batch");
    assert!(!batch.stale);
    assert!(batch.counts.total > 0);
    assert!(matches!(
        ready_view.viewer,
        Some(ViewerOverlay::Curve { ref values, .. }) if !values.is_empty() && values.len() <= 2048
    ));

    // Also verify ViewerOverlay for an events socket (`snap.events`).
    app.node_graph_action(NodeGraphAction::SetViewerTarget {
        node: Some("snap".into()),
        socket: Some("events".into()),
    })
    .unwrap();
    app.node_graph_action(NodeGraphAction::Run).unwrap();
    wait_until(&mut app, |a| a.node_graph_view().status == "ready");
    let event_view = app.node_graph_view();
    assert!(matches!(
        event_view.viewer,
        Some(ViewerOverlay::Events { ref times_ms, .. }) if !times_ms.is_empty()
    ));

    // Editing a node parameter while running cancels the in-flight job cleanly.
    app.node_graph_action(NodeGraphAction::Run).unwrap();
    app.node_graph_action(NodeGraphAction::SetParam {
        node_id: "peaks_kick".into(),
        param: "threshold".into(),
        value: serde_json::json!(1.05),
    })
    .unwrap();
    assert_ne!(app.node_graph_view().status, "running");

    // Undo param edit back to 1.0 and get a fresh Ready batch.
    app.node_graph_action(NodeGraphAction::Undo).unwrap();
    app.node_graph_action(NodeGraphAction::Run).unwrap();
    wait_until(&mut app, |a| a.node_graph_view().status == "ready");
    let before_hits = app.document().unwrap().composer.hitsounds().moments().iter().map(|m| m.hits().count()).sum::<usize>();

    // Mutate composer via public command -> batch becomes stale and Apply refuses while keeping batch visible.
    app.command("selectAll");
    app.command("delete");
    let stale_view = app.node_graph_view();
    assert!(stale_view.batch.as_ref().unwrap().stale);
    assert!(app.node_graph_action(NodeGraphAction::Apply).is_err());
    assert!(app.node_graph_view().batch.is_some());

    // Undo map edit -> re-run -> Apply as one undo step -> SelectLastHits -> undo restores hit counts.
    app.command("undo");
    app.node_graph_action(NodeGraphAction::Run).unwrap();
    wait_until(&mut app, |a| a.node_graph_view().status == "ready");
    app.node_graph_action(NodeGraphAction::Apply).unwrap();
    let after_hits: usize = app.document().unwrap().composer.hitsounds().moments().iter().map(|m| m.hits().count()).sum();
    assert!(after_hits > before_hits);
    assert!(app.node_graph_view().can_select_last_hits);
    app.node_graph_action(NodeGraphAction::SelectLastHits).unwrap();
    assert!(!app.document().unwrap().composer.selected.is_empty());
    app.command("undo");
    let undone_hits: usize = app.document().unwrap().composer.hitsounds().moments().iter().map(|m| m.hits().count()).sum();
    assert_eq!(undone_hits, before_hits);

    // Export JSON -> LoadPreset("empty") -> ImportJson restores identical structural hash.
    let exported = app.node_graph_export_json().unwrap();
    let original_hash = app.node_graph_view().graph_hash;
    app.node_graph_action(NodeGraphAction::LoadPreset { name: "empty".into() }).unwrap();
    assert_eq!(app.node_graph_view().nodes.len(), 0);
    app.node_graph_action(NodeGraphAction::ImportJson { json: exported }).unwrap();
    assert_eq!(app.node_graph_view().graph_hash, original_hash);

    // Bindings persist cleanly across save_session + reopen.
    app.save_session().unwrap();
    let mut reopened = Application::headless_with_projects(scratch.path("data"));
    reopened.request_open(osu_path);
    wait_until(&mut reopened, |a| !a.busy && a.document().is_some());
    let reopened_slots = reopened.node_graph_view().slots;
    assert_eq!(
        reopened_slots.iter().find(|s| s.slot == "kick").and_then(|s| s.bound_lane_key.as_deref()),
        Some(lane_key(Hitsound::Finish).as_str())
    );
}

#[test]
fn range_change_stales_batch_map_edit_cancels_run_viewer_clears_and_placeholder_infers_sockets() {
    let scratch = Scratch::new("node-graph-app-fixes");
    let (mut app, _osu_path) = open_fixture(&scratch);

    app.node_graph_action(NodeGraphAction::SetBinding {
        slot: "kick".into(),
        lane_key: Some(lane_key(Hitsound::Finish)),
    })
    .unwrap();
    app.node_graph_action(NodeGraphAction::SetParam {
        node_id: "peaks_kick".into(),
        param: "threshold".into(),
        value: serde_json::json!(1.0),
    })
    .unwrap();
    app.node_graph_action(NodeGraphAction::SetViewerTarget {
        node: Some("flux_kick".into()),
        socket: Some("curve".into()),
    })
    .unwrap();
    app.node_graph_action(NodeGraphAction::Run).unwrap();
    wait_until(&mut app, |a| a.node_graph_view().status == "ready");
    assert!(app.node_graph_view().viewer.is_some());

    // 1. Clearing viewer target immediately clears timeline overlay.
    app.node_graph_action(NodeGraphAction::SetViewerTarget {
        node: None,
        socket: None,
    })
    .unwrap();
    assert!(app.node_graph_view().viewer.is_none());

    // 2. Switching to locator range marks the batch stale (`Range`) and refuses Apply.
    app.locator_left = 100.0;
    app.locator_right = 1500.0;
    app.node_graph_action(NodeGraphAction::SetRangeMode {
        use_locator_range: true,
    })
    .unwrap();
    let range_stale = app.node_graph_view();
    let batch = range_stale.batch.as_ref().expect("batch remains visible");
    assert!(batch.stale);
    assert_eq!(batch.stale_reason.as_deref(), Some("Range"));
    assert!(app.node_graph_action(NodeGraphAction::Apply).is_err());

    // 3. Map edit while a run is in flight cancels the job on `poll()`.
    app.node_graph_action(NodeGraphAction::Run).unwrap();
    assert_eq!(app.node_graph_view().status, "running");
    app.command("selectAll");
    app.command("delete");
    app.poll();
    assert_ne!(app.node_graph_view().status, "running");

    // 4. Placeholder node exposes inferred input/output sockets in `node_graph_view()`.
    let placeholder_doc = r#"{
  "format": "dawsu-graph",
  "version": 1,
  "name": "Placeholder",
  "description": "",
  "nodes": [
    { "id": "src", "type": "input.song", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 0.0, "y": 0.0 } },
    { "id": "future", "type": "future.ml_stem", "typeVersion": 9, "params": {}, "muted": false, "ui": { "x": 200.0, "y": 0.0 } },
    { "id": "view", "type": "output.viewer", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 400.0, "y": 0.0 } }
  ],
  "links": [
    { "from": { "node": "src", "socket": "audio" }, "to": { "node": "future", "socket": "stem_in" } },
    { "from": { "node": "future", "socket": "drums_out" }, "to": { "node": "view", "socket": "value" } }
  ],
  "outputs": []
}"#;
    app.node_graph_action(NodeGraphAction::ImportJson {
        json: placeholder_doc.to_owned(),
    })
    .unwrap();
    let ph_view = app.node_graph_view();
    let ph = ph_view
        .nodes
        .iter()
        .find(|n| n.id == "future")
        .expect("placeholder node");
    assert!(ph.placeholder.is_some());
    assert_eq!(ph.inputs.len(), 1);
    assert_eq!(ph.inputs[0].id, "stem_in");
    assert_eq!(ph.outputs.len(), 1);
    assert_eq!(ph.outputs[0].id, "drums_out");
}

#[test]
fn starter_presets_validate_cleanly_and_populate_per_node_summaries() {
    let scratch = Scratch::new("node-graph-starter-presets");
    let (mut app, _) = open_fixture(&scratch);

    for preset in ["simple_kick", "simple_hihat", "drum_stem"] {
        app.node_graph_action(NodeGraphAction::LoadPreset { name: preset.into() }).unwrap();
        let v = app.node_graph_view();
        assert!(v.validation_issues.is_empty(), "{preset}: {:?}", v.validation_issues);
        assert!(v.nodes.len() <= 10, "{preset} should stay compact");
    }

    app.node_graph_action(NodeGraphAction::LoadPreset { name: "simple_kick".into() }).unwrap();
    app.node_graph_action(NodeGraphAction::SetParam {
        node_id: "peaks_kick".into(),
        param: "threshold".into(),
        value: serde_json::json!(1.0),
    })
    .unwrap();
    app.node_graph_action(NodeGraphAction::Run).unwrap();
    wait_until(&mut app, |a| a.node_graph_view().status == "ready");

    let json = serde_json::to_value(app.node_graph_view()).unwrap();
    let nodes = json["nodes"].as_array().unwrap();
    let flux = nodes.iter().find(|n| n["id"] == "flux_kick").unwrap();
    assert_eq!(flux["summary"]["kind"], "curve");
    assert!(!flux["summary"]["sparkline"].as_array().unwrap().is_empty());
    let peaks = nodes.iter().find(|n| n["id"] == "peaks_kick").unwrap();
    assert_eq!(peaks["summary"]["kind"], "events");
    assert!(peaks["summary"]["count"].as_u64().unwrap() > 0);
}
