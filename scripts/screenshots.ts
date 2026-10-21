/*
 * The screenshots in docs/images, taken again: `make screenshots`.
 *
 * Starts the debug binary on a data directory of its own, with a password made
 * up for the run, fills it with a workspace that has something in every
 * corner, and drives a headless Chromium over the DevTools protocol through
 * the views the docs talk about. Nothing outside the temporary directory is
 * read or written except docs/images. Needs `bun run build` in web/ and
 * `cargo build` first, which the make target does.
 */
import { spawn, spawnSync, which, type Subprocess } from 'bun'
import { mkdtempSync, rmSync, writeFileSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const root = resolve(import.meta.dir, '..')
const out = join(root, 'docs/images')
const port = 4299
const base = `http://127.0.0.1:${port}`
const user = 'demo'
const password = crypto.randomUUID()
const dir = mkdtempSync(join(tmpdir(), 'workspace-shots-'))
const chromium = process.env.CHROMIUM ?? 'chromium'

const children: Subprocess[] = []
function cleanup() {
  for (const child of children) child.kill()
  rmSync(dir, { recursive: true, force: true })
}
// A crash still stops the server and the browser; the directory may outlive
// them then, since Chromium writes its profile until it has exited.
process.on('exit', cleanup)
async function finish() {
  for (const child of children) child.kill()
  await Promise.all(children.map((child) => child.exited))
  rmSync(dir, { recursive: true, force: true })
  process.exit(0)
}

const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms))

async function waitFor(url: string) {
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(url)).ok) return
    } catch {
      /* not up yet */
    }
    await sleep(200)
  }
  throw new Error(`${url} never answered`)
}

// --- the server --------------------------------------------------------------

children.push(
  spawn([join(root, 'target/debug/narl-workspace')], {
    cwd: root,
    env: {
      ...process.env,
      WORKSPACE_DATA_DIR: join(dir, 'data'),
      WORKSPACE_BIND: `127.0.0.1:${port}`,
      WORKSPACE_USER: user,
      WORKSPACE_PASSWORD: password,
      WORKSPACE_SECRET: crypto.randomUUID() + crypto.randomUUID(),
      WORKSPACE_TIMEZONE: 'Europe/Berlin',
      RUST_LOG: 'warn',
    },
    stdout: 'ignore',
    stderr: 'inherit',
  }),
)
await waitFor(`${base}/api/health`)

const login = await fetch(`${base}/api/login`, {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ username: user, password }),
})
const cookie = (login.headers.get('set-cookie') ?? '').split(';')[0]
if (!login.ok || !cookie) throw new Error(`login: ${login.status}`)
const [cookieName, cookieValue] = cookie.split('=')

async function api<T = any>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(`${base}/api${path}`, {
    method,
    headers: { cookie, 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${await response.text()}`)
  return response.json() as Promise<T>
}

async function upload(parent: number, name: string, bytes: string | Uint8Array, type: string) {
  const form = new FormData()
  form.append('file', new File([bytes], name, { type }))
  const response = await fetch(`${base}/api/nodes/${parent}/files`, { method: 'POST', headers: { cookie }, body: form })
  if (!response.ok) throw new Error(`upload ${name}: ${response.status}`)
}

// --- the workspace -------------------------------------------------------------

const day = 24 * 3600
const midnight = new Date()
midnight.setHours(0, 0, 0, 0)
const today = Math.floor(midnight.getTime() / 1000)
const at = (days: number, hour: number) => today + days * day + hour * 3600

const doc = (parent_id: number, name: string, body: string, created_at?: number) =>
  api('POST', '/nodes', { parent_id, name, body, created_at })
const folder = (parent_id: number, name: string) => api('POST', '/nodes', { parent_id, name, kind: 'folder' })

// A new database starts with diary, work and uni.
const spaces = await api<{ id: number; slug: string }[]>('GET', '/spaces')
const space = async (slug: string) =>
  spaces.find((space) => space.slug === slug)?.id ?? (await api('POST', '/spaces', { name: slug })).id
const diary = await space('diary')
const work = await space('work')
const uni = await space('uni')
const home = await space('home')

// work
const projects = await folder(work, 'projects')
const pipeline = await doc(
  projects.id,
  'Ingest pipeline',
  `Moving the nightly import off cron and onto a queue, so a slow source no longer holds up the rest. See [[Runbook]] for how it is operated today.

## Why

- the cron job runs **every source in series**; one slow API delays all of them
- a failed run is retried by hand, the next morning
- nobody can tell from the outside how far a run has come

## Shape

| stage | owner | state |
| --- | --- | --- |
| fetch | worker pool | done |
| parse | worker pool | in review |
| store | one writer | open |

\`\`\`rust
async fn drain(queue: &Queue, store: &Store) -> anyhow::Result<()> {
    while let Some(job) = queue.next().await? {
        let rows = parse(&job.payload)?;
        store.write(job.source, rows).await?;
        queue.ack(job.id).await?;
    }
    Ok(())
}
\`\`\`

## Throughput

With $n$ sources of mean latency $\\bar t$, the serial run takes about $n \\bar t$; with $w$ workers it is closer to

$$
T \\approx \\frac{n \\bar t}{w} + t_{\\max}
$$

## Open questions

1. Does the writer need a lock per source, or is one queue enough?
2. Where does the dead-letter queue live?
`,
)
await doc(projects.id, 'Search rewrite', `A sketch of the new search API.\n\nDepends on the [[Ingest pipeline]] landing first.\n`)
const runbook = await doc(
  work,
  'Runbook',
  `How the nightly import is operated until the [[Ingest pipeline]] replaces it.

## When the import fails

1. read the log of the run: \`journalctl -u import --since today\`
2. re-run the one source that failed: \`import --source <name>\`
3. write down what happened in the incident notes
`,
)
// Two saves over the first text, so the runbook has a history to show.
await api('PUT', `/nodes/${runbook.id}`, {
  name: 'Runbook',
  body: `How the nightly import is operated until the [[Ingest pipeline]] replaces it.

## When the import fails

1. read the log of the run: \`journalctl -u import --since today\`
2. re-run the one source that failed: \`import --source <name> --resume\`
3. if it fails twice, page the owner of that source
4. write down what happened in the incident notes
`,
})
const data = await folder(work, 'data')
await upload(
  data.id,
  'throughput.csv',
  'day,sources,rows,duration_s,failures\n' +
    Array.from({ length: 14 }, (_, i) => `2026-10-${String(i + 1).padStart(2, '0')},${38 + (i % 3)},${120000 + i * 3150},${2400 - i * 61},${i % 5 === 0 ? 1 : 0}`).join('\n') +
    '\n',
  'text/csv',
)
await upload(
  data.id,
  'import-2026-10-14.log',
  `2026-10-14T02:00:01Z INFO  import: 41 sources queued
2026-10-14T02:00:03Z INFO  fetch: weather-api 200 in 812ms
2026-10-14T02:03:47Z WARN  fetch: tariffs-api slow, 9.4s
2026-10-14T02:04:12Z ERROR parse: tariffs-api: unexpected field "valid_from"
2026-10-14T02:41:55Z INFO  import: 40 of 41 sources stored
`,
  'text/plain',
)
const certs = await folder(work, 'certificates')
await upload(certs.id, 'gateway.pem', readFileSync(join(root, 'src/routes/testdata/cert.pem')), 'application/x-pem-file')
const templates = await folder(work, 'templates')
await doc(templates.id, 'meeting', `# {{title}}\n\n{{weekday}}, {{date}} at {{time}}\n\n## Agenda\n\n- \n\n## Decisions\n\n- \n`)

const lists = (await api('GET', `/spaces/${work}/board`)).lists as { id: number; name: string }[]
const [backlog, doing, done] = lists
await api('POST', `/lists/${backlog.id}/cards`, { title: 'Renew the gateway certificate', due_at: at(-1, 12) })
await api('POST', `/lists/${backlog.id}/cards`, { title: 'Write the dead-letter queue proposal', due_at: at(3, 12) })
await api('POST', `/lists/${backlog.id}/cards`, { title: 'Ask ops about a staging queue' })
const weekly = await api('POST', `/lists/${backlog.id}/cards`, { title: 'Weekly import report', due_at: at(0, 17) })
await api('PUT', `/cards/${weekly.id}`, { repeat: 'weekly' })
await api('POST', `/lists/${doing.id}/cards`, { title: 'Parse stage review', node_id: pipeline.id, due_at: at(1, 12) })
await api('POST', `/lists/${doing.id}/cards`, { title: 'Benchmark the worker pool' })
const finished = await api('POST', `/lists/${backlog.id}/cards`, { title: 'Move fetch onto the queue' })
await api('POST', `/cards/${finished.id}/move`, { list_id: done.id })

// uni
const algebra = await folder(uni, 'linear algebra')
await doc(
  algebra.id,
  'Eigenvalues',
  `For a square matrix $A$, a scalar $\\lambda$ is an **eigenvalue** when there is a vector $v \\neq 0$ with $Av = \\lambda v$. They are the roots of

$$
\\det(A - \\lambda I) = 0
$$

## Example

$$
A = \\begin{pmatrix} 2 & 1 \\\\ 1 & 2 \\end{pmatrix}
\\quad\\Rightarrow\\quad
\\lambda_1 = 3,\\; \\lambda_2 = 1
$$

The trace is the sum of the eigenvalues, $\\operatorname{tr} A = 4$, and the determinant their product, $\\det A = 3$.
`,
)
await doc(algebra.id, 'Exercise sheet 4', `- [x] 4.1 characteristic polynomial\n- [x] 4.2 diagonalise\n- [ ] 4.3 Jordan form\n`)
const thesis = await folder(uni, 'thesis')
await doc(thesis.id, 'Outline', `## Chapters\n\n1. Introduction\n2. Related work\n3. Method\n4. Evaluation\n`)
const uniLists = (await api('GET', `/spaces/${uni}/board`)).lists as { id: number }[]
await api('POST', `/lists/${uniLists[0].id}/cards`, { title: 'Hand in exercise sheet 4', due_at: at(2, 10) })

// home
await doc(home, 'Groceries', `- oat milk\n- coffee\n- lemons\n`)
await api('PUT', `/nodes/${home}`, { name: 'home', has_board: false })

// diary
const entries: [number, string, string][] = [
  [-1, 'Queue day', 'Fetch runs on the queue now. The tariffs source still fails on a new field.'],
  [-2, 'Long run', 'Ten kilometres along the river, slower than last week and better for it.'],
  [-3, 'Sheet 4', 'Diagonalised everything in sight. Jordan form tomorrow.'],
  [-5, 'Rain', 'Stayed in and read.'],
  [-8, 'Planning', 'Wrote down why the nightly import should move to a queue.'],
  [-12, 'Market', 'Lemons, coffee, a plant that will not survive the winter.'],
  [-365, 'A year ago', 'First entry in the new workspace.'],
]
for (const [offset, name, body] of entries) await doc(diary, name, body + '\n', at(offset, 20))

// --- the browser ----------------------------------------------------------------

const profile = join(dir, 'chromium')
children.push(
  spawn(
    [chromium, '--headless=new', '--remote-debugging-port=9333', `--user-data-dir=${profile}`, '--hide-scrollbars', '--no-first-run', 'about:blank'],
    { stdout: 'ignore', stderr: 'ignore' },
  ),
)
await waitFor('http://127.0.0.1:9333/json/version')
const targets = (await (await fetch('http://127.0.0.1:9333/json/list')).json()) as { type: string; webSocketDebuggerUrl: string }[]
const socket = new WebSocket(targets.find((target) => target.type === 'page')!.webSocketDebuggerUrl)
await new Promise((ready) => socket.addEventListener('open', ready, { once: true }))

let next = 0
const pending = new Map<number, (value: any) => void>()
socket.addEventListener('message', (event) => {
  const message = JSON.parse(String(event.data))
  if (message.id !== undefined) pending.get(message.id)?.(message.result ?? message.error)
})
function send(method: string, params: object = {}): Promise<any> {
  const id = ++next
  socket.send(JSON.stringify({ id, method, params }))
  return new Promise((done) => pending.set(id, done))
}

async function viewport(width: number, height: number, mobile = false) {
  await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: mobile ? 3 : 2, mobile })
  await send('Emulation.setTouchEmulationEnabled', { enabled: mobile, maxTouchPoints: mobile ? 5 : 0 })
  await send('Emulation.setEmitTouchEventsForMouse', { enabled: mobile })
}

async function open(path: string, signedIn = true) {
  await send('Network.clearBrowserCookies')
  if (signedIn) await send('Network.setCookie', { name: cookieName, value: cookieValue, url: base })
  await send('Page.navigate', { url: base + path })
  await sleep(1500)
}

const named: Record<string, { code: string; keyCode: number }> = {
  Escape: { code: 'Escape', keyCode: 27 },
  Enter: { code: 'Enter', keyCode: 13 },
  Tab: { code: 'Tab', keyCode: 9 },
}

/** Keys as typed: `gh`, `?`, `<Escape>` for a named one, `<A-h>` with Alt. */
async function keys(sequence: string, pause = 250) {
  for (const token of sequence.match(/<A-\w>|<\w+>|./g) ?? []) {
    if (token.startsWith('<A-')) {
      const key = token[3]
      const params = { key, code: `Key${key.toUpperCase()}`, modifiers: 1 }
      await send('Input.dispatchKeyEvent', { type: 'rawKeyDown', ...params })
      await send('Input.dispatchKeyEvent', { type: 'keyUp', ...params })
    } else if (token.startsWith('<')) {
      const key = token.slice(1, -1)
      const { code, keyCode } = named[key]
      const text = key === 'Enter' ? '\r' : undefined
      await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode: keyCode, text })
      await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: keyCode })
    } else {
      const upper = token !== token.toLowerCase()
      const code = /[a-z]/i.test(token) ? `Key${token.toUpperCase()}` : undefined
      const params = { key: token, code, text: token, modifiers: upper ? 8 : 0 }
      await send('Input.dispatchKeyEvent', { type: 'keyDown', ...params })
      await send('Input.dispatchKeyEvent', { type: 'keyUp', ...params })
    }
    await sleep(pause)
  }
}

async function shot(name: string) {
  await sleep(600)
  const { data } = await send('Page.captureScreenshot', { format: 'png' })
  const file = join(out, `${name}.png`)
  writeFileSync(file, Buffer.from(data, 'base64'))
  // A dark UI fits in 256 colours at a third of the size; skipped without ImageMagick.
  if (which('magick')) spawnSync(['magick', file, '-dither', 'None', '-colors', '256', '-define', 'png:compression-level=9', `PNG8:${file}`])
  console.log(`docs/images/${name}.png`)
}

await send('Page.enable')
await send('Network.enable')
// The agenda opens itself once a day; here it opens when asked.
await send('Page.addScriptToEvaluateOnNewDocument', {
  source: `localStorage.setItem('diary:agenda-seen', new Date().toDateString())`,
})

await viewport(1440, 900)

await open('/', false)
await shot('login')

await open('/n/work/projects/ingest-pipeline')
await shot('document')

// From the folder: the pipeline in the focused tile, the rewrite beside it.
await open('/n/work/projects')
await keys('jl')
await sleep(800)
await keys('<A-h>kL')
await sleep(800)
await shot('tiles')

await open('/n/work/runbook')
await keys('gh', 400)
await sleep(800)
await shot('history')

await open('/n/work/runbook')
await keys('i')
await sleep(500)
await shot('editor')

await open('/n/uni/linear-algebra/eigenvalues')
await shot('math')

await open('/n/work')
await keys('b')
await sleep(800)
await shot('board')

await keys('t')
await sleep(800)
await shot('agenda')
await keys('<Escape>')

await open('/n/diary')
await keys('C')
await sleep(800)
await shot('calendar')
await keys('<Escape>')

await open('/n/work')
await keys('/')
await send('Input.insertText', { text: 'tariffs' })
await sleep(300)
await keys('<Enter>')
await sleep(800)
await shot('search')

await open('/n/work/data/throughput-csv')
await shot('file-table')

await open('/n/work/certificates/gateway-pem')
await shot('file-certificate')

await open('/n/work')
await keys('w')
await sleep(800)
await shot('overview')
await keys('<Escape>')

await keys('?')
await sleep(800)
await shot('help')

await viewport(390, 844, true)
await open('/n/work/projects/ingest-pipeline')
await shot('mobile-document')
await open('/n/work')
await shot('mobile-tree')

socket.close()
await finish()
