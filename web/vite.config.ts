import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

const backend = process.env.REWINDTV_BACKEND ?? 'http://localhost:8420';

export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: { '/api': backend, '/stream': backend, '/iptv': backend },
  },
  build: { target: 'es2020', chunkSizeWarningLimit: 900 },
});
