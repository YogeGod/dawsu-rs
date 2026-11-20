// The Export and drum detection windows have no app.js and so no act(): each calls its own Rust
// commands directly. withGlobalTauri puts the core API on window; each window's capability
// (capabilities/export.json, capabilities/drum.json) allows only that window's commands.
export const invoke = (command, args) => globalThis.window.__TAURI__.core.invoke(command, args);
