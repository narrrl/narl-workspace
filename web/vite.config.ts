import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import type { Plugin, Rollup } from 'vite'

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

/*
 * src/sw.ts, built beside the app as /sw.js — unhashed, since its address is
 * what the browser remembers. Two names are written into it:
 *
 *   __PRECACHE__  what index.html needs to paint: the entry, what it imports,
 *                 their CSS, and the latin faces of the two fonts
 *   __VERSION__   a hash of every file name in the build, so a build that
 *                 changes anything changes sw.js too, and the browser takes
 *                 the new worker and drops the old shell
 */
const serviceWorker: Plugin = {
  name: 'service-worker',
  apply: 'build',
  buildStart() {
    this.emitFile({ type: 'chunk', id: '/src/sw.ts', fileName: 'sw.js' })
  },
  generateBundle(_, bundle) {
    const chunks = Object.values(bundle).filter((file): file is Rollup.OutputChunk => file.type === 'chunk')
    const entry = chunks.find((chunk) => chunk.isEntry && chunk.facadeModuleId?.endsWith('index.html'))
    const sw = bundle['sw.js']
    if (!entry || sw?.type !== 'chunk') return this.error('service worker: no entry, or no sw.js')

    const shell = new Set<string>()
    const walk = (name: string) => {
      if (shell.has(name)) return
      shell.add(name)
      const chunk = bundle[name]
      if (chunk?.type !== 'chunk') return
      chunk.viteMetadata?.importedCss.forEach((css) => shell.add(css))
      chunk.imports.forEach(walk)
    }
    walk(entry.fileName)
    for (const name of Object.keys(bundle)) {
      if (/\/(jetbrains-mono|noto-sans)-latin(-ext)?-wght-normal-[^/]+\.woff2$/.test(name)) shell.add(name)
    }

    // FNV-1a: only a change has to show, so nothing stronger is needed.
    let hash = 0x811c9dc5
    for (const char of Object.keys(bundle).sort().join('\n')) hash = Math.imul(hash ^ char.charCodeAt(0), 0x01000193)
    const version = (hash >>> 0).toString(16)
    sw.code = sw.code
      .replaceAll('__VERSION__', JSON.stringify(version))
      .replaceAll('__PRECACHE__', JSON.stringify([...shell].map((name) => `/${name}`)))
  },
}

export default defineConfig({
  plugins: [svelte(), katexWoff2Only, serviceWorker],
  build: { target: 'es2022', chunkSizeWarningLimit: 900 },
  server: {
    port: 4243,
    proxy: {
      '/api': { target: 'http://127.0.0.1:4242', changeOrigin: false },
    },
  },
})
