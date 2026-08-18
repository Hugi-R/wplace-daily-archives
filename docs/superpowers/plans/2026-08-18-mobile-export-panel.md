# Mobile Export Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the export panel usable on mobile portrait by docking it to the top of the viewport as a scrollable sheet (~65vh max) with a tighter layout, via a single pure-CSS `@media (orientation: portrait)` block.

**Architecture:** `frontend/index.html` is a single self-contained vanilla-JS file; all styles live in one inline `<style>` block using CSS variables. The change is CSS-only: one media query overrides `#export-panel`'s fixed geometry (right drawer → top sheet) and compacts spacing/typography/bounds grid. No JS changes; open/close, chrome-hiding, bounds/z/date logic, and the overlay canvas are untouched.

**Tech Stack:** Vanilla HTML/CSS/JS, Maplibre GL, `build_server.sh` (release tileserver + `wasm-pack`), serve with `DATA_PATH=tmp`.

## Global Constraints

- Trigger is `orientation: portrait` (applies to phones and tablets in portrait). Landscape and desktop keep the current 280px right-side drawer.
- Top sheet: `top: 0; left: 0; right: 0; width: auto; height: auto; max-height: 65vh; border-left: none; border-bottom: 1px solid var(--border-color);` — content scrolls via the existing `overflow-y: auto`.
- Tighter layout: panel `padding: 8px 12px` / `font-size: 13px`; `h2` 14px with `margin-bottom: 6px`; `section` `margin-bottom: 6px`; `.export-bounds` becomes `grid-template-columns: repeat(4, 1fr)`.
- Keep `z-index: 200`, `background: var(--panel-bg)`, `box-shadow: var(--shadow-lg)`, `box-sizing: border-box` from the base rule.
- `#export-close` stays absolute top-right (now against the sheet's top-right corner) — no change.
- Do not add gratuitous comments; keep new CSS in the existing `<style>` block using its CSS variables.
- `frontend` is a standalone workspace; verify through `build_server.sh` + serving `tmp/`.

---

### Task 1: Portrait top-sheet CSS override

**Files:**
- Modify: `frontend/index.html` (insert a media query after the `body.export-open …` rules, before `</style>` at line ~284)

**Interfaces:**
- Consumes: existing `#export-panel` base rule (frontend/index.html:237-252) and its `#export-close`, `#export-panel h2`, `#export-panel section`, `.export-bounds` rules.
- Produces: a `@media (orientation: portrait)` block that overrides the above for portrait screens. No symbols consumed or produced for later tasks (single-task plan).

- [ ] **Step 1: Add the portrait media query**

In `frontend/index.html`, immediately after the `body.export-open #extra, … { display: none !important; }` rule (line ~283) and before `</style>`, add:

```css
    @media (orientation: portrait) {
      #export-panel {
        top: 0;
        left: 0;
        right: 0;
        width: auto;
        height: auto;
        max-height: 65vh;
        border-left: none;
        border-bottom: 1px solid var(--border-color);
        padding: 8px 12px;
        font-size: 13px;
      }
      #export-panel h2 { margin: 0 0 6px; font-size: 14px; }
      #export-panel section { margin-bottom: 6px; }
      #export-panel .export-bounds { grid-template-columns: repeat(4, 1fr); }
      #export-panel .export-bounds label { font-size: 11px; margin: 2px 0; }
    }
```

- [ ] **Step 2: Verify the base (landscape/desktop) layout is untouched**

Confirm the rule was inserted only inside the new media query and that no existing `#export-panel` line was altered:
Run: `git diff frontend/index.html`
Expected: only the `@media (orientation: portrait) { … }` block added; every pre-existing line unchanged.

- [ ] **Step 3: Rebuild and serve**

Run: `./build_server.sh`
Run: `DATA_PATH=tmp ./tmp/wpda-tileserver`
Expected: server starts on http://localhost:8080 with the rebuilt `index.html.tmpl` (which carries the new CSS).

- [ ] **Step 4: Verify portrait behavior**

Browser http://localhost:8080, DevTools device toolbar:
- **Phone portrait** (e.g. 390×844): click `Export` → panel spans the full width, docked to the top edge, `max-height` ≈ 65vh; the map remains visible below; content taller than the sheet scrolls inside it.
- **Tablet portrait** (e.g. 768×1024): same top-sheet behavior.
- **Landscape / desktop**: click `Export` → panel is still the 280px right-side full-height drawer with the original spacing (padding 12px, font-size 14px, 2×2 bounds grid).
- **Close (×)** → panel hides, chrome (`#version-select`, left column, zoom buttons, transparency toggle) is restored.
- **Bounds preview**: with the panel open, the blue selection rectangle still tracks the map and updates when typing bounds / panning the map (map interactive below the sheet).
- **Regression**: PNG and APNG export still run and download; tile counts / z selector / date selectors behave as before.

- [ ] **Step 5: Commit**

```bash
git add frontend/index.html
git commit -m "feat(frontend): dock export panel to top as scrollable sheet on portrait screens"
```

---

## Self-Review

**Spec coverage:**
- §"Portrait media query": Task 1 Step 1.
- §"Panel repositioning": Task 1 Step 1 (`top/left/right`, `height:auto`, `width:auto`, `max-height:65vh`, `border-left:none`, `border-bottom`, unchanged `z-index`/background/shadow; `#export-close` untouched).
- §"Tighter layout": Task 1 Step 1 (padding 8px 12px, font-size 13px, h2 14px/mb 6px, section mb 6px, `.export-bounds` 4-column).
- §"Unchanged" (open/close JS, chrome hiding, landscape): Step 2 + Step 4 verification.
- §"Testing & verification": Steps 3–4 (portrait phone + tablet, landscape regression, close, preview rectangle, PNG/APNG).

**Placeholder scan:** no TBD/TODO; the CSS block is complete and exact; verification steps name concrete viewport sizes and behaviors.

**Type consistency:** n/a (CSS-only; element IDs referenced in the new rule — `#export-panel`, `h2`, `section`, `.export-bounds`, `label` — match the existing markup at frontend/index.html:327-353).
