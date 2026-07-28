// src/middleware.ts
// Dev-only mock backend: returns fake versions and 404 for tiles
// Skips entirely during build (import.meta.env.PROD)

import { defineMiddleware } from 'astro:middleware';

const FAKE_VERSIONS = Array.from({ length: 10 }, (_, i) => ({
  version: String(100 + i),
  date: new Date(2025, 7, 1 + i).toISOString().split('T')[0],
}));

export const onRequest = defineMiddleware(
  async function mockBackend(context, next) {
    if (import.meta.env.PROD) {
      return next();
    }

    if (context.url.pathname === '/api/versions') {
      return new Response(JSON.stringify(FAKE_VERSIONS), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (context.url.pathname.startsWith('/tiles/')) {
      return new Response('tile not found', { status: 404 });
    }

    return next();
  },
);
