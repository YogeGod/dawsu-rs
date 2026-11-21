# NOW.md — Current Sprint (`dawsu-rs`)

**Feature freeze is ON.** Do not add new windows, crates, or M4 Node Graph features until the core hitsound editor feels rock-solid on real beatmaps and matches 3.1.4's day-to-day mapper ergonomics.

- **Legacy `apps/tauri/dist/app.js`:** Retired (already split into `apps/tauri/ui/src/editor/*.js` and Svelte 5 parts).
- **Single source of truth for sprint tasks:** `E:/Obsidian/Workspace/dawsu-rewrite/now.md` (mirror changes there, not in multiple competing lists).
- **Real-app check rule:** Always run `npm run build` + `npm test` (in `apps/tauri/ui`), `node --test apps/tauri/tests/*.test.mjs`, and `cargo test --workspace --locked` before merging, and verify the real built binary boots cleanly. Never merge a PR before checks finish.
