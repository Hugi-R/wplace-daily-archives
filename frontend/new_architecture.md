# Wplace Daily Archives — Modernization Handoff

**Status:** Architecture decided, no implementation started.
**Scope:** Frontend rewrite of the single-file HTML map viewer, small backend additions, feature parity + agreed evolutions.

---

## 1. Context / why this rewrite

The current app is a single-file HTML page (inline CSS + inline `<script type="module">`) built on MapLibre GL JS 5.7.1, loaded as a classic UMD `<script>` global. It handles map rendering, a custom worker pool for tile decompression (WASM-backed), WASM-driven screenshot/video export, and hand-rolled UI chrome (version slider, coord form, context menu, canvas tile-grid overlay), all as top-level mutable globals with no module boundaries.

**Forcing constraint:** MapLibre GL JS 6.0.0 (current, released this year) ships **ESM-only** — the UMD bundles (`maplibre-gl.js`, `maplibre-gl-csp.js`) are no longer published. The existing `<script src=".../maplibre-gl.js">` global-script pattern cannot work with v6. A bundler/module pipeline is required regardless of any other decision below.

---

## 2. Decisions made

| Area | Decision |
|---|---|
| Map library | MapLibre GL JS 6.0.0 (ESM import) |
| Meta-framework | **Astro 6** |
| Interactive components | **Svelte** islands |
| Language | TypeScript throughout |
| Build tool | Vite (via Astro) |
| Worker RPC | **Comlink**, wrapping a thin custom pool |
| i18n | Astro's built-in i18n routing + **Paraglide (inlang)** for compile-time messages |
| Backend changes | Minimal — see §5 |
| Web workers / WASM | Unchanged in role, reorganized into typed modules |

### Why Astro + Svelte islands

- Hard constraint from stakeholder: **LCP p50 < 800ms** (current p50 800ms / p90 1700ms — no regression budget).
- Astro ships static HTML by default and only hydrates explicitly marked components ("islands"), each as its own small JS chunk loaded as an ES module — as opposed to a full-page hydration model (Next/SvelteKit-style), which would ship more JS on the critical path for no benefit here, since the map itself can never be server-rendered anyway.
- Astro is SSG (static HTML at build time) — good for indexing (crawlers get real HTML with no JS-execution dependency), which was the other stated constraint ("must play nice with... web indexing").
- Svelte chosen for islands over React: smaller runtime, compiles close to vanilla JS, keeps the per-island JS cost low — consistent with the LCP budget. React was considered and rejected on this basis; Astro supports it as a drop-in alternative later if needed since it's framework-agnostic per-island.
- Caveat carried forward: Astro's primary use case is content-heavy sites with a few interactive widgets; this app is closer to "one page, mostly interactive." Still a reasonable fit — fewer islands than typical, but each still benefits from independent lazy hydration, and static chrome (encart, labels, locale strings) still ships as zero-JS HTML.

### Why Comlink for the worker pool (not a bigger library, not status quo)

Current `TileWorkerPool` hand-builds `{type, taskId, buffers}` envelopes per call across four near-duplicate methods (`decompress`, `diff_decompress`, `downscale_4to1`, `diff_downscale_4to1`), has no `worker.onerror` handling (a crashed worker gets stuck `busy: true` forever), and — critically — **discards the `AbortSignal` MapLibre already provides** in `addProtocol`'s callback, meaning tiles are fully decompressed even after the map has panned/zoomed past them.

Comlink turns the worker's exposed surface into typed async methods (`decompress(layer, version, buffer)`, etc.), proxied over `postMessage`, while still supporting explicit `Comlink.transfer()` for zero-copy transferables — no perf regression versus today. Pooling logic (round-robin / least-busy over N workers) stays custom since Comlink doesn't provide it, but collapses to one small implementation instead of four duplicated call sites.

`workerpool` (npm) was considered as an alternative (less code, built-in pooling) and rejected — the pool logic needed here is simple enough that the code-savings gap is small, while Comlink's structural typing pays off every time the worker protocol changes (e.g. adding the `layer` param below).

---

## 3. New module boundaries

```
src/
  map/        MapLibre setup, style generation (version + layer aware),
              merged:// protocol registration, canvas tile-grid overlay
  workers/    Comlink-wrapped pool: typed RPC contract, abort wiring,
              crash recovery/respawn, dynamic pool sizing
  media/      screenshot/video generation, driven by an explicit config
              object (zoom, tile bounds, date range) instead of the
              current viewport-derived approach
  state/      version, layer, viewport, ui-visibility, locale —
              single source of truth (nanostores or equivalent),
              synced to URL params; shared across islands
  i18n/       Paraglide-generated message functions per locale
  islands/    Svelte components: map mount (client:only), version+layer
              control, coord form, context menu, stats panel,
              video-config form, encart
  pages/      Astro routes, one per locale (/en/, /es/, /pt/, /ja/)
```

### Island hydration strategy
- Map mount: `client:only="svelte"` (no server-render possible; MapLibre needs a real browser/canvas)
- Version/layer control, coord form, context menu: `client:idle`
- Video-config form: `client:visible` or `client:idle` (low priority, opened on demand)
- Everything static (encart copy, page shell, locale strings for non-interactive text): zero JS, plain Astro/HTML

### Worker pool changes, concretely
- Typed methods replace the `type`-string envelope; TypeScript catches protocol mismatches at compile time instead of failing silently at runtime.
- Pool size defaults to `min(navigator.hardwareConcurrency ?? 4, 6)` instead of a hardcoded 4, capped to bound per-worker WASM memory footprint.
- `worker.onerror` triggers: reject in-flight task, respawn replacement worker, so a crash doesn't permanently strand a pool slot.
- `AbortSignal` from `addProtocol`'s callback is threaded into the pool call: queued-but-undispatched tasks are dropped for free on abort; dispatched tasks forward the abort into the worker for cooperative cancellation, **pending confirmation the WASM/Rust side actually supports cancelling mid-decompress** (open item, see §6).

---

## 4. Evolutions in scope

### Hide most of the UI
Low-risk. A single `uiVisible` boolean in the shared state store, gating island render/CSS, bound to a hotkey.

### Alternate data layer
Confirmed meaning: **swap the current overlay for a different one**, with the heavy lifting staying in the WASM worker (an argument specifies which layer to render). Threaded end-to-end as a new `layer` parameter:
- `pool.decompress(layer, version, buffer)` (and the other three worker methods)
- Protocol scheme: `merged://tiles/{layer}/{version}/{z}/{x}/{y}.png`
- Cache API keys gain the layer segment (`tile-${layer}-${version}-${z}-${x}-${y}`) so switching layers doesn't evict other layers' cached tiles
- `getMapStyle()` takes `layer` alongside `version`, same `setStyle`-and-reregister pattern already used for version switching
- New island (dropdown/toggle) drives it, same shape as the version slider

### Improved video export UI
Replaces today's implicit derivation (viewport bounds + hardcoded `to = 0xFFFFFFFF`) with an explicit form: zoom level, tile range, start/end date. Lives in `media/`, produces the same `(x1, y1, x2, y2, from, to)` tuple the WASM call already expects — no WASM-side change needed.

### Multi-language support
English, Spanish, Portuguese, Japanese. Astro's built-in i18n routing (`/en/`, `/es/`, `/pt/`, `/ja/`, proper `hreflang`) + Paraglide for message strings — chosen over a runtime i18n library (e.g. i18next) specifically to protect the LCP budget, since Paraglide compiles to per-page function calls rather than runtime dictionary lookups.

---

## 5. Backend changes (small, as scoped)

- New `GET /api/versions` JSON endpoint, replacing the current `//$$VERSION_OPTIONS$$` server-side template injection.
- Tile URL scheme gains a layer segment: `/tiles/{layer}/{week}/{z}/{x}/{y}.zst` — additive change to existing static file layout, no protocol redesign.
- Everything else (tile serving, WASM module, `log_user_message` bridge) unchanged.

---

## 6. Resolved items (previously open)

1. **Deployment model — confirmed.** Single Docker container serving both static and dynamic assets, behind Cloudflare CDN — chosen deliberately to keep deployment simple at current scale. Same origin for everything, so no CORS handling needed between `/api/versions`, `/tiles/*`, and the Astro-built frontend. The proposed architecture (§1–§5) requires no changes for this — Astro's static output and the Rust backend's dynamic routes both live behind the same container/edge, exactly as assumed.
2. **WASM cooperative cancellation — confirmed feasible, now in scope.** The Rust/WASM decompress routines will support bailing out mid-operation on abort, not just discarding an already-completed result. This means the `AbortSignal`-forwarding described in §3 delivers real CPU savings on rapid pan/zoom, not just queue-level savings.
   - **Added benefit beyond tile decompression:** the same cancellation path applies to video rendering — a user adjusting the video-config form (zoom/tile range/date range) mid-generation can have the in-flight render actually stop, rather than running to completion in the background before the new request starts. Worth reflecting in the `media/` module design: the video-config form's submit handler should hold onto an `AbortController` and cancel-and-resubmit on parameter change, rather than only supporting a single fire-and-forget generation per session.
3. **Version list update frequency — confirmed: once per day, but hot-reloadable (no rebuild+redeploy) is a hard requirement.** This rules out build-time inlining — revised decision: `/api/versions` is fetched **client-side at runtime**, from the `/api/versions` route already implemented by the Docker container's dynamic side (§5). New versions become visible the moment the backend serves them, no deploy involved.
   - **LCP impact is minimal by construction, not by luck:** this fetch happens inside the version/layer control island, which hydrates on `client:idle` — it is not on the path to the map's own paint, and the payload is a small JSON array, not render-blocking markup. The map island mounts and paints independently of whether this fetch has resolved yet.
   - **Cache at the edge, not in the browser, to control freshness:** since it's already behind Cloudflare (§6.1), set a short `Cache-Control`/CDN cache TTL on `/api/versions` (e.g. 5–15 minutes) rather than caching indefinitely — keeps repeat visits fast while still surfacing new versions same-day without a deploy. Exact TTL is a product call (freshness vs. edge cache hit rate), not an architectural one.
