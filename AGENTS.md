# wplace-daily-archives
A website and its tooling to display daily map archive of wplace.live.

# frontend
The website, powered by MapLibre GL JS + Astro + Svelte. A WASM webworker handle tile rendering.
Port from a single HTML+JS webpage, still available at `frontend/old`

# tileserver
The Rust backend for the website.

# pipeline
Rust tools to produce the map tiles displayed by the website.