import {defineConfig} from 'astro/config';
import sitemap from '@astrojs/sitemap';
import {deployment} from './scripts/deployment.mjs';

export default defineConfig({
  ...deployment(),
  output: 'static',
  trailingSlash: 'always',
  integrations: [sitemap()],
  markdown: {
    shikiConfig: {
      theme: 'github-dark-default',
      wrap: false,
      langAlias: {velin: 'text'},
    },
  },
});
