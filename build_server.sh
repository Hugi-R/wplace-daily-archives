#!/bin/bash
set -e

cargo build --release
(cd frontend && ~/.cargo/bin/wasm-pack build --target web --no-default-features)
cp ./target/release/wpda-tileserver ./tmp
cp ./frontend/pkg/wimage_wasm.js ./frontend/pkg/wimage_wasm_bg.wasm ./tmp/assets/
cp ./frontend/index.html ./tmp/index.html.tmpl
mkdir -p ./tmp/i18n
cp ./tileserver/i18n/*.json ./tmp/i18n/
cp ./frontend/assets/* ./tmp/assets/
