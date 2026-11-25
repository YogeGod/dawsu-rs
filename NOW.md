# NOW.md — Current Sprint (`dawsu-rs`)

`origin/main` is `db6385f` (`v1.1.0-alpha.1` shipped). The M3 and v1.1 sprint is archived in `E:/Obsidian/Workspace/dawsu-rewrite/now.md`.

**Feature freeze is ON.** No new windows, crates or features until the post-v1.1 tech-debt cleanup is done.

## Active: post-v1.1 tech-debt cleanup (5 PRs)

1. **PR 1 — Release, Clippy, Docs** (in progress): release workflow fixes, `cargo clippy --workspace --all-targets --locked -- -D warnings` clean, refreshed README, FEATURE-COVERAGE and NOW.
2. **PR 2 — Split `main.rs`** (`apps/tauri/src/main.rs`).
3. **PR 3 — Split `dawsu-application/src/lib.rs`.**
4. **PR 4 — Frontend cleanup:** untrack `dist/ui/*`, split the 4 giant Svelte modals.
5. **PR 5 — Workspace deps, `ort` feature gate, production `unwrap` cleanup, test suite split.**

## Rules

- **Single source of truth for sprint tasks:** `E:/Obsidian/Workspace/dawsu-rewrite/now.md`. Mirror changes there, not in competing lists.
- **Real-app check:** run `npm --prefix apps/tauri/ui run build`, `npm --prefix apps/tauri/ui test`, `node --test apps/tauri/tests/*.test.mjs`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` and `node tools/everyday-check.mjs` before merging. Verify the built binary boots. Never merge a PR before checks finish.
