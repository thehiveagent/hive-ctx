# hive-ctx

Rust core + N-API TypeScript bindings monorepo.

## Layout

- `crates/hive-ctx-core`: Rust core compiled to a `.node` binary via `napi-rs`
- `packages/bindings`: TypeScript bindings wrapping the native addon

Core modules (Rust): `graph`, `memory`, `fingerprint`, `classifier`, `retrieval`, `pipeline`.

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

