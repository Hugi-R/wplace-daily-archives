import { defineConfig } from 'astro/config';
import svelte from '@astrojs/svelte';
import paraglide from '@inlang/paraglide-astro';

export default defineConfig({
  integrations: [
    svelte(),
    paraglide({
      project: './project.inlang',
      outdir: './src/paraglide',
    }),
  ],
  vite: {
    worker: {
      format: 'es',
    },
  },
  i18n: {
    defaultLocale: 'en',
    locales: ['en', 'es', 'pt', 'ja'],
    routing: {
      prefixDefaultLocale: true,
      redirectToDefaultLocale: false,
    },
  },
});
