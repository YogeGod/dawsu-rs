//! End-to-end M4 Node Graph integration test:
//! `.osu` -> decompile -> Composer -> dawsu-graph + dawsu-graph-audio -> dawsu-editor-nodal
//! -> apply_proposals -> single undo -> byte-identical `.osu` round-trip.

use dawsu_editor::tempo::{TempoMap, TimingSection};
use dawsu_editor_composer::Composer;
use dawsu_editor_nodal::{
    AnalysisStamp, ApplyOptions, NodalSession, ProposalBatch, SlotBindings, apply_proposals,
    select_batch_hits,
};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_graph::{AnalysisRange, EvalCache, EvalTarget, NodeId, NodeRegistry, TimingGrid, TimingLine, evaluate_graph};
use dawsu_graph_audio::{
    DecodedAudio, StaticProviders, classic_kick_snare_graph, register_audio_nodes,
};
use dawsu_hitsounding::{Compiled, Decompiled, Placement, compile, decompile};
use dawsu_osu_format::OsuFile;
use dawsu_test_support::workspace_root;
use std::collections::BTreeMap;
use std::sync::Arc;

const RATE: u32 = 44_100;

const TEST_OSU: &str = concat!(
    "osu file format v14\n",
    "[General]\n",
    "SampleSet:Soft\n",
    "Mode:0\n",
    "[Metadata]\n",
    "Version:NodalE2E\n",
    "[TimingPoints]\n",
    "0,500,4,2,0,45,1,0\n",
    "[HitObjects]\n",
    "100,100,0,1,0,0:0:0:0:\n",
    "200,100,500,1,0,0:0:0:0:\n",
    "300,100,1000,1,0,0:0:0:0:\n",
    "400,100,1500,1,0,0:0:0:0:\n",
    "100,100,2000,1,0,0:0:0:0:\n",
    "200,100,2500,1,0,0:0:0:0:\n",
);

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

fn std_lane(sound: Hitsound) -> SampleIdentity {
    SampleIdentity::Standard {
        set: SampleSet::Soft,
        sound: Sound::Hit(sound),
        index: CustomIndex::new(0),
    }
}

#[test]
fn m4_node_graph_e2e_apply_compile_and_undo_preserves_byte_identical_osu() {
    let original_bytes = TEST_OSU.as_bytes().to_vec();
    let osu_file = OsuFile::read(original_bytes.clone()).file;
    let Decompiled::Done(hits) = decompile(&osu_file, &[], &|| false, &mut |_, _| {}) else {
        panic!("expected valid osu map");
    };
    let mut composer = Composer::new(hits);
    composer.tempo = TempoMap::new(vec![TimingSection {
        beat_length_ms: 500.0,
        ..TimingSection::DEFAULT
    }]);

    let song = Arc::new(DecodedAudio::from_mono(pattern_samples(), RATE, 0.0));
    let audio_hash = song.content_hash.clone();
    let duration_ms = song.duration_ms();
    let timing = Arc::new(TimingGrid::new(vec![TimingLine {
        start_ms: 0.0,
        beat_length_ms: 500.0,
    }]));
    let providers = Arc::new(StaticProviders::with_song(song, timing));
    let mut registry = NodeRegistry::new();
    register_audio_nodes(&mut registry, providers.clone(), providers).unwrap();

    let preset = classic_kick_snare_graph(&registry).unwrap();
    let mut nodal = NodalSession::new(preset);
    nodal.clear_dirty(&nodal.dirty_nodes().iter().cloned().collect::<Vec<_>>());
    nodal
        .set_param(&"peaks_kick".into(), "threshold", serde_json::json!(1.0))
        .unwrap();
    nodal
        .set_param(&"peaks_snare".into(), "threshold", serde_json::json!(1.0))
        .unwrap();
    assert!(!nodal.dirty_nodes().is_empty());

    let range = AnalysisRange::new(0.0, duration_ms).unwrap();
    let mut cache = EvalCache::default();
    let eval = evaluate_graph(
        nodal.document(),
        &registry,
        &mut cache,
        EvalTarget::AllLaneOutputs,
        range,
        Some(duration_ms),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert!(!eval.slot_events["kick"].events.is_empty());
    assert!(!eval.slot_events["snare"].events.is_empty());

    let bindings = SlotBindings::from([
        ("kick".to_owned(), std_lane(Hitsound::Finish)),
        ("snare".to_owned(), std_lane(Hitsound::Clap)),
    ]);
    let slot_nodes = BTreeMap::from([
        ("kick".to_owned(), NodeId::from("out_kick")),
        ("snare".to_owned(), NodeId::from("out_snare")),
    ]);
    let stamp = AnalysisStamp::new(
        composer.history.version(),
        0.0,
        audio_hash,
        eval.graph_hash.clone(),
        range,
    );
    let batch = ProposalBatch::from_eval_result(
        "batch-1",
        &nodal.document().name,
        &eval,
        &bindings,
        &slot_nodes,
        stamp.clone(),
    );
    assert!(!batch.proposals.is_empty());

    let report = apply_proposals(
        &mut composer,
        &batch,
        &stamp,
        &ApplyOptions::default(),
    )
    .unwrap();
    assert!(report.counts.added > 0);
    let record = report.record.expect("added hits produce a batch record");
    assert_eq!(select_batch_hits(&mut composer, &record).len(), report.counts.added);

    // Compile after apply: succeeds cleanly and modifies hit objects.
    let Compiled::Done { file: applied_file, problems } = compile(
        &osu_file,
        composer.hitsounds(),
        Placement::GreenLines,
        &|| false,
        &mut |_, _| {},
    ) else {
        panic!("expected compile to finish");
    };
    assert!(problems.is_empty(), "unexpected compile problems: {problems:?}");
    let applied_bytes = applied_file.to_bytes();
    assert_ne!(applied_bytes, original_bytes);

    // Single undo removes all graph-added hits and compiles back byte-identically.
    composer.undo();
    let Compiled::Done { file: undone_file, problems } = compile(
        &osu_file,
        composer.hitsounds(),
        Placement::GreenLines,
        &|| false,
        &mut |_, _| {},
    ) else {
        panic!("expected compile to finish");
    };
    assert!(problems.is_empty());
    assert_eq!(undone_file.to_bytes(), original_bytes);
}
