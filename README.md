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
TODO

## AI Disclose
An AI spec-driven development workflow was used to develop pretty much everything in this repo, find the specs at [docs/superpowers/specs](/docs/superpowers/specs). [obra/superpowers](https://github.com/obra/superpowers) is great!

## Thanks
A big thank you to
- [murolem/wplace-archives](https://github.com/murolem/wplace-archives) for nearly one year of wplace archive, and figuring out the trick I now use on my own archiver.
- [bczhc/wplace-diffs](https://github.com/bczhc/wplace-diffs) to have made a compression of murolem's archive, allowing me to bootsrap the new version.