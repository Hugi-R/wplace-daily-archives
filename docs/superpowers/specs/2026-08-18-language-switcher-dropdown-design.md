# Language switcher dropdown design

## Goal

Replace the current inline language switcher (a row of three text links in the top bar) with a compact two-glyph icon button that opens a dropdown menu. Primary motivation: better space usage on mobile, where the top bar (`#version-select`) is tight. Secondary: a more familiar, universally understood trigger.

## Current behavior

- The `{{LANG_SWITCHER}}` placeholder in `frontend/index.html` (top bar, line 323) is replaced server-side by `render_index` (`tileserver/src/main.rs:429-438`) with:

  ```html
  <nav class="lang-switcher">
    <a href="/en/" [class="lang-active"]>English</a>
    <a href="/ja/" [class="lang-active"]>日本語</a>
    <a href="/es/" [class="lang-active"]>Español</a>
  </nav>
  ```

- Styled by the `.lang-switcher` rules in `frontend/index.html:225-234` (right-aligned, `margin-left: auto`, 15px, active = bold + underline).

## Scope decisions

- Trigger is an inline SVG showing two glyphs — Latin "A" and Japanese あ — in `currentColor`.
- No text label on the trigger (no aria-label / title / dict key). A "change language" title assumes the reader understands the current language, which defeats the purpose; the icon is self-explanatory.
- Dropdown items show native names (English / 日本語 / Español) with a checkmark on the active language.
- Standard open/close behavior: toggle on click/tap, close on selection, outside click, or Escape.
- Implementation via native `<details>/<summary>` plus a small inline JS handler for outside-click and Escape. No-JS users still get a working switcher.
- The three items remain real `<a href="/{lang}/">` links; hreflang/canonical generation is untouched. No new translation keys.

## Design

### Section 1 — Markup (server-side, `render_index`)

The `{{LANG_SWITCHER}}` replacement becomes:

```html
<details class="lang-switcher" id="lang-switcher">
  <summary aria-haspopup="menu">
    <svg class="lang-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <text x="3" y="17" font-family="sans-serif" font-size="12" font-weight="600" fill="currentColor">A</text>
      <text x="12" y="17" font-family="sans-serif" font-size="12" font-weight="600" fill="currentColor">あ</text>
    </svg>
  </summary>
  <ul role="menu" class="lang-menu">
    <li><a role="menuitem" href="/{lang}/" class="lang-option [lang-active]">{native label}</a></li>
    ...
  </ul>
</details>
```

- `lang-active` is emitted on the current language's link; it renders a checkmark via CSS.
- The markup contains no `{{t:` markers, so the existing `{{t:key}}` replacement loop and the leftover-marker guard are unaffected.
- The icon is an inline SVG with two `<text>` glyphs ("A", "あ") using `currentColor`, inheriting the header text color.

### Section 2 — CSS and behavior (`frontend/index.html`)

Replace the current `.lang-switcher` rules (lines 225-234):

```css
.lang-switcher { margin-left: auto; position: relative; }
.lang-switcher summary {
  list-style: none;                 /* hide disclosure triangle */
  cursor: pointer;
  display: inline-flex; align-items: center; justify-content: center;
  width: 34px; height: 34px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--panel-bg-weak);
  color: var(--button-text);
}
.lang-switcher summary::-webkit-details-marker { display: none; }
.lang-switcher .lang-icon { width: 22px; height: 22px; }
.lang-menu {
  position: absolute; right: 0; top: calc(100% + 6px);
  margin: 0; padding: 6px 0; list-style: none;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  box-shadow: var(--shadow-lg);
  min-width: 150px; font-size: 15px; z-index: 110;
}
.lang-menu a {
  display: flex; justify-content: space-between; gap: 12px;
  padding: 8px 16px; text-decoration: none; color: var(--button-text);
}
.lang-menu a:hover { background: var(--panel-bg-weak); }
.lang-menu a.lang-active { font-weight: 600; }
.lang-menu a.lang-active::after { content: "✓"; font-weight: 600; }
```

Behavior — `<details>` provides native click/Enter/Space toggle. The only JS added (existing inline `<script>`):

- Close on outside click: a `document` click listener removes the `open` attribute when the click is outside `#lang-switcher`.
- Close on Escape: a `keydown` listener closes and refocuses the summary.

### Section 3 — Data flow and testing

- No new dict keys; `tileserver/i18n/{en,ja,es}.json` unchanged (still 55 keys).
- `hreflang`, canonical, and `{{LANG_PATH}}` generation untouched.
- Tests (`tileserver/src/main.rs`):
  - Update `lang_path_serves_localized_page`: the active-link assertion changes from `href="/ja/" class="lang-active">日本語</a>` to the new markup (`<a role="menuitem" href="/ja/" class="lang-option lang-active">日本語</a>`), and add a guard that the rendered page contains the dropdown (`<details class="lang-switcher"` and `aria-haspopup="menu"`).
  - All other tests (redirects, real-template render, key parity) unaffected.

## Verification

- `cd tileserver && cargo test` — all tests pass (25/25 after update).
- `cd tileserver && cargo clippy --all-targets` — no new warnings.
- Manual smoke render of `/en/`, `/ja/`, `/es/`: icon shows, dropdown opens/closes, active language checked, links navigate.
