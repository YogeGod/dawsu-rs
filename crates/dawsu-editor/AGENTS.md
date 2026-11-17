# dawsu-editor

The editing kernel. Its design doc is the vault's `dawsu-rewrite/design/tdd-editor-core.md`.

- May reference: `dawsu-foundation` and the standard library.
- Must not reference: any domain crate (`dawsu-hitsounding`, `dawsu-keysounding`, `dawsu-graph`, `dawsu-osu-format`), audio, `dawsu-application`, the UI or a UI toolkit. `tests/architecture.rs` checks the dependency list.
- No static mutable state. Command handlers are plain `fn` pointers, so they cannot capture a document.
- Commands are declared beside the feature that owns them. This crate declares only the kernel's own (history, clipboard, selection, tools, snap, zoom, locators, bookmarks, grid seeking); the composers and `dawsu-application` declare theirs.
