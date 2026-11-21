# dawsu-graph-audio

Built-in DSP and audio analysis nodes for the node graph (Layer 4 · Engines). Design doc: `dawsu-rewrite/design/tdd-node-graph.md`.

- May reference: `dawsu-foundation`, `dawsu-graph`, `serde`, `serde_json`, and the standard library.
- Must never reference: `dawsu-audio`, `dawsu-audio-instruments`, domain crates (`dawsu-hitsounding`, `dawsu-keysounding`), `dawsu-osu-format`, `dawsu-editor*`, `dawsu-application`, `dawsu-ui`, or audio libraries (`cpal`, `symphonia`, `signalsmith-stretch`).
- Audio arrives already decoded via `AudioProvider` so every node can be tested headlessly with synthetic audio.
- Never switch on drum or lane names (`"kick"`, `"snare"`) in node logic; slots are data.
