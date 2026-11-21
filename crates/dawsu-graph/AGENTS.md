# dawsu-graph

The offline, pull-based node-graph engine (Layer 4 · Engines). Design doc: `dawsu-rewrite/design/tdd-node-graph.md`.

- May reference: `dawsu-foundation`, `serde`, `serde_json`, and the standard library.
- Must never reference: `dawsu-audio`, `dawsu-audio-instruments`, `dawsu-graph-audio`, `dawsu-osu-format`, `dawsu-osu-client`, `dawsu-hitsounding`, `dawsu-keysounding`, `dawsu-editor`, `dawsu-editor-composer`, `dawsu-editor-nodal`, `dawsu-application`, `dawsu-ui`, or any audio/UI library (Rule R1 / R7).
- No static mutable state. Evaluation must be deterministic, pull-based, and cancellable over an explicit `AnalysisRange`.
- Stable string ids (`NodeTypeId`, `SocketId`, `ParamId`) must never be replaced with display names.
