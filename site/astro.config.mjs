import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import { readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

function documentationPages(directory, prefix = '') {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return documentationPages(path, `${prefix}/${entry.name}`);
    if (!entry.name.endsWith('.html') || entry.name === '404.html' || entry.name === 'print.html') return [];
    return [`https://djangors.vercel.app/docs${prefix}/${entry.name}`];
  });
}

const bookDirectory = join('..', 'docs', 'book');
const docsDirectory = statSync(bookDirectory, { throwIfNoEntry: false }) ? bookDirectory : join('public', 'docs');
const documentationUrls = statSync(docsDirectory, { throwIfNoEntry: false })
  ? documentationPages(docsDirectory)
  : ['https://djangors.vercel.app/docs/'];

export default defineConfig({
  output: 'static',
  site: 'https://djangors.vercel.app',
  integrations: [sitemap({ customPages: documentationUrls })],
  vite: {
    build: {
      // Without an explicit target the CSS minifier rewrites
      // `@media (max-width: 480px)` into Media Queries Level 4 range syntax
      // (`@media (width <= 480px)`), which iOS Safari only understands from
      // 16.4. On an older iPhone every breakpoint would then fail to match and
      // the desktop layout would be served — precisely the case the responsive
      // styles exist for. Pinning the target keeps the classic syntax.
      cssTarget: ['safari14', 'chrome87', 'firefox78', 'edge88'],
    },
  },
});
