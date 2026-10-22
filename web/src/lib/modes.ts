/*
 * Which keys are heard right now, and in what order: a stack of the
 * contexts in keymap.ts, the topmost first. A key goes down the stack until
 * a context takes it.
 *
 *   always     Ctrl-S, Alt-h/j/k/l — heard in the editor too
 *   a sheet    help, info, trash …; nothing under one hears a key
 *   drawer     the scratchpad, while it is down
 *   workspace  Esc out of fullscreen, Q
 *   writing    a document being written takes every plain key back
 *   tile       what has the focus: the tree, or a view — board, agenda,
 *              calendar, a card's back
 *   views      what the views share: q, gt, b
 *   global     spaces, t, T, C, S, :, ?
 *
 * The `[y/N]` prompt sits over all of it, in App.svelte, and the command
 * line keeps its own keys while it has them.
 */
import { actions, type Run } from './actions'
import { bindings, type Binding, type Context } from './keymap'
import { workspace } from './store.svelte'

/** The sheets with keys of their own, under the same name as the sheet. */
const sheets = new Set<string>(['help', 'overview', 'media', 'trash'])

/** The stack, the topmost first. */
export function stack(): Context[] {
  const modes: Context[] = ['always']
  const overlay = workspace.overlay
  if (overlay !== 'none') {
    // The info sheet hears the keys of the part on show, then its own Tab.
    if (overlay === 'info') modes.push(workspace.infoSection, 'info')
    else if (sheets.has(overlay)) modes.push(overlay as Context)
    return [...modes, 'sheet']
  }
  if (workspace.scratchOpen && !workspace.scratchEditing) modes.push('drawer')
  modes.push('workspace')
  const focus = workspace.focus
  if (focus === 'list' || focus === 'node') {
    if (workspace.editing && focus === 'node') modes.push('writing')
    modes.push('tree')
  } else {
    // While the description is written, the card's back has no keys of its own.
    if (!(focus === 'card' && workspace.describing)) modes.push(focus)
    modes.push('views')
  }
  modes.push('global')
  return modes
}

const lone = new Set(['Shift', 'Control', 'Alt', 'AltGraph', 'Meta', 'CapsLock', 'OS'])

/**
 * A key as the table spells it: `j`, `G`, `Enter`, `Shift-Tab`, `Ctrl-d`,
 * `Alt-Shift-h`, `1`, `Shift-1`. Digits, and letters under Alt, are read
 * from `event.code`, so they are the same key on every layout — and on a
 * Mac, where Alt turns h into ˙. Shift is only spelled out where the key
 * itself does not already say it. A modifier on its own is no key at all.
 */
export function keyName(event: KeyboardEvent): string | null {
  if (lone.has(event.key)) return null
  const digit = /^Digit(\d)$/.exec(event.code)?.[1]
  const letter = event.altKey && !event.ctrlKey && !event.metaKey ? /^Key([A-Z])$/.exec(event.code)?.[1].toLowerCase() : undefined
  const key = digit ?? letter ?? (event.key === ' ' ? 'Space' : event.key)
  const shift = event.shiftKey && (key.length > 1 || digit !== undefined || letter !== undefined)
  const mods = [event.ctrlKey && 'Ctrl', event.metaKey && 'Meta', event.altKey && 'Alt', shift && 'Shift']
  return [...mods.filter(Boolean), key].join('-')
}

const plain = (name: string) => !/^(Ctrl|Meta|Alt)-/.test(name)

const byContext = new Map<Context, Binding[]>()
for (const binding of bindings as readonly Binding[]) {
  if (!binding.keys) continue
  byContext.set(binding.context, [...(byContext.get(binding.context) ?? []), binding])
}

function find(context: Context, name: string): Binding | undefined {
  const own = byContext.get(context) ?? []
  // A shifted key nothing asked for is the key itself: Shift-Space is Space.
  return own.find((binding) => binding.keys!.includes(name)) ?? (name.startsWith('Shift-') ? find(context, name.slice(6)) : undefined)
}

function run(binding: Binding, name: string, event: KeyboardEvent): boolean {
  const action = (actions[binding.context] as Record<string, Run>)[binding.action!]
  return action(name, event) !== false
}

/**
 * Hand a key to the stack. `pending` is the first half of a two-key `gg`,
 * `dd`, `]]` … still waiting; what comes back is the half waiting now.
 */
export function dispatch(event: KeyboardEvent, pending: string, line: boolean, typing: boolean): string {
  const name = keyName(event)
  if (name === null) return pending
  // What the editor took for itself — its own Ctrl-S among them — is done;
  // running it again here would send a second save racing the first.
  if (typing && event.defaultPrevented) return pending
  const listening = !line && !typing
  const heard = (binding: Binding | undefined) => !!binding && (listening || !!binding.typing)

  // A sheet hears what it hears and lets nothing past it; the drawer and a
  // document being written take every plain key that reaches them.
  const reach: Context[] = []
  for (const context of stack()) {
    reach.push(context)
    if (context === 'sheet') break
    if (plain(name) && listening && byContext.get(context)?.some((binding) => binding.keys!.includes('*'))) break
  }

  const take = (binding: Binding, key: string) => {
    if (!run(binding, key, event)) return false
    event.preventDefault()
    return true
  }

  // The second half of a pair goes first, wherever in the stack the pair is.
  if (pending && listening) {
    for (const context of reach) {
      const binding = find(context, pending + name)
      if (heard(binding) && take(binding!, pending + name)) return ''
    }
  }
  for (const context of reach) {
    const binding = find(context, name)
    if (heard(binding) && take(binding!, name)) return ''
    // The first half of a pair this context knows waits for the second.
    if (listening && !binding && byContext.get(context)?.some((entry) => entry.keys!.some((key) => key.length === 2 && key[0] === name))) {
      event.preventDefault()
      return name
    }
    const rest = find(context, '*')
    if (listening && plain(name) && heard(rest) && take(rest!, name)) return ''
  }
  return ''
}
