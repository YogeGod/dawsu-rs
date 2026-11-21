//! Headless node-graph parity, laziness, caching, diagnostics, and node tests for `dawsu-graph-audio`.

use dawsu_graph::{
    AnalysisRange, DiagnosticSeverity, Endpoint, EvalCache, EvalError, EvalTarget, GraphDocument,
    Link, NodeData, NodeRegistry, NodeUi, SocketValue, TimingGrid, TimingLine, evaluate_graph,
    validate_graph,
};
use dawsu_graph_audio::{
    DecodedAudio, StaticProviders, classic_kick_snare_graph, register_audio_nodes,
    set_classic_sensitivity, set_classic_snap_muted,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

const RATE: u32 = 44_100;

fn fixture(name: &str) -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .join("tests/fixtures/classic-3.1.4")
        .join(name)
}

fn pattern_samples() -> Vec<f32> {
    std::fs::read(fixture("pattern.f32"))
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}

fn f64s(name: &str) -> Vec<f64> {
    std::fs::read(fixture(name))
        .unwrap()
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect()
}

fn onsets_json() -> Value {
    serde_json::from_slice(&std::fs::read(fixture("onsets.json")).unwrap()).unwrap()
}

fn setup_for_samples(samples: Vec<f32>, timing: Arc<TimingGrid>) -> (NodeRegistry, f64) {
    let song = Arc::new(DecodedAudio::from_mono(samples, RATE, 0.0));
    let duration_ms = song.duration_ms();
    let providers = Arc::new(StaticProviders::with_song(song, timing));
    let mut reg = NodeRegistry::new();
    register_audio_nodes(&mut reg, providers.clone(), providers).unwrap();
    (reg, duration_ms)
}

#[test]
fn classic_preset_loads_validates_and_matches_3_1_4_curves_bit_for_bit() {
    let (reg, duration_ms) = setup_for_samples(pattern_samples(), Arc::new(TimingGrid::default()));
    let doc = classic_kick_snare_graph(&reg).unwrap();
    let report = validate_graph(&doc, &reg);
    assert!(report.is_valid(), "unexpected issues: {:?}", report.issues);

    let range = AnalysisRange::new(0.0, duration_ms).unwrap();
    let mut cache = EvalCache::default();

    for (node_id, fixture_name) in [
        ("flux_kick", "curve-kick.f64"),
        ("flux_body", "curve-body.f64"),
        ("flux_wires", "curve-noise.f64"),
        ("flux_hat", "curve-hat.f64"),
        ("snare_curve", "curve-snare.f64"),
    ] {
        let res = evaluate_graph(
            &doc,
            &reg,
            &mut cache,
            EvalTarget::Socket(Endpoint::new(node_id, "curve")),
            range,
            Some(duration_ms),
            &|| false,
            &mut |_| {},
        )
        .unwrap();
        let Some(SocketValue::Curve(curve)) = res.socket_preview else {
            panic!("missing curve from {node_id}");
        };
        let expected = f64s(fixture_name);
        assert_eq!(curve.values.len(), expected.len(), "{node_id} length");
        for (i, (a, b)) in curve.values.iter().zip(&expected).enumerate() {
            assert_eq!(a.to_bits(), b.to_bits(), "{node_id} frame {i}: {a} vs {b}");
        }
    }
}

#[test]
fn classic_preset_matches_3_1_4_onsets_at_all_sensitivities() {
    let (reg, duration_ms) = setup_for_samples(pattern_samples(), Arc::new(TimingGrid::default()));
    let mut doc = classic_kick_snare_graph(&reg).unwrap();
    // Bypass snap so we compare raw onset picking + kick-bleed against `onsets.json`.
    set_classic_snap_muted(&mut doc, true);
    let range = AnalysisRange::new(0.0, duration_ms).unwrap();
    let fixture = onsets_json();
    let mut cache = EvalCache::default();

    for (scale, key) in [(2.0, "2.00"), (1.5, "1.50"), (1.25, "1.25"), (1.0, "1.00")] {
        set_classic_sensitivity(&mut doc, scale);
        let res = evaluate_graph(
            &doc,
            &reg,
            &mut cache,
            EvalTarget::AllLaneOutputs,
            range,
            Some(duration_ms),
            &|| false,
            &mut |_| {},
        )
        .unwrap();

        let mut actual = Vec::new();
        for (slot, list) in &res.slot_events {
            for ev in &list.events {
                actual.push((slot.clone(), ev.time_ms, (ev.strength * 10_000.0).round_ties_even() / 10_000.0));
            }
        }
        actual.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

        let expected = fixture[key].as_array().unwrap();
        assert_eq!(actual.len(), expected.len(), "scale {scale}");
        for ((drum, time_ms, score), exp) in actual.iter().zip(expected) {
            assert_eq!(drum, exp["drum"].as_str().unwrap(), "scale {scale}");
            assert_eq!(
                format!("{time_ms:.4}"),
                format!("{:.4}", exp["time_ms"].as_f64().unwrap()),
                "scale {scale}"
            );
            assert_eq!(
                format!("{score:.4}"),
                format!("{:.4}", exp["score"].as_f64().unwrap()),
                "scale {scale}"
            );
        }
    }
}

#[test]
fn pull_evaluation_and_caching_on_classic_preset() {
    let (reg, duration_ms) = setup_for_samples(pattern_samples(), Arc::new(TimingGrid::default()));
    let mut doc = classic_kick_snare_graph(&reg).unwrap();
    let range = AnalysisRange::new(0.0, duration_ms).unwrap();
    let mut cache = EvalCache::default();

    // Pulling just `flux_kick.curve` must skip snare, peak pick, measure, and snap nodes.
    let pull = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Socket(Endpoint::new("flux_kick", "curve")),
        range,
        Some(duration_ms),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert!(!pull.evaluated_nodes.contains(&"flux_body".into()));
    assert!(!pull.evaluated_nodes.contains(&"snare_curve".into()));
    assert!(!pull.evaluated_nodes.contains(&"peaks_kick".into()));
    assert!(!pull.evaluated_nodes.contains(&"snap".into()));

    // Running AllLaneOutputs reuses cached `range..flux_kick` nodes and evaluates the remaining ones.
    let full = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::AllLaneOutputs,
        range,
        Some(duration_ms),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert!(full.cached_nodes.contains(&"hpss".into()));
    assert!(full.cached_nodes.contains(&"flux_kick".into()));
    assert!(!full.evaluated_nodes.contains(&"hpss".into()));

    // Changing sensitivity hits upstream `hpss`/curves and re-evaluates only peak pick onward.
    set_classic_sensitivity(&mut doc, 1.0);
    let resens = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::AllLaneOutputs,
        range,
        Some(duration_ms),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert!(resens.cached_nodes.contains(&"snare_curve".into()));
    assert!(resens.evaluated_nodes.contains(&"peaks_kick".into()));
    assert!(resens.evaluated_nodes.contains(&"peaks_snare".into()));

    // Cancelled evaluation leaves cache empty.
    let mut fresh_cache = EvalCache::default();
    let err = evaluate_graph(
        &doc,
        &reg,
        &mut fresh_cache,
        EvalTarget::AllLaneOutputs,
        range,
        Some(duration_ms),
        &|| true,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(err, EvalError::Cancelled);
    assert_eq!(fresh_cache.stats().entries, 0);
}

#[test]
fn diagnostics_for_missing_audio_empty_range_silence_short_range_and_lag() {
    // 1. Missing song audio reports error on `song` and "waiting for input" downstream.
    let missing_providers = Arc::new(StaticProviders::missing_song(
        "file missing",
        Arc::new(TimingGrid::default()),
    ));
    let mut reg = NodeRegistry::new();
    register_audio_nodes(&mut reg, missing_providers.clone(), missing_providers).unwrap();
    let doc = classic_kick_snare_graph(&reg).unwrap();
    let mut cache = EvalCache::default();
    let res = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::AllLaneOutputs,
        AnalysisRange::new(0.0, 2000.0).unwrap(),
        Some(2000.0),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert!(res.diagnostics[&"song".into()][0].message.contains("The song could not be decoded."));
    assert_eq!(res.diagnostics[&"mono".into()][0].message, "waiting for input");
    assert!(res.slot_events.is_empty());

    // 2. Range beyond song audio emits "The selected range contains no song audio."
    let (reg, duration_ms) = setup_for_samples(vec![0.5; 4410], Arc::new(TimingGrid::default()));
    let mut cache = EvalCache::default();
    let empty_range = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::AllLaneOutputs,
        AnalysisRange::new(duration_ms + 100.0, duration_ms + 1000.0).unwrap(),
        Some(duration_ms),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert!(
        empty_range.diagnostics[&"song".into()][0]
            .message
            .contains("contains no song audio")
    );

    // 3. Silent audio yields no events and Info (not Error).
    let (reg_silent, dur_silent) = setup_for_samples(vec![0.0; RATE as usize], Arc::new(TimingGrid::default()));
    let silent = evaluate_graph(
        &doc,
        &reg_silent,
        &mut cache,
        EvalTarget::AllLaneOutputs,
        AnalysisRange::new(0.0, dur_silent).unwrap(),
        Some(dur_silent),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(silent.diagnostics[&"mono".into()][0].severity, DiagnosticSeverity::Info);
    assert!(silent.slot_events["kick"].events.is_empty());
    assert!(silent.slot_events["snare"].events.is_empty());

    // 4. Too-short audio yields no events and Info.
    let (reg_short, dur_short) = setup_for_samples(vec![0.5; 100], Arc::new(TimingGrid::default()));
    let short = evaluate_graph(
        &doc,
        &reg_short,
        &mut cache,
        EvalTarget::AllLaneOutputs,
        AnalysisRange::new(0.0, dur_short).unwrap(),
        Some(dur_short),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(short.diagnostics[&"stft".into()][0].severity, DiagnosticSeverity::Info);
    assert!(short.slot_events["kick"].events.is_empty());

    // 5. Excessive lag (> 50 ms) triggers a Warning naming the measured value on `events.snap`.
    // 120 BPM from 0 ms -> beat = 500 ms, 1/4 step = 125 ms. Shift pattern by +62 ms.
    let timing = Arc::new(TimingGrid::new(vec![TimingLine {
        start_ms: 0.0,
        beat_length_ms: 500.0,
    }]));
    let mut delayed = vec![0.0_f32; (RATE as f64 * 0.062) as usize];
    delayed.extend(pattern_samples());
    let (reg_lag, dur_lag) = setup_for_samples(delayed, timing);
    let lag_res = evaluate_graph(
        &doc,
        &reg_lag,
        &mut cache,
        EvalTarget::AllLaneOutputs,
        AnalysisRange::new(0.0, dur_lag).unwrap(),
        Some(dur_lag),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert!(
        lag_res.diagnostics[&"snap".into()]
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Warning && d.message.contains("exceeds")),
        "expected excessive lag warning, got: {:?}",
        lag_res.diagnostics.get(&"snap".into())
    );
}

#[test]
fn filter_time_sampler_stem_file_and_viewer_nodes_work_deterministically() {
    let song = Arc::new(DecodedAudio::from_mono(vec![0.2; RATE as usize * 2], RATE, 0.0));
    let stem = Arc::new(DecodedAudio::from_mono(vec![0.5; RATE as usize / 2], RATE, 0.0));
    let timing = Arc::new(TimingGrid::new(vec![TimingLine {
        start_ms: 0.0,
        beat_length_ms: 500.0,
    }]));
    let mut providers = StaticProviders::with_song(song, timing);
    providers.files.insert("drums.wav".into(), Ok(stem));
    let providers = Arc::new(providers);
    let mut reg = NodeRegistry::new();
    register_audio_nodes(&mut reg, providers.clone(), providers).unwrap();

    let mut doc = GraphDocument::new("nodes");
    let n = |id: &str, ty: &str, params: BTreeMap<String, Value>| NodeData {
        id: id.into(),
        type_id: ty.into(),
        type_version: 1,
        params,
        muted: false,
        ui: NodeUi::default(),
        placeholder: None,
    };
    doc.nodes.push(n(
        "stem",
        "input.file",
        BTreeMap::from([
            ("path".into(), serde_json::json!("drums.wav")),
            ("expectLengthOfSong".into(), serde_json::json!(true)),
        ]),
    ));
    doc.nodes.push(n(
        "filt",
        "audio.filter",
        BTreeMap::from([("mode".into(), serde_json::json!("bandpass"))]),
    ));
    doc.nodes.push(n("stft", "spectral.stft", BTreeMap::new()));
    doc.nodes.push(n(
        "flux",
        "onset.flux",
        BTreeMap::from([("normalise".into(), serde_json::json!("off"))]),
    ));
    doc.nodes.push(n("timing", "input.timing", BTreeMap::new()));
    doc.nodes.push(n(
        "sampler",
        "events.sample_time",
        BTreeMap::from([
            ("mode".into(), serde_json::json!("beat-grid")),
            ("divisor".into(), serde_json::json!(4)),
        ]),
    ));
    doc.nodes.push(n("view", "output.viewer", BTreeMap::new()));

    doc.links.push(Link::new(Endpoint::new("stem", "audio"), Endpoint::new("filt", "audio")));
    doc.links.push(Link::new(Endpoint::new("filt", "audio"), Endpoint::new("stft", "audio")));
    doc.links.push(Link::new(
        Endpoint::new("stft", "spectrogram"),
        Endpoint::new("flux", "spectrogram"),
    ));
    doc.links.push(Link::new(Endpoint::new("flux", "curve"), Endpoint::new("sampler", "curve")));
    doc.links.push(Link::new(Endpoint::new("timing", "timing"), Endpoint::new("sampler", "timing")));
    doc.links.push(Link::new(Endpoint::new("sampler", "events"), Endpoint::new("view", "value")));

    let mut cache = EvalCache::default();
    let res = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["view".into()]),
        AnalysisRange::new(0.0, 500.0).unwrap(),
        Some(2000.0),
        &|| false,
        &mut |_| {},
    )
    .unwrap();

    // Stem length (500 ms) vs song length (2000 ms) warns.
    assert!(
        res.diagnostics[&"stem".into()]
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Warning && d.message.contains("stem length"))
    );
    // Beat-grid sampler at 120 BPM 1/4 over [0, 500) yields 4 events (0, 125, 250, 375 ms).
    let Some(SocketValue::Events(events)) = res.viewer_values.get(&"view".into()) else {
        panic!("expected viewer events");
    };
    assert_eq!(events.events.len(), 4);
    assert_eq!(events.events[0].time_ms, 0.0);
    assert_eq!(events.events[1].time_ms, 125.0);
    assert_eq!(events.events[2].time_ms, 250.0);
    assert_eq!(events.events[3].time_ms, 375.0);
}

#[test]
fn boundary_params_bypass_without_secondary_input_and_song_normalisation_work() {
    // 1. Muted events.snap without timing input connected still bypasses events cleanly;
    //    silenceBelow = 0.0 on all-zero audio produces silent buffer (no NaN);
    //    window = 1 clamps cleanly without panic; divisors = [] snaps off.
    let samples = pattern_samples();
    let song = Arc::new(DecodedAudio::from_mono(samples.clone(), RATE, 0.0));
    let dur = song.duration_ms();
    let mut providers = StaticProviders::with_song(song, Arc::new(TimingGrid::default()));
    providers.files.insert(
        "silence.wav".into(),
        Ok(Arc::new(DecodedAudio::from_mono(vec![0.0; 4410], RATE, 0.0))),
    );
    providers.files.insert(
        "offset.wav".into(),
        Ok(Arc::new(DecodedAudio::from_mono(samples, RATE, 250.0))),
    );
    let providers = Arc::new(providers);
    let mut reg = NodeRegistry::new();
    register_audio_nodes(&mut reg, providers.clone(), providers).unwrap();

    // silenceBelow=0 + window=1
    let n = |id: &str, ty: &str, params: BTreeMap<String, Value>| NodeData {
        id: id.into(),
        type_id: ty.into(),
        type_version: 1,
        params,
        muted: false,
        ui: NodeUi::default(),
        placeholder: None,
    };
    let mut doc = GraphDocument::new("edge");
    doc.nodes.push(n(
        "f",
        "input.file",
        BTreeMap::from([("path".into(), serde_json::json!("silence.wav"))]),
    ));
    doc.nodes.push(n(
        "m",
        "audio.mono",
        BTreeMap::from([("silenceBelow".into(), serde_json::json!(0.0))]),
    ));
    doc.nodes.push(n(
        "s",
        "spectral.stft",
        BTreeMap::from([("window".into(), serde_json::json!(1))]),
    ));
    doc.links.push(Link::new(Endpoint::new("f", "audio"), Endpoint::new("m", "audio")));
    doc.links.push(Link::new(Endpoint::new("m", "audio"), Endpoint::new("s", "audio")));

    let mut cache = EvalCache::default();
    let res = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Socket(Endpoint::new("m", "audio")),
        AnalysisRange::new(0.0, 100.0).unwrap(),
        Some(100.0),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let Some(SocketValue::Audio(buf)) = res.socket_preview else {
        panic!("expected audio");
    };
    assert!(buf.silent);
    assert!(buf.channels[0].iter().all(|v| v.is_finite() && *v == 0.0));

    // Verify stft with window=1 succeeds without panicking.
    evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["s".into()]),
        AnalysisRange::new(0.0, 100.0).unwrap(),
        Some(100.0),
        &|| false,
        &mut |_| {},
    )
    .unwrap();

    // 2. normaliseOver = "song" on audio.mono keeps peak scaling invariant across sub-ranges.
    let mut norm_doc = GraphDocument::new("song-norm");
    norm_doc.nodes.push(n("song", "input.song", BTreeMap::new()));
    norm_doc.nodes.push(n(
        "mono",
        "audio.mono",
        BTreeMap::from([("normaliseOver".into(), serde_json::json!("song"))]),
    ));
    norm_doc.links.push(Link::new(Endpoint::new("song", "audio"), Endpoint::new("mono", "audio")));

    let full_res = evaluate_graph(
        &norm_doc,
        &reg,
        &mut cache,
        EvalTarget::Socket(Endpoint::new("mono", "audio")),
        AnalysisRange::new(0.0, dur).unwrap(),
        Some(dur),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let sub_res = evaluate_graph(
        &norm_doc,
        &reg,
        &mut cache,
        EvalTarget::Socket(Endpoint::new("mono", "audio")),
        AnalysisRange::new(500.0, 1000.0).unwrap(),
        Some(dur),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let (Some(SocketValue::Audio(full_buf)), Some(SocketValue::Audio(sub_buf))) =
        (full_res.socket_preview, sub_res.socket_preview)
    else {
        panic!("expected audio buffers");
    };
    let start_idx = (500.0 * f64::from(RATE) / 1000.0).round() as usize;
    assert_eq!(
        sub_buf.channels[0][0].to_bits(),
        full_buf.channels[0][start_idx].to_bits()
    );

    // 3. input.file inherits decoded offset_ms when offsetMs is omitted, and events.condition
    //    accepts numeric lhs while reporting invalid clauses.
    let mut cond_doc = GraphDocument::new("cond");
    cond_doc.nodes.push(n(
        "f",
        "input.file",
        BTreeMap::from([("path".into(), serde_json::json!("offset.wav"))]),
    ));
    cond_doc.nodes.push(n(
        "bad_cond",
        "events.condition",
        BTreeMap::from([("clauses".into(), serde_json::json!("not-an-array"))]),
    ));
    let mut bypassed_snap = n("snap", "events.snap", BTreeMap::new());
    bypassed_snap.muted = true;
    cond_doc.nodes.push(bypassed_snap);
    cond_doc.nodes.push(n(
        "c",
        "events.condition",
        BTreeMap::from([(
            "clauses".into(),
            serde_json::json!([[{ "lhs": 0.5, "op": "<", "factor": 1.0, "rhs": "strength" }]]),
        )]),
    ));
    let file_out = evaluate_graph(
        &cond_doc,
        &reg,
        &mut cache,
        EvalTarget::Socket(Endpoint::new("f", "audio")),
        AnalysisRange::new(250.0, 500.0).unwrap(),
        Some(dur + 250.0),
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let Some(SocketValue::Audio(offset_buf)) = file_out.socket_preview else {
        panic!("expected audio");
    };
    assert_eq!(offset_buf.start_ms, 250.0);
}

