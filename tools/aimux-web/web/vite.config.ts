import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'

// Dev-only proxy: the console is normally served by the Rust backend
// (which serves `dist/` and `/api` from the same origin). For `npm run dev`,
// point the API at a locally running backend, e.g.:
//   cargo run -p aimux-web -- --port 8787 --no-open
const API_TARGET = process.env.AIMUX_WEB_API ?? 'http://127.0.0.1:8787'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    proxy: {
      '/api': { target: API_TARGET, changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: false,
  },
})
