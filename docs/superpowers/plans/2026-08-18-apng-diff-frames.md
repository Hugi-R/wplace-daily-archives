# APNG Diff-Only Frames Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every APNG frame (except frame 0) contain only the diff against the previously emitted canvas, and skip frames in which nothing changed, so week-boundary full base frames no longer bloat the export.

**Architecture:** Change is confined to the `wimage` crate's `apng_from_history` (in `wimage/src/tilehistory.rs`). New private helpers maintain an accumulated canvas, write only differing pixels into the emitted frame, drop fully-transparent frames, and patch the `acTL` `num_frames` chunk field after encoding (the count is only known after generation). The tileserver is untouched.

**Tech Stack:** Rust 2024, `wimage` crate, `png` 0.18 (Encoder/Decoder), `zstd` (via `CompressedImage`), `anyhow`. Tests run inside the `wimage` crate with `cargo test -p wimage`.

## Global Constraints

- All code changes are in `/run/media/system/DataBtrfs/wplace/wplace-image/wimage/src/tilehistory.rs` (the `wimage` crate). Do not touch `tileserver`, `pipeline`, or `frontend`.
- `wimage` is a separate git repo rooted at `/run/media/system/DataBtrfs/wplace/wplace-image`. All git commands run from that directory.
- All `cargo` test/build/clippy commands run from `/run/media/system/DataBtrfs/wplace/wplace-image`.
- Frame 0 (the APNG base) is ALWAYS emitted, even if it matches the white background. Only later frames may be dropped.
- The `acTL` chunk's `num_frames` is patched after `writer.finish()` (single-pass, no frame buffering, no two-pass generation).
- Keep `DIFF_NO_CHANGE` (254) and `TRANSPARENT` (0) semantics: unchanged pixels stay transparent in emitted frames; `TRANSPARENT` maps to the background color.
- Playback timing changes (skipped frames shorten duration) are accepted; do not restore "one frame per stored date".
- No `apply_diff_img` remains after Task 5 (it is superseded).

---

### Task 1: Streaming CRC-32 helper

**Files:**
- Modify: `wimage/src/tilehistory.rs` (insert `crc32_update` after the existing `apply_diff_img`, i.e. after line ~401; keep it private)
- Test: `wimage/src/tilehistory.rs` inside `mod tests`

**Interfaces:**
- Produces: `fn crc32_update(crc: u32, data: &[u8]) -> u32` — streaming CRC-32 (IEEE, reflected polynomial `0xEDB88320`), chainable: pass `0` first, then the previous result. Used by Task 4 to recompute the patched `acTL` chunk CRC.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` (after the existing `make_paletted_with_changes` helper, keep `use super::*` which brings `crc32_update` into scope):

```rust
    // -- crc32 tests --

    #[test]
    fn crc32_matches_known_vector() {
        // Standard CRC-32 check value for the ASCII string "123456789".
        assert_eq!(crc32_update(0, b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn crc32_chains_across_calls() {
        let data: Vec<u8> = b"acTL".to_vec().into_iter()
            .chain(2u32.to_be_bytes())
            .chain(0u32.to_be_bytes())
            .collect();
        let one_shot = crc32_update(0, &data);
        let first = crc32_update(0, b"acTL");
        let chained = crc32_update(crc32_update(first, &2u32.to_be_bytes()), &0u32.to_be_bytes());
        assert_eq!(one_shot, chained);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wimage crc32`
Expected: FAIL with "cannot find function `crc32_update` in this scope"/compile error.

- [ ] **Step 3: Write the implementation**

After the `apply_diff_img` function (it stays for now; it is removed in Task 5), add:

```rust
/// Streaming CRC-32 (IEEE, reflected polynomial 0xEDB88320), as required by the
/// PNG spec for chunk CRCs. Chain calls by feeding the previous result back in,
/// starting with 0. Used to fix the acTL CRC after patching its frame count.
fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut crc = crc ^ 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    crc ^ 0xFFFF_FFFF
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wimage crc32`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add wimage/src/tilehistory.rs
git commit -m "feat: add streaming CRC-32 helper for PNG chunk patching"
```

---

### Task 2: Canvas-diff apply helper

**Files:**
- Modify: `wimage/src/tilehistory.rs` (insert `apply_diff_to_canvas` next to `apply_diff_img`)
- Test: `wimage/src/tilehistory.rs` inside `mod tests`

**Interfaces:**
- Consumes: `PalettedImage` (`width`, `height`, `indices`), `palette::DIFF_NO_CHANGE`, `palette::TRANSPARENT`.
- Produces: `fn apply_diff_to_canvas(src: &PalettedImage, dst: &mut PalettedImage, canvas: &mut PalettedImage, tile_x_offset: i64, tile_y_offset: i64, background: u8) -> bool` — writes `src`'s effect into `dst` (emitted frame) and `canvas` (accumulated state) only where the resulting value differs from `canvas[px]`; returns whether any pixel changed.

**Semantics (per pixel in `src`):**
- `DIFF_NO_CHANGE` → leave `dst` transparent and `canvas` untouched (continue).
- `TRANSPARENT` → resulting value is `background`.
- otherwise → resulting value is the pixel itself.
- If the resulting value equals the current `canvas` pixel, write nothing (frame stays transparent there). Else write it to both `dst` and `canvas`.
- Tile placement uses the tile's actual dimensions (`offset_x = tile_x_offset * src.width`, `offset_y = tile_y_offset * src.height`), so tests can use small tiles.

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests`:

```rust
    // -- apply_diff_to_canvas tests --

    #[test]
    fn canvas_writes_changed_pixels_only() {
        let src = make_paletted_with_changes(2, 2, palette::WHITE, &[(0, 10), (3, 20)]);
        let mut frame = make_paletted(2, 2, palette::TRANSPARENT);
        let mut canvas = make_paletted(2, 2, palette::WHITE);
        assert!(apply_diff_to_canvas(&src, &mut frame, &mut canvas, 0, 0, palette::WHITE));
        assert_eq!(canvas.indices, vec![10, 5, 5, 20]);
        assert_eq!(frame.indices, vec![10, 0, 0, 20]);
    }

    #[test]
    fn canvas_identical_image_is_noop() {
        let src = make_paletted(2, 2, palette::WHITE);
        let mut frame = make_paletted(2, 2, palette::TRANSPARENT);
        let mut canvas = make_paletted(2, 2, palette::WHITE);
        assert!(!apply_diff_to_canvas(&src, &mut frame, &mut canvas, 0, 0, palette::WHITE));
        assert_eq!(frame.indices, vec![0; 4]);
        assert_eq!(canvas.indices, vec![palette::WHITE; 4]);
    }

    #[test]
    fn canvas_no_change_pixels_preserve_canvas() {
        // DIFF_NO_CHANGE must not overwrite the canvas and emits transparent.
        let src = make_paletted_with_changes(2, 2, palette::DIFF_NO_CHANGE, &[(0, 9)]);
        let mut frame = make_paletted(2, 2, palette::TRANSPARENT);
        let mut canvas = make_paletted_with_changes(2, 2, palette::WHITE, &[(1, 3)]);
        assert!(apply_diff_to_canvas(&src, &mut frame, &mut canvas, 0, 0, palette::WHITE));
        assert_eq!(frame.indices, vec![9, 0, 0, 0]);
        assert_eq!(canvas.indices, vec![9, 3, 5, 5]);
    }

    #[test]
    fn canvas_transparent_maps_to_background() {
        let src = make_paletted(2, 2, palette::TRANSPARENT);
        let mut frame = make_paletted(2, 2, palette::TRANSPARENT);
        let mut canvas = make_paletted(2, 2, palette::BLACK);
        assert!(apply_diff_to_canvas(&src, &mut frame, &mut canvas, 0, 0, palette::WHITE));
        assert_eq!(frame.indices, vec![palette::WHITE; 4]);
        assert_eq!(canvas.indices, vec![palette::WHITE; 4]);
    }

    #[test]
    fn canvas_places_tiles_at_offset() {
        // 2x1 grid of 2x2 tiles => canvas 4x2; change a pixel in the right tile.
        let left = make_paletted(2, 2, palette::WHITE);
        let right = make_paletted_with_changes(2, 2, palette::WHITE, &[(0, 7)]);
        let mut frame = make_paletted(4, 2, palette::TRANSPARENT);
        let mut canvas = make_paletted(4, 2, palette::WHITE);
        assert!(!apply_diff_to_canvas(&left, &mut frame, &mut canvas, 0, 0, palette::WHITE));
        assert!(apply_diff_to_canvas(&right, &mut frame, &mut canvas, 1, 0, palette::WHITE));
        assert_eq!(canvas.indices, vec![5, 5, 5, 5, 7, 5, 5, 5]);
        assert_eq!(frame.indices, vec![0, 0, 0, 0, 7, 0, 0, 0]);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wimage canvas_`
Expected: FAIL with "cannot find function `apply_diff_to_canvas`".

- [ ] **Step 3: Write the implementation**

Add after the existing `apply_diff_img` function:

```rust
/// Apply `src` (a tile image or diff image) to both the emitted frame `dst` and the
/// accumulated canvas, but only where the resulting value differs from the canvas.
/// Pixels already matching the canvas stay transparent in the frame, so the APNG
/// only ever records real canvas changes. Returns whether any pixel changed.
fn apply_diff_to_canvas(
    src: &PalettedImage,
    dst: &mut PalettedImage,
    canvas: &mut PalettedImage,
    tile_x_offset: i64,
    tile_y_offset: i64,
    background: u8,
) -> bool {
    assert!(dst.width == canvas.width && dst.height == canvas.height);
    let offset_x = tile_x_offset as usize * src.width;
    let offset_y = tile_y_offset as usize * src.height;

    let mut changed = false;
    for y in 0..src.height {
        let src_row = y * src.width;
        let dst_row = (y + offset_y) * dst.width + offset_x;
        for x in 0..src.width {
            let v = src.indices[src_row + x];
            if v == palette::DIFF_NO_CHANGE {
                continue;
            }
            let value = if v == palette::TRANSPARENT { background } else { v };
            let pos = dst_row + x;
            if canvas.indices[pos] != value {
                canvas.indices[pos] = value;
                dst.indices[pos] = value;
                changed = true;
            }
        }
    }
    changed
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wimage canvas_`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add wimage/src/tilehistory.rs
git commit -m "feat: add canvas-diff frame apply helper for APNG"
```

---

### Task 3: build_apng_frame (diff-only frame builder)

**Files:**
- Modify: `wimage/src/tilehistory.rs` (insert `build_apng_frame` next to `apply_diff_to_canvas`)
- Test: `wimage/src/tilehistory.rs` inside `mod tests`

**Interfaces:**
- Consumes: `apply_diff_to_canvas` (Task 2), `init_img_from_tile_coords`, `TileHistory.imgs`, `CompressedImage::to_paletted`, `DateHours`.
- Produces: `fn build_apng_frame(history: &HashMap<(u16, u16), TileHistory>, current: &mut PalettedImage, date: DateHours, frame_index: usize, min_x: u16, min_y: u16, max_x: u16, max_y: u16) -> Option<PalettedImage>` — `Some(frame)` if the date changes the canvas (or `frame_index == 0`), else `None`.

Note: until Task 5, `build_apng_frame` is unused by `apng_from_history`, so `cargo test` will emit a `dead_code` warning. That is expected and disappears in Task 5.

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests`:

```rust
    // -- build_apng_frame tests --

    fn th_from_entries(entries: &[(u32, PalettedImage)]) -> TileHistory {
        let mut th = TileHistory { imgs: HashMap::new() };
        for (date, img) in entries {
            th.imgs.insert(DateHours(*date), img.to_compressed_bytes().unwrap());
        }
        th
    }

    #[test]
    fn build_frame_0_is_full_canvas() {
        let big = make_paletted(1000, 1000, 10);
        let mut history = HashMap::new();
        history.insert((0u16, 0u16), th_from_entries(&[(0, big)]));
        let mut current = init_img_from_tile_coords(0, 0, 0, 0, palette::WHITE);
        let frame = build_apng_frame(&history, &mut current, DateHours(0), 0, 0, 0, 0, 0).unwrap();
        assert_eq!(frame.indices, vec![10; 1_000_000]);
        assert_eq!(current.indices, vec![10; 1_000_000]);
    }

    #[test]
    fn build_frame_identical_to_canvas_is_none() {
        let mut history = HashMap::new();
        history.insert((0u16, 0u16), th_from_entries(&[
            (0, make_paletted(1000, 1000, 10)),
            (5, make_paletted(1000, 1000, 10)),
        ]));
        let mut current = init_img_from_tile_coords(0, 0, 0, 0, palette::WHITE);
        assert!(build_apng_frame(&history, &mut current, DateHours(0), 0, 0, 0, 0, 0).is_some());
        assert!(build_apng_frame(&history, &mut current, DateHours(5), 1, 0, 0, 0, 0).is_none());
        assert_eq!(current.indices, vec![10; 1_000_000]);
    }

    #[test]
    fn build_frame_full_base_diff_from_canvas_emits_changed_pixels() {
        // makebase-style boundary base: mostly identical to the canvas, a few pixels differ.
        let base = make_paletted_with_changes(1000, 1000, 10, &[(0, 7), (1000, 7)]);
        let mut history = HashMap::new();
        history.insert((0u16, 0u16), th_from_entries(&[
            (0, make_paletted(1000, 1000, 10)),
            (168, base),
        ]));
        let mut current = init_img_from_tile_coords(0, 0, 0, 0, palette::WHITE);
        build_apng_frame(&history, &mut current, DateHours(0), 0, 0, 0, 0, 0);
        let frame = build_apng_frame(&history, &mut current, DateHours(168), 1, 0, 0, 0, 0).unwrap();
        assert_eq!(frame.indices[0], 7);
        assert_eq!(frame.indices[1000], 7);
        assert_eq!(frame.indices[1], palette::TRANSPARENT);
        assert_eq!(current.indices[0], 7);
    }

    #[test]
    fn build_frame_with_no_image_at_date_is_none() {
        let mut history = HashMap::new();
        history.insert((0u16, 0u16), th_from_entries(&[(0, make_paletted(1000, 1000, 10))]));
        let mut current = init_img_from_tile_coords(0, 0, 0, 0, palette::WHITE);
        build_apng_frame(&history, &mut current, DateHours(0), 0, 0, 0, 0, 0);
        assert!(build_apng_frame(&history, &mut current, DateHours(99), 1, 0, 0, 0, 0).is_none());
        assert_eq!(current.indices, vec![10; 1_000_000]);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wimage build_frame`
Expected: FAIL with "cannot find function `build_apng_frame`".

- [ ] **Step 3: Write the implementation**

Add after `apply_diff_to_canvas`:

```rust
/// Build the diff-only frame for `date`, rendering each stored image against the
/// accumulated canvas `current`. Frame 0 is always emitted as the full canvas (the
/// APNG base frame); any later frame in which no pixel differs from the canvas is
/// dropped (returns None).
fn build_apng_frame(
    history: &HashMap<(u16, u16), TileHistory>,
    current: &mut PalettedImage,
    date: DateHours,
    frame_index: usize,
    min_x: u16,
    min_y: u16,
    max_x: u16,
    max_y: u16,
) -> Option<PalettedImage> {
    let mut frame_img = init_img_from_tile_coords(
        min_x as i64, min_y as i64, max_x as i64, max_y as i64, palette::TRANSPARENT,
    );
    let mut changed = false;
    for y in min_y..(max_y + 1) {
        for x in min_x..(max_x + 1) {
            if let Some(th) = history.get(&(x, y)) {
                if let Some(img_data) = th.imgs.get(&date) {
                    let img = img_data.to_paletted().unwrap();
                    changed |= apply_diff_to_canvas(
                        &img,
                        &mut frame_img,
                        current,
                        (x - min_x) as i64,
                        (y - min_y) as i64,
                        palette::WHITE,
                    );
                }
            }
        }
    }
    if frame_index == 0 {
        Some(current.clone())
    } else if changed {
        Some(frame_img)
    } else {
        None
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wimage build_frame`
Expected: 4 passed. (A `dead_code` warning for `build_apng_frame` and `apply_diff_to_canvas` is expected; it disappears in Task 5.)

- [ ] **Step 5: Commit**

```bash
git add wimage/src/tilehistory.rs
git commit -m "feat: add diff-only frame builder for APNG export"
```

---

### Task 4: patch_apng_frame_count

**Files:**
- Modify: `wimage/src/tilehistory.rs` (insert `patch_apng_frame_count` after `build_apng_frame`)
- Test: `wimage/src/tilehistory.rs` inside `mod tests`

**Interfaces:**
- Consumes: `crc32_update` (Task 1).
- Produces: `fn patch_apng_frame_count(out: &mut [u8], count: u32) -> anyhow::Result<()>` — walks the PNG chunk stream from the 8-byte signature, locates the `acTL` chunk, overwrites its first 4 data bytes (BE `num_frames`) with `count`, recomputes that chunk's CRC, and returns. Errors on missing/truncated/malformed chunks.

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests` (helpers to synthesize PNG chunks; they reuse the production `crc32_update`, which Task 1 verified against the standard check vector):

```rust
    // -- patch_apng_frame_count tests --

    fn png_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(chunk_type);
        out.extend_from_slice(data);
        let mut crc = crc32_update(0, chunk_type);
        crc = crc32_update(crc, data);
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }

    fn ihdr_data(width: u32, height: u32) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&width.to_be_bytes());
        d.extend_from_slice(&height.to_be_bytes());
        d.extend_from_slice(&[8, 3, 0, 0, 0]); // bit depth 8, indexed color, no interlace
        d
    }

    #[test]
    fn patch_updates_actl_count_and_crc() {
        let mut apng = b"\x89PNG\r\n\x1a\n".to_vec();
        apng.extend_from_slice(&png_chunk(b"IHDR", &ihdr_data(2, 2)));
        let actl_data: Vec<u8> = 1u32.to_be_bytes().into_iter()
            .chain(0u32.to_be_bytes())
            .collect();
        apng.extend_from_slice(&png_chunk(b"acTL", &actl_data)); // placeholder count 1
        apng.extend_from_slice(&png_chunk(b"IEND", &[]));

        patch_apng_frame_count(&mut apng, 3).unwrap();

        // Walk the chunks and verify the acTL count and CRC.
        let mut offset = 8;
        let mut seen_actl = false;
        while offset < apng.len() {
            let length = u32::from_be_bytes(apng[offset..offset + 4].try_into().unwrap()) as usize;
            let ty = &apng[offset + 4..offset + 8];
            if ty == b"acTL" {
                seen_actl = true;
                let nf = u32::from_be_bytes(apng[offset + 8..offset + 12].try_into().unwrap());
                assert_eq!(nf, 3);
                let mut crc = crc32_update(0, ty);
                crc = crc32_update(crc, &apng[offset + 8..offset + 8 + length]);
                let stored = u32::from_be_bytes(
                    apng[offset + 8 + length..offset + 12 + length].try_into().unwrap(),
                );
                assert_eq!(stored, crc);
            }
            offset += 12 + length;
        }
        assert!(seen_actl);
    }

    #[test]
    fn patch_errors_without_actl_chunk() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&png_chunk(b"IHDR", &ihdr_data(2, 2)));
        png.extend_from_slice(&png_chunk(b"IEND", &[]));
        assert!(patch_apng_frame_count(&mut png, 1).is_err());
    }

    #[test]
    fn patch_errors_on_truncated_chunk() {
        let mut bad = b"\x89PNG\r\n\x1a\n".to_vec();
        bad.extend_from_slice(&[0u8, 0, 0, 50]); // claims a 50-byte chunk that never follows
        assert!(patch_apng_frame_count(&mut bad, 1).is_err());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wimage patch_`
Expected: FAIL with "cannot find function `patch_apng_frame_count`".

- [ ] **Step 3: Write the implementation**

Add after `build_apng_frame`:

```rust
/// Rewrite the `acTL` chunk's `num_frames` field to the given count and fix its CRC.
/// The animated frame count is only known after generation (empty frames are
/// skipped), but the PNG encoder requires it up front, so a placeholder is patched
/// here after `writer.finish()`.
fn patch_apng_frame_count(out: &mut [u8], count: u32) -> anyhow::Result<()> {
    let mut offset = 8; // PNG signature
    while offset < out.len() {
        if offset + 4 > out.len() {
            anyhow::bail!("truncated PNG chunk header at offset {offset}");
        }
        let length = u32::from_be_bytes(out[offset..offset + 4].try_into().unwrap()) as usize;
        let type_start = offset + 4;
        let data_start = offset + 8;
        let data_end = data_start + length;
        let crc_end = data_end + 4;
        if crc_end > out.len() {
            anyhow::bail!("truncated PNG chunk at offset {offset}");
        }
        if &out[type_start..data_start] == b"acTL" {
            if length < 8 {
                anyhow::bail!("malformed acTL chunk");
            }
            out[data_start..data_start + 4].copy_from_slice(&count.to_be_bytes());
            let mut crc = crc32_update(0, &out[type_start..data_start]);
            crc = crc32_update(crc, &out[data_start..data_end]);
            out[data_end..crc_end].copy_from_slice(&crc.to_be_bytes());
            return Ok(());
        }
        offset = crc_end;
    }
    anyhow::bail!("acTL chunk not found")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wimage patch_`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add wimage/src/tilehistory.rs
git commit -m "feat: patch acTL frame count after APNG generation"
```

---

### Task 5: Rewire apng_from_history to diff-only frames

**Files:**
- Modify:
  - `wimage/src/tilehistory.rs` — replace `apng_from_history` (currently lines ~302-379) and delete `apply_diff_img` (lines ~381-401)
- Test: `wimage/src/tilehistory.rs` inside `mod tests`

**Interfaces:**
- Consumes: `build_apng_frame` (Task 3), `patch_apng_frame_count` (Task 4).
- Produces: `pub fn apng_from_history(history: HashMap<(u16, u16), TileHistory>, frame_delay_ms: u16) -> anyhow::Result<Vec<u8>>` with the same signature, now emitting only non-empty diff frames and returning a valid APNG whose `acTL` count matches the written frames.

- [ ] **Step 1: Write the failing test**

Add inside `mod tests` (the old `apng_from_history` emits one frame per stored date — 4 dates — so this test starts RED):

```rust
    // -- apng_from_history integration tests --

    #[test]
    fn apng_skips_unchanged_frames_and_patches_count() {
        // Single tile timeline with a seeded-style boundary and a diff:
        //   0   : full image all 10                      -> frame 0 (kept)
        //   10  : full image all 10 (identical)          -> skipped
        //   20  : all 10 except (0,0)=12                 -> frame (only that pixel)
        //   500 : all 12                                 -> frame (10s -> 12s)
        let mut history = HashMap::new();
        history.insert((0u16, 0u16), th_from_entries(&[
            (0, make_paletted(1000, 1000, 10)),
            (10, make_paletted(1000, 1000, 10)),
            (20, make_paletted_with_changes(1000, 1000, 10, &[(0, 12)])),
            (500, make_paletted(1000, 1000, 12)),
        ]));

        let apng = apng_from_history(history, 200).unwrap();

        // 1. acTL num_frames is patched to the number of actually-written frames,
        //    and one fcTL chunk is present per written frame.
        assert_eq!(actl_num_frames(&apng), 3);
        assert_eq!(chunk_count(&apng, b"fcTL"), 3);

        // 2. The output decodes as a valid animated PNG with the same frame count.
        let mut reader = png::Decoder::new(std::io::Cursor::new(&apng)).read_info().unwrap();
        assert_eq!(reader.info().animation_control.as_ref().unwrap().num_frames, 3);
        let mut frames = 0;
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
        loop {
            match reader.next_frame(&mut buf) {
                Ok(_) => frames += 1,
                Err(_) => break,
            }
        }
        assert_eq!(frames, 3);
    }

    fn chunk_count(data: &[u8], chunk_type: &[u8; 4]) -> usize {
        let mut count = 0;
        let mut offset = 8;
        while offset + 12 <= data.len() {
            let length = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
            if &data[offset + 4..offset + 8] == chunk_type.as_slice() {
                count += 1;
            }
            offset += 12 + length;
        }
        count
    }

    fn actl_num_frames(data: &[u8]) -> u32 {
        let mut offset = 8;
        while offset + 12 <= data.len() {
            let length = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
            if &data[offset + 4..offset + 8] == b"acTL" {
                return u32::from_be_bytes(data[offset + 8..offset + 12].try_into().unwrap());
            }
            offset += 12 + length;
        }
        panic!("acTL chunk not found");
    }
```

Note: `th_from_entries` is defined in Task 3.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p wimage apng_skips_unchanged_frames`
Expected: FAIL — the current implementation emits 4 frames and never patches the count, so `actl_frames == 4` (or the fcTL/IDAT count mismatch) fails the assertion.

- [ ] **Step 3: Rewrite apng_from_history**

Replace the entire `apng_from_history` body (from `pub fn apng_from_history(` through its closing `}` before `fn apply_diff_img`) with:

```rust
pub fn apng_from_history(history: HashMap<(u16, u16), TileHistory>, frame_delay_ms: u16) -> anyhow::Result<Vec<u8>> {
    assert!(history.len() >= 1, "need at least one tile history to create APNG");
    let mut date_set: HashSet<DateHours> = HashSet::new();
    let mut min_x: u16 = u16::MAX;
    let mut min_y: u16 = u16::MAX;
    let mut max_x: u16 = 0;
    let mut max_y: u16 = 0;

    for (x, y) in history.keys() {
        let (x, y) = (*x, *y);
        let th = history.get(&(x, y)).unwrap();
        for date in th.imgs.keys() {
            date_set.insert(*date);
        }
        if x < min_x {
            min_x = x;
        }
        if y < min_y {
            min_y = y;
        }
        if x > max_x {
            max_x = x;
        }
        if y > max_y {
            max_y = y;
        }
    }

    let sorted_dates: Vec<DateHours> = {
        let mut v: Vec<DateHours> = date_set.into_iter().collect();
        v.sort_by_key(|d| d.0);
        v
    };

    let target_img = init_img_from_tile_coords(min_x as i64, min_y as i64, max_x as i64, max_y as i64, palette::WHITE);

    assert!(sorted_dates.len() >= 1, "need at least one frame for APNG");
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, target_img.width as u32, target_img.height as u32);
    encoder.set_color(ColorType::Indexed);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(png::Compression::Balanced);

    // Build palette (RGB triples) and tRNS (alpha table)
    let pal = &palette::PNG_PALETTE_NO_DIFF;
    encoder.set_palette(&pal.0);
    encoder.set_trns(pal.1.as_slice());
    // Placeholder count: empty frames are skipped, so the real count is patched
    // into the acTL chunk after generation.
    encoder.set_animated(sorted_dates.len() as u32, 0)?;
    encoder.set_blend_op(png::BlendOp::Over)?;
    encoder.set_frame_delay(frame_delay_ms, 1000)?;
    let mut writer = encoder.write_header()?;

    let mut current = target_img.clone();
    let mut frame_count: u32 = 0;
    for (frame_index, date) in sorted_dates.iter().enumerate() {
        if let Some(frame) = build_apng_frame(
            &history,
            &mut current,
            *date,
            frame_index,
            min_x,
            min_y,
            max_x,
            max_y,
        ) {
            writer.write_image_data(&frame.indices)?;
            frame_count += 1;
        }
    }
    writer.finish()?;

    // Frame 0 is always emitted, so frame_count >= 1.
    patch_apng_frame_count(&mut out, frame_count)?;
    Ok(out)
}
```

- [ ] **Step 4: Delete apply_diff_img and run all tests**

Delete the now-unused `apply_diff_img` function (the old canvas-blind helper) entirely.

Run: `cargo test -p wimage`
Expected: All tests pass, including the new `apng_skips_unchanged_frames_and_patches_count` and no `dead_code` warnings.

- [ ] **Step 5: Verify with clippy and the dependent crates**

Run: `cargo clippy -p wimage --all-targets`
Expected: No warnings.

Run from `/run/media/system/DataBtrfs/wplace/wplace-daily-archives`: `cargo build --workspace`
Expected: Compiles (pipeline and tileserver depend on wimage via path).

- [ ] **Step 6: Commit**

Run from `/run/media/system/DataBtrfs/wplace/wplace-image`:

```bash
git add wimage/src/tilehistory.rs
git commit -m "feat: emit only per-canvas diffs in APNG and skip unchanged frames"
```

---

### Task 6 (manual): Browser verification on real gapped data

**Files:**
- None (verification only).

**Goal:** Confirm the export UI produces a smaller APNG with the same final frame on real gapped data.

- [ ] **Step 1: Rebuild the WASM frontend**

Run from `/run/media/system/DataBtrfs/wplace/wplace-daily-archives`:

```bash
(cd frontend && ~/.cargo/bin/wasm-pack build --target web --no-default-features)
cp ./frontend/pkg/wimage_wasm.js ./frontend/pkg/wimage_wasm_bg.wasm ./tmp/assets/
```

- [ ] **Step 2: Start the tile server against the gapped weeks**

Run from `/run/media/system/DataBtrfs/wplace/wplace-daily-archives`:

```bash
DATA_PATH=tmp PORT=8080 ./tmp/wpda-tileserver
```

- [ ] **Step 3: Compare APNG size before/after**

Open the UI (`http://localhost:8080`), open Export → APNG, pick a date range spanning the gapped weeks (73, 83, 84 under `tmp/weeks`; start before week 83 and end at the latest version), and export a small tile area.

Expected: the APNG downloads noticeably smaller than the pre-change build while the **last frame shows the exact same final state**. Spot-check by scrubbing the animation to its end.

- [ ] **Step 4: Record the result**

Note the APNG byte size and confirm the final frame matches. No commit is needed for this task (it is verification only).