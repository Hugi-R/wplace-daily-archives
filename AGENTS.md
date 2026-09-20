## Structure
- **pipeline**: build compressed archive.
- **frontend**: frontend to display the archive files using MapLibre. Static HTML/CSS/JS with Rust WASM.
- **tileserver**: simple backend to serve the frontend and the archive files.

## i18n
The folder `tileserver/i18n` contains the translations for all user facing text. When adding new text, or updating existing text, all files in this folder must be updated. The reference file is en.json.