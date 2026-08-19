# syntax=docker/dockerfile:1
#
# Minimal image for the WPDA tile server.
# Build:  docker build -t wpda-tileserver .
# Run:    docker run -p 8080:8080 -v /path/to/weeks:/data/weeks wpda-tileserver

# ---- Stage 1: build the WASM lib (frontend) ----
FROM rust:1.97-alpine AS wasm
RUN apk add --no-cache binaryen clang llvm curl tar
RUN rustup target add wasm32-unknown-unknown
ARG WASM_PACK_VERSION=0.15.0
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