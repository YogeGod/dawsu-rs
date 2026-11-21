# dawsu-editor-nodal

Undoable graph editing operations, proposal conversion, and `apply_proposals` (Layer 5 · Editing). Design doc: `dawsu-rewrite/design/tdd-node-graph.md`.

- May reference: `dawsu-foundation`, `dawsu-editor`, `dawsu-graph`, `dawsu-hitsounding`, `dawsu-editor-composer`, `serde`, `serde_json`, and the standard library.
- Must never reference: `dawsu-graph-audio`, `dawsu-graph-stems` (Rule R9), `dawsu-osu-format` (in production dependencies), `dawsu-audio*`, `dawsu-application`, `dawsu-ui`, or any UI toolkit.
- Graph edits go through `dawsu-editor::EditHistory` and track dirty downstream nodes; invalid links are refused without modifying history.
- Applying proposals delegates anchoring, deduplication, and single-step undo to `dawsu-editor-composer::Composer::add_batch` after checking `AnalysisStamp`.
