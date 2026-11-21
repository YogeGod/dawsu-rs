//! Core graph engine tests: strict typing, cycle checks, lazy pull evaluation, dirty propagation,
//! range/scope-keyed LRU caching, atomic cancellation, determinism, migrations, and placeholders.

use dawsu_graph::{
    AnalysisRange, Curve, DiagnosticSeverity, DirtyTracker, Endpoint, EvalCache, EvalContext,
    EvalError, EvalTarget, EventList, GraphDocument, GraphEvent, Link, LinkError, NodeData,
    NodeInputs, NodeOutputs, NodeParams, NodeRegistry, NodeSpec, NodeType, NodeTypeId, NodeUi,
    ParamSpec, SocketId, SocketSpec, SocketType, SocketValue, ValidationIssue, can_connect,
    evaluate_graph, validate_graph,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct ConstCurveNode {
    runs: Arc<AtomicUsize>,
}

impl NodeType for ConstCurveNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("test.curve"),
            type_version: 1,
            inputs: vec![],
            outputs: vec![SocketSpec::exact("curve", SocketType::Curve, false)],
            params: vec![
                ParamSpec::number("value", 1.0, Some(0.0), Some(100.0)),
                ParamSpec::enumeration("normaliseOver", "range", &["range", "song"]),
            ],
            bypass_passthrough: None,
        }
    }

    fn evaluate(
        &self,
        _inputs: &NodeInputs,
        params: &NodeParams<'_>,
        context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if context.is_cancelled() {
            return Err(EvalError::Cancelled);
        }
        let v = params.f64_or("value", 1.0);
        let scope_bonus = if params.str_or("normaliseOver", "range") == "song" {
            10.0
        } else {
            0.0
        };
        let curve = Arc::new(Curve {
            values: vec![v + scope_bonus, context.range.from_ms, context.range.to_ms],
            hop_ms: 10.0,
            start_ms: context.range.from_ms,
        });
        Ok(NodeOutputs::with_value("curve", SocketValue::Curve(curve)))
    }
}

struct ScaleCurveNode {
    runs: Arc<AtomicUsize>,
    cancel_on_eval: Option<Arc<AtomicBool>>,
}

impl NodeType for ScaleCurveNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("test.scale"),
            type_version: 3,
            inputs: vec![SocketSpec::exact("curve", SocketType::Curve, true)],
            outputs: vec![SocketSpec::exact("curve", SocketType::Curve, false)],
            params: vec![ParamSpec::number("gain", 2.0, Some(0.0), None)],
            bypass_passthrough: Some((SocketId::from("curve"), SocketId::from("curve"))),
        }
    }

    fn migrate_params(
        &self,
        from_version: u32,
        mut params: BTreeMap<String, serde_json::Value>,
    ) -> Result<BTreeMap<String, serde_json::Value>, String> {
        match from_version {
            1 => {
                if let Some(old) = params.remove("scale") {
                    params.insert("factor".to_owned(), old);
                }
                Ok(params)
            }
            2 => {
                if let Some(old) = params.remove("factor") {
                    params.insert("gain".to_owned(), old);
                }
                Ok(params)
            }
            other => Err(format!("unexpected migration from v{other}")),
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if let Some(flag) = &self.cancel_on_eval {
            flag.store(true, Ordering::SeqCst);
        }
        if context.is_cancelled() {
            return Err(EvalError::Cancelled);
        }
        let Some(SocketValue::Curve(curve)) = inputs.get("curve") else {
            return Err(EvalError::NodeFailed {
                node: "scale".into(),
                message: "missing curve".into(),
            });
        };
        let gain = params.f64_or("gain", 2.0);
        let scaled = Arc::new(Curve {
            values: curve.values.iter().map(|x| x * gain).collect(),
            hop_ms: curve.hop_ms,
            start_ms: curve.start_ms,
        });
        Ok(NodeOutputs::with_value("curve", SocketValue::Curve(scaled)))
    }
}

struct CurveToEventsNode {
    runs: Arc<AtomicUsize>,
}

impl NodeType for CurveToEventsNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("test.to_events"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("curve", SocketType::Curve, true)],
            outputs: vec![SocketSpec::exact("events", SocketType::Events, false)],
            params: vec![],
            bypass_passthrough: None,
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        _params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let Some(SocketValue::Curve(curve)) = inputs.get("curve") else {
            return Ok(NodeOutputs::default());
        };
        let events = Arc::new(EventList::sorted(vec![GraphEvent {
            time_ms: curve.start_ms + 10.0,
            detected_at_ms: curve.start_ms + 10.0,
            frame: 1,
            strength: curve.values[0],
            tag: "hit".into(),
            measures: BTreeMap::new(),
        }]));
        Ok(NodeOutputs::with_value("events", SocketValue::Events(events)))
    }
}

struct LaneSinkNode {
    runs: Arc<AtomicUsize>,
}

impl NodeType for LaneSinkNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("output.lane"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("events", SocketType::Events, true)],
            outputs: vec![],
            params: vec![ParamSpec::json(
                "slot",
                SocketType::Enum,
                serde_json::json!("slot"),
            )],
            bypass_passthrough: None,
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let Some(SocketValue::Events(events)) = inputs.get("events") else {
            return Ok(NodeOutputs::default());
        };
        Ok(NodeOutputs {
            slot_output: Some((params.str_or("slot", "slot").to_owned(), events.clone())),
            ..Default::default()
        })
    }
}

fn test_registry() -> (NodeRegistry, Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let curve_runs = Arc::new(AtomicUsize::new(0));
    let scale_runs = Arc::new(AtomicUsize::new(0));
    let events_runs = Arc::new(AtomicUsize::new(0));
    let sink_runs = Arc::new(AtomicUsize::new(0));
    let mut reg = NodeRegistry::new();
    reg.register(Arc::new(ConstCurveNode {
        runs: curve_runs.clone(),
    }))
    .unwrap();
    reg.register(Arc::new(ScaleCurveNode {
        runs: scale_runs.clone(),
        cancel_on_eval: None,
    }))
    .unwrap();
    reg.register(Arc::new(CurveToEventsNode {
        runs: events_runs.clone(),
    }))
    .unwrap();
    reg.register(Arc::new(LaneSinkNode { runs: sink_runs }))
        .unwrap();
    (reg, curve_runs, scale_runs, events_runs)
}

fn node(id: &str, type_id: &str, version: u32, params: BTreeMap<String, serde_json::Value>) -> NodeData {
    NodeData {
        id: id.into(),
        type_id: type_id.into(),
        type_version: version,
        params,
        muted: false,
        ui: NodeUi::default(),
        placeholder: None,
    }
}

#[test]
fn strict_socket_types_and_reserved_audio_clip_id() {
    assert_eq!(
        serde_json::to_string(&SocketType::AudioClip).unwrap(),
        "\"audio-clip\""
    );
    let (reg, _, _, _) = test_registry();
    let mut doc = GraphDocument::new("types");
    doc.nodes.push(node("c", "test.curve", 1, BTreeMap::new()));
    doc.nodes.push(node("out", "output.lane", 1, BTreeMap::new()));

    // Curve -> Events is rejected (no implicit conversion).
    let bad = Link::new(Endpoint::new("c", "curve"), Endpoint::new("out", "events"));
    assert_eq!(
        can_connect(&doc, &reg, &bad),
        Err(LinkError::TypeMismatch {
            from: SocketType::Curve,
            to: SocketType::Events,
        })
    );

    // Matching Curve -> Curve connects cleanly.
    doc.nodes.push(node("s", "test.scale", 3, BTreeMap::new()));
    let ok = Link::new(Endpoint::new("c", "curve"), Endpoint::new("s", "curve"));
    assert!(can_connect(&doc, &reg, &ok).is_ok());
}

#[test]
fn cycle_detection_refuses_new_cycles_and_reports_loaded_cycles() {
    let (reg, _, _, _) = test_registry();
    let mut doc = GraphDocument::new("cycles");
    doc.nodes.push(node("s1", "test.scale", 3, BTreeMap::new()));
    doc.nodes.push(node("s2", "test.scale", 3, BTreeMap::new()));
    doc.nodes.push(node("s3", "test.scale", 3, BTreeMap::new()));

    // Self-loop refused.
    assert_eq!(
        can_connect(
            &doc,
            &reg,
            &Link::new(Endpoint::new("s1", "curve"), Endpoint::new("s1", "curve"))
        ),
        Err(LinkError::SelfLoop("s1".into()))
    );

    // s1 -> s2 -> s3 valid; s3 -> s1 refused as cycle.
    doc.links
        .push(Link::new(Endpoint::new("s1", "curve"), Endpoint::new("s2", "curve")));
    doc.links
        .push(Link::new(Endpoint::new("s2", "curve"), Endpoint::new("s3", "curve")));
    let closing = Link::new(Endpoint::new("s3", "curve"), Endpoint::new("s1", "curve"));
    assert_eq!(
        can_connect(&doc, &reg, &closing),
        Err(LinkError::CreatesCycle {
            from: "s3".into(),
            to: "s1".into()
        })
    );

    // A loaded graph containing that bad link keeps it and reports CycleDetected.
    doc.links.push(closing);
    let report = validate_graph(&doc, &reg);
    assert!(report.issues.iter().any(|i| matches!(i, ValidationIssue::CycleDetected(_))));
    let mut cache = EvalCache::default();
    let err = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["s3".into()]),
        AnalysisRange::new(0.0, 1000.0).unwrap(),
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(err, EvalError::Cycle(_)));
}

#[test]
fn pull_evaluation_skips_unrelated_branch_and_tracks_dirty_downstream() {
    let (reg, curve_runs, scale_runs, _) = test_registry();
    let mut doc = GraphDocument::new("pull");
    doc.nodes.push(node("c1", "test.curve", 1, BTreeMap::new()));
    doc.nodes.push(node("s1", "test.scale", 3, BTreeMap::new()));
    doc.nodes.push(node("e1", "test.to_events", 1, BTreeMap::new()));
    doc.nodes.push(node(
        "o1",
        "output.lane",
        1,
        BTreeMap::from([("slot".into(), serde_json::json!("kick"))]),
    ));
    // Unrelated second branch
    doc.nodes.push(node("c2", "test.curve", 1, BTreeMap::new()));
    doc.nodes.push(node("e2", "test.to_events", 1, BTreeMap::new()));
    doc.nodes.push(node(
        "o2",
        "output.lane",
        1,
        BTreeMap::from([("slot".into(), serde_json::json!("snare"))]),
    ));
    doc.links
        .push(Link::new(Endpoint::new("c1", "curve"), Endpoint::new("s1", "curve")));
    doc.links
        .push(Link::new(Endpoint::new("s1", "curve"), Endpoint::new("e1", "curve")));
    doc.links
        .push(Link::new(Endpoint::new("e1", "events"), Endpoint::new("o1", "events")));
    doc.links
        .push(Link::new(Endpoint::new("c2", "curve"), Endpoint::new("e2", "curve")));
    doc.links
        .push(Link::new(Endpoint::new("e2", "events"), Endpoint::new("o2", "events")));

    let mut dirty = DirtyTracker::new();
    dirty.mark_dirty(&doc, &["s1".into()]);
    assert!(!dirty.is_dirty(&"c1".into()));
    assert!(dirty.is_dirty(&"s1".into()));
    assert!(dirty.is_dirty(&"e1".into()));
    assert!(dirty.is_dirty(&"o1".into()));
    assert!(!dirty.is_dirty(&"c2".into()));
    assert!(!dirty.is_dirty(&"o2".into()));

    let mut cache = EvalCache::default();
    let res = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["o1".into()]),
        AnalysisRange::new(0.0, 1000.0).unwrap(),
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(res.evaluated_nodes, vec!["c1".into(), "s1".into(), "e1".into(), "o1".into()]);
    assert_eq!(curve_runs.load(Ordering::SeqCst), 1);
    assert_eq!(scale_runs.load(Ordering::SeqCst), 1);
    assert!(res.slot_events.contains_key("kick"));
    assert!(!res.slot_events.contains_key("snare"));
}

#[test]
fn cache_keys_respect_range_scope_params_and_lru_eviction() {
    let (reg, curve_runs, _, _) = test_registry();
    let mut doc = GraphDocument::new("cache");
    doc.nodes.push(node("c", "test.curve", 1, BTreeMap::new()));
    let mut cache = EvalCache::default();
    let r1 = AnalysisRange::new(0.0, 1000.0).unwrap();
    let r2 = AnalysisRange::new(0.0, 2000.0).unwrap();

    // First run misses.
    let res1 = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["c".into()]),
        r1,
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(res1.evaluated_nodes, vec!["c".into()]);
    assert_eq!(curve_runs.load(Ordering::SeqCst), 1);

    // Identical run hits cache.
    let res2 = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["c".into()]),
        r1,
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(res2.cached_nodes, vec!["c".into()]);
    assert_eq!(curve_runs.load(Ordering::SeqCst), 1);

    // Different range misses cache.
    evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["c".into()]),
        r2,
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(curve_runs.load(Ordering::SeqCst), 2);

    // Changing normaliseOver misses cache.
    doc.nodes[0]
        .params
        .insert("normaliseOver".into(), serde_json::json!("song"));
    evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["c".into()]),
        r2,
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(curve_runs.load(Ordering::SeqCst), 3);

    // Determinism: structural hashes & values match identically across runs.
    let res3 = evaluate_graph(
        &doc,
        &reg,
        &mut EvalCache::default(),
        EvalTarget::Nodes(vec!["c".into()]),
        r2,
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let res4 = evaluate_graph(
        &doc,
        &reg,
        &mut EvalCache::default(),
        EvalTarget::Nodes(vec!["c".into()]),
        r2,
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(res3.graph_hash, res4.graph_hash);
    assert_eq!(
        evaluate_graph(
            &doc,
            &reg,
            &mut EvalCache::default(),
            EvalTarget::Socket(Endpoint::new("c", "curve")),
            r2,
            None,
            &|| false,
            &mut |_| {},
        )
        .unwrap()
        .socket_preview,
        evaluate_graph(
            &doc,
            &reg,
            &mut EvalCache::default(),
            EvalTarget::Socket(Endpoint::new("c", "curve")),
            r2,
            None,
            &|| false,
            &mut |_| {},
        )
        .unwrap()
        .socket_preview
    );

    // Tiny LRU cache evicts oldest entry.
    let mut tiny = EvalCache::new(100);
    evaluate_graph(
        &doc,
        &reg,
        &mut tiny,
        EvalTarget::Nodes(vec!["c".into()]),
        r1,
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    evaluate_graph(
        &doc,
        &reg,
        &mut tiny,
        EvalTarget::Nodes(vec!["c".into()]),
        r2,
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(tiny.stats().entries, 1);
    assert_eq!(tiny.stats().evictions, 1);
}

#[test]
fn cancelled_run_writes_no_cache_and_bypass_passes_through() {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let mut reg = NodeRegistry::new();
    reg.register(Arc::new(ConstCurveNode {
        runs: Arc::new(AtomicUsize::new(0)),
    }))
    .unwrap();
    reg.register(Arc::new(ScaleCurveNode {
        runs: Arc::new(AtomicUsize::new(0)),
        cancel_on_eval: Some(cancel_flag.clone()),
    }))
    .unwrap();

    let mut doc = GraphDocument::new("cancel-test");
    doc.nodes.push(node("c", "test.curve", 1, BTreeMap::new()));
    doc.nodes.push(node("s", "test.scale", 3, BTreeMap::new()));
    doc.links
        .push(Link::new(Endpoint::new("c", "curve"), Endpoint::new("s", "curve")));

    let mut cache = EvalCache::default();
    let err = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["s".into()]),
        AnalysisRange::new(0.0, 1000.0).unwrap(),
        None,
        &|| cancel_flag.load(Ordering::SeqCst),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(err, EvalError::Cancelled);
    assert_eq!(cache.stats().entries, 0, "cancelled run must write nothing to cache");

    // Muting `s` bypasses `curve -> curve` without scaling.
    cancel_flag.store(false, Ordering::SeqCst);
    doc.nodes[1].muted = true;
    let res = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Socket(Endpoint::new("s", "curve")),
        AnalysisRange::new(0.0, 500.0).unwrap(),
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let Some(SocketValue::Curve(curve)) = res.socket_preview else {
        panic!("expected curve passthrough");
    };
    assert_eq!(curve.values, vec![1.0, 0.0, 500.0]);
}

#[test]
fn migrations_and_unknown_placeholders_survive_round_trip() {
    let (reg, _, _, _) = test_registry();
    let raw = r#"{
  "format": "dawsu-graph",
  "version": 1,
  "name": "Mixed",
  "description": "Contains v1 node and unknown node",
  "nodes": [
    { "id": "s", "type": "test.scale", "typeVersion": 1, "params": { "scale": 3.5 }, "muted": false, "ui": { "x": 10.0, "y": 20.0 } },
    { "id": "u", "type": "future.ml_stem", "typeVersion": 9, "params": { "model": "htdemucs" }, "customField": [1, 2, 3], "muted": false, "ui": { "x": 50.0, "y": 60.0 } },
    { "id": "e", "type": "test.to_events", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 100.0, "y": 0.0 } }
  ],
  "links": [
    { "from": { "node": "u", "socket": "curve" }, "to": { "node": "s", "socket": "curve" } },
    { "from": { "node": "s", "socket": "curve" }, "to": { "node": "e", "socket": "curve" } }
  ],
  "outputs": [ { "slot": "kick" } ]
}
"#;
    let doc = GraphDocument::from_json(raw, &reg).unwrap();
    // `s` migrated v1 -> v2 -> v3 (`scale` -> `factor` -> `gain`).
    let s = doc.node(&"s".into()).unwrap();
    assert_eq!(s.type_version, 3);
    assert_eq!(s.params.get("gain"), Some(&serde_json::json!(3.5)));

    // `u` is a placeholder with inferred sockets and preserved customField.
    let u = doc.node(&"u".into()).unwrap();
    assert!(u.placeholder.is_some());
    let (_, outs) = doc.inferred_placeholder_sockets(&"u".into());
    assert!(outs.contains(&SocketId::from("curve")));

    let saved = doc.to_json().unwrap();
    assert!(!saved.contains('\r'), "to_json must emit LF line endings");
    let saved_val: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved_val["nodes"][1]["customField"], serde_json::json!([1, 2, 3]));
    assert_eq!(saved_val["nodes"][1]["typeVersion"], serde_json::json!(9));

    // Evaluating downstream reports "needs node u (unknown type ...)".
    let mut cache = EvalCache::default();
    let res = evaluate_graph(
        &doc,
        &reg,
        &mut cache,
        EvalTarget::Nodes(vec!["e".into()]),
        AnalysisRange::new(0.0, 1000.0).unwrap(),
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    assert!(res.diagnostics[&"u".into()][0].message.contains("unknown type"));
    assert_eq!(res.diagnostics[&"s".into()][0].severity, DiagnosticSeverity::Error);
    assert!(res.diagnostics[&"s".into()][0].message.contains("needs node u"));
    assert_eq!(res.diagnostics[&"e".into()][0].message, "waiting for input");
}
