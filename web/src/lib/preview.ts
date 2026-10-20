/** What the file pane shows a file as. */
export type PreviewKind = 'image' | 'video' | 'audio' | 'pdf' | 'markdown' | 'table' | 'text' | 'cert' | 'none'

/*
 * Uploads keep a MIME type only when it cannot execute, so a `.json` or a
 * `.pem` is stored as `application/octet-stream`. The name says more.
 */
const TEXT = new Set(
  (
    'txt text log out err diff patch json jsonl ndjson yaml yml toml ini cfg conf env properties ' +
    'xml html htm css scss js mjs cjs ts tsx jsx svelte vue rs go py rb php java kt swift c h cc cpp hpp cs ' +
    'sh bash zsh fish ps1 bat sql graphql proto lua pl r tex bib srt vtt gitignore dockerfile makefile pub'
  ).split(' '),
)
const CERT = new Set(['pem', 'crt', 'cer', 'der', 'csr', 'key', 'p8', 'p12', 'pfx'])

/** `notes.md` → `md`; `Makefile` → `makefile`; a dotfile is its own name. */
function extension(name: string): string {
  const lower = name.toLowerCase()
  const dot = lower.lastIndexOf('.')
  return dot === -1 ? lower : lower.slice(dot + 1)
}

/**
 * By name first: `.ts` is TypeScript here, whatever the MIME tables say
 * about MPEG transport streams. `none` may still turn out to be text — see
 * `looksLikeText`.
 */
export function previewKind(name: string, mime: string): PreviewKind {
  const ext = extension(name)
  if (CERT.has(ext)) return 'cert'
  if (ext === 'md' || ext === 'markdown') return 'markdown'
  if (ext === 'csv' || ext === 'tsv') return 'table'
  if (TEXT.has(ext)) return 'text'
  const [type] = mime.split('/')
  if (type === 'image' || type === 'video' || type === 'audio') return type
  if (mime === 'application/pdf') return 'pdf'
  if (mime === 'text/plain') return 'text'
  return 'none'
}

/** No NUL bytes and valid UTF-8, a character cut off at the end aside. */
export function looksLikeText(bytes: Uint8Array): boolean {
  if (bytes.includes(0)) return false
  try {
    new TextDecoder('utf-8', { fatal: true }).decode(bytes, { stream: true })
    return true
  } catch {
    return false
  }
}

/** CSV or TSV into rows, quotes respected, at most `limit` of them. */
export function parseTable(text: string, separator: string, limit: number): string[][] {
  const rows: string[][] = []
  let row: string[] = []
  let cell = ''
  let quoted = false
  for (let i = 0; i < text.length && rows.length < limit; i++) {
    const c = text[i]
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') {
        cell += '"'
        i++
      } else if (c === '"') quoted = false
      else cell += c
    } else if (c === '"' && cell === '') quoted = true
    else if (c === separator) {
      row.push(cell)
      cell = ''
    } else if (c === '\n' || c === '\r') {
      if (c === '\r' && text[i + 1] === '\n') i++
      row.push(cell)
      rows.push(row)
      row = []
      cell = ''
    } else cell += c
  }
  if ((cell !== '' || row.length > 0) && rows.length < limit) rows.push([...row, cell])
  return rows
}

/** The separator of a table file, by its name. */
export const separatorOf = (name: string) => (extension(name) === 'tsv' ? '\t' : ',')
