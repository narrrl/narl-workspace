import { tick } from 'svelte'
import { api, ApiError, isContainer, type BackupLogin, type BackupStatus, type NodeSummary } from './api'
import { openHelp } from './Help.svelte'
import { infoSections, renewLink, toggleSharing } from './Info.svelte'
import { loadSettings, openSettings, settingsSections, type SettingsSection } from './Settings.svelte'
import { workspace, type InfoSection } from './store.svelte'
import { emptyTrash } from './Trash.svelte'
import { formatDay, parseDay, parseDue, relative, titleHref, type Heading, type ScrollAmount, type SidewaysAmount } from './util'

export type { ScrollAmount, SidewaysAmount }

/** Filled in by App.svelte so ex-commands can reach the DOM-bound bits. */
export const hooks = {
  pickFiles: () => {},
  /** Files for the folder being browsed, rather than for the document. */
  pickTreeFiles: (_encrypt: boolean) => {},
  pickFolder: () => {},
  /** `root`: into the workspace itself, where each directory is a space. */
  pickArchive: (_root: boolean) => {},
  focusEditor: () => {},
  /** Filled in while a document is being written: into its title field. */
  focusTitle: () => {},
  focusList: () => {},
  insertText: (_text: string) => {},
  /* Filled in while a document is on screen and not being edited: the reading
     pane owns the scrollback, so the list keys have somewhere to send it. */
  scrollReader: (_amount: ScrollAmount): boolean => false,
  /* `zh` / `zl`: the wide table or code block at the top of the view,
     sideways; false when nothing on screen is wider than the column. */
  scrollWide: (_amount: SidewaysAmount): boolean => false,
  /* `F`: what holds the links and buttons on screen, null when nothing does. */
  hintRoot: (): HTMLElement | null => null,
  /* Filled in by the file pane: what Enter does there, false when nothing. */
  fileAction: (): boolean => false,
  /* `:source`: a markdown file between rendered and raw; false when the pane
     shows no markdown. */
  toggleSource: (): boolean => false,
  /* `]]` / `[[` in whatever is being read; false when it has no headings. */
  jumpHeading: (_direction: 1 | -1): boolean => false,
  /* `gO`: the headings of what is on screen, read or being written, and the
     one the reader or the cursor is in. */
  headings: (): { headings: Heading[]; current: number } => ({ headings: [], current: 0 }),
  /** How many space cards the overview fits in a row, for j/k to step by. */
  overviewColumns: (): number => 1,
  openOverview: () => {},
  openCommandLine: (_initial: string) => {},
  /** Ctrl-P: a search across every space. */
  openGlobalSearch: () => {},
  /** Esc with the line open but the keyboard elsewhere: the line away; false when it is not open. */
  closeCommandLine: (): boolean => false,
  /** `F`: letters over the links and buttons inside `root`. */
  showHints: (_root: HTMLElement) => {},
  /** The board on screen, for `F` over its cards. */
  boardRoot: (): HTMLElement | null => null,
}

/** Help groups the command list under these headings, in this order. */
export const groups = ['tree', 'board', 'documents', 'tiles', 'editing', 'sharing', 'vault', 'view'] as const
export type Group = (typeof groups)[number]

export interface CommandSpec {
  name: string
  aliases?: string[]
  args?: string
  group: Group
  help: string
  run: (arg: string) => unknown
  /** What the argument can be, for Tab on the command line. */
  complete?: (arg: string) => string[]
  /**
   * The forcing or everything variant, `:w!`, `:search!` — vim hands the
   * bang over as an argument. A `!` means nothing else: a command that does
   * a second thing takes a word for it, `:space new`.
   */
  bang?: { help: string; run: (arg: string) => unknown }
  /** Among the few the first page of the help lists. */
  essential?: true
}

/**
 * The first word of `arg` when it is one of `verbs`, and what follows it:
 * `rm work` is `['rm', 'work']`. Anything else is `['', arg]`, so a name
 * that is not a verb is the plain command's argument.
 */
function verb(arg: string, verbs: readonly string[]): [string, string] {
  const match = /^\s*(\S+)(?:\s+([\s\S]*))?$/.exec(arg)
  if (match && verbs.includes(match[1])) return [match[1], (match[2] ?? '').trim()]
  return ['', arg.trim()]
}

/** Close the focused tile; the index of the one that has the focus after. */
const closeFocused = () => {
  workspace.tiles.close()
  return workspace.tiles.focused
}

/**
 * The info sheet over the open document, at `section` or where it was last
 * left. Every part is read now, so each one's count is in its tab: the
 * headings from the pane, the versions and backlinks from the server, and
 * the cards with the document.
 */
export async function openInfo(section?: InfoSection) {
  if (!requireOpen()) return
  const { headings, current } = hooks.headings()
  workspace.sheets.outline = headings
  workspace.sheets.outlineCursor = current
  await workspace.guard(async () => {
    // One after the other: each writes what is typed first, and two saves
    // of the same text at once would race.
    await workspace.sheets.loadHistory()
    await workspace.sheets.loadBacklinks()
    // Picked afresh each time, from the first: the open card leads.
    workspace.sheets.cardsCursor = 0
    workspace.sheets.filesCursor = 0
    if (section) workspace.sheets.infoSection = section
    workspace.sheets.open('info')
  })
}

/** A document is open and in focus — being written or read — rather than the list. */
export const readingDocument = () =>
  workspace.editing || (workspace.focus === 'node' && workspace.open?.kind === 'document')

/** The board's cursor, or the card turned over beside it, has the keys. */
const onBoard = () => workspace.focus === 'board' || workspace.focus === 'card'

const requireOpen = () => {
  if (!workspace.open) {
    workspace.say('no document open', 'error')
    return null
  }
  return workspace.open
}

/** A card about `node`, titled after it unless named. */
const cardAbout = (node: NodeSummary, title: string) =>
  workspace.guard(() => workspace.cards.createCard(title.trim() || node.name.trim() || node.slug, node.id))

const TITLE_HREF = titleHref('')

/**
 * Where a link in a document goes. `/n/` links are the workspace's own — the
 * importer writes them between documents — and are opened in place, as is a
 * `[[title]]`, or the offer to write it when no document has it; anything
 * else would take the workspace's tab with it, so it gets a tab of its own,
 * as does a workspace link asked for with `gx`.
 */
export function followLink(href: string, tab = false) {
  if (href.startsWith(TITLE_HREF)) {
    const title = decodeURIComponent(href.slice(TITLE_HREF.length))
    const path = workspace.open?.wikilinks[title]
    if (tab) {
      if (!path) return workspace.say(`no document called "${title}" yet`, 'error')
      href = `/n/${path}`
    } else return void workspace.guard(() => workspace.tree.followTitle(title))
  }
  if (href.startsWith('/n/') && !tab) {
    const path = href.slice(3).split('#')[0]
    return void workspace.guard(() => workspace.tree.openPath(decodeURIComponent(path)))
  }
  window.open(new URL(href, location.href).href, '_blank', 'noopener,noreferrer')
}

/**
 * Focus a tile and hand it the keys: its editor while it is being written,
 * otherwise the shell, whose keys then act on that tile.
 */
export async function focusTile(index: number) {
  if (!workspace.tiles.all[index]) return
  workspace.tiles.focusOn(index)
  // The tile's editor fills the hooks once it has seen it has the focus.
  await tick()
  if (workspace.editing && workspace.pane === 'document') hooks.focusEditor()
  else hooks.focusList()
}

/** `gt` / `gT`, `:bn` / `:bp`: the next or previous tile, round and round. */
export function cycleTile(delta: 1 | -1) {
  const count = workspace.tiles.all.length
  if (count < 2) return workspace.say('only one tile — L on a row opens another', 'error')
  return focusTile((workspace.tiles.focused + delta + count) % count)
}

/** `:ls`'s line for one tile: number, `%` for the focused one, `[+]` unsaved. */
const describeTile = (index: number) => {
  const tile = workspace.tiles.all[index]
  const name = tile.kind !== 'node' ? tile.kind : tile.open ? tile.open.name.trim() || tile.open.slug : '(empty)'
  return `${index + 1}${index === workspace.tiles.focused ? '%' : ''} ${name}${tile.dirty ? ' [+]' : ''}`
}

/** `:space rm`: a space, the one you are in unless named, into the trash. */
async function removeSpace(name: string) {
  const wanted = name.toLowerCase()
  const space = wanted ? workspace.tree.spaces.find((s) => s.slug === wanted || s.name.toLowerCase() === wanted) : workspace.tree.space
  if (!space) return workspace.say(`no such space: ${wanted || '(none open)'}`, 'error')
  if (!(await workspace.prompts.ask(`delete the space "${space.name}" and everything in it? it goes to the trash.`))) return
  await workspace.guard(() => workspace.tree.deleteSpace(space.id))
}

/** Move the cursor and follow it, so `:next` reads as well as `j` then `Enter`. */
const jumpTo = async (index: number) => {
  if (workspace.tree.rows === 0) return workspace.say('nothing here', 'error')
  workspace.tree.cursor = Math.min(Math.max(index, 0), workspace.tree.rows - 1)
  await workspace.guard(() => workspace.tree.openSelected())
}

/** A folder among the children currently listed, by slug or by name. */
const findFolder = (target: string) => {
  const wanted = target.toLowerCase()
  return workspace.tree.nodes.find(
    (node) =>
      isContainer(node) && (node.slug === wanted || node.name.toLowerCase() === wanted),
  )
}

/** The folders listed here, by slug, then the two ways out — for `:cd` and `:mv`. */
const folderNames = () => [
  ...workspace.tree.nodes.filter(isContainer).map((node) => node.slug),
  '..',
  '/',
]

const spaceNames = () => workspace.tree.spaces.map((space) => space.slug)

/** The capture tokens' names, for Tab after `:token rm`; null until fetched. */
let tokenNames: string[] | null = null

/*
 * The titles of the documents a search finds for what is typed after
 * `:append`, for Tab — fetched as the typing goes, so by the time Tab is
 * pressed they are usually there.
 */
let titlesFor = ''
let titles: string[] = []
function documentTitles(arg: string): string[] {
  const wanted = arg.trim()
  if (wanted && wanted !== titlesFor) {
    titlesFor = wanted
    void api.listNodes({ q: wanted }).then(
      (found) => {
        if (titlesFor !== wanted) return
        titles = found.filter((node) => node.kind === 'document').map((node) => node.name.trim() || node.slug)
      },
      () => {},
    )
  }
  return titles
}

/** The document in a tile, the focused one first: what a note goes to the end of unless told otherwise. */
function documentOnScreen(): string | undefined {
  const tile = [workspace.tile, ...workspace.tiles.all].find((t) => t.open?.kind === 'document')
  return tile?.open ? `/n/${tile.open.path.map((crumb) => crumb.slug).join('/')}` : undefined
}

/** The templates of the space last asked about, for `:new` to complete. */
let templateCache: { space: number; names: string[] } | null = null

/** Tab cannot wait for the server: the names come on the press after the first. */
function templateNames(): string[] {
  const here = workspace.tree.spaceId
  if (here === null) return []
  if (templateCache?.space !== here) {
    templateCache = { space: here, names: [] }
    void api.templates(here).then(
      (templates) => (templateCache = { space: here, names: templates.map((t) => t.slug) }),
      () => (templateCache = null),
    )
  }
  return templateCache.names
}

/** `:encrypt` and `:decrypt`: a file, or everything under a folder. */
const sealTarget = (encrypt: boolean) =>
  workspace.guard(async () => {
    const node = workspace.target
    if (!node || node.kind === 'document') {
      return workspace.say('only files go into the vault — put the cursor on a file or a folder', 'error')
    }
    if (node.file && node.file.encrypted === encrypt) {
      return workspace.say(encrypt ? 'already in the vault' : 'not in the vault')
    }
    if (!encrypt && workspace.tree.parentSealed) {
      return workspace.say(`this ${node.kind} is inside a sealed folder — decrypt that folder instead`, 'error')
    }
    const name = node.name || node.slug
    if (isContainer(node)) {
      const question = encrypt
        ? `seal ${node.kind} "${name}" — every file in it now, and every file added later? documents stay readable`
        : `unseal ${node.kind} "${name}" and take every file in it out of the vault?`
      if (!(await workspace.prompts.ask(question))) return
    } else if (!encrypt) {
      if (!(await workspace.prompts.ask(`store "${name}" unencrypted again, and in the readable files/ view on proton drive?`))) return
    }
    if (!(await workspace.vault.ensureUnlocked())) return
    workspace.say(encrypt ? 'encrypting …' : 'decrypting …')
    await workspace.vault.seal(node.id, encrypt)
  })

/** `3 minutes ago`, or `never`. Backups are read at a glance, not to the second. */
const ago = (at: number | null) => {
  if (!at) return 'never'
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - at)
  if (seconds < 90) return `${seconds}s ago`
  if (seconds < 5400) return `${Math.round(seconds / 60)}m ago`
  if (seconds < 172800) return `${Math.round(seconds / 3600)}h ago`
  return `${Math.round(seconds / 86400)}d ago`
}

/** "firefox on linux", out of a user-agent string, as far as it says. */
export function browserOf(agent: string): string {
  const browser = /Edg\//.test(agent)
    ? 'edge'
    : /Firefox\//.test(agent)
      ? 'firefox'
      : /Chrome\//.test(agent)
        ? 'chrome'
        : /Safari\//.test(agent)
          ? 'safari'
          : /curl\//.test(agent)
            ? 'curl'
            : 'a browser'
  const system = /Android/.test(agent)
    ? 'android'
    : /iPhone|iPad/.test(agent)
      ? 'ios'
      : /Mac OS X/.test(agent)
        ? 'macos'
        : /Windows/.test(agent)
          ? 'windows'
          : /Linux/.test(agent)
            ? 'linux'
            : ''
  return system ? `${browser} on ${system}` : browser
}

/** The mirror in a line. */
export function describeBackup(status: BackupStatus): string {
  if (!status.configured) return 'proton drive backups are off — :backup login <proton-username> sets them up'
  if (status.last_error) return `backup failed: ${status.last_error}`
  const where = status.device ? `${status.device} · ` : ''
  const state = status.running ? 'running now' : status.pending ? 'changes waiting' : 'up to date'
  return `${where}${state} · last ${ago(status.last_success_at)}`
}

/** The vault in a line: open or locked, and what it is busy with. */
export function describeVault(): string {
  const vault = workspace.vault.status
  if (!vault?.configured) return 'no vault yet'
  const left = vault.locks_in ? ` · locks in ${Math.ceil(vault.locks_in / 60)}m unless used` : ''
  const rekey = vault.rekey
    ? vault.unlocked
      ? ` · re-sealing after the password change, ${vault.rekey.total ? `${vault.rekey.done}/${vault.rekey.total}` : 'counting'}`
      : ' · re-sealing after the password change waits for :vault unlock'
    : ''
  return vault.unlocked ? `open${left}${rekey}` : `locked${rekey}`
}

/** The first time: a password for the vault, asked twice. */
async function setupVault() {
  const password = await workspace.prompts.askSecret('new vault password, 10+ characters — it cannot be recovered')
  if (password === null) return
  const again = await workspace.prompts.askSecret('the same again')
  if (again === null) return
  if (again !== password) return workspace.say('the two passwords differ — nothing was set', 'error')
  workspace.vault.status = await api.setupVault(password)
  workspace.say('vault ready and open — :encrypt seals what is under the cursor, :upload seal seals new files')
}

async function changeVaultPassword() {
  const current = await workspace.prompts.askSecret('current vault password')
  if (current === null) return
  const password = await workspace.prompts.askSecret('new vault password, 10+ characters')
  if (password === null) return
  const again = await workspace.prompts.askSecret('the same again')
  if (again === null) return
  if (again !== password) return workspace.say('the two passwords differ — nothing was changed', 'error')
  workspace.vault.status = await api.changeVaultPassword(current, password)
  workspace.say(
    workspace.vault.status.rekey
      ? `vault password changed — re-sealing ${workspace.vault.status.sealed} file(s) under the new key; leave the vault open, :vault shows how far`
      : 'vault password changed',
  )
}

/** The open document, or the one under the cursor, as a `.md` file in the downloads. */
async function exportDocument(node: NodeSummary) {
  const { name, body, created_at } = node.id === workspace.open?.id ? workspace.draft : await api.getNode(node.id)
  const text = name.trim() ? `# ${name.trim()}\n\n${body}` : body
  const url = URL.createObjectURL(new Blob([text], { type: 'text/markdown' }))
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = exportName(name.trim(), created_at)
  anchor.click()
  URL.revokeObjectURL(url)
  workspace.say(`wrote ${anchor.download}`)
}

/**
 * Log the mirror in to Proton: the disclosure Proton asks for, then the
 * password, then whatever the account asks for next — a two-factor code, or
 * Proton's human check in a sheet of its own.
 */
async function protonLogin(username: string) {
  const status = await api.backupStatus()
  const unsealed = status.sealed_session
    ? ''
    : ' The session will be stored unsealed on the data volume — set WORKSPACE_SESSION_KEY_FILE to seal it.'
  if (!(await workspace.prompts.ask(`${status.disclosure ?? ''}${unsealed} Log in as ${username}?`))) return
  const password = await workspace.prompts.askSecret(`proton password for ${username}`)
  if (password === null) return
  workspace.say('logging in to proton …')
  let answer: BackupLogin = await api.backupLogin(username, password)
  for (let round = 0; answer.next !== 'done'; round++) {
    if (round === 5) return workspace.say('proton keeps asking — try again later', 'error')
    if (answer.next === 'totp') {
      const code = await workspace.prompts.askSecret('proton two-factor code')
      if (code === null) return workspace.say('proton login abandoned')
      try {
        answer = await api.backupTotp(code)
      } catch (error) {
        // A wrong code leaves the login waiting; ask again.
        if (!(error instanceof ApiError) || error.status !== 400 || !/two-factor code/.test(error.message)) throw error
        workspace.say(error.message, 'error')
      }
    } else {
      const token = await workspace.prompts.askVerification(answer.url ?? '')
      if (token === null) return workspace.say('proton login abandoned')
      workspace.say('logging in to proton …')
      answer = await api.backupCaptcha(token)
    }
  }
  const device = answer.status.device ? ` as ${answer.status.device}` : ''
  workspace.say(`proton drive connected${device} — the first backup follows shortly, :backup! runs one now`)
}

/** A filename that survives a download folder: `2026-09-05-first-light.md`. */
const exportName = (title: string, at: number) => {
  const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')
  return `${formatDay(at)}${slug ? `-${slug}` : ''}.md`
}

/*
 * Out of the editor, one level down: what is waiting is written first, and
 * where that fails the text stays on screen to be tried again or let go with
 * `:q!`. A document `o` made and nothing was written in is not kept.
 */
async function leaveEditor() {
  if (workspace.drafts.writing) return void workspace.drafts.leaveWriting()
  if (await workspace.drafts.abandonIfBlank()) return hooks.focusList()
  if (workspace.dirty && !(await workspace.guard(() => workspace.drafts.save()))) return
  workspace.editing = false
  hooks.focusList()
}

export const commands: CommandSpec[] = [
  {
    name: 'write',
    aliases: ['w'],
    group: 'editing',
    essential: true,
    help: "save what is being written — the open document, a card's description, a note — rather than in a moment",
    run: () => workspace.guard(() => (workspace.drafts.writing ? workspace.drafts.saveWriting() : workspace.drafts.save())),
    bang: {
      help: 'save even over a newer version saved somewhere else since this one was opened',
      run: () => workspace.guard(() => workspace.drafts.save({ force: true })),
    },
  },
  {
    name: 'wq',
    aliases: ['x'],
    group: 'editing',
    help: 'leave the editor with everything written, as :q does',
    run: () => leaveEditor(),
    bang: {
      help: 'save over a newer version, as :w! does, and leave the editor',
      run: async () => {
        if (await workspace.guard(() => workspace.drafts.save({ force: true }))) await leaveEditor()
      },
    },
  },
  {
    name: 'quit',
    aliases: ['q'],
    group: 'editing',
    essential: true,
    help: 'leave the editor with what is waiting written first, or close the document when already reading',
    run: async () => {
      if (workspace.drafts.writing || workspace.editing) return leaveEditor()
      // A document `o` made and nothing was written in is not kept.
      if (await workspace.drafts.abandonIfBlank()) return hooks.focusList()
      if (workspace.dirty) {
        workspace.say('unsaved changes — :w to write, :q! to discard', 'error')
        return
      }
      if (workspace.tiles.all.length > 1) return focusTile(closeFocused())
      workspace.tiles.close()
      hooks.focusList()
    },
    bang: {
      help: 'leave the editor, letting go of what is not saved yet',
      run: async () => {
        if (workspace.drafts.writing) return void workspace.drafts.leaveWriting(true)
        const open = workspace.open
        workspace.editing = false
        workspace.dirty = false
        if (open) {
          workspace.drafts.dropStash(open.id)
          await workspace.guard(() => workspace.tree.openNode(open.id))
        }
        hooks.focusList()
      },
    },
  },
  {
    name: 'space',
    aliases: ['sp'],
    args: '[name] | new <name> | name <text> | rm [name]',
    group: 'tree',
    help: 'switch to a space, or list them — new makes one, name renames the one you are in, rm puts one in the trash with everything in it (asks first)',
    complete: (arg) => (arg.startsWith('rm ') ? spaceNames().map((name) => `rm ${name}`) : ['new ', 'name ', 'rm ', ...spaceNames()]),
    run: async (arg) => {
      const [what, rest] = verb(arg, ['new', 'name', 'rm'])
      if (what === 'new') {
        if (!rest) return workspace.say('usage: :space new <name>', 'error')
        return workspace.guard(() => workspace.tree.createSpace(rest))
      }
      if (what === 'name') {
        const space = workspace.tree.space
        if (!space) return workspace.say('no space open', 'error')
        if (!rest) return workspace.say('usage: :space name <text>', 'error')
        return workspace.guard(() => workspace.tree.renameNode(space.id, rest))
      }
      if (what === 'rm') return removeSpace(rest)
      const wanted = rest.toLowerCase()
      if (!wanted) {
        return workspace.say(workspace.tree.spaces.map((space) => space.slug).join(' · ') || 'no spaces')
      }
      const space = workspace.tree.spaces.find(
        (candidate) => candidate.slug === wanted || candidate.name.toLowerCase() === wanted,
      )
      if (!space) return workspace.say(`no such space: ${wanted} — :space new ${wanted} makes it`, 'error')
      await workspace.guard(() => workspace.tree.enterSpace(space.id))
    },
  },
  {
    name: 'mkdir',
    args: '<name>',
    group: 'tree',
    help: 'make a folder here',
    run: async (arg) => {
      if (!arg.trim()) return workspace.say('usage: :mkdir <name>', 'error')
      await workspace.guard(() => workspace.tree.createFolder(arg.trim()))
    },
  },
  {
    name: 'cd',
    args: '<name|..|/>',
    group: 'tree',
    help: 'enter a folder, .. goes up, / back to the space',
    complete: folderNames,
    run: async (arg) => {
      const target = arg.trim()
      await workspace.guard(async () => {
        if (!target || target === '..') return workspace.tree.up()
        if (target === '/') {
          const space = workspace.tree.spaceId
          return space === null ? undefined : workspace.tree.goTo(space)
        }
        const folder = findFolder(target)
        if (!folder) return workspace.say(`no such folder here: ${target}`, 'error')
        await workspace.tree.goTo(folder.id)
      })
    },
  },
  {
    name: 'import',
    aliases: ['imp'],
    args: '[zip [/]]',
    group: 'tree',
    help: 'upload a folder into this one — markdown becomes documents, links follow; zip takes loose files or a .zip instead, and zip / pours an :export! back in, spaces and boards and all',
    complete: () => ['zip', 'zip /'],
    run: (arg) => {
      const [what, rest] = verb(arg, ['zip'])
      if (what === 'zip' && (rest === '' || rest === '/')) return hooks.pickArchive(rest === '/')
      if (arg.trim()) return workspace.say('usage: :import [zip [/]]', 'error')
      hooks.pickFolder()
    },
  },
  {
    name: 'board',
    group: 'board',
    help: "this space's board, in a tile — the last one shown when this space has none",
    run: () => workspace.guard(() => workspace.tiles.show('board')),
  },
  {
    name: 'plan',
    aliases: ['special'],
    group: 'board',
    help: 'the board, the agenda and the calendar, tiled ahead of what is open — S puts them up, and every view away again; 1 … 9 on the board picks the board',
    run: () => workspace.guard(() => workspace.tiles.togglePlan()),
  },
  {
    name: 'card',
    aliases: ['c'],
    args: '[title]',
    group: 'board',
    help: "add a card — reading a document, one about it, titled after it unless named, and gf on it comes back; on the board to the list under the cursor, elsewhere to this space's inbox",
    run: async (arg) => {
      const open = workspace.open
      if (readingDocument() && open) return cardAbout(open, arg)
      if (!arg.trim()) return workspace.say('usage: :card <title>', 'error')
      await workspace.guard(() => workspace.cards.createCard(arg.trim()))
    },
  },
  {
    name: 'due',
    args: '<when|->',
    group: 'board',
    help: "due date of the card in focus — on the board, its back, in the agenda, or the open document's: today, tm, fri, +3d, +2w, 10-31, 2026-10-31 — - clears",
    complete: () => ['today', 'tm', 'fri', '+3d'],
    run: async (arg) => {
      const target = arg.trim()
      if (target === '-' || target === '') return workspace.guard(() => workspace.cards.patchCard({ due_at: null }))
      const at = parseDue(target)
      if (at === null) return workspace.say(`not a date: ${target} — try fri, +3d or 2026-09-30`, 'error')
      await workspace.guard(async () => {
        if (await workspace.cards.patchCard({ due_at: at })) workspace.say(`due ${formatDay(at)}`)
      })
    },
  },
  {
    name: 'repeat',
    args: '<rule|->',
    group: 'board',
    help: 'once the card is done, a copy comes back in the inbox: daily, weekly, monthly, yearly, +3d, +2w, +6m — - stops it',
    complete: () => ['daily', 'weekly', 'monthly', 'yearly', '+2w', '-'],
    run: async (arg) => {
      const rule = arg.trim()
      const repeat = rule === '-' || rule === '' ? null : rule
      await workspace.guard(async () => {
        const card = await workspace.cards.patchCard({ repeat })
        // The server's spelling of the rule: `+14d` is kept as `+2w`.
        if (card) workspace.say(card.repeat === null ? 'repeats no more' : `repeats ${card.repeat} once done`)
      })
    },
  },
  {
    name: 'done',
    group: 'board',
    help: 'move the card in focus into the done list, or back out to the inbox',
    run: () => workspace.guard(() => workspace.cards.toggleDone()),
  },
  {
    name: 'scratch',
    aliases: ['sc'],
    args: '[text]',
    group: 'documents',
    help: 'the scratchpad, lowered over whatever is on screen — Q does the same; with text, a note on it without leaving',
    run: async (arg) => {
      if (!arg.trim()) return workspace.guard(() => workspace.scratch.toggle())
      await workspace.guard(() => workspace.scratch.add(arg.trim()))
    },
  },
  {
    name: 'append',
    args: '[document]',
    group: 'documents',
    help: 'put the scratch note at the end of a document, by its title — Tab completes it — or the one on screen, and take it off the scratchpad',
    complete: documentTitles,
    run: async (arg) => {
      if (!workspace.scratch.open) return workspace.say('no scratch note — Q lowers the scratchpad', 'error')
      const target = arg.trim() || documentOnScreen()
      if (!target) return workspace.say('usage: :append <document> — or open the one it goes to', 'error')
      await workspace.guard(() => workspace.scratch.appendNote(target))
    },
  },
  {
    name: 'keep',
    args: '[title]',
    group: 'documents',
    help: 'make the scratch note a document in the folder being browsed — titled by its first line unless given one — and take it off the scratchpad',
    run: async (arg) => {
      if (!workspace.scratch.open) return workspace.say('no scratch note — Q lowers the scratchpad', 'error')
      await workspace.guard(() => workspace.scratch.keepNote(arg))
    },
  },
  {
    name: 'task',
    aliases: ['step'],
    args: '[add] <text> | edit <text>',
    group: 'board',
    help: "add a step to the open card's checklist, below the cursor — edit rewords the step under it; add is only needed for a step that starts with edit",
    run: async (arg) => {
      if (workspace.focus !== 'card') return workspace.say('no card turned over — Enter on the board turns one', 'error')
      const [what, text] = verb(arg, ['add', 'edit'])
      if (!text) return workspace.say(`usage: :task ${what === 'edit' ? 'edit' : '[add]'} <text>`, 'error')
      await workspace.guard(() => (what === 'edit' ? workspace.cards.renameTask(text) : workspace.cards.addTask(text)))
    },
  },
  {
    name: 'label',
    args: '<name>',
    group: 'board',
    help: 'put a label on the card, or take it off when it has it — each name keeps its colour everywhere',
    complete: () => workspace.cards.boardLabels,
    run: async (arg) => {
      const card = workspace.cards.focusedCard
      if (!arg.trim() && card) return workspace.say(card.labels.join(' · ') || 'no labels — :label <name> adds one')
      await workspace.guard(() => workspace.cards.toggleLabel(arg))
    },
  },
  {
    name: 'list',
    args: '<name> | name <text> | rm',
    group: 'board',
    help: 'add a column to the board — name renames the one under the cursor, rm deletes it and every card on it (asks first)',
    complete: () => ['name ', 'rm'],
    run: async (arg) => {
      if (!workspace.tiles.view('board')) return workspace.say('no board open — :board', 'error')
      const [what, rest] = verb(arg, ['name', 'rm'])
      const list = workspace.cards.list
      if (what === 'name') {
        if (!list) return workspace.say('no list selected', 'error')
        if (!rest) return workspace.say('usage: :list name <text>', 'error')
        return workspace.guard(() => workspace.cards.renameList(rest))
      }
      if (what === 'rm') {
        if (!list) return workspace.say('no list selected', 'error')
        const cards = list.cards.length
        if (!(await workspace.prompts.ask(`delete the list "${list.name}" and its ${cards} card${cards === 1 ? '' : 's'}?`))) return
        return workspace.guard(() => workspace.cards.deleteList())
      }
      if (!rest) return workspace.say('usage: :list <name>', 'error')
      await workspace.guard(() => workspace.cards.createList(rest))
    },
  },
  {
    name: 'move',
    aliases: ['mv'],
    args: '<folder|..|/>',
    group: 'tree',
    essential: true,
    help: 'move the marked nodes, or else the selected one, into a folder here, up one level, or to the root of the space',
    complete: folderNames,
    run: async (arg) => {
      const node = workspace.tree.marked[0] ?? workspace.target
      if (!node) return workspace.say('nothing selected', 'error')
      const target = arg.trim()
      if (!target) return workspace.say('usage: :mv <folder|..|/>', 'error')

      // `..` is up from where the node is, which in a search result is not
      // the folder being browsed.
      const up = async () => {
        if (node.parent_id === null) return undefined
        const parent = await api.getNode(node.parent_id)
        return parent.kind === 'space' ? undefined : (parent.parent_id ?? undefined)
      }
      const destination =
        target === '..' ? await up() : target === '/' ? (workspace.tree.spaceId ?? undefined) : findFolder(target)?.id
      if (destination === undefined) return workspace.say(`no such folder: ${target}`, 'error')
      if (destination === node.id) return workspace.say('a folder cannot hold itself', 'error')
      if (workspace.tree.marked.length) return workspace.guard(() => workspace.tree.moveNodes(workspace.tree.marked, destination))
      await workspace.guard(() => workspace.tree.moveNode(node.id, destination))
    },
  },
  {
    name: 'new',
    aliases: ['n', 'o'],
    args: '[template] [yyyy-mm-dd] [name]',
    group: 'documents',
    essential: true,
    help: "start a new document here — from one of the space's templates, dated, named; in the diary from templates/day",
    complete: (arg) => (arg.includes(' ') ? [] : templateNames()),
    run: async (arg) => {
      await workspace.guard(async () => {
        const words = arg.trim().split(/\s+/).filter(Boolean)
        const here = workspace.tree.spaceId
        const templates = words.length && here !== null ? await api.templates(here) : []
        if (words.length && here !== null) templateCache = { space: here, names: templates.map((t) => t.slug) }
        const wanted = words[0]?.toLowerCase()
        const template = templates.find((t) => t.slug === wanted || t.name.toLowerCase() === wanted)
        if (template) words.shift()
        let created_at: number | undefined
        if (words.length && /^\d{4}-\d{2}-\d{2}$/.test(words[0])) {
          const at = parseDay(words[0])
          if (at === null) return workspace.say(`not a date: ${words[0]}`, 'error')
          created_at = at
          words.shift()
        }
        await workspace.tree.createDocument(words.join(' '), { created_at, template_id: template?.id })
      })
      hooks.focusEditor()
    },
  },
  {
    name: 'cal',
    aliases: ['calendar'],
    group: 'documents',
    help: 'the diary by month — h/l a day, j/k a week, H/L a month, Enter opens the day or starts it',
    run: () =>
      workspace.guard(async () => {
        if (workspace.tiles.view('calendar')) return workspace.tiles.show('calendar')
        await workspace.diary.openCalendar()
      }),
  },
  {
    name: 'agenda',
    aliases: ['ag'],
    group: 'board',
    help: 'what is late, due this week, or in hand — across every board',
    run: () =>
      workspace.guard(async () => {
        if (!workspace.tiles.view('agenda')) workspace.cards.agendaCursor = 0
        await workspace.tiles.show('agenda')
      }),
  },
  {
    name: 'today',
    args: '[text]',
    group: 'documents',
    help: "open today's diary entry, starting one if there is none yet; with text, a timed line at its end without going there",
    run: async (arg) => {
      if (arg.trim()) return workspace.guard(() => workspace.diary.logToday(arg.trim()))
      await workspace.guard(() => workspace.diary.openDay())
      hooks.focusEditor()
    },
  },
  {
    name: 'edit',
    aliases: ['e'],
    args: '[id]',
    group: 'editing',
    help: 'edit the open document, or open a node by id',
    run: async (arg) => {
      const node = arg ? undefined : workspace.target
      if (node?.kind === 'file') {
        return workspace.say(`${node.slug} is a file — nothing to write in, :name renames it`, 'error')
      }
      if (node && node.kind !== 'document') {
        return workspace.say(`${node.slug} is a ${node.kind} — l opens it`, 'error')
      }
      const id = arg ? Number(arg) : node?.id
      if (!id || Number.isNaN(id)) return workspace.say('usage: :e <id>', 'error')
      await workspace.guard(() => workspace.tree.openNode(id, true))
      hooks.focusEditor()
    },
    bang: {
      help: 'throw away the draft and re-read the document from the server',
      run: async () => {
        const open = requireOpen()
        if (!open) return
        workspace.dirty = false
        workspace.drafts.dropStash(open.id)
        await workspace.guard(() => workspace.tree.openNode(open.id, workspace.editing))
        workspace.say(`${open.slug} reloaded`)
      },
    },
  },
  {
    name: 'next',
    group: 'documents',
    help: 'open the next node down the list',
    run: () => jumpTo(workspace.tree.cursor + 1),
  },
  {
    name: 'prev',
    group: 'documents',
    help: 'open the previous node',
    run: () => jumpTo(workspace.tree.cursor - 1),
  },
  {
    name: 'close',
    aliases: ['clo'],
    group: 'tiles',
    help: 'close the focused tile — Alt-q; asks when its draft could not be written',
    run: () =>
      workspace.guard(async () => {
        await workspace.drafts.flushTile()
        if (workspace.dirty) {
          const name = workspace.open?.slug ?? 'the draft'
          if (!(await workspace.prompts.ask(`${name} could not be written — close anyway? the draft stays in this browser`))) return
        }
        await focusTile(closeFocused())
      }),
  },
  {
    name: 'only',
    aliases: ['on'],
    group: 'tiles',
    help: 'close every tile but the focused one',
    run: () => workspace.guard(() => workspace.tiles.only()),
  },
  {
    name: 'ls',
    aliases: ['buffers', 'files'],
    group: 'tiles',
    help: 'list the open tiles, % on the focused one',
    run: () => workspace.say(workspace.tiles.all.map((_, index) => describeTile(index)).join(' · ')),
  },
  {
    name: 'buffer',
    aliases: ['b'],
    args: '<n|name>',
    group: 'tiles',
    help: 'focus a tile by its number in :ls, or by the start of its name',
    complete: () => workspace.tiles.all.flatMap((tile) => (tile.kind !== 'node' ? [tile.kind] : tile.open ? [tile.open.slug] : [])),
    run: (arg) => {
      const wanted = arg.trim().toLowerCase()
      if (!wanted) return workspace.say(describeTile(workspace.tiles.focused))
      const index = /^\d+$/.test(wanted)
        ? Number(wanted) - 1
        : workspace.tiles.all.findIndex((tile) =>
            tile.kind !== 'node'
              ? tile.kind.startsWith(wanted)
              : tile.open && (tile.open.slug.startsWith(wanted) || tile.open.name.toLowerCase().startsWith(wanted)),
          )
      if (!workspace.tiles.all[index]) return workspace.say(`E86: no tile ${arg.trim()} — :ls lists them`, 'error')
      return focusTile(index)
    },
  },
  {
    name: 'bnext',
    aliases: ['bn'],
    group: 'tiles',
    help: 'the next tile — gt',
    run: () => cycleTile(1),
  },
  {
    name: 'bprevious',
    aliases: ['bp', 'bN'],
    group: 'tiles',
    help: 'the previous tile — gT',
    run: () => cycleTile(-1),
  },
  {
    name: 'first',
    group: 'documents',
    help: 'open the first node in the list',
    run: () => jumpTo(0),
  },
  {
    name: 'last',
    group: 'documents',
    help: 'open the last node in the list',
    run: () => jumpTo(workspace.tree.rows - 1),
  },
  {
    name: 'random',
    group: 'documents',
    help: 'open something here at random — good for re-reading',
    run: () => jumpTo(Math.floor(Math.random() * workspace.tree.nodes.length)),
  },
  {
    name: 'delete',
    aliases: ['d', 'rm'],
    args: '[id]',
    group: 'documents',
    help: 'delete a node, and everything under it, into the trash (asks first) — u brings it back',
    run: async (arg) => {
      // On the board the thing under the cursor is a card, not a node.
      if (onBoard() && !arg) {
        const card = workspace.cards.card
        if (!card) return workspace.say('no card selected', 'error')
        if (!(await workspace.prompts.ask(`delete the card "${card.title}"?`))) return
        return workspace.guard(() => workspace.cards.deleteCard())
      }
      // Whatever has focus: the row under the cursor in the list, the open
      // document only while the document pane is the one being looked at.
      const id = arg ? Number(arg) : workspace.target?.id
      if (!id || Number.isNaN(id)) return workspace.say('nothing to delete', 'error')
      const node = workspace.open?.id === id ? workspace.open : workspace.tree.nodes.find((n) => n.id === id)
      const name = node ? `"${node.name.trim() || node.slug}"` : `#${id}`
      const what =
        node && isContainer(node)
          ? `delete the ${node.kind} ${name} and everything in it?`
          : `delete ${name}?`
      if (!(await workspace.prompts.ask(`${what} it goes to the trash.`))) return
      await workspace.guard(() => workspace.tree.deleteNode(id))
    },
  },
  {
    name: 'name',
    aliases: ['title', 'rename'],
    args: '<text>',
    group: 'editing',
    help: 'rename what has focus — document, folder, file, or card on a board; bare, while writing, into the title — :space name and :list name rename those',
    run: async (arg) => {
      if (!arg.trim() && workspace.editing && workspace.pane === 'document') return hooks.focusTitle()
      if (onBoard()) {
        if (!workspace.cards.card) return workspace.say('no card here — :list name renames the list', 'error')
        if (!arg.trim()) return workspace.say('usage: :name <text>', 'error')
        return workspace.guard(() => workspace.cards.patchCard({ title: arg.trim() }))
      }
      if (!arg.trim()) return workspace.say('usage: :name <text>', 'error')

      // The open document is renamed through the draft, so an unsaved body is
      // written with the new name instead of being left behind by it.
      const node = workspace.target
      if (node && node.id === workspace.open?.id) {
        workspace.draft.name = arg
        workspace.dirty = true
        return workspace.guard(() => workspace.drafts.save())
      }

      if (!node) return workspace.say('nothing selected — :space name renames the space', 'error')
      await workspace.guard(() => workspace.tree.renameNode(node.id, arg))
    },
  },
  {
    name: 'history',
    aliases: ['hist'],
    group: 'documents',
    help: "the open document's earlier versions as diffs — j/k, Enter brings one back (gh)",
    run: async () => {
      await openInfo('history')
    },
  },
  {
    name: 'backlinks',
    aliases: ['bl'],
    group: 'documents',
    help: 'the documents that link to the open one — j/k, Enter opens one (gb)',
    run: async () => {
      await openInfo('backlinks')
    },
  },
  {
    name: 'cards',
    group: 'documents',
    help: 'the cards about the open document — its own, and the ones whose summary links here: j/k picks the one :due, :label, Space and H/L act on, Enter turns it over on its board (gc)',
    run: () => openInfo('cards'),
  },
  {
    name: 'trash',
    args: '[purge]',
    group: 'tree',
    essential: true,
    help: 'what was deleted — j/k, Enter brings it back, x deletes it for good; :trash purge empties it',
    complete: () => ['purge'],
    run: async (arg) => {
      if (arg.trim() === 'purge') return emptyTrash()
      if (arg.trim()) return workspace.say('usage: :trash [purge]', 'error')
      await workspace.guard(async () => {
        await workspace.sheets.loadTrash()
        workspace.sheets.trashCursor = 0
        workspace.sheets.open('trash')
      })
    },
  },
  {
    name: 'date',
    args: '<yyyy-mm-dd>',
    group: 'editing',
    help: 're-date the open document',
    run: async (arg) => {
      if (!requireOpen()) return
      const at = parseDay(arg)
      if (at === null) return workspace.say('usage: :date 2026-09-05', 'error')
      workspace.draft.created_at = at
      workspace.dirty = true
      await workspace.guard(() => workspace.drafts.save())
    },
  },
  {
    name: 'share',
    args: '[off|new]',
    group: 'sharing',
    essential: true,
    help: 'publish the document behind an unguessable link and copy it — off takes it back, new mints a fresh link and the old one stops working; both ask first, and :info share shows where it stands',
    complete: () => ['off', 'new'],
    run: async (arg) => {
      const node = workspace.target
      if (!node) return workspace.say('nothing selected', 'error')
      const what = arg.trim()
      if (what === 'off') return node.shared ? toggleSharing(node) : workspace.say('not shared')
      if (what === 'new') return renewLink(node)
      if (what) return workspace.say('usage: :share [off|new]', 'error')
      if (node.shared) return workspace.say('already shared — :share new for a fresh link, :share off to take it back')
      await workspace.guard(() => workspace.tree.toggleShare(node.id))
    },
  },
  {
    name: 'copy',
    aliases: ['yank'],
    group: 'sharing',
    help: 'copy the whole document to the clipboard as markdown',
    run: async () => {
      if (!requireOpen()) return
      const { name, body } = workspace.draft
      const text = name.trim() ? `# ${name.trim()}\n\n${body}` : body
      workspace.say((await workspace.copy(text)) ? 'document copied' : 'the clipboard said no', 'info')
    },
  },
  {
    name: 'export',
    aliases: ['exp'],
    group: 'sharing',
    essential: true,
    help: 'download what has focus — the file under the cursor as it is, a document as a .md file with what is typed so far',
    run: () =>
      workspace.guard(async () => {
        const node = workspace.target
        if (!node) return workspace.say('nothing selected — :export! downloads everything', 'error')
        if (node.kind === 'document') return exportDocument(node)
        if (!node.file) return workspace.say(`a ${node.kind} goes with :export!, the whole workspace`, 'error')
        if (node.file.encrypted && !(await workspace.vault.ensureUnlocked())) return
        const anchor = document.createElement('a')
        anchor.href = node.file.url
        anchor.download = node.name
        anchor.click()
        workspace.say(`downloading ${node.name}`)
      }),
    bang: {
      help: 'download the whole workspace — every space, document and file — as a zip',
      run: async () => {
        // A plain navigation, so the browser streams the archive straight to
        // disk instead of the tab holding all of it in memory first.
        await workspace.drafts.flush()
        // Sealed files go into the archive opened, which needs the vault:
        // asked here, because a refusal after the navigation would replace
        // the whole app with an error page.
        await workspace.vault.load()
        if ((workspace.vault.status?.sealed ?? 0) > 0 && !(await workspace.vault.ensureUnlocked())) {
          return workspace.say('the export holds sealed files — it needs the vault open', 'error')
        }
        workspace.say('building the archive — the download starts on its own')
        location.href = '/api/export'
      },
    },
  },
  {
    name: 'source',
    group: 'documents',
    help: 'show an open markdown file as written, or rendered again',
    run: () => {
      if (!hooks.toggleSource()) workspace.say('only an open markdown file has a source to show', 'error')
    },
  },
  {
    name: 'upload',
    aliases: ['up'],
    args: '[seal]',
    group: 'editing',
    essential: true,
    help: 'files for the open document — while writing, embedded at the cursor — or, from the list, into this folder; seal puts them into this folder sealed in the vault, encrypted before they touch the disk',
    complete: () => ['seal'],
    run: (arg) => {
      const what = arg.trim()
      if (what === 'seal') {
        return workspace.guard(async () => {
          if (await workspace.vault.ensureUnlocked()) hooks.pickTreeFiles(true)
        })
      }
      if (what) return workspace.say('usage: :upload [seal]', 'error')
      return readingDocument() ? hooks.pickFiles() : hooks.pickTreeFiles(false)
    },
  },
  {
    name: 'outline',
    aliases: ['toc'],
    group: 'documents',
    help: "the open document's headings to pick from — gO, in the editor too",
    run: () => openInfo('outline'),
  },
  {
    name: 'info',
    args: '[outline|history|backlinks|cards|files|share]',
    group: 'documents',
    help: 'what is known about the open document: its headings, earlier versions, backlinks, cards, files and sharing — Tab between them (gi)',
    complete: () => infoSections,
    run: (arg) => {
      const section = arg.trim()
      if (section && !infoSections.includes(section as InfoSection)) return workspace.say('usage: :info [outline|history|backlinks|cards|files|share]', 'error')
      return openInfo((section || undefined) as InfoSection | undefined)
    },
  },
  {
    name: 'search',
    aliases: ['se'],
    args: '[text]',
    group: 'documents',
    help: 'full-text search in this space (empty clears)',
    run: (arg) => workspace.guard(() => workspace.tree.search(arg, false)),
    bang: {
      help: 'full-text search across every space — Ctrl-P',
      run: (arg) => workspace.guard(() => workspace.tree.search(arg, true)),
    },
  },
  {
    name: 'noh',
    aliases: ['nohlsearch'],
    group: 'documents',
    help: 'clear the search filter',
    run: () => workspace.guard(() => workspace.tree.search('')),
  },
  {
    name: 'reload',
    aliases: ['r'],
    group: 'documents',
    help: 're-read the current folder from the server',
    run: () =>
      workspace.guard(async () => {
        await workspace.tree.refresh()
        workspace.say(`${workspace.tree.nodes.length} here`)
      }),
  },
  {
    name: 'stats',
    group: 'view',
    help: 'word, line and node counts',
    run: () => {
      const body = workspace.open ? workspace.draft.body : ''
      const words = body.split(/\s+/).filter(Boolean).length
      const open = workspace.open ? `${workspace.open.slug} ${words}w ${body.split('\n').length}L · ` : ''
      workspace.say(
        `${open}${workspace.tree.here || '/'} · ${workspace.tree.nodes.length} node${workspace.tree.nodes.length === 1 ? '' : 's'}${
          workspace.tree.query ? ` matching "${workspace.tree.query}"` : ''
        }`,
      )
    },
  },
  {
    name: 'set',
    args: '[option]',
    group: 'view',
    help: 'vim, novim, clipboard, noclipboard, board, noboard — bare, the settings sheet',
    complete: () => ['vim', 'novim', 'clipboard', 'noclipboard', 'board', 'noboard'],
    run: (arg) => {
      const option = arg.trim()
      if (!option) return openSettings('options')
      if (option === 'vim') return workspace.setVim(true), workspace.say('vim keys on')
      if (option === 'novim') return workspace.setVim(false), workspace.say('vim keys off')
      if (option === 'clipboard')
        return workspace.setClipboard(true), workspace.say('yanks go to the system clipboard')
      if (option === 'noclipboard')
        return workspace.setClipboard(false), workspace.say('yanks stay in vim registers')
      // A board is a property of the space, not of this browser, so unlike the
      // options above this one goes to the server.
      if (option === 'board') return workspace.guard(() => workspace.cards.setBoard(true))
      if (option === 'noboard') return workspace.guard(() => workspace.cards.setBoard(false))
      workspace.say(`unknown option: ${option}`, 'error')
    },
  },
  {
    name: 'backup',
    group: 'view',
    args: '[login <proton-username> | logout]',
    help: 'the proton drive mirror, in the settings sheet — login sets it up, logout forgets its session',
    complete: () => ['login ', 'logout'],
    run: (arg) =>
      workspace.guard(async () => {
        const trimmed = arg.trim()
        if (trimmed === '') return openSettings('backup')
        const loginMatch = /^login\s+(\S+)$/.exec(trimmed)
        if (loginMatch) return protonLogin(loginMatch[1])
        if (trimmed === 'logout') {
          if (!(await workspace.prompts.ask('Forget the Proton Drive session? What is mirrored stays on Drive.'))) return
          await api.backupLogout()
          return workspace.say('proton drive session forgotten — backups are off')
        }
        workspace.say('usage: :backup [login <proton-username> | logout]', 'error')
      }),
    bang: {
      help: 'back up to proton drive now, rather than when the workspace falls quiet',
      run: () =>
        workspace.guard(async () => {
          // Whatever is in the editor belongs in the backup that was just
          // asked for, so it goes to the server before the mirror runs.
          await workspace.drafts.flush()
          workspace.say('backing up …')
          const status = await api.backupNow()
          if (status.last_error) return workspace.say(`backup failed: ${status.last_error}`, 'error')
          const last = status.last
          workspace.say(
            last
              ? `backed up · ${last.uploaded} file${last.uploaded === 1 ? '' : 's'} uploaded, ${last.skipped} unchanged`
              : 'proton drive backups are off — :backup login <proton-username> sets them up',
            last ? 'info' : 'error',
          )
        }),
    },
  },
  {
    name: 'sessions',
    args: '[rm <n>]',
    group: 'view',
    help: 'every browser signed in, in the settings sheet — rm signs the n-th out, and locks the vault',
    run: (arg) =>
      workspace.guard(async () => {
        const trimmed = arg.trim()
        if (trimmed === '' || trimmed === 'ls' || trimmed === 'list') return openSettings('sessions')
        const sessions = await api.sessions()
        const rmMatch = /^rm\s+(\d+)$/.exec(trimmed)
        if (!rmMatch) return workspace.say('usage: :sessions [rm <n>]', 'error')
        const target = sessions[Number(rmMatch[1]) - 1]
        if (!target) return workspace.say(`no session ${rmMatch[1]} — :sessions lists them`, 'error')
        if (target.current) return workspace.say('that is this browser — :logout ends it', 'error')
        await api.revokeSession(target.id)
        await workspace.vault.load()
        workspace.say(`signed out ${browserOf(target.user_agent)}, seen ${relative(target.last_seen)} — the vault is locked`)
      }),
  },
  {
    name: 'sshkey',
    args: '[add <name> <key> | rm <name> | vault <name> on|off]',
    group: 'view',
    help: 'the keys allowed to mount the workspace over sftp, in the settings sheet — add, rm, and vault on|off for which may read sealed files',
    complete: () => ['add ', 'rm ', 'vault '],
    run: (arg) =>
      workspace.guard(async () => {
        const trimmed = arg.trim()
        if (trimmed === '' || trimmed === 'ls' || trimmed === 'list') return openSettings('keys')
        const addMatch = /^add\s+(\S+)\s+(.+)$/.exec(trimmed)
        if (addMatch) {
          const [, name, key] = addMatch
          const added = await api.addSshKey(name, key)
          return workspace.say(`added ${added.name} (${added.fingerprint})`)
        }
        const rmMatch = /^rm\s+(\S+)$/.exec(trimmed)
        if (rmMatch) {
          const keys = await api.listSshKeys()
          const target = keys.find((k) => k.name === rmMatch[1])
          if (!target) return workspace.say(`no key named ${rmMatch[1]}`, 'error')
          await api.removeSshKey(target.id)
          return workspace.say(`removed ${target.name}`)
        }
        // A key reads sealed files over the mount only once given vault access.
        const vaultMatch = /^vault\s+(\S+)\s+(on|off)$/.exec(trimmed)
        if (vaultMatch) {
          const keys = await api.listSshKeys()
          const target = keys.find((k) => k.name === vaultMatch[1])
          if (!target) return workspace.say(`no key named ${vaultMatch[1]}`, 'error')
          const on = vaultMatch[2] === 'on'
          await api.setSshKeyVault(target.id, on)
          return workspace.say(
            on
              ? `${target.name} reads sealed files over the mount while the vault is open`
              : `${target.name} no longer reads sealed files`,
          )
        }
        workspace.say('usage: :sshkey [add <name> <key> | rm <name> | vault <name> on|off]', 'error')
      }),
  },
  {
    name: 'token',
    args: '[add <name> | rm <name>]',
    group: 'view',
    help: 'the tokens that capture into the workspace from outside it, in the settings sheet — add shows a new one once and copies it, rm retires one',
    complete: (arg) => {
      // Tab cannot wait for the server, so the names are fetched the first
      // time it is pressed here and offered from then on.
      if (tokenNames === null) {
        tokenNames = []
        void api.listTokens().then((tokens) => (tokenNames = tokens.map((t) => t.name)), () => (tokenNames = null))
      }
      return arg.startsWith('rm ') ? tokenNames.map((name) => `rm ${name}`) : ['add ', 'rm ']
    },
    run: (arg) =>
      workspace.guard(async () => {
        const trimmed = arg.trim()
        if (trimmed === '' || trimmed === 'ls' || trimmed === 'list') return openSettings('tokens')
        const addMatch = /^add\s+(.+)$/.exec(trimmed)
        if (addMatch) {
          const added = await api.addToken(addMatch[1].trim())
          tokenNames = null
          const copied = await workspace.copy(added.token)
          return workspace.say(
            `token ${added.name}: ${added.token} — ${copied ? 'copied; ' : ''}shown this once, the server keeps only its hash`,
            'info',
            true,
          )
        }
        const rmMatch = /^rm\s+(.+)$/.exec(trimmed)
        if (rmMatch) {
          const tokens = await api.listTokens()
          const target = tokens.find((t) => t.name === rmMatch[1].trim())
          if (!target) return workspace.say(`no token named ${rmMatch[1].trim()}`, 'error')
          await api.removeToken(target.id)
          tokenNames = null
          return workspace.say(`removed ${target.name} — it captures nothing from now on`)
        }
        workspace.say('usage: :token [add <name> | rm <name>]', 'error')
      }),
  },
  {
    name: 'vault',
    args: '[unlock|lock|passwd]',
    group: 'vault',
    help: 'the vault in the settings sheet, or, the first time, set its password — unlock opens it, lock closes it now, passwd changes the password and re-seals every file under a new key',
    complete: () => ['unlock', 'lock', 'passwd'],
    run: (arg) =>
      workspace.guard(async () => {
        const what = arg.trim()
        await workspace.vault.load()
        if (what === 'unlock') {
          if (workspace.vault.status?.unlocked) return workspace.say('the vault is already open')
          return workspace.vault.ensureUnlocked()
        }
        if (what === 'lock') {
          workspace.vault.status = await api.lockVault()
          if (workspace.open?.file?.encrypted) workspace.open = await api.getNode(workspace.open.id)
          return workspace.say('vault locked')
        }
        if (what === 'passwd') return changeVaultPassword()
        if (what) return workspace.say('usage: :vault [unlock|lock|passwd]', 'error')
        if (!workspace.vault.status?.configured) return setupVault()
        await openSettings('vault')
      }),
  },
  {
    name: 'encrypt',
    aliases: ['seal'],
    group: 'vault',
    help: 'seal the file under the cursor in the vault — a folder stays sealed: every file in it, and every file added later',
    run: () => sealTarget(true),
  },
  {
    name: 'decrypt',
    aliases: ['unseal'],
    group: 'vault',
    help: 'take the file under the cursor back out of the vault — on a sealed folder, unseal it and every file inside',
    run: () => sealTarget(false),
  },
  {
    name: 'help',
    aliases: ['h'],
    args: '[all]',
    group: 'view',
    essential: true,
    help: 'the keys to start with — ? again, or :help all, for every key and command',
    complete: () => ['all'],
    run: (arg) => {
      const all = arg.trim() === 'all'
      if (workspace.sheets.shown === 'help' && !all) return workspace.sheets.close()
      openHelp(all)
    },
  },
  {
    name: 'settings',
    args: '[options|vault|backup|sessions|keys|tokens]',
    group: 'view',
    essential: true,
    help: 'the settings sheet: vim and the clipboard, the vault, the backup, the browsers signed in, ssh keys and capture tokens — Tab between them (,)',
    complete: () => settingsSections,
    run: (arg) => {
      const section = arg.trim()
      if (section && !settingsSections.includes(section as SettingsSection)) {
        return workspace.say(`usage: :settings [${settingsSections.join('|')}]`, 'error')
      }
      return openSettings((section || undefined) as SettingsSection | undefined)
    },
  },
  {
    name: 'logout',
    group: 'view',
    help: 'end the session',
    run: () => workspace.guard(() => workspace.logout()),
    bang: {
      help: 'end every session, on every device — for a lost phone or a shared computer',
      run: () => workspace.guard(() => workspace.logout(true)),
    },
  },
]

/**
 * Old spellings, kept for one release so the fingers and notes that know
 * them still work: each runs as the line it is now. Help and Tab do not
 * offer them.
 */
const renamed: Record<string, string | ((arg: string) => unknown)> = {
  'space!': 'space new',
  rmspace: 'space rm',
  rms: 'space rm',
  'list!': 'list rm',
  'task!': 'task edit',
  'upload!': 'upload seal',
  'import!': 'import zip',
  unshare: 'share off',
  link: 'share new',
  clear: 'noh',
  download: 'export',
  dl: 'export',
  unlock: 'vault unlock',
  lock: 'vault lock',
  passwd: 'vault passwd',
  // On the board it renamed the list, elsewhere the space.
  'name!': (arg) => runCommand(`${onBoard() ? 'list' : 'space'} name ${arg}`),
  // It linked the card to the document under the cursor, read or not.
  'card!': (arg) => {
    const node = workspace.target
    if (node?.kind !== 'document') return workspace.say('no document to link — open one first', 'error')
    return cardAbout(node, arg)
  },
  unmark: () => workspace.tree.unmarkAll(),
}

const lookup = new Map<string, CommandSpec>()
for (const spec of commands) {
  lookup.set(spec.name, spec)
  for (const alias of spec.aliases ?? []) lookup.set(alias, spec)
}

/** Run an ex-command line such as `w`, `date 2026-01-01` or `set novim`. */
export async function runCommand(line: string): Promise<void> {
  const input = line.trim().replace(/^:/, '')
  if (!input) return

  if (input.startsWith('/')) {
    await workspace.guard(() => workspace.tree.search(input.slice(1)))
    return
  }

  const [head, ...rest] = input.split(/\s+/)
  const arg = rest.join(' ')

  const old = renamed[head]
  if (old !== undefined) {
    await (typeof old === 'string' ? runCommand(`${old} ${arg}`) : old(arg))
    return
  }

  // `:12` jumps to a node by id, the way `:12` jumps to a line in vim.
  if (/^\d+$/.test(head)) {
    await workspace.guard(() => workspace.tree.openNode(Number(head)))
    return
  }

  // `:q!`, `:quit!` — the bang is part of the command name, not an argument.
  const forced = head.endsWith('!')
  const spec = lookup.get(forced ? head.slice(0, -1) : head)
  if (!spec) {
    workspace.say(`E492: not an editor command: ${head}`, 'error')
    return
  }
  if (forced) {
    if (!spec.bang) return workspace.say(`E477: no ! allowed: ${head}`, 'error')
    await spec.bang.run(arg)
  } else await spec.run(arg)
  // What a line changed shows in the settings sheet it was typed over.
  if (workspace.sheets.shown === 'settings') await workspace.guard(() => loadSettings())
}

/**
 * What Tab offers for the argument of `line`, a command name and the start of
 * an argument: whatever the command's `complete` lists that begins that way.
 * Null when the name is not finished yet, or names no command.
 */
export function argCompletions(line: string): string[] | null {
  const match = /^:?(\S+)\s(.*)$/.exec(line)
  if (!match) return null
  const [, head, arg] = match
  if (head.endsWith('!')) return []
  const spec = lookup.get(head)
  if (!spec) return null
  const wanted = arg.toLowerCase()
  return (spec.complete?.(arg) ?? []).filter((candidate) => candidate.toLowerCase().startsWith(wanted) && candidate !== arg)
}

/** Command names for the `:` completion menu. */
export function completions(prefix: string): CommandSpec[] {
  const head = prefix.trim().replace(/^:/, '').split(/\s+/)[0] ?? ''
  if (!head) return commands
  return commands.filter(
    (spec) => spec.name.startsWith(head) || spec.aliases?.some((a) => a.startsWith(head)),
  )
}
