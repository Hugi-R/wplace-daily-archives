# Settings Tooltips Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a click-to-toggle `?` info button next to each of the five settings-panel options, showing a server-translated tooltip (native Popover API, identical desktop/mobile behavior).

**Architecture:** Pure HTML/CSS/vanilla-JS change confined to `frontend/index.html` plus 6 new i18n keys in all 7 language files. Tooltips use the native Popover API (`popover` attribute + `popovertarget` buttons): top-layer rendering escapes the panel's `overflow-y: auto` clipping, and light-dismiss (outside-click, Esc, re-click) is native. ~15 lines of JS position the tooltip near its button and close open tooltips when the panel closes.

**Tech Stack:** Static HTML template with server-side `{{t:KEY}}` substitution (Rust tileserver), inline CSS, inline vanilla JS, native Popover API (Chrome 114+, Safari 17+, Firefox 125+ — fine for this WebGL2/MapLibre app).

**Spec:** `docs/superpowers/specs/2026-10-17-settings-tooltips-design.md`

**File map:**
- Modify: `tileserver/i18n/en.json` (reference) + `es.json`, `ja.json`, `ko.json`, `pt-BR.json`, `ru.json`, `tr.json`
- Modify: `frontend/index.html` (markup ~lines 512-535, CSS ~lines 179-214, JS ~lines 1527-1577)

---

### Task 1: i18n keys (all 7 languages)

The tileserver hard-fails on any key-set mismatch between language files, and the existing test `real_translation_files_have_identical_key_sets` (in `tileserver/src/main.rs`) enforces this against the real files. Add the 6 new keys red-green: English first (test fails), then all other languages (test passes).

**Files:**
- Modify: `tileserver/i18n/en.json`
- Modify: `tileserver/i18n/es.json`, `ja.json`, `ko.json`, `pt-BR.json`, `ru.json`, `tr.json`

- [ ] **Step 1: Add the 6 keys to `en.json` only**

In `tileserver/i18n/en.json`, insert immediately after the line `"denoise": "Denoise",` (before `"download_tile"`):

```json
  "help": "Help",
  "basemap_hint": "The background map rendering. Vector is the default Wplace map; Raster is a faster option.",
  "layerswap_hint": "Controls how version changes are rendered. Swap produces no visual artefact but is slow; Reload is immediate but produces a blank flash; Overlay draws the new tiles on top, then removes the old ones.",
  "supersampling_hint": "How many images are used to render a tile — the more, the sharper, but at a huge cost in bandwidth and computing.",
  "show_tile_overlay_hint": "Displays an overlay with the tile grid and coordinates.",
  "denoise_hint": "Removes lone pixels in the final image; best results when combined with supersampling ×4.",
```

- [ ] **Step 2: Run the key-parity test, expect FAIL**

Run: `cargo test real_translation_files_have_identical_key_sets`
(working directory: `tileserver/`; first run may take a few minutes to compile)

Expected: FAIL — e.g. `i18n files parse and share keys` / `Result::unwrap()` on an `Err` mentioning missing keys in `es`/`ja`/etc.

- [ ] **Step 3: Add the same 6 keys to the other 6 languages**

Each translation MUST reuse the exact option-label wording already displayed in that language (e.g. the `es` layer-swap hint uses the same words as `layerswap_swap`/`layerswap_reload`/`layerswap_overlay` in `es.json`). Insert each block immediately after that file's `"denoise": …` line.

In `tileserver/i18n/es.json`, after `"denoise": "Eliminar ruido",`:

```json
  "help": "Ayuda",
  "basemap_hint": "El renderizado del mapa de fondo. Vectorial es el mapa Wplace predeterminado; Ráster es una opción más rápida.",
  "layerswap_hint": "Controla cómo se representan los cambios de versión. Intercambiar no produce ningún artefacto visual pero es lento; Recargar es inmediato pero produce un destello en blanco; Superponer dibuja las teselas nuevas encima y luego elimina las antiguas.",
  "supersampling_hint": "Cuántas imágenes se usan para renderizar una tesela: cuanto más, más nítida, pero a un gran costo de ancho de banda y cómputo.",
  "show_tile_overlay_hint": "Muestra una superposición con la cuadrícula de teselas y sus coordenadas.",
  "denoise_hint": "Elimina píxeles aislados de la imagen final; mejores resultados combinado con supermuestreo ×4.",
```

In `tileserver/i18n/ja.json`, after `"denoise": "ノイズ除去",`:

```json
  "help": "ヘルプ",
  "basemap_hint": "背景地図の描画方法です。ベクターは Wplace のデフォルトの地図、ラスターはより高速なオプションです。",
  "layerswap_hint": "バージョン変更の描画方法を制御します。スワップは視覚的なアーティファクトを生みませんが遅いです。リロードは即時ですが空白のフラッシュが発生します。オーバーレイは新しいタイルを上に描画してから古いタイルを削除します。",
  "supersampling_hint": "1つのタイルを描画するために使用する画像の数です。多いほど鮮明になりますが、帯域幅と計算コストは大幅に増えます。",
  "show_tile_overlay_hint": "タイルグリッドと座標を表示するオーバーレイを表示します。",
  "denoise_hint": "最終画像内の単独ピクセルを除去します。スーパーサンプリング×4と組み合わせると最良の結果が得られます。",
```

In `tileserver/i18n/ko.json`, after `"denoise": "노이즈 제거",`:

```json
  "help": "도움말",
  "basemap_hint": "배경 지도 렌더링 방식입니다. 벡터는 기본 Wplace 지도이고, 래스터는 더 빠른 옵션입니다.",
  "layerswap_hint": "버전 변경 렌더링 방식을 제어합니다. 스왑은 시각적 아티팩트가 없지만 느립니다. 다시 로드는 즉각적이지만 흰깜빡임이 발생합니다. 오버레이는 새 타일을 위에 그린 다음 오래된 타일을 제거합니다.",
  "supersampling_hint": "하나의 타일을 렌더링하는 데 사용되는 이미지 개수입니다. 많을수록 선명하지만 대역폭과 연산 비용이 크게 듭니다.",
  "show_tile_overlay_hint": "타일 격자와 좌표를 표시하는 오버레이를 표시합니다.",
  "denoise_hint": "최종 이미지에서 단일 픽셀을 제거합니다. 슈퍼샘플링 ×4와 함께 사용하면 가장 좋은 결과를 얻을 수 있습니다.",
```

In `tileserver/i18n/pt-BR.json`, after `"denoise": "Remover ruído",`:

```json
  "help": "Ajuda",
  "basemap_hint": "A renderização do mapa de fundo. Vetor é o mapa padrão do Wplace; Raster é uma opção mais rápida.",
  "layerswap_hint": "Controla como as mudanças de versão são renderizadas. Trocar não produz artefatos visuais, mas é lento; Recarregar é imediato, mas produz um flash em branco; Sobrepor desenha os novos tiles por cima e depois remove os antigos.",
  "supersampling_hint": "Quantas imagens são usadas para renderizar um tile: quanto mais, mais nítido, mas com enorme custo de banda e processamento.",
  "show_tile_overlay_hint": "Exibe uma sobreposição com a grade de tiles e as coordenadas.",
  "denoise_hint": "Remove pixels isolados na imagem final; melhores resultados combinados com superamostragem ×4.",
```

In `tileserver/i18n/ru.json`, after `"denoise": "Шумоподавление",`:

```json
  "help": "Справка",
  "basemap_hint": "Отрисовка фоновой карты. Векторная — карта Wplace по умолчанию; Растровая — более быстрый вариант.",
  "layerswap_hint": "Управляет способом отрисовки изменений версии. Замена не даёт визуальных артефактов, но работает медленно; Перезагрузка происходит сразу, но даёт белую вспышку; Наложение рисует новые тайлы поверх, а затем удаляет старые.",
  "supersampling_hint": "Сколько изображений используется для отрисовки одного тайла: чем больше, тем чётче, но с огромной нагрузкой на канал и вычисления.",
  "show_tile_overlay_hint": "Показывает оверлей с сеткой тайлов и их координатами.",
  "denoise_hint": "Убирает одиночные пиксели в итоговом изображении; лучше всего работает в сочетании с суперсэмплингом ×4.",
```

In `tileserver/i18n/tr.json`, after `"denoise": "Gürültü temizleme",`:

```json
  "help": "Yardım",
  "basemap_hint": "Arka plan haritasının işlenme biçimi. Vektör, varsayılan Wplace haritasıdır; Raster daha hızlı bir seçenektir.",
  "layerswap_hint": "Sürüm değişikliklerinin nasıl işleneceğini kontrol eder. Takas görsel artefakt oluşturmaz ancak yavaştır; Yeniden yükleme anlıktır ancak beyaz bir boşluk oluşturur; Bindirme yeni karoları önce üstüne çizer, sonra eskileri kaldırır.",
  "supersampling_hint": "Bir karoyu işlemek için kullanılan görüntü sayısı: arttıkça daha keskin olur, ancak bant genişliği ve işlem maliyeti çok yüksek olur.",
  "show_tile_overlay_hint": "Karo ızgarası ve koordinatlarını gösteren bir katman gösterir.",
  "denoise_hint": "Son görüntüdeki yalnız pikselleri kaldırır; en iyi sonuçlar süper örnekleme ×4 ile birleştiğinde alınır.",
```

- [ ] **Step 4: Re-run the key-parity test, expect PASS**

Run: `cargo test real_translation_files_have_identical_key_sets`
(working directory: `tileserver/`)

Expected: PASS — all 7 languages parse and share the identical key set (78 keys each; the test also asserts 40..=100 keys per language).

- [ ] **Step 5: Run the template-render test, expect PASS**

Run: `cargo test real_template_renders_all_languages_without_leftovers`
(working directory: `tileserver/`)

Expected: PASS — renders the real `frontend/index.html` for all 7 languages; unused extra dict keys are fine (they are only injected into `window.I18N`).

- [ ] **Step 6: Commit**

```bash
git add tileserver/i18n/en.json tileserver/i18n/es.json tileserver/i18n/ja.json tileserver/i18n/ko.json tileserver/i18n/pt-BR.json tileserver/i18n/ru.json tileserver/i18n/tr.json
git commit -m "feat(i18n): add settings tooltip keys"
```

---

### Task 2: Settings-panel markup + CSS

Restructure the five settings into a label+`?`-button flex header row, add the five hidden popover tooltip divs, and style the new elements. Control ids and the existing checkbox `<label><input …> text</label>` form stay untouched, so all existing JS wiring keeps working.

**Files:**
- Modify: `frontend/index.html:515-534` (markup)
- Modify: `frontend/index.html:200` (label CSS) and after `:214` (new rules)

- [ ] **Step 1: Replace the settings markup**

In `frontend/index.html`, replace the block from `<label for="settings-basemap">` (line 515) through the denoise checkbox label (line 534) — i.e. everything between `<h2>{{t:settings}}</h2>` and the closing `</div>` of `#settings-panel` — with:

```html
    <div class="setting-head">
      <label for="settings-basemap">{{t:basemap}}</label>
      <button type="button" class="help-btn" popovertarget="tip-basemap" aria-label="{{t:help}}: {{t:basemap}}">?</button>
    </div>
    <select id="settings-basemap">
      <option value="vector">{{t:basemap_vector}}</option>
      <option value="raster">{{t:basemap_raster}}</option>
    </select>
    <div id="tip-basemap" class="help-tip" popover>{{t:basemap_hint}}</div>
    <div class="setting-head">
      <label for="settings-layerswap">{{t:layerswap}}</label>
      <button type="button" class="help-btn" popovertarget="tip-layerswap" aria-label="{{t:help}}: {{t:layerswap}}">?</button>
    </div>
    <select id="settings-layerswap">
      <option value="swap">{{t:layerswap_swap}}</option>
      <option value="reload">{{t:layerswap_reload}}</option>
      <option value="overlay">{{t:layerswap_overlay}}</option>
    </select>
    <div id="tip-layerswap" class="help-tip" popover>{{t:layerswap_hint}}</div>
    <div class="setting-head">
      <label for="settings-supersampling">{{t:supersampling}}</label>
      <button type="button" class="help-btn" popovertarget="tip-supersampling" aria-label="{{t:help}}: {{t:supersampling}}">?</button>
    </div>
    <select id="settings-supersampling">
      <option value="1">1</option>
      <option value="2">2 ({{t:default}})</option>
      <option value="4">4</option>
      <option value="8">8</option>
    </select>
    <div id="tip-supersampling" class="help-tip" popover>{{t:supersampling_hint}}</div>
    <div class="setting-head">
      <label><input type="checkbox" id="toggle-tile-overlay"> {{t:show_tile_overlay}}</label>
      <button type="button" class="help-btn" popovertarget="tip-tile-overlay" aria-label="{{t:help}}: {{t:show_tile_overlay}}">?</button>
    </div>
    <div id="tip-tile-overlay" class="help-tip" popover>{{t:show_tile_overlay_hint}}</div>
    <div class="setting-head">
      <label><input type="checkbox" id="toggle-denoise"> {{t:denoise}}</label>
      <button type="button" class="help-btn" popovertarget="tip-denoise" aria-label="{{t:help}}: {{t:denoise}}">?</button>
    </div>
    <div id="tip-denoise" class="help-tip" popover>{{t:denoise_hint}}</div>
```

Do not touch the `#settings-close` button or the `<h2>` above this block.

- [ ] **Step 2: Update the label CSS rule**

In the `<style>` block, replace:

```css
    #settings-panel label { display: block; margin: 8px 0 2px; }
```

with:

```css
    #settings-panel label { display: block; }
    #settings-panel .setting-head {
      display: flex;
      align-items: center;
      gap: 6px;
      margin: 8px 0 2px;
    }
    #settings-panel .setting-head label { margin: 0; }
```

- [ ] **Step 3: Add help-btn and help-tip rules**

In the `<style>` block, insert immediately after the existing rule `#settings-close:hover { color: var(--muted-text-strong); }` (line ~214):

```css
    #settings-panel .help-btn {
      flex: none;
      width: 18px;
      height: 18px;
      padding: 0;
      border: 1px solid var(--border-color);
      border-radius: 50%;
      background: transparent;
      color: var(--muted-text);
      font-size: 12px;
      line-height: 1;
      cursor: pointer;
    }
    #settings-panel .help-btn:hover { color: var(--muted-text-strong); }
    #settings-panel .help-tip {
      position: fixed;
      z-index: 30;
      margin: 0;
      max-width: 240px;
      padding: 8px 10px;
      border: 1px solid var(--border-color);
      border-radius: 6px;
      background: var(--panel-bg);
      box-shadow: var(--shadow-lg);
      color: var(--button-text);
      font-size: 13px;
      line-height: 1.35;
    }
    #settings-panel .help-tip::backdrop { background: transparent; }
```

Notes: `margin: 0` resets the UA popover margin; `::backdrop` stays transparent (light-dismiss still works because the backdrop captures the click); `position: fixed` + top layer means the panel's `overflow-y: auto` can never clip the tooltip. Exact pixel values may be tuned later (explicitly deferred by the spec).

- [ ] **Step 4: Verify the template still renders for all languages**

Run: `cargo test real_template_renders_all_languages_without_leftovers`
(working directory: `tileserver/`)

Expected: PASS — the new `{{t:help}}`, `{{t:basemap_hint}}`, … markers all resolve (keys exist from Task 1); no leftover `{{t:` in any rendered language.

- [ ] **Step 5: Commit**

```bash
git add frontend/index.html
git commit -m "feat(frontend): add settings help buttons and tooltip markup"
```

---

### Task 3: Tooltip positioning JS + panel-close cleanup

~15 lines of JS: position each tooltip near its `?` button when it opens, close any open tooltip when the panel closes, and make the first Esc close only the open tooltip (native) rather than the whole panel.

**Files:**
- Modify: `frontend/index.html:1527-1577` (settings-panel `DOMContentLoaded` block)

- [ ] **Step 1: Add tooltip positioning and close-on-panel-close logic**

In the existing `// --- Settings panel ---` `DOMContentLoaded` handler, change `setOpen` from:

```js
      function setOpen(open) {
        settingsPanel.classList.toggle('open', open);
        settingsButton.setAttribute('aria-expanded', open ? 'true' : 'false');
      }
```

to:

```js
      function setOpen(open) {
        settingsPanel.classList.toggle('open', open);
        settingsButton.setAttribute('aria-expanded', open ? 'true' : 'false');
        if (!open) closeHelpTooltips();
      }
```

- [ ] **Step 2: Change the Escape handler so a tooltip absorbs the first Esc**

In the same handler, replace:

```js
      document.addEventListener('keydown', function(e) {
        if (e.key === 'Escape' && settingsPanel.classList.contains('open')) {
          setOpen(false);
          settingsButton.focus();
        }
      });
```

with:

```js
      document.addEventListener('keydown', function(e) {
        if (e.key === 'Escape' && settingsPanel.classList.contains('open')) {
          if (document.querySelector('.help-tip:popover-open')) return;
          setOpen(false);
          settingsButton.focus();
        }
      });
```

(Native popover light-dismiss closes the tooltip itself; the next Esc then reaches this handler and closes the panel.)

- [ ] **Step 3: Add the positioning + closeHelpTooltips code**

Still inside the same `DOMContentLoaded` handler, insert immediately after the Escape-handler block (before `function applySettings()`):

```js
      // Position settings help tooltips near their ? buttons when opened.
      document.querySelectorAll('.help-btn').forEach(function(btn) {
        const tip = document.getElementById(btn.getAttribute('popovertarget'));
        if (!tip) return;
        tip.addEventListener('toggle', function() {
          if (!tip.matches(':popover-open')) return;
          const r = btn.getBoundingClientRect();
          const gap = 6;
          let top = r.bottom + gap;
          let left = r.left;
          if (top + tip.offsetHeight > window.innerHeight - 4) {
            top = r.top - gap - tip.offsetHeight;
          }
          if (left + tip.offsetWidth > window.innerWidth - 4) {
            left = window.innerWidth - 4 - tip.offsetWidth;
          }
          if (left < 4) left = 4;
          if (top < 4) top = 4;
          tip.style.left = left + 'px';
          tip.style.top = top + 'px';
        });
      });

      function closeHelpTooltips() {
        document.querySelectorAll('.help-tip:popover-open').forEach(function(tip) {
          tip.hidePopover();
        });
      }
```

(`closeHelpTooltips` is referenced by `setOpen` from Step 1; function declarations hoist within the handler scope, so definition order is fine.)

- [ ] **Step 4: Commit**

```bash
git add frontend/index.html
git commit -m "feat(frontend): position settings tooltips and close them with the panel"
```

---

### Task 4: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full tileserver test suite**

Run: `cargo test`
(working directory: `tileserver/`)

Expected: all tests PASS — in particular `real_translation_files_have_identical_key_sets` (78 keys × 7 languages) and `real_template_renders_all_languages_without_leftovers` (all `{{t:…}}` markers resolve).

- [ ] **Step 2: Run the frontend unit tests**

Run: `npm test`
(working directory: `frontend/`)

Expected: all tests PASS (map-styles tests; unaffected by this change, run as a regression guard).

- [ ] **Step 3: Manual browser checklist**

Start the tileserver as usual and open the app, then verify:

- [ ] Settings panel shows a small `?` next to each of the five setting labels
- [ ] Clicking a `?` opens its tooltip below the button, left-aligned with it
- [ ] Clicking elsewhere, clicking the `?` again, or pressing Esc closes the tooltip
- [ ] With a tooltip open, pressing Esc closes only the tooltip; a second Esc closes the panel
- [ ] Closing the panel via the gear button or ✕ while a tooltip is open also closes the tooltip
- [ ] Tooltip near the bottom edge of the viewport flips above its button; near the right edge it stays inside the viewport
- [ ] Tooltips are not clipped by the panel (they float above the map)
- [ ] Selects and checkboxes still work exactly as before (basemap/layer-swap/supersampling changes, tile-grid and denoise toggles)
- [ ] Mobile viewport (devtools responsive mode): tapping `?` toggles the tooltip; tapping outside dismisses it
- [ ] No console errors
- [ ] Switch language via the language switcher and spot-check that hint wording matches that language's option labels
