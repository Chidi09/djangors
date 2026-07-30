import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';

export default defineConfig({
  output: 'static',
  site: 'https://djangors.vercel.app',
  integrations: [sitemap()],
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
