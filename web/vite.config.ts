import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: {
      '/api': 'http://127.0.0.1:3000',
      '/healthz': 'http://127.0.0.1:3000',
      // A share link's data; the page itself (`/s/{token}`, `?t=` and all) stays with Vite. Vite
      // matches the URL with its query, so the token stops at `?` too.
      '^/s/[^/?#]+/': 'http://127.0.0.1:3000',
    },
  },
});
