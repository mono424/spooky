// @ts-check
import { defineConfig } from 'astro/config';
import tailwind from '@astrojs/tailwind';
import react from '@astrojs/react';
import mdx from '@astrojs/mdx';

// https://astro.build/config
export default defineConfig({
  site: process.env.ASTRO_SITE || 'https://sp00ky.cloud',
  base: process.env.ASTRO_BASE_PATH || '/',
  prefetch: true,
  markdown: {
    shikiConfig: {
      themes: {
        light: 'github-light',
        dark: 'github-dark',
      },
      wrap: true,
    },
  },
  integrations: [
    tailwind({
      applyBaseStyles: false,
    }),
    react(),
    mdx(),
  ],
  vite: {
    ssr: {
      noExternal: ['holographic-sticker'],
    },
  },
});
