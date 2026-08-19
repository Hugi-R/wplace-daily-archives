# Tileserver Minimal Docker Image Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the WPDA tile server as a fully static scratch image, built in Docker from the wasm lib (`wasm-pack`) and the `wpda-tileserver` binary.

**Architecture:** Three build stages (wasm lib → static server binary → data assembly) plus a `scratch` final stage. `wimage` becomes a pinned git URL dependency so the Docker build does not depend on an absolute host path.

**Tech Stack:** Dockerfile (multi-stage), Alpine (musl) Rust builds (`rust:1.97-alpine`), `wasm-pack` 0.13.1, binaryen `wasm-opt`, cargo workspace (`wpda-tileserver`).

## Global Constraints

- Rust image pinned to `rust:1.97-alpine` (matches local dev toolchain 1.97.1; flaking on `rust:1-alpine` avoided).
- `wasm-pack` pinned to `v0.13.1` (matches local); installed from the musl release tarball `wasm-pack-v0.13.1-x86_64-unknown-linux-musl.tar.gz` at `https://github.com/rustwasm/wasm-pack/releases/download/v0.13.1/`.
- `wimage` git dependency pinned to `rev = "d718548"` (`main` HEAD of `https://github.com/Hugi-R/wplace-image`) in **both** `frontend/Cargo.toml` and `pipeline/Cargo.toml`.
- wasm build command (verbatim, matches `build_server.sh`): `wasm-pack build --target web --no-default-features`.
- server build command: `cargo build --release -p wpda-tileserver` (never builds `pipeline`).
- Final image: `scratch`, non-root `USER 65532:65532`, `ENV PORT=8080 DATA_PATH=/data`, `EXPOSE 8080`, `ENTRYPOINT ["/wpda-tileserver"]`.
- Runtime data layout (`DATA_PATH=/data`): `weeks/` (runtime-only, volume-mounted), `index.html.tmpl`, `i18n/*.json`, `assets/{tile-worker.js,wimage_wasm.js,wimage_wasm_bg.wasm}`, `favicon.ico`.
- `.dockerignore` must exclude the 4.7 GB `bench.db`, `profile.json.gz`, `target/`, `tmp/`, `.git` — the context is otherwise unusable.
- The `pipeline/` sources are copied into the build stage in full but never compiled (`-p wpda-tileserver`); this keeps cargo's workspace-member resolution unambiguous without adding meaningful stage weight.

---
## File Structure

- Modify: `frontend/Cargo.toml:17` — `wimage` path → git URL (pinned rev).
- Modify: `pipeline/Cargo.toml:7` — `wimage` path → git URL (pinned rev).
- Regenerate: `frontend/Cargo.lock`, `Cargo.lock` (root) — `wimage` source switches from path to `git+https://github.com/Hugi-R/wplace-image`.
- Create: `Dockerfile` — multi-stage build described above.
- Create: `.dockerignore` — exclude build junk and huge data files from the context.

### Task 1: [Dependency] Pin `wimage` as a git URL in all cargo files

**Files:**
- Modify: `frontend/Cargo.toml:17`
- Modify: `pipeline/Cargo.toml:7`
- Regenerate: `frontend/Cargo.lock`, `Cargo.lock`

**Interfaces:**
- Produces: `frontend/Cargo.lock` and root `Cargo.lock` each containing a `[[package]] name = "wimage"` entry whose `source` starts with `git+https://github.com/Hugi-R/wplace-image`; `wwimage-wasm`/`wpda-pipeline` depend on it as before.

- [ ] **Step 1: Edit `frontend/Cargo.toml` to use the git dependency**

In `frontend/Cargo.toml`, replace line 17:

```toml
wimage = { path = "/run/media/system/DataBtrfs/wplace/wplace-image/wimage" }
```

with:

```toml
wimage = { git = "https://github.com/Hugi-R/wplace-image", rev = "d718548" }
```

- [ ] **Step 2: Edit `pipeline/Cargo.toml` to use the git dependency**

In `pipeline/Cargo.toml`, replace line 7:

```toml
wimage = { path = "/run/media/system/DataBtrfs/wplace/wplace-image/wimage" }
```

with:

```toml
wimage = { git = "https://github.com/Hugi-R/wplace-image", rev = "d718548" }
```

- [ ] **Step 3: Regenerate the root lockfile**

Run: `cargo update -p wimage`
Expected: cargo fetches the pinned commit `d718548` from GitHub and rewrites `Cargo.lock`. Verify:

Run: `grep -A4 'name = "wimage"' Cargo.lock`
Expected: the `wimage` package block has a `source = "git+https://github.com/Hugi-R/wplace-image?rev=d718548#<sha>"` line.

- [ ] **Step 4: Regenerate the frontend lockfile**

Run: `cargo update -p wimage`
(from the repo root against the root workspace does **not** touch `frontend/Cargo.lock`, which is a separate workspace) then run `cd frontend && cargo update -p wimage`.

Expected: `frontend/Cargo.lock` `wimage` block gains the same `source = git+https...` line.

- [ ] **Step 5: Verify the host builds still work**

Run: `cargo build --release -p wpda-tileserver`
Expected: succeeds, emits `target/release/wpda-tileserver`. No warnings about a missing path dependency.

Run: `cd frontend && ~/.cargo/bin/wasm-pack build --target web --no-default-features`
Expected: succeeds, regenerates `frontend/pkg/wimage_wasm.js` and `frontend/pkg/wimage_wasm_bg.wasm` (same artifacts as before, now built from the git-source wimage).

- [ ] **Step 6: Commit**

```bash
git add frontend/Cargo.toml frontend/Cargo.lock pipeline/Cargo.toml Cargo.lock
git commit -m "build(deps): use wimage from git URL pinned to rev d718548"
```

### Task 2: [Dockerfile] Minimal multi-stage image

**Files:**
- Create: `Dockerfile`
- Create: `.dockerignore`

**Interfaces:**
- Consumes: Task 1's git-URL `wimage` deps present in `frontend/Cargo.lock` and root `Cargo.lock`.
- Produces: `Dockerfile` that yields image `wpda-tileserver` running the server with static assets baked into `/data` and `weeks/` as the single runtime mount point.

- [ ] **Step 1: Write `.dockerignore`**

Create `.dockerignore` at the repo root:

```gitignore
.git
target
frontend/target
frontend/pkg
tmp
tmp*
bench.db
*.db
profile.json.gz
.worktrees
.swival
.agents
docs
tasks
```

- [ ] **Step 2: Write the `Dockerfile`**

Create `Dockerfile` at the repo root:

```dockerfile
# syntax=docker/dockerfile:1
#
# Minimal image for the WPDA tile server.
# Build:  docker build -t wpda-tileserver .
# Run:    docker run -p 8080:8080 -v /path/to/weeks:/data/weeks wpda-tileserver

# ---- Stage 1: build the WASM lib (frontend) ----
FROM rust:1.97-alpine AS wasm
RUN apk add --no-cache binaryen curl tar
RUN rustup target add wasm32-unknown-unknown
ARG WASM_PACK_VERSION=0.13.1
RUN curl -sSfL \
    https://github.com/rustwasm/wasm-pack/releases/download/v${WASM_PACK_VERSION}/wasm-pack-v${WASM_PACK_VERSION}-x86_64-unknown-linux-musl.tar.gz \
    | tar -xz --strip-components=1 -C /usr/local/bin \
    && wasm-pack --version
WORKDIR /build/frontend
COPY frontend/ ./
RUN wasm-pack build --target web --no-default-features

# ---- Stage 2: build the fully static tileserver binary ----
FROM rust:1.97-alpine AS server
RUN apk add --no-cache gcc musl-dev
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY tileserver/ tileserver/
COPY pipeline/ pipeline/
RUN cargo fetch
RUN cargo build --release -p wpda-tileserver

# ---- Stage 3: assemble the static data directory ----
FROM alpine:3.22 AS assemble
WORKDIR /data
RUN mkdir -p weeks
COPY --from=wasm /build/frontend/pkg/wimage_wasm.js assets/wimage_wasm.js
COPY --from=wasm /build/frontend/pkg/wimage_wasm_bg.wasm assets/wimage_wasm_bg.wasm
COPY frontend/assets/tile-worker.js assets/tile-worker.js
COPY frontend/index.html index.html.tmpl
COPY frontend/favicon.ico favicon.ico
COPY tileserver/i18n/ i18n/

# ---- Final image ----
FROM scratch
COPY --from=server /build/target/release/wpda-tileserver /wpda-tileserver
COPY --from=assemble /data /data
ENV PORT=8080
ENV DATA_PATH=/data
EXPOSE 8080
USER 65532:65532
ENTRYPOINT ["/wpda-tileserver"]
```

Notes:
- `wasm-pack build --target web` reads `package.metadata.wasm-pack.profile.release` (`wasm-opt -Oz --enable-bulk-memory`) and requires `wasm-opt` from `binaryen` (Alpine community repo).
- The musl Alpine build makes `target/release/wpda-tileserver` fully static; `scratch` needs nothing else.
- `pipeline/` is copied for workspace resolution only; `-p wpda-tileserver` never compiles it.

- [ ] **Step 3: Validate the Dockerfile parses**

The Dockerfile is a syntax/whitespace artifact; a build is the only real check and requires Docker (not installed on this machine). Confirm with the user:

Run (on a machine with Docker, per the user): `docker build -t wpda-tileserver .`

- [ ] **Step 4: Smoke-test the image**

With a weeks directory containing at least one `w*_*.db`:

```bash
docker run --rm -p 8080:8080 -v /path/to/weeks:/data/weeks wpda-tileserver
curl -fsS -o /dev/null -w '%{http_code} %{content_type}\n' http://localhost:8080/   # 200 text/html
curl -fsS -o /dev/null -w '%{http_code} %{content_type}\n' http://localhost:8080/assets/wimage_wasm.js   # 200 application/javascript
```

If no weeks dir exists yet, the container exits 1 with "no week database files found" — expected, not a failure of the image.

- [ ] **Step 5: Commit**

```bash
git add Dockerfile .dockerignore
git commit -m "build(docker): minimal scratch image for wpda tileserver"
```

## Verification

- [ ] Host builds pass after the dep switch (Task 1 Step 5).
- [ ] User-reported `docker build` succeeds on `rust:1.97-alpine`.
- [ ] Smoke-test curls return 200 with the expected content types.
- [ ] `docker image inspect` shows `Entrypoint: ["/wpda-tileserver"]`, `User: 65532:65532`, `ExposedPorts: {"8080/tcp"}`.

## Self-Review Notes

- Spec sections mapped: git dep (Section 1 → Task 1), multi-stage Dockerfile (Section 2 → Task 2), runtime layout/SCHEMA (Section 3 → Task 2 Step 2 + run command), testing (spec testing → Task 2 Steps 3-4).
- No placeholders; skip-conditions are explicit (no-weeks startup exit is documented as expected behavior).
- The `pipeline/` copy choice: copying its sources (rather than a bare manifest) guarantees cargo's workspace resolution succeeds even if cargo resolves pipeline's manifest; it is never compiled.