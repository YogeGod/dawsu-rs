// The Export window has no app.js and so no act(): it calls its own Rust commands directly.
// withGlobalTauri puts the core API on window; its capability allows only the export commands.
export const invoke = (command, args) => globalThis.window.__TAURI__.core.invoke(command, args);
