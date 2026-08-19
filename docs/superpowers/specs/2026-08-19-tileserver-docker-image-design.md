# Tileserver minimal Docker image design

**Date:** 2026-08-19
**Status:** Approved

## Goal

Build the WPDA tile server into a minimal Docker image. The image build must compile the tileserver (`wpda-tileserver`) as a fully static binary and produce its runtime assets: the WASM lib (`wimage_wasm.js`, `wimage_wasm_bg.wasm`) built with `wasm-pack`, the worker script `tile-worker.js`, the rendered `index.html.tmpl`, the `i18n/*.json` dictionaries, and `favicon.ico`. The final image must be as small as possible (fully static, `scratch` runtime).

## Background / constraints (explored)

- The server reads a `DATA_PATH` directory (default `.`) containing `weeks/*.db`, `index.html.tmpl`, `i18n/{en,ja,es}.json`, `assets/*`, and `favicon.ico`. It listens on `PORT` (default 8080), requiring at least one `weeks/w*_*.db` file at startup (`main.rs:129-133`).
- `build_server.sh` reproduces the current manual build: `cargo build --release` then `(cd frontend && wasm-pack build --target web --no-default-features)`, then assembles the binary + assets into `tmp/`.
- The workspace root (`Cargo.toml`, resolver 3) contains `pipeline` and `tileserver`. The binary is built with `cargo build --release -p wpda-tileserver`; `pipeline` is not a dependency of `tileserver` and must not be built for the image.
- `frontend/` is detached from the root workspace (declares its own `[workspace]`) and needs `wasm32-unknown-unknown` + `wasm-pack` + `wasm-opt` (binaryen).
- `rusqlite` uses the `bundled` feature → compiles SQLite C source; a musl build needs a C toolchain (`musl-dev`/`gcc`) on the build image.
- The current dev binary is dynamically linked glibc; a fully static build requires an Alpine (musl) builder.
- Blocking quirk: `wimage` is currently a **path dependency with an absolute machine-specific path** (`/run/media/system/DataBtrfs/wplace/wplace-image/wimage`) in both `frontend/Cargo.toml` and `pipeline/Cargo.toml`. A Docker build context rooted at this repo cannot reach that path. Resolution (user choice): switch `wimage` to a **git URL dependency** in all cargo files, pinned to rev `d718548` on `main` of `github.com/Hugi-R/wplace-image`.
- No `docker`/`podman` is installed on the dev machine; verification of the intended final image must be done by the user (they can build/confirm on request), while the local machine can verify the git-dep switch by rebuilding wasm + tileserver.

## Scope decisions

- Add a `Dockerfile` at the repo root building the wasm lib and the tileserver binary, assembling them into a `scratch` final stage.
- `wimage` becomes a git dependency (pinned rev) in `frontend/Cargo.toml` and `pipeline/Cargo.toml`; both `Cargo.lock` files regenerated. Local dev workflow keeps working via the git dep.
- Runtime: non-root user (UID/GID 65532), `DATA_PATH=/data`, `PORT=8080`; only static read-only files shipped in the image; `weeks/*.db` supplied at runtime via a mounted volume. `docker run -v ...:/data/weeks`.
- No `HEALTHCHECK`, no `SYSTEMD`-style init, no shell in the final image.

## Design

### Section 1 — `wimage` dependency becomes a git URL

`frontend/Cargo.toml` and `pipeline/Cargo.toml` replace:

```toml
wimage = { path = "/run/media/system/DataBtrfs/wplace/wplace-image/wimage" }
```

with:

```toml
wimage = { git = "https://github.com/Hugi-R/wplace-image", rev = "d718548" }
```

Regenerate both `Cargo.lock` files (`frontend/Cargo.lock` and root `Cargo.lock`). Notably:
- `wimage` itself declares several workspace-independent deps (`png`, `zstd`, `chrono`, `clap`) — these are already resolved under the pinned revision.
- Keep `frontend` detached from the root workspace; its `Cargo.lock` is updated independently.
- The pinned `rev` gives reproducible builds; bumping to a newer wimage means updating `rev` (document in plan).

### Section 2 — Dockerfile (multi-stage)

Stage 1, `wasm` (builds the WASM lib) — based on `rust:1-alpine`:
- Install `wasm32-unknown-unknown` target, `binaryen` (wasm-opt), and `wasm-pack` (pinned version, e.g. v0.13.1).
- Copy `frontend/` (source + `Cargo.toml` + `Cargo.lock`).
- Run `wasm-pack build --target web --no-default-features --release`. Output: `pkg/wimage_wasm.js`, `pkg/wimage_wasm_bg.wasm`.

Stage 2, `server` (builds the static tileserver binary) — based on `rust:1-alpine` (musl by default):
- Install `gcc` + `musl-dev` (needed to compile bundled SQLite).
- Copy workspace files: `Cargo.toml`, `Cargo.lock`, `tileserver/`, and `pipeline/Cargo.toml`. Cargo requires every member listed in the root manifest (`members = ["pipeline", "tileserver"]`) to exist for workspace resolution, but `pipeline` is never built (`-p wpda-tileserver`), so only its manifest is copied — not its sources.
- Run `cargo build --release -p wpda-tileserver`. Produce `target/release/wpda-tileserver` (native triple = musl static).

Stage 3, `assemble` — small Alpine stage to stage the static assets into an `Dockerfile` `COPY`able dir:
- `index.html.tmpl` ← `frontend/index.html`
- `i18n/*.json` ← `tileserver/i18n/*.json`
- `assets/tile-worker.js` ← `frontend/assets/tile-worker.js`
- `assets/wimage_wasm.js`, `assets/wimage_wasm_bg.wasm` ← wasm stage `pkg/`
- `favicon.ico` ← `frontend/favicon.ico`
- Write a `weeks/` dir so the mount point exists.

Final stage — `scratch`:
- `COPY --from=server /target/release/wpda-tileserver /wpda-tileserver`
- `COPY --from=assemble /data /data`
- `ENV DATA_PATH=/data PORT=8080`
- `EXPOSE 8080`
- `USER 65532:65532` (numeric UID so no `/etc/passwd` needed; static files owned accordingly or world-readable)
- `ENTRYPOINT ["/wpda-tileserver"]`

### Section 3 — Runtime layout

- `DATA_PATH=/data` with the static assets baked in; `/data/weeks` mounted at runtime holds `w*_*.db`. The DB files must be world-readable (or owned by UID 65532) since the process runs as 65532.
- `PORT=8080` default; bind is already `0.0.0.0:PORT` in `main.rs:900`.
- Graceful shutdown via `ctrl_c` (`main.rs:911-916`) works; `--init`/pid1 not required for `axum::serve` with a single process.
- Suggested run: `docker run -p 8080:8080 -v /path/to/weeks:/data/weeks <image>`.

## Error handling

- Startup without `weeks/*.db` fails with "no week database files found" and the process exits with status 1 (`main.rs:129-133`, `896`); container restart policy is the caller's concern.
- Missing/partial assets at build time fail the corresponding `COPY`/build step (fail-fast).

## Testing

- Local (this machine): rebuild both artifacts after the git-dep switch to prove the switch compiles:
  - tileserver: `cargo build --release -p wpda-tileserver`
  - wasm: `(cd frontend && wasm-pack build --target web --no-default-features)`
- Docker build (user, on a machine with Docker): `docker build -t wpda-tileserver .` then run with a mounted weeks dir and verify `curl localhost:8080/`.
- Optional local static-cross-check: if we add `rustup target add x86_64-unknown-linux-musl` and `musl-tools` locally, `cargo build --target x86_64-unknown-linux-musl -p wpda-tileserver` replicates the container's static binary; `file` should report "statically linked". This is a build check the user can opt into; not required for the Dockerfile itself.