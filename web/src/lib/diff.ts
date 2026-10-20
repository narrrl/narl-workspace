/** One line of a diff: kept, only in the old text, or only in the new. */
export type DiffLine = { kind: 'same' | 'gone' | 'added'; text: string }

/** A run of unchanged lines too long to be worth showing, folded to a count. */
export type DiffRow = DiffLine | { kind: 'fold'; count: number }

/*
 * Past this many lines in the part that differs, the table a longest common
 * subsequence needs would be tens of megabytes; the change is shown as the
 * old lines out and the new ones in instead. Documents are rarely near it.
 */
const MAX_CELLS = 4_000_000

/** Lines of `before` and `after`, matched up the way `diff` would. */
export function diffLines(before: string, after: string): DiffLine[] {
  const a = before.split('\n')
  const b = after.split('\n')

  // What both start and end with needs no table.
  let start = 0
  while (start < a.length && start < b.length && a[start] === b[start]) start++
  let end = 0
  while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) end++

  const head = a.slice(0, start).map((text): DiffLine => ({ kind: 'same', text }))
  const tail = a.slice(a.length - end).map((text): DiffLine => ({ kind: 'same', text }))
  const x = a.slice(start, a.length - end)
  const y = b.slice(start, b.length - end)

  if (x.length * y.length > MAX_CELLS) {
    return [
      ...head,
      ...x.map((text): DiffLine => ({ kind: 'gone', text })),
      ...y.map((text): DiffLine => ({ kind: 'added', text })),
      ...tail,
    ]
  }

  // lcs[i][j]: the longest common subsequence of x[i..] and y[j..].
  const width = y.length + 1
  const lcs = new Uint32Array((x.length + 1) * width)
  for (let i = x.length - 1; i >= 0; i--) {
    for (let j = y.length - 1; j >= 0; j--) {
      lcs[i * width + j] =
        x[i] === y[j] ? lcs[(i + 1) * width + j + 1] + 1 : Math.max(lcs[(i + 1) * width + j], lcs[i * width + j + 1])
    }
  }

  const middle: DiffLine[] = []
  let i = 0
  let j = 0
  while (i < x.length && j < y.length) {
    if (x[i] === y[j]) {
      middle.push({ kind: 'same', text: x[i] })
      i++
      j++
    } else if (lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
      middle.push({ kind: 'gone', text: x[i++] })
    } else {
      middle.push({ kind: 'added', text: y[j++] })
    }
  }
  while (i < x.length) middle.push({ kind: 'gone', text: x[i++] })
  while (j < y.length) middle.push({ kind: 'added', text: y[j++] })

  return [...head, ...middle, ...tail]
}

/** The diff with only `context` unchanged lines kept around each change. */
export function folded(lines: DiffLine[], context = 3): DiffRow[] {
  const near = new Uint8Array(lines.length)
  lines.forEach((line, index) => {
    if (line.kind === 'same') return
    for (let k = Math.max(index - context, 0); k <= Math.min(index + context, lines.length - 1); k++) near[k] = 1
  })
  const rows: DiffRow[] = []
  let skipped = 0
  lines.forEach((line, index) => {
    if (near[index]) {
      if (skipped) rows.push({ kind: 'fold', count: skipped })
      skipped = 0
      rows.push(line)
    } else skipped++
  })
  if (skipped) rows.push({ kind: 'fold', count: skipped })
  return rows
}
