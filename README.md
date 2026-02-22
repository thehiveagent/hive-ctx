# hive-ctx

Rust core + N-API TypeScript bindings monorepo.

## Layout

- `crates/hive-ctx-core`: Rust core compiled to a `.node` binary via `napi-rs`
- `packages/bindings`: TypeScript bindings wrapping the native addon

Core modules (Rust): `graph`, `memory`, `fingerprint`, `classifier`, `retrieval`, `pipeline`.

## Knowledge graph

- SQLite schema lives inside `crates/hive-ctx-core/src/graph.rs` (tables: `nodes`, `edges` plus indexes).
- Nodes represent entities (`person`, `place`, `project`, `concept`, `emotion`, `state`) with timestamps and a decay score for emotional/state nodes.
- Entity extraction for `graph_add_node` runs regex-driven heuristics on raw text (`crates/hive-ctx-core/src/graph.rs`) so there are no ML/API dependencies.
- Edges store typed relationships with timestamps, and traversal discovers neighbors up to `N` hops while `graph_decay_update` ages emotional/state nodes over time.
- Exposed addon APIs: `graph_add_node`, `graph_add_edge`, `graph_query`, `graph_traverse`, `graph_decay_update` (via `HiveCtxEngine`).

## Memory store

- `crates/hive-ctx-core/src/memory.rs` implements a 3-tier episode archive backed by SQLite (`tier1_entries`, `tier2_summaries`, `tier3_crystallized`) plus `memory_meta`.
- Tier 1 holds raw conversations and expires after 24 hours; Tier 2 stores compressed 2-3 sentence summaries retained for 30 days; Tier 3 stores crystallized facts merged into the knowledge graph and never deleted.
- `memory_compress` moves Tier 1 → Tier 2 nightly while skipping unchanged text via `blake3` hashes; `memory_crystallize` runs monthly to push Tier 2 summaries into Tier 3, running `graph_add_node` on the extracted facts.
- Exposed addon APIs: `memory_store`, `memory_retrieve`, `memory_compress`, `memory_crystallize`, `memory_stats` (via `HiveCtxEngine`).

## Classifier & fingerprint

- `crates/hive-ctx-core/src/classifier.rs` implements a heuristic message classifier that scores each incoming message along temporal, personal, technical, and emotional axes (0.0–1.0) plus a type (`casual`, `question`, `task`, `emotional`) and session state (`COLD_START`, `WARM`, `CONTEXT_SHIFT`, `EMOTIONAL_SHIFT`, `TASK_MODE`).
- `crates/hive-ctx-core/src/fingerprint.rs` compiles profile data into a key-value token-efficient fingerprint, tracking deltas since the last compile and automatically expanding into a full compile whenever the classifier reports context shifts, task mode, or a cold start.
- Exposed addon APIs: `classify_message`, `fingerprint_compile` (via `HiveCtxEngine`) so JS clients can reuse the same session context.

## Rust API (exported to Node)

The main exported struct is `HiveCtxEngine`:

- `new(storage_path: string, budget_tokens?: number)`

## Development

Build the native addon into `packages/bindings/`:

```bash
npm install
npm run build:native
```

Build the TypeScript bindings:

```bash
npm run build
```

By default the bindings load `packages/bindings/hive_ctx.node`. Override with:

```bash
HIVE_CTX_NATIVE_PATH=/absolute/path/to/hive_ctx.node node -e "require('./packages/bindings/dist')"
```

## Rust checks

```bash
cargo check
```
