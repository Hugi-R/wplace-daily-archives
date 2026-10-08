# layerswap settings-pane option design

Date: 2026-10-08
Status: approved (pending spec review)
Scope: `frontend/index.html` + all 7 `tileserver/i18n/*.json`
Depends on: `docs/superpowers/specs/2026-10-08-layerswap-overlay-design.md` (strategy already implemented in `map-styles.js`)

## Context

The settings panel (`#settings-panel` in `frontend/index.html`) has a
`#settings-layerswap` select with `swap` / `reload` options. The new `overlay`
strategy (visible new layer on top, old removed after load) is URL-selectable
(`?layerswap=overlay`) but not exposed in the pane. Worse, `applySettings`
collapses any non-`reload` value to `swap`, so overlay cannot stick via the UI.

## Decisions (user-confirmed)

- English label: **`Overlay (progressive)`** (rejected: bare `Overlay`, `Overlay (no flash)`).
- Non-English labels: drafted in the same spirit for all 7 files now (user corrects in review).

## Changes

### 1. Markup (`frontend/index.html`, after the reload option)

```html
<option value="overlay">{{t:layerswap_overlay}}</option>
```

No init change needed: `layerswapSel.value = WplaceMapState.strategy` already
reflects the URL-provided strategy once the option exists.

### 2. Wiring (`applySettings`, `frontend/index.html`)

Replace:

```js
const strategy = layerswapSel.value === 'reload' ? 'reload' : 'swap';
```

with:

```js
const strategy = ['swap', 'reload', 'overlay'].includes(layerswapSel.value)
  ? layerswapSel.value
  : 'swap';
```

`setWplaceVersion` already routes `overlay`; unknown values fall back to `swap`.

### 3. i18n (`tileserver/i18n/*.json`, after `layerswap_reload`)

| file | `layerswap_overlay` (draft) |
|------|------------------------------|
| en.json | Overlay (progressive) |
| es.json | Superponer (progresivo) |
| ja.json | オーバーレイ（プログレッシブ） |
| ko.json | 오버레이 (점진적) |
| pt-BR.json | Sobrepor (progressivo) |
| ru.json | Наложение (прогрессивное) |
| tr.json | Bindirme (aşamalı) |

Non-English strings are best-effort drafts; native review welcome.

## Verification

- `npm test` in `frontend/` stays green (no `map-styles.js` change).
- Manual: open panel → overlay option listed; select → sticks across changes;
  `?layerswap=overlay` → option preselected.

## Non-goals

- No new controls or layout changes; no strategy behavior changes.
