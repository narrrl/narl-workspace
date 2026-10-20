import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import type { Plugin } from 'vite'

/*
 * KaTeX lists each font as woff2, woff and ttf. Every browser the app runs in
 * takes woff2, and the other two would be another megabyte in the binary.
 */
const katexWoff2Only: Plugin = {
  name: 'katex-woff2-only',
  enforce: 'pre',
  transform(code, id) {
    if (!/katex(\.min)?\.css$/.test(id)) return
    return code.replace(/,\s*url\([^)]*\.(?:woff|ttf)\)\s*format\("(?:woff|truetype)"\)/g, '')
  },
}

export default defineConfig({
  plugins: [svelte(), katexWoff2Only],
  build: { target: 'es2022', chunkSizeWarningLimit: 900 },
  server: {
    port: 4243,
    proxy: {
      '/api': { target: 'http://127.0.0.1:4242', changeOrigin: false },
    },
  },
})
