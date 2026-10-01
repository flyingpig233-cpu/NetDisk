# AGENTS.md

## Project
Early-stage Rust netdisk (cloud storage) backend. `master` has no commits yet. `how I made it.md` is a Chinese dev log describing intent, not a spec.
Roadmap in that log: (1) local filesystem + metadata, (2) user/auth, (3) HTTP API, (4) sharing/recycle/teams.

## Commands
- `cargo check` / `cargo build` / `cargo run` (binary is `target/debug/NetDisk`).
- No tests, CI, linter, formatter, or codegen config exist; `cargo test` runs nothing.
- Edition 2024, builds on rustc 1.98. Current warnings (unused code) are expected.

## Known gotchas
- `FileManager::new()` stores its DB at `$HOME/.netdisk/meta.db`, expanding `$HOME` and creating the dir on startup (src/file_system/file_manager.rs:6-18). Server binds `0.0.0.0:3000`.
- DB layer is unfinished: no `diesel.toml`, `migrations/`, or `schema.rs`. `new_file` creates the content file but does not insert metadata.
- Both `rusqlite` and `diesel` (sqlite) are dependencies, but only Diesel is used — don't reach for rusqlite. The log mentions `sea-orm`; that is outdated.
- `src/main.rs` imports `std::fs::File` unnecessarily; ignore/remove stray imports when touching it.

## Architecture
- `src/main.rs` — Axum 0.8 HTTP entrypoint.
- `src/file_system/` — storage + metadata. Design: file contents stored flat in one directory, content-addressed (filename = content hash) so files can be shared; naming/ownership lives in SQLite via `FileMeta` (src/file_system/file_meta.rs).
