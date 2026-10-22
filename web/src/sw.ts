/*
 * The service worker: what makes the workspace an app on a phone.
 *
 * - The app shell — index.html, the logo, and what they need to paint — is
 *   kept, so the app opens with no network and says the server cannot be
 *   reached, rather than the browser's error page.
 * - Every hashed file under /assets is kept the first time it is fetched;
 *   they never change, so a lazy chunk is fetched once.
 * - The share sheet posts to /share. The text is kept here, in the cache
 *   `share`, and the page takes it onto the scratchpad once someone is logged
 *   in (`Scratchpad.takeShared`). The post never reaches the server.
 *
 * Nothing from /api is touched or kept: the caches hold the public shell only,
 * so logging out leaves nothing private behind.
 *
 * The worker is built by `serviceWorker` in vite.config.ts, which writes in
 * the two names below. The DOM types have no worker events, so the few used
 * here are spelled out; a module, so they stay its own.
 */

export {}

declare const __VERSION__: string
declare const __PRECACHE__: string[]

interface ExtendableEvent extends Event {
  waitUntil(promise: Promise<unknown>): void
}
interface FetchEvent extends ExtendableEvent {
  readonly request: Request
  respondWith(response: Response | Promise<Response>): void
}
interface Scope {
  skipWaiting(): Promise<void>
  clients: { claim(): Promise<void> }
  addEventListener(type: 'install' | 'activate', listener: (event: ExtendableEvent) => void): void
  addEventListener(type: 'fetch', listener: (event: FetchEvent) => void): void
}

const worker = self as unknown as Scope
const SHELL = `shell-${__VERSION__}`

worker.addEventListener('install', (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(SHELL)
      await cache.addAll(['/index.html', '/favicon.svg', ...__PRECACHE__])
      await worker.skipWaiting()
    })(),
  )
})

worker.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      for (const name of await caches.keys()) {
        if (name.startsWith('shell-') && name !== SHELL) await caches.delete(name)
      }
      await worker.clients.claim()
    })(),
  )
})

worker.addEventListener('fetch', (event) => {
  const request = event.request
  const url = new URL(request.url)
  if (url.origin !== location.origin) return
  if (request.method === 'POST' && url.pathname === '/share') return event.respondWith(share(request))
  if (request.method !== 'GET') return
  if (url.pathname.startsWith('/assets/')) return event.respondWith(asset(request))
  if (request.mode === 'navigate') return event.respondWith(fresh(request, '/index.html'))
  if (url.pathname === '/favicon.svg') return event.respondWith(fresh(request, '/favicon.svg'))
})

/** A hashed file: from the cache, or fetched once and kept. */
async function asset(request: Request): Promise<Response> {
  const cache = await caches.open(SHELL)
  const kept = await cache.match(request)
  if (kept) return kept
  const response = await fetch(request)
  if (response.ok) await cache.put(request, response.clone())
  return response
}

/*
 * A page — /, /n/…, /s/… — is index.html whatever the path, and the logo is
 * the logo. The server's is newer, so it is asked first; the kept one answers
 * when it cannot be reached, or a proxy in front of it says it is down.
 */
async function fresh(request: Request, kept: string): Promise<Response> {
  try {
    const response = await fetch(request)
    if (response.status < 502) return response
    return (await caches.match(kept)) ?? response
  } catch (error) {
    const copy = await caches.match(kept)
    if (copy) return copy
    throw error
  }
}

/*
 * The share sheet: a title, some text, a link — any of them may be missing,
 * and Android often puts the link in the text as well. They become one note,
 * the title on its first line.
 */
async function share(request: Request): Promise<Response> {
  const form = await request.formData()
  const field = (name: string) => String(form.get(name) ?? '').trim()
  const [title, text, link] = [field('title'), field('text'), field('url')]
  const lines = [title && title !== text ? title : '', text, link && !text.includes(link) ? link : ''].filter(Boolean)
  if (lines.length) {
    const cache = await caches.open('share')
    await cache.put(`/share/${Date.now()}`, new Response(lines.join('\n\n')))
  }
  return Response.redirect('/', 303)
}
