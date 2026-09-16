import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  root: fileURLToPath(new URL('.', import.meta.url)),
  plugins: [react(), tailwindcss()],
  server: {
    host: '0.0.0.0', port: 5173, strictPort: true,
    fs: { allow: [fileURLToPath(new URL('../..', import.meta.url))] },
    proxy: Object.fromEntries(['/dev/', '/health/', '/auth/', '/session', '/v1/'].map(path =>
      [path, { target: process.env.MASONWING_API_URL ?? 'http://127.0.0.1:39851', changeOrigin: false }])),
  },
  build: { outDir: 'dist', sourcemap: true },
});
