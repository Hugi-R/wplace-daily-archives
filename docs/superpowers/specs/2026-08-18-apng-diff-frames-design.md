# APNG diff-only frames design (skip boundary full images)

**Date:** 2026-08-18
**Status:** Approved

## Goal

`apng_from_history` renders each week-boundary full base frame as a complete image, roughly doubling the APNG size on a short export and growing linearly worse on longer APNGs. Make every frame of the APNG contain only the diff against the previously emitted canvas, and skip frames in which nothing changed. Apply the change in the WASM rendered (the `wimage` crate), keeping the server untouched to avoid server CPU cost.

## Background / constraints (explored)

- `apng_from_history` (wimage/src/tilehistory.rs:302) already emits per-date frames that are transparent except where changed, but at week-boundary dates the stored `TileHistory` entry is a **full base** frame (no `DIFF_NO_CHANGE` markers). `apply_diff_img` (tilehistory.rs:381) then writes every pixel, producing a full-size frame.
- The PNG encoder must declare the animated frame count in the `acTL` chunk (`set_animated(num_frames, 0)`) *before* any frame is written.
- Approach A (renderer-side) chosen over server-side pre-diffing to keep CPU off the server. The frontend WASM path (`frontend/src/lib.rs`) already calls `apng_from_history`; no change there.
- Diff semantics used in the store: `DIFF_NO_CHANGE` (254) = pixel unchanged since the tile's reference; `TRANSPARENT` (0) = erase to background. Full frames carry no `254`s.
- Real data shapes: seeded weeks have a base identical to the previous week tail; `makebase` (fresh-snapshot) weeks have a base differing from the prior canvas. After the boundary-rename fix, the canvas always equals a week's reference image before its diffs apply.

## Scope decisions

- Change is confined to `wimage/src/tilehistory.rs` (`apng_from_history` and helpers).
- Empty (fully transparent) frames after this date are skipped; frame 0 (the APNG base) is always emitted, even if it matches the white background.
- APNG `acTL` count is patched in-place after encoding (single pass, memory-safe).
- Playback timing changes: skipped frames no longer hold a slot, so an APNG with entirely static periods has fewer frames and plays faster relative to wall-clock. Accepted (user choice).

## Design

### Section 1 — Diff-only frames against an accumulated canvas

Replace the per-date frame logic in `apng_from_history` with an accumulated-canvas approach:

- Maintain `current: PalettedImage`, the true emitted canvas, initialized to the white `target_img` clone.
- Frame 0: emit the full canvas (base frame), built by overlaying the first date's stored images onto `target_img` as today.
- Frames ≥ 1: build a transparent `frame_img`; for each tile that has an image at this date, apply a new helper that per-pixel:
  - `DIFF_NO_CHANGE` → skip (tile unchanged, canvas matches the tile's reference after the boundary fix);
  - `TRANSPARENT` → maps to the background value;
  - otherwise the stored value is the new pixel content;
  - only when the *resulting* value differs from `current[px]` is it written to both `frame_img[px]` and `current[px]`. Equal pixels stay transparent (blended by the viewer).
- Track whether any pixel was written for the date; if none, the frame is dropped.
- Consequences on real data: seeded-week boundary bases become empty frames (skipped, no size cost); fresh-snapshot (`makebase`) bases emit only the genuinely changed pixels (small, correct). Any input quirk is handled by construction: the APNG only ever records real canvas changes.

### Section 2 — Frame counting vs the PNG encoder

The `acTL` frame count is only known after frame generation. Chosen approach:

- Generate and write each non-`None` frame sequentially (single pass, one tile decode/diff per frame, peak memory ≈ 2× canvas).
- After `writer.finish()`, patch the `acTL` `num_frames` field in the in-memory `out` bytes: walk the PNG chunk stream (8-byte signature, then length/type/data/CRC) to locate the `acTL` chunk, overwrite its `num_frames` u32 count, and recompute that chunk's CRC.

Rejected alternatives:
- Buffer all emitted frames then encode — O(frames × canvas) WASM memory (≈700MB worst case for a 6×6-tile export); freeze/OOM risk.
- Two-pass generation — doubles the tile decode/diff cost on an expensive export path.

### Section 3 — Code structure & testability

- Extract per-date frame building into `build_apng_frame(...)` returning `Option<PalettedImage>` (`None` = frame dropped; frame 0 always `Some`), so frame behavior is unit-testable without PNG parsing.
- New helper `patch_apng_frame_count(out: &mut [u8], count: u32)` for the `acTL` fix-up.
- The canvas-diff helper supersedes the existing `apply_diff_img` (tilehistory.rs:381; removed), which is canvas-blind.

## Error handling

- Existing assumptions retained: tile coordinates/lengths match canvas bounds; `target_img` stride equals tile offsets; stored images decode without error (unwraps as today).
- `patch_apng_frame_count`: if the `acTL` chunk is unexpectedly absent, return/treat as internal error (assert) rather than silently corrupting output.

## Testing

In `wimage/src/tilehistory.rs` tests:

1. Canvas-diff helper: differing pixels written to frame and `current`; identical pixels stay transparent; `DIFF_NO_CHANGE` writes nothing; `TRANSPARENT` maps to background.
2. Empty-frame skip: identical consecutive dates → later date yields `None`; frame 0 always emitted.
3. Boundary-base scenarios: seeded-style base identical to canvas → dropped; makebase-style base differing from canvas → frame contains only changed pixels.
4. `patch_apng_frame_count`: on synthesized bytes, patches exactly the `acTL` `num_frames` field and recomputes CRC.
5. Integration: build a small multi-week `TileHistory` with gaps via `set()`, run `apng_from_history`, re-parse output bytes — valid chunk structure (IHDR/acTL/fcTL/IDAT) and `acTL` count equals the number of written frames.

No dependency on a full APNG decoder; byte-level chunk parsing in tests.

## Non-goals

- No server-side changes (kept deliberately out to save server CPU).
- No change to `/diff/all` stream shape; the boundary-rename fix remains as-is.
- No PNG/APNG re-encoding in JS.