//! Unit tests for `NodalSession` state transitions, dirty tracking, and undo/redo.

use dawsu_editor_nodal::{NodalError, NodalSession};
use dawsu_graph::{
    Endpoint, EvalContext, EvalError, GraphDocument, Link, LinkError, NodeData, NodeInputs,
    NodeOutputs, NodeParams, NodeRegistry, NodeSpec, NodeType, NodeTypeId, NodeUi, ParamSpec,
    PlaceholderInfo, PlaceholderReason, SocketId, SocketSpec, SocketType,
};
use std::collections::BTreeMap;
use std::sync::Arc;

struct SrcNode;
impl NodeType for SrcNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("nodal.src"),
            type_version: 1,
            inputs: vec![],
            outputs: vec![SocketSpec::exact("curve", SocketType::Curve, false)],
            params: vec![ParamSpec::number("val", 1.0, None, None)],
            bypass_passthrough: None,
        }
    }
    fn evaluate(
        &self,
        _inputs: &NodeInputs,
        _params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        Ok(NodeOutputs::default())
    }
}

struct MidNode;
impl NodeType for MidNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("nodal.mid"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("curve", SocketType::Curve, true)],
            outputs: vec![SocketSpec::exact("events", SocketType::Events, false)],
            params: vec![ParamSpec::number("gain", 1.0, None, None)],
            bypass_passthrough: None,
        }
    }
    fn evaluate(
        &self,
        _inputs: &NodeInputs,
        _params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        Ok(NodeOutputs::default())
    }
}

struct PassCurveNode;
impl NodeType for PassCurveNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("nodal.pass"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("curve", SocketType::Curve, true)],
            outputs: vec![SocketSpec::exact("curve", SocketType::Curve, false)],
            params: vec![],
            bypass_passthrough: Some((SocketId::from("curve"), SocketId::from("curve"))),
        }
    }
    fn evaluate(
        &self,
        _inputs: &NodeInputs,
        _params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        Ok(NodeOutputs::default())
    }
}

fn registry() -> NodeRegistry {
    let mut reg = NodeRegistry::new();
    reg.register(Arc::new(SrcNode)).unwrap();
    reg.register(Arc::new(MidNode)).unwrap();
    reg.register(Arc::new(PassCurveNode)).unwrap();
    reg
}

#[test]
fn add_connect_param_mute_duplicate_and_undo_redo() {
    let reg = registry();
    let mut session = NodalSession::new(GraphDocument::new("test"));
    assert!(!session.is_dirty());

    let n1 = session.add_node(&reg, &"nodal.src".into(), 10.0, 20.0).unwrap();
    let n2 = session.add_node(&reg, &"nodal.pass".into(), 110.0, 20.0).unwrap();
    let n3 = session.add_node(&reg, &"nodal.mid".into(), 210.0, 20.0).unwrap();
    assert!(session.is_dirty());

    session
        .connect(&reg, Link::new(Endpoint::new(n1.clone(), "curve"), Endpoint::new(n2.clone(), "curve")))
        .unwrap();
    session
        .connect(&reg, Link::new(Endpoint::new(n2.clone(), "curve"), Endpoint::new(n3.clone(), "curve")))
        .unwrap();

    // Invalid link (Events -> Curve on unconnected input) is refused without bumping revision/undo.
    let n4 = session.add_node(&reg, &"nodal.pass".into(), 310.0, 20.0).unwrap();
    let rev_before = session.revision();
    let bad = session.connect(
        &reg,
        Link::new(Endpoint::new(n3.clone(), "events"), Endpoint::new(n4.clone(), "curve")),
    );
    assert!(matches!(
        bad,
        Err(NodalError::LinkRefused(LinkError::TypeMismatch { .. }))
    ));
    assert_eq!(session.revision(), rev_before);
    session.remove_nodes(&[n4]).unwrap();
    // Remove the extra delete step by undoing add+delete? Actually let's just check 3 nodes remain:
    assert_eq!(session.document().nodes.len(), 3);

    // Clear dirty, then edit n1 param -> n1, n2, n3 become dirty.
    session.clear_dirty(&[n1.clone(), n2.clone(), n3.clone()]);
    assert!(session.dirty_nodes().is_empty());
    session.set_param(&n1, "val", serde_json::json!(4.2)).unwrap();
    assert!(session.dirty_nodes().contains(&n1));
    assert!(session.dirty_nodes().contains(&n2));
    assert!(session.dirty_nodes().contains(&n3));

    // Move drag is one undo step and does NOT dirty DSP state.
    session.clear_dirty(&[n1.clone(), n2.clone(), n3.clone()]);
    session.begin_move();
    session.move_nodes(&[(n1.clone(), 50.0, 60.0)]).unwrap();
    session.move_nodes(&[(n1.clone(), 90.0, 80.0)]).unwrap();
    session.commit_move();
    assert!(session.dirty_nodes().is_empty());
    assert_eq!(session.document().node(&n1).unwrap().ui.x, 90.0);

    // Toggle mute + box-select + duplicate keeps internal links.
    assert!(session.toggle_muted(&n2).unwrap());
    let boxed = session.box_select(0.0, 0.0, 150.0, 150.0, false);
    assert_eq!(boxed.len(), 2);
    let dupes = session.duplicate_selected(20.0, 300.0).unwrap();
    assert_eq!(dupes.len(), 2);
    assert_eq!(session.document().nodes.len(), 5);
    assert_eq!(session.document().links.len(), 3);

    // Undo duplicate restores 3 nodes and 2 links; redo restores 5 nodes.
    assert!(session.undo());
    assert_eq!(session.document().nodes.len(), 3);
    assert_eq!(session.document().links.len(), 2);
    assert!(session.redo());
    assert_eq!(session.document().nodes.len(), 5);

    // Deleting selected removes only the 2 duplicates.
    session.selection_mut().replace(dupes);
    assert_eq!(session.remove_selected().unwrap(), 2);
    assert_eq!(session.document().nodes.len(), 3);

    // Save state tracking.
    session.mark_saved();
    assert!(!session.is_dirty());
    session.disconnect(&Link::new(Endpoint::new(n2.clone(), "curve"), Endpoint::new(n3.clone(), "curve"))).unwrap();
    assert!(session.is_dirty());
}

#[test]
fn placeholder_nodes_cannot_be_mutated_and_cancel_move_restores_position() {
    let mut doc = GraphDocument::new("placeholder");
    doc.nodes.push(NodeData {
        id: "u1".into(),
        type_id: "unknown.type".into(),
        type_version: 1,
        params: BTreeMap::new(),
        muted: false,
        ui: NodeUi { x: 10.0, y: 20.0 },
        placeholder: Some(PlaceholderInfo {
            reason: PlaceholderReason::UnknownType,
            raw: serde_json::json!({"id":"u1","type":"unknown.type","typeVersion":1}),
        }),
    });
    let mut session = NodalSession::new(doc);
    assert_eq!(
        session.set_param(&"u1".into(), "x", serde_json::json!(1)),
        Err(NodalError::PlaceholderImmutable("u1".into()))
    );
    assert_eq!(
        session.set_muted(&"u1".into(), true),
        Err(NodalError::PlaceholderImmutable("u1".into()))
    );

    session.begin_move();
    session.move_nodes(&[("u1".into(), 999.0, 888.0)]).unwrap();
    session.cancel_move();
    assert_eq!(session.document().node(&"u1".into()).unwrap().ui.x, 10.0);
}
