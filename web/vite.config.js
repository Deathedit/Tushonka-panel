import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// `npm run dev` serves the UI on :5173 and forwards API + WebSocket calls to a running panel.
export default defineConfig({
  plugins: [svelte()],
  build: { outDir: 'dist', emptyOutDir: true, chunkSizeWarningLimit: 1500 },
  server: {
    proxy: {
      '/api': { target: 'http://127.0.0.1:8080', ws: true, changeOrigin: false },
    },
  },
});
