//! Unit tests for proposal conversion, stale-analysis checks, single-step undo, dedupe, and batch re-selection.

use dawsu_editor::tempo::{TempoMap, TimingSection};
use dawsu_editor_nodal::{
    AnalysisStamp, ApplyError, ApplyOptions, ProposalBatch, STALE_ANALYSIS_MESSAGE, SlotBindings,
    StaleReason, apply_proposals, select_batch_hits,
};
use dawsu_editor_composer::Composer;
use dawsu_foundation::SampleIdentity;
use dawsu_graph::{AnalysisRange, EventList, GraphEvalResult, GraphEvent, NodeId};
use dawsu_hitsounding::{Decompiled, Place, decompile};
use dawsu_osu_format::OsuFile;
use std::collections::BTreeMap;
use std::sync::Arc;

const MAP_TEXT: &str = "osu file format v14\n[General]\nSampleSet:Soft\nMode:0\n[Metadata]\nVersion:Test\n[TimingPoints]\n0,500,4,2,0,45,1,0\n[HitObjects]\n100,100,1000,1,0,0:0:0:0:\n100,100,1500,1,0,0:0:0:0:\n";

fn lane(name: &str) -> SampleIdentity {
    SampleIdentity::File(format!("{name}.wav"))
}

fn composer() -> Composer {
    let file = OsuFile::read(MAP_TEXT.as_bytes().to_vec()).file;
    let files = vec!["kick.wav".to_owned(), "snare.wav".to_owned()];
    let Decompiled::Done(mut hits) = decompile(&file, &files, &|| false, &mut |_, _| {}) else {
        panic!("fixture map")
    };
    hits.add_folder_samples(&files);
    let mut c = Composer::new(hits);
    c.tempo = TempoMap::new(vec![TimingSection {
        beat_length_ms: 500.0,
        ..TimingSection::DEFAULT
    }]);
    c
}

fn sample_eval_result(range: AnalysisRange) -> GraphEvalResult {
    let kick_events = Arc::new(EventList::sorted(vec![
        GraphEvent {
            time_ms: 1005.12346,
            detected_at_ms: 1001.0,
            frame: 100,
            strength: 0.91234,
            tag: "kick".into(),
            measures: BTreeMap::new(),
        },
        GraphEvent {
            time_ms: 1250.0,
            detected_at_ms: 1246.0,
            frame: 125,
            strength: 0.75,
            tag: "kick".into(),
            measures: BTreeMap::new(),
        },
    ]));
    let snare_events = Arc::new(EventList::sorted(vec![GraphEvent {
        time_ms: 1495.0,
        detected_at_ms: 1491.0,
        frame: 149,
        strength: 0.85,
        tag: "snare".into(),
        measures: BTreeMap::new(),
    }]));
    let mut slot_events = BTreeMap::new();
    slot_events.insert("kick".into(), kick_events);
    slot_events.insert("snare".into(), snare_events);
    GraphEvalResult {
        range,
        graph_hash: "graph_sha".into(),
        slot_events,
        ..Default::default()
    }
}

fn base_stamp(range: AnalysisRange) -> AnalysisStamp {
    AnalysisStamp::new(0, 0.0, "audio_sha", "graph_sha", range)
}

#[test]
fn conversion_rounds_to_4_decimals_ignores_unbound_slots_and_applies_as_one_undo_step() {
    let mut c = composer();
    let before = c.hitsounds().clone();
    let range = AnalysisRange::new(0.0, 2000.0).unwrap();
    let eval = sample_eval_result(range);
    let stamp = base_stamp(range);

    // Bind only `kick`; `snare` is unbound and produces no proposals.
    let bindings = SlotBindings::from([("kick".to_owned(), lane("kick"))]);
    let slot_nodes = BTreeMap::from([("kick".to_owned(), NodeId::from("out_kick"))]);
    let mut batch =
        ProposalBatch::from_eval_result("b1", "Classic", &eval, &bindings, &slot_nodes, stamp.clone());
    assert_eq!(batch.proposals.len(), 2);
    assert_eq!(batch.proposals[0].time_ms, 1005.1235);
    assert_eq!(batch.proposals[0].strength, 0.9123);

    // Uncheck the unmatched hit at 1250 ms; only 1005.1235 -> circle@1000 remains.
    let second_id = batch.proposals[1].id.clone();
    assert!(batch.set_checked(&second_id, false));

    let report = apply_proposals(&mut c, &batch, &stamp, &ApplyOptions::default()).unwrap();
    assert_eq!(report.counts.added, 1);
    assert_eq!(report.counts.on_objects, 1);
    let record = report.record.unwrap();
    assert_eq!(record.hits.len(), 1);

    // Single undo step restores exact prior hitsounds.
    assert!(c.history.can_undo());
    c.undo();
    assert_eq!(c.hitsounds(), &before);
    c.redo();
    assert_eq!(select_batch_hits(&mut c, &record).len(), 1);
}

#[test]
fn stale_stamp_rejects_every_field_without_modifying_composer() {
    let mut c = composer();
    let range = AnalysisRange::new(0.0, 2000.0).unwrap();
    let eval = sample_eval_result(range);
    let stamp = base_stamp(range);
    let bindings = SlotBindings::from([("kick".to_owned(), lane("kick"))]);
    let batch = ProposalBatch::from_eval_result(
        "b1",
        "Classic",
        &eval,
        &bindings,
        &BTreeMap::new(),
        stamp.clone(),
    );

    let cases = [
        (
            AnalysisStamp::new(1, 0.0, "audio_sha", "graph_sha", range),
            StaleReason::MapRevision,
        ),
        (
            AnalysisStamp::new(0, 12.0, "audio_sha", "graph_sha", range),
            StaleReason::SongOffset,
        ),
        (
            AnalysisStamp::new(0, 0.0, "other_audio", "graph_sha", range),
            StaleReason::SourceAudio,
        ),
        (
            AnalysisStamp::new(0, 0.0, "audio_sha", "other_graph", range),
            StaleReason::GraphHash,
        ),
        (
            AnalysisStamp::new(
                0,
                0.0,
                "audio_sha",
                "graph_sha",
                AnalysisRange::new(100.0, 2000.0).unwrap(),
            ),
            StaleReason::Range,
        ),
    ];

    for (current, expected_reason) in cases {
        let err = apply_proposals(&mut c, &batch, &current, &ApplyOptions::default()).unwrap_err();
        assert_eq!(
            err,
            ApplyError::StaleAnalysis {
                reason: expected_reason,
                message: STALE_ANALYSIS_MESSAGE.to_owned(),
            }
        );
        assert!(!c.history.can_undo());
    }
}

#[test]
fn dedupe_unmatched_and_range_edges_work_and_zero_hits_adds_no_undo_step() {
    let mut c = composer();
    c.add(1000.0, lane("kick"), 80, 5.0);
    let rev_after_seed = c.history.version();

    let range = AnalysisRange::new(1000.0, 1500.0).unwrap();
    let events = Arc::new(EventList::sorted(vec![
        // Duplicate of existing kick at 1000 ms (within 2 ms).
        GraphEvent {
            time_ms: 1001.5,
            detected_at_ms: 1001.5,
            frame: 10,
            strength: 0.9,
            tag: "kick".into(),
            measures: BTreeMap::new(),
        },
        // Unmatched hit at 1250 ms.
        GraphEvent {
            time_ms: 1250.0,
            detected_at_ms: 1250.0,
            frame: 20,
            strength: 0.8,
            tag: "kick".into(),
            measures: BTreeMap::new(),
        },
        // Outside half-open [1000, 1500) at 1500.0 ms -> skipped.
        GraphEvent {
            time_ms: 1500.0,
            detected_at_ms: 1500.0,
            frame: 30,
            strength: 0.95,
            tag: "kick".into(),
            measures: BTreeMap::new(),
        },
    ]));
    let mut slot_events = BTreeMap::new();
    slot_events.insert("kick".into(), events);
    let eval = GraphEvalResult {
        range,
        graph_hash: "g".into(),
        slot_events,
        ..Default::default()
    };
    let stamp = AnalysisStamp::new(rev_after_seed, 0.0, "a", "g", range);
    let bindings = SlotBindings::from([("kick".to_owned(), lane("kick"))]);
    let batch = ProposalBatch::from_eval_result(
        "b2",
        "Classic",
        &eval,
        &bindings,
        &BTreeMap::new(),
        stamp.clone(),
    );

    // Without include_unmatched -> 1 duplicate, 1 unmatched, 0 added -> no undo step added.
    let dry = apply_proposals(&mut c, &batch, &stamp, &ApplyOptions::default()).unwrap();
    assert_eq!((dry.counts.added, dry.counts.duplicates, dry.counts.unmatched), (0, 1, 1));
    assert!(dry.record.is_none());
    assert_eq!(c.history.version(), rev_after_seed);

    // With include_unmatched -> places the 1250 ms hit as loose hit.
    let opts = ApplyOptions {
        include_unmatched: true,
        ..ApplyOptions::default()
    };
    let applied = apply_proposals(&mut c, &batch, &stamp, &opts).unwrap();
    assert_eq!(applied.counts.added, 1);
    assert_eq!(applied.counts.loose, 1);
    assert!(
        c.hitsounds()
            .moments()
            .iter()
            .any(|m| m.place() == Place::NoObject && m.time().ms() == 1250.0 && m.has_hit(&lane("kick")))
    );
}

#[test]
fn unrefreshed_caller_stamp_and_tiny_negative_times_are_rejected() {
    let mut c = composer();
    let range = AnalysisRange::new(0.0, 2000.0).unwrap();
    let events = Arc::new(EventList::sorted(vec![
        GraphEvent {
            time_ms: -0.00004,
            detected_at_ms: 0.0,
            frame: 0,
            strength: 0.9,
            tag: "kick".into(),
            measures: BTreeMap::new(),
        },
        GraphEvent {
            time_ms: 1000.0,
            detected_at_ms: 1000.0,
            frame: 100,
            strength: 0.9,
            tag: "kick".into(),
            measures: BTreeMap::new(),
        },
    ]));
    let eval = GraphEvalResult {
        range,
        graph_hash: "g".into(),
        slot_events: BTreeMap::from([("kick".into(), events)]),
        ..Default::default()
    };
    let stamp = AnalysisStamp::new(c.history.version(), 0.0, "a", "g", range);
    let bindings = SlotBindings::from([("kick".to_owned(), lane("kick"))]);
    let batch = ProposalBatch::from_eval_result(
        "b3",
        "Classic",
        &eval,
        &bindings,
        &BTreeMap::new(),
        stamp.clone(),
    );
    // Tiny negative event (-0.00004) was excluded before round4.
    assert_eq!(batch.proposals.len(), 1);
    assert_eq!(batch.proposals[0].time_ms, 1000.0);

    // Mutate composer without refreshing `stamp`; passing stale `stamp` as `current_stamp` still fails!
    c.add(1500.0, lane("snare"), 80, 5.0);
    let err = apply_proposals(&mut c, &batch, &stamp, &ApplyOptions::default()).unwrap_err();
    assert_eq!(
        err,
        ApplyError::StaleAnalysis {
            reason: StaleReason::MapRevision,
            message: STALE_ANALYSIS_MESSAGE.to_owned(),
        }
    );
}
