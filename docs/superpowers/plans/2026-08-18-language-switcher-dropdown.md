# Language Switcher Dropdown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the three-link inline language switcher with a two-glyph icon button (A + あ) that opens a native `<details>` dropdown.

**Architecture:** The `{{LANG_SWITCHER}}` replacement in `render_index` (tileserver/src/main.rs) emits a `<details class="lang-switcher">` whose `<summary>` holds the inline SVG icon and whose `<ul>` holds the three `<a href="/{lang}/">` links (active one marked with `lang-active`, checkmark via CSS). Native `<details>` provides no-JS toggle and keyboard access; a small inline JS handler adds outside-click and Escape close. CSS in `frontend/index.html` replaces the current `.lang-switcher` rules.

**Tech Stack:** Rust/axum server (existing), plain HTML/CSS/JS in `frontend/index.html`.

## Global Constraints

- No new translation dict keys; `tileserver/i18n/{en,ja,es}.json` stays at 55 keys (identical key sets).
- The three links remain real `<a href="/{lang}/">` elements (no-JS + SEO/hreflang untouched).
- No aria-label/title text on the trigger — the icon is self-explanatory.
- hreflang, canonical, `{{LANG_PATH}}`, and `{{t:key}}` generation are untouched.
- The rendered page must still contain no leftover template markers.

---

### Task 1: Dropdown language switcher

**Files:**
- Modify: `frontend/index.html` — `.lang-switcher` CSS (lines 225-234); JS handler after the encart `DOMContentLoaded` block (line 543)
- Modify: `tileserver/src/main.rs` — `render_index` switcher generation (lines 429-439); `lang_path_serves_localized_page` test (lines ~1375-1381)
- Test: `tileserver/src/main.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Lang::ALL`, `Lang::path()`, `Lang::label()` (from `tileserver/src/i18n.rs`), `lang` param of `render_index`.
- Produces: `render_index` emits `<details class="lang-switcher" id="lang-switcher">` markup filled by the existing `{{LANG_SWITCHER}}` replacement.

- [ ] **Step 1: Write the failing test**

In `tileserver/src/main.rs`, `lang_path_serves_localized_page` currently asserts the old link markup:

```rust
assert!(
    body.contains("href=\"/ja/\" class=\"lang-active\">日本語</a>"),
    "{body}"
);
```

Replace that assertion with the new markup and add dropdown-structure guards:

```rust
assert!(
    body.contains("<a role=\"menuitem\" href=\"/ja/\" class=\"lang-option lang-active\">日本語</a>"),
    "{body}"
);
assert!(
    body.contains("<details class=\"lang-switcher\" id=\"lang-switcher\">"),
    "{body}"
);
assert!(body.contains("aria-haspopup=\"menu\""), "{body}");
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd tileserver && cargo test lang_path_serves_localized_page -- --nocapture`
Expected: FAIL (the rendered page still has `<nav class="lang-switcher">` links, not the `<details>` dropdown).

- [ ] **Step 3: Update the switcher markup in `render_index`**

In `tileserver/src/main.rs`, replace the current switcher block (lines 429-439):

```rust
    let mut switcher = String::from("<nav class=\"lang-switcher\">\n");
    for l in Lang::ALL {
        let cls = if l == lang { " class=\"lang-active\"" } else { "" };
        switcher.push_str(&format!(
            "  <a href=\"/{}/\"{cls}>{}</a>\n",
            l.path(),
            l.label()
        ));
    }
    switcher.push_str("</nav>");
    content = content.replace("{{LANG_SWITCHER}}", &switcher);
```

with:

```rust
    let mut switcher = String::from(
        "<details class=\"lang-switcher\" id=\"lang-switcher\">\n\
         <summary aria-haspopup=\"menu\">\n\
         <svg class=\"lang-icon\" viewBox=\"0 0 24 24\" fill=\"none\" aria-hidden=\"true\">\n\
         <text x=\"3\" y=\"17\" font-family=\"sans-serif\" font-size=\"12\" font-weight=\"600\" fill=\"currentColor\">A</text>\n\
         <text x=\"12\" y=\"17\" font-family=\"sans-serif\" font-size=\"12\" font-weight=\"600\" fill=\"currentColor\">あ</text>\n\
         </svg>\n\
         </summary>\n\
         <ul role=\"menu\" class=\"lang-menu\">\n",
    );
    for l in Lang::ALL {
        let cls = if l == lang { " lang-active" } else { "" };
        switcher.push_str(&format!(
            "  <li><a role=\"menuitem\" href=\"/{}/\" class=\"lang-option{cls}\">{}</a></li>\n",
            l.path(),
            l.label()
        ));
    }
    switcher.push_str("</ul>\n</details>");
    content = content.replace("{{LANG_SWITCHER}}", &switcher);
```

- [ ] **Step 4: Replace the switcher CSS**

In `frontend/index.html`, replace the current `.lang-switcher` rules (lines 225-234):

```css
    .lang-switcher {
      margin-left: auto;
      display: flex;
      gap: 10px;
      font-size: 15px;
      white-space: nowrap;
    }
    .lang-switcher a { text-decoration: none; }
    .lang-switcher a:hover { text-decoration: underline; }
    .lang-switcher a.lang-active { font-weight: bold; text-decoration: underline; }
```

with:

```css
    .lang-switcher { margin-left: auto; position: relative; }
    .lang-switcher summary {
      list-style: none;
      cursor: pointer;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 34px;
      height: 34px;
      border: 1px solid var(--border-color);
      border-radius: 6px;
      background: var(--panel-bg-weak);
      color: var(--button-text);
    }
    .lang-switcher summary::-webkit-details-marker { display: none; }
    .lang-switcher .lang-icon { width: 22px; height: 22px; }
    .lang-menu {
      position: absolute;
      right: 0;
      top: calc(100% + 6px);
      margin: 0;
      padding: 6px 0;
      list-style: none;
      background: var(--panel-bg);
      border: 1px solid var(--border-color);
      border-radius: 6px;
      box-shadow: var(--shadow-lg);
      min-width: 150px;
      font-size: 15px;
      z-index: 110;
    }
    .lang-menu a {
      display: flex;
      justify-content: space-between;
      gap: 12px;
      padding: 8px 16px;
      text-decoration: none;
      color: var(--button-text);
    }
    .lang-menu a:hover { background: var(--panel-bg-weak); }
    .lang-menu a.lang-active { font-weight: 600; }
    .lang-menu a.lang-active::after { content: "✓"; font-weight: 600; }
```

- [ ] **Step 5: Add the outside-click / Escape JS handler**

In `frontend/index.html`, immediately after the existing encart `DOMContentLoaded` block (ends line 543), add:

```js
    // --- Language switcher dropdown ---
    document.addEventListener('DOMContentLoaded', function() {
      var langSwitcher = document.getElementById('lang-switcher');
      var langSummary = langSwitcher ? langSwitcher.querySelector('summary') : null;
      if (langSwitcher) {
        document.addEventListener('click', function(e) {
          if (!langSwitcher.contains(e.target)) langSwitcher.open = false;
        });
        document.addEventListener('keydown', function(e) {
          if (e.key === 'Escape' && langSwitcher.open) {
            langSwitcher.open = false;
            if (langSummary) langSummary.focus();
          }
        });
      }
    });
```

- [ ] **Step 6: Run the tests**

Run: `cd tileserver && cargo test`
Expected: all pass (25/25) — including the updated `lang_path_serves_localized_page`, whose rendered-page assertions (`<details class="lang-switcher" ...>`, `aria-haspopup="menu"`, active-link markup) now pass.

Run: `cd tileserver && cargo clippy --all-targets`
Expected: no new warnings (only the pre-existing non-root-package `profiles` notice).

- [ ] **Step 7: Commit**

```bash
git add tileserver/src/main.rs frontend/index.html
git commit -m "feat(i18n): replace language list with icon dropdown switcher"
```

- [ ] **Step 8: Manual smoke verification**

Render the page for `/en/`, `/ja/`, `/es/` (e.g. via the running dev server after a rebuild) and confirm:
- The top bar shows the compact A+あ icon button (no text row).
- Clicking/tapping toggles the dropdown; the active language link shows the ✓ checkmark.
- Clicking outside or pressing Escape closes the dropdown, and the summary regains focus on Escape.
- Selecting a language navigates to `/ja/` (etc.) as before.
- With JS disabled, clicking the `summary` still opens the native `<details>` menu and links work.