# Wplace Daily Archives
The new version of [wplace.eralyon.net](https://wplace.eralyon.net), rewritten in Rust, with improved compression tech that enable new features.

See also:
- [wplace-archiver](https://github.com/Hugi-R/wplace-archiver/) responsible for archiving [wplace.live](https://wplace.live).
- [wimage](https://github.com/Hugi-R/wplace-image) a Rust crate for compressing/processing wplace images.

## Architecture
- [pipeline](/pipeline) tools for converting and compressing the raw wplace archives.
- [frontend](/frontend) the frontend for wplace.eralyon.net. HTML+CSS+JS+WASM.
- [tileserver](/tileserver) the backend, does static tamplating and handle the tile API.

## Quickstart

### Prerequisites

- [Rust](https://rustup.rs) (stable, edition 2024)
- [wasm-pack](https://wasm-bindgen.github.io/wasm-pack/) (for the frontend WASM)

### Building & running the tile server

Use the provided `build_server.sh` to build everything and assemble a runnable server directory:

```sh
./build_server.sh # builds the server + frontend WASM, fills ./tmp
```

This produces `tmp/` containing `wpda-tileserver`, `assets/`, `index.html.tmpl`, `osm000.png` and `i18n/`. You need to add the `weeks/` directory yourself.

Run `wpda-tileserver` pointing `DATA_PATH` at that directory:

```sh
DATA_PATH=./tmp PORT=8080 ./tmp/wpda-tileserver
```

The server expects this layout under `DATA_PATH`:

```
weeks/w<version>_<any>.db   # one read-only SQLite DB per week snapshot
assets/                     # static assets served at /assets/ (mostly for the WASM worker)
index.html.tmpl             # homepage template, rendered per language
favicon.ico  osm000.png     # site icon and map background for the preview
i18n/*.json                 # translations
```

`PORT` defaults to `8080` and `DATA_PATH` to `.`. The server expect `/` to redirect by `Accept-Language` and serve the tile API at `/tiles/<version>/<z>/<x>/<y>.zst` and `/diff/all/<z>/<x>/<y>.zst`.

### Docker

A minimal multi-stage Dockerfile builds the static WASM + server into a `scratch` image:

```sh
docker build -t wpda-tileserver .
docker run -p 8080:8080 -v /path/to/weeks:/data/weeks wpda-tileserver
```

### Building the archives

Weekly archives come from [Hugi-R/wplace-archives](https://huggingface.co/buckets/Hugi-R/wplace-archives). The `wpda-pipeline` binary (built with `cargo build --release -p wpda-pipeline`) turns raw PNG tiles into compressed week DBs.

Start a week from a `full_*.db` base:

```sh
wpda-pipeline makebase --base full/full_XXX.db \
    --output /path/to/weeks/w83_0.db
```

Then ingest each daily `inc_*.db` increment, which merges the new tiles and updates the week:

```sh
wpda-pipeline ingest --archives /path/to/weeks --increment ~/Téléchargements/inc_XXX.db
```

`makeweek.sh` automates this full base + incremental workflow for a week number.

## AI Disclose
An AI spec-driven development workflow was used to develop pretty much everything in this repo, find the specs at [docs/superpowers/specs](/docs/superpowers/specs). [obra/superpowers](https://github.com/obra/superpowers) is great!

## Thanks
A big thank you to
- [murolem/wplace-archives](https://github.com/murolem/wplace-archives) for nearly one year of wplace archive, and figuring out the trick I now use on my own archiver.
- [bczhc/wplace-diffs](https://github.com/bczhc/wplace-diffs) to have made a compression of murolem's archive, allowing me to bootsrap the new version.