import { api, isContainer, type BackupStatus } from './api'
import { workspace } from './store.svelte'
import { formatDay, parseDay, parseDue, type ScrollAmount } from './util'

export type { ScrollAmount }

/** Filled in by App.svelte so ex-commands can reach the DOM-bound bits. */
export const hooks = {
  pickFiles: () => {},
  /** Files for the folder being browsed, rather than for the document. */
  pickTreeFiles: (_encrypt: boolean) => {},
  pickFolder: () => {},
  /** `root`: into the workspace itself, where each directory is a space. */
  pickArchive: (_root: boolean) => {},
  focusEditor: () => {},
  focusList: () => {},
  insertText: (_text: string) => {},
  /* Filled in while a document is on screen and not being edited: the reading
     pane owns the scrollback, so the list keys have somewhere to send it. */
  scrollReader: (_amount: ScrollAmount): boolean => false,
  /** How many space cards the overview fits in a row, for j/k to step by. */
  overviewColumns: (): number => 1,
  openOverview: () => {},
  openCommandLine: (_initial: string) => {},
}

/** Help groups the command list under these headings, in this order. */
export const groups = ['tree', 'board', 'documents', 'editing', 'sharing', 'vault', 'view'] as const
export type Group = (typeof groups)[number]

export interface CommandSpec {
  name: string
  aliases?: string[]
  args?: string
  group: Group
  help: string
  run: (arg: string) => void | Promise<void>
  /** The forcing variant, `:name!` — vim hands the bang over as an argument. */
  bang?: { help: string; run: (arg: string) => void | Promise<void> }
}

const requireOpen = () => {
  if (!workspace.open) {
    workspace.say('no document open', 'error')
    return null
  }
  return workspace.open
}

/** Move the cursor and follow it, so `:next` reads as well as `j` then `Enter`. */
const jumpTo = async (index: number) => {
  if (workspace.nodes.length === 0) return workspace.say('nothing here', 'error')
  workspace.cursor = Math.min(Math.max(index, 0), workspace.nodes.length - 1)
  await workspace.guard(() => workspace.openSelected())
}

/** A folder among the children currently listed, by slug or by name. */
const findFolder = (target: string) => {
  const wanted = target.toLowerCase()
  return workspace.nodes.find(
    (node) =>
      isContainer(node) && (node.slug === wanted || node.name.toLowerCase() === wanted),
  )
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
    if (isContainer(node)) {
      const verb = encrypt ? 'seal every file in' : 'take every file in'
      const tail = encrypt ? 'into the vault' : 'out of the vault'
      if (!(await workspace.ask(`${verb} ${node.kind} "${node.name || node.slug}" ${tail}?`))) return
    } else if (!encrypt) {
      const name = node.name || node.slug
      if (!(await workspace.ask(`store "${name}" unencrypted again, and in the readable files/ view on proton drive?`))) return
    }
    if (!(await workspace.ensureUnlocked())) return
    workspace.say(encrypt ? 'encrypting …' : 'decrypting …')
    await workspace.seal(node.id, encrypt)
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

const describeBackup = (status: BackupStatus) => {
  if (!status.configured) {
    return workspace.say('proton drive backups are off — run `narl-workspace proton-login` on the server', 'error')
  }
  if (status.last_error) {
    return workspace.say(`backup failed: ${status.last_error}`, 'error')
  }
  const where = status.device ? `${status.device} · ` : ''
  const state = status.running ? 'running now' : status.pending ? 'changes waiting' : 'up to date'
  workspace.say(`${where}${state} · last ${ago(status.last_success_at)}`)
}

/** A filename that survives a download folder: `2026-09-05-first-light.md`. */
const exportName = (title: string, at: number) => {
  const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')
  return `${formatDay(at)}${slug ? `-${slug}` : ''}.md`
}

export const commands: CommandSpec[] = [
  {
    name: 'write',
    aliases: ['w'],
    group: 'editing',
    help: 'save the open document',
    run: () => workspace.guard(() => workspace.save()),
    bang: { help: 'same as :w — the bang is accepted out of habit', run: () => workspace.guard(() => workspace.save()) },
  },
  {
    name: 'wq',
    aliases: ['x'],
    group: 'editing',
    help: 'save and leave insert/editor mode',
    run: async () => {
      await workspace.guard(() => workspace.save())
      workspace.editing = false
      hooks.focusList()
    },
    bang: {
      help: 'same as :wq — the bang is accepted out of habit',
      run: async () => {
        await workspace.guard(() => workspace.save())
        workspace.editing = false
        hooks.focusList()
      },
    },
  },
  {
    name: 'quit',
    aliases: ['q'],
    group: 'editing',
    help: 'close the editor, or the document when already reading',
    run: async () => {
      // A document `o` made and nothing was written in is not kept.
      if (await workspace.abandonIfBlank()) {
        if (workspace.returnTo === 'board') await workspace.guard(() => workspace.returnToBoard())
        return hooks.focusList()
      }
      if (workspace.dirty) {
        workspace.say('unsaved changes — :w to write, :q! to discard', 'error')
        return
      }
      if (workspace.editing) workspace.editing = false
      else if (workspace.returnTo === 'board') return void workspace.guard(() => workspace.returnToBoard())
      else {
        workspace.open = null
        workspace.pane = 'list'
      }
      hooks.focusList()
    },
    bang: {
      help: 'discard changes and close the editor',
      run: async () => {
        const open = workspace.open
        workspace.editing = false
        workspace.dirty = false
        if (open) {
          workspace.dropStash(open.id)
          await workspace.guard(() => workspace.openNode(open.id))
        }
        hooks.focusList()
      },
    },
  },
  {
    name: 'space',
    aliases: ['sp'],
    args: '[name]',
    group: 'tree',
    help: 'switch to a space, or list them',
    run: async (arg) => {
      const wanted = arg.trim().toLowerCase()
      if (!wanted) {
        return workspace.say(workspace.spaces.map((space) => space.slug).join(' · ') || 'no spaces')
      }
      const space = workspace.spaces.find(
        (candidate) => candidate.slug === wanted || candidate.name.toLowerCase() === wanted,
      )
      if (!space) return workspace.say(`no such space: ${wanted} — :space! ${wanted} makes it`, 'error')
      await workspace.guard(() => workspace.enterSpace(space.id))
    },
    bang: {
      help: 'create a space with that name and switch to it',
      run: async (arg) => {
        if (!arg.trim()) return workspace.say('usage: :space! <name>', 'error')
        await workspace.guard(() => workspace.createSpace(arg.trim()))
      },
    },
  },
  {
    name: 'mkdir',
    args: '<name>',
    group: 'tree',
    help: 'make a folder here',
    run: async (arg) => {
      if (!arg.trim()) return workspace.say('usage: :mkdir <name>', 'error')
      await workspace.guard(() => workspace.createFolder(arg.trim()))
    },
  },
  {
    name: 'cd',
    args: '<name|..|/>',
    group: 'tree',
    help: 'enter a folder, .. goes up, / back to the space',
    run: async (arg) => {
      const target = arg.trim()
      await workspace.guard(async () => {
        if (!target || target === '..') return workspace.up()
        if (target === '/') {
          const space = workspace.spaceId
          return space === null ? undefined : workspace.goTo(space)
        }
        const folder = findFolder(target)
        if (!folder) return workspace.say(`no such folder here: ${target}`, 'error')
        await workspace.goTo(folder.id)
      })
    },
  },
  {
    name: 'import',
    aliases: ['imp'],
    group: 'tree',
    help: 'upload a folder into this one — markdown becomes documents, links follow',
    run: () => hooks.pickFolder(),
    bang: {
      help: 'import loose files, or a .zip of a folder; :import! / pours an :export! back in, spaces and boards and all',
      run: (arg) => hooks.pickArchive(arg.trim() === '/'),
    },
  },
  {
    name: 'board',
    aliases: ['b'],
    group: 'board',
    help: "show this space's board, or go back to the tree",
    run: () => workspace.guard(() => workspace.toggleBoardView()),
  },
  {
    name: 'card',
    aliases: ['c'],
    args: '<title>',
    group: 'board',
    help: "add a card — on the board to the list under the cursor, elsewhere to this space's inbox",
    run: async (arg) => {
      if (!arg.trim()) return workspace.say('usage: :card <title>', 'error')
      await workspace.guard(() => workspace.createCard(arg.trim()))
    },
    bang: {
      help: 'add a card linked to the open document — Enter on it opens the document',
      run: async (arg) => {
        // On the board the tree is not on screen, so only the open document
        // can be meant; elsewhere it is whatever has focus.
        const node = workspace.view === 'board' ? workspace.open : workspace.target
        if (!node || node.kind !== 'document') {
          return workspace.say('no document to link — open one first', 'error')
        }
        const title = arg.trim() || node.name.trim() || node.slug
        await workspace.guard(() => workspace.createCard(title, node.id))
      },
    },
  },
  {
    name: 'due',
    args: '<when|->',
    group: 'board',
    help: 'due date of the card: today, tm, fri, +3d, +2w, 10-31, 2026-10-31 — - clears',
    run: async (arg) => {
      if (workspace.view !== 'board') return workspace.say('no board open — :board', 'error')
      const target = arg.trim()
      if (target === '-' || target === '') {
        return workspace.guard(() => workspace.patchCard({ due_at: null }))
      }
      const at = parseDue(target)
      if (at === null) return workspace.say(`not a date: ${target} — try fri, +3d or 2026-09-30`, 'error')
      await workspace.guard(() => workspace.patchCard({ due_at: at }))
      workspace.say(`due ${formatDay(at)}`)
    },
  },
  {
    name: 'done',
    group: 'board',
    help: 'move the selected card into the done list, or back out to the inbox',
    run: () => {
      if (workspace.view !== 'board') return workspace.say('no board open — :board', 'error')
      return workspace.guard(() => workspace.toggleDone())
    },
  },
  {
    name: 'list',
    args: '<name>',
    group: 'board',
    help: 'add a column to the board',
    run: async (arg) => {
      if (workspace.view !== 'board') return workspace.say('no board open — :board', 'error')
      if (!arg.trim()) return workspace.say('usage: :list <name>', 'error')
      await workspace.guard(() => workspace.createList(arg.trim()))
    },
    bang: {
      help: 'delete the column under the cursor and every card on it',
      run: async () => {
        if (workspace.view !== 'board') return workspace.say('no board open — :board', 'error')
        const list = workspace.list
        if (!list) return workspace.say('no list selected', 'error')
        const cards = list.cards.length
        if (!(await workspace.ask(`delete the list "${list.name}" and its ${cards} card${cards === 1 ? '' : 's'}?`))) return
        await workspace.guard(() => workspace.deleteList())
      },
    },
  },
  {
    name: 'move',
    aliases: ['mv'],
    args: '<folder|..>',
    group: 'tree',
    help: 'move the selected node into a folder here, or up one level',
    run: async (arg) => {
      const node = workspace.target
      if (!node) return workspace.say('nothing selected', 'error')
      const target = arg.trim()
      if (!target) return workspace.say('usage: :mv <folder|..>', 'error')

      const destination =
        target === '..' ? workspace.path.at(-2)?.id : findFolder(target)?.id
      if (destination === undefined) return workspace.say(`no such folder: ${target}`, 'error')
      if (destination === node.id) return workspace.say('a folder cannot hold itself', 'error')
      await workspace.guard(() => workspace.moveNode(node.id, destination))
    },
  },
  {
    name: 'new',
    aliases: ['n', 'o'],
    args: '[yyyy-mm-dd]',
    group: 'documents',
    help: 'start a new document here, optionally dated',
    run: async (arg) => {
      await workspace.guard(async () => {
        await workspace.createDocument()
        if (arg) {
          const at = parseDay(arg)
          if (at === null) return workspace.say(`not a date: ${arg}`, 'error')
          workspace.draft.created_at = at
          workspace.dirty = true
          await workspace.save()
        }
      })
      hooks.focusEditor()
    },
  },
  {
    name: 'agenda',
    aliases: ['ag'],
    group: 'board',
    help: 'what is late, due this week, or in hand — across every board',
    run: () =>
      workspace.guard(async () => {
        await workspace.loadAgenda()
        workspace.agendaCursor = 0
        workspace.overlay = workspace.overlay === 'agenda' ? 'none' : 'agenda'
      }),
  },
  {
    name: 'today',
    aliases: ['t'],
    group: 'documents',
    help: "open today's diary entry, starting one if there is none yet",
    run: async () => {
      await workspace.guard(async () => {
        // The diary is one space among several now, so :today goes there first
        // rather than writing the day into whatever folder was being browsed.
        const home = workspace.spaces.find((space) => space.role === 'diary')
        if (home && workspace.parentId !== home.id) await workspace.enterSpace(home.id)
        else if (workspace.query) await workspace.search('')

        const midnight = new Date()
        midnight.setHours(0, 0, 0, 0)
        const from = Math.floor(midnight.getTime() / 1000)
        const today = workspace.nodes.find(
          (node) =>
            node.kind === 'document' && node.created_at >= from && node.created_at < from + 86400,
        )
        if (today) await workspace.openNode(today.id, true)
        else await workspace.createDocument()
      })
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
      await workspace.guard(() => workspace.openNode(id, true))
      hooks.focusEditor()
    },
    bang: {
      help: 'throw away the draft and re-read the document from the server',
      run: async () => {
        const open = requireOpen()
        if (!open) return
        workspace.dirty = false
        workspace.dropStash(open.id)
        await workspace.guard(() => workspace.openNode(open.id, workspace.editing))
        workspace.say(`${open.slug} reloaded`)
      },
    },
  },
  {
    name: 'next',
    aliases: ['bn'],
    group: 'documents',
    help: 'open the next node down the list',
    run: () => jumpTo(workspace.cursor + 1),
  },
  {
    name: 'prev',
    aliases: ['bp'],
    group: 'documents',
    help: 'open the previous node',
    run: () => jumpTo(workspace.cursor - 1),
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
    run: () => jumpTo(workspace.nodes.length - 1),
  },
  {
    name: 'random',
    group: 'documents',
    help: 'open something here at random — good for re-reading',
    run: () => jumpTo(Math.floor(Math.random() * workspace.nodes.length)),
  },
  {
    name: 'delete',
    aliases: ['d', 'rm'],
    args: '[id]',
    group: 'documents',
    help: 'delete a node, and everything under it (asks first)',
    run: async (arg) => {
      // On the board the thing under the cursor is a card, not a node.
      if (workspace.view === 'board') {
        const card = workspace.card
        if (!card) return workspace.say('no card selected', 'error')
        if (!(await workspace.ask(`delete the card "${card.title}"?`))) return
        return workspace.guard(() => workspace.deleteCard())
      }
      // Whatever has focus: the row under the cursor in the list, the open
      // document only while the document pane is the one being looked at.
      const id = arg ? Number(arg) : workspace.target?.id
      if (!id || Number.isNaN(id)) return workspace.say('nothing to delete', 'error')
      const node = workspace.open?.id === id ? workspace.open : workspace.nodes.find((n) => n.id === id)
      const name = node ? `"${node.name.trim() || node.slug}"` : `#${id}`
      const what =
        node && isContainer(node)
          ? `delete the ${node.kind} ${name} and everything in it?`
          : `delete ${name}?`
      if (!(await workspace.ask(`${what} this cannot be undone.`))) return
      await workspace.guard(() => workspace.deleteNode(id))
    },
  },
  {
    name: 'name',
    aliases: ['title', 'rename'],
    args: '<text>',
    group: 'editing',
    help: 'rename what is under the cursor — document, folder, or card on a board',
    run: async (arg) => {
      if (workspace.view === 'board') {
        if (!arg.trim()) return workspace.say('usage: :name <text>', 'error')
        return workspace.guard(() => workspace.patchCard({ title: arg.trim() }))
      }
      if (!arg.trim()) return workspace.say('usage: :name <text>', 'error')

      // The open document is renamed through the draft, so an unsaved body is
      // written with the new name instead of being left behind by it.
      const node = workspace.target
      if (node && node.id === workspace.open?.id) {
        workspace.draft.name = arg
        workspace.dirty = true
        return workspace.guard(() => workspace.save())
      }

      if (!node) return workspace.say('nothing selected — :name! renames the space', 'error')
      await workspace.guard(() => workspace.renameNode(node.id, arg))
    },
    bang: {
      help: 'rename the space you are in — on the board, the list under the cursor',
      run: async (arg) => {
        if (workspace.view === 'board') {
          if (!arg.trim()) return workspace.say('usage: :name! <list name>', 'error')
          return workspace.guard(() => workspace.renameList(arg.trim()))
        }
        const space = workspace.space
        if (!space) return workspace.say('no space open', 'error')
        if (!arg.trim()) return workspace.say('usage: :name! <text>', 'error')
        await workspace.guard(() => workspace.renameNode(space.id, arg))
      },
    },
  },
  {
    name: 'rmspace',
    aliases: ['rms'],
    args: '[name]',
    group: 'tree',
    help: 'delete a space and everything in it (asks first)',
    run: async (arg) => {
      const wanted = arg.trim().toLowerCase()
      const space = wanted
        ? workspace.spaces.find((s) => s.slug === wanted || s.name.toLowerCase() === wanted)
        : workspace.space
      if (!space) return workspace.say(`no such space: ${wanted || '(none open)'}`, 'error')
      if (!(await workspace.ask(`delete the space "${space.name}" and everything in it? this cannot be undone.`))) {
        return
      }
      await workspace.guard(() => workspace.deleteSpace(space.id))
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
      await workspace.guard(() => workspace.save())
    },
  },
  {
    name: 'share',
    group: 'sharing',
    help: 'publish the document behind an unguessable link and copy it',
    run: async () => {
      const node = workspace.target
      const id = node?.id
      if (!id) return workspace.say('nothing selected', 'error')
      if (node?.shared) return workspace.say('already shared — :link for a new link, :unshare to revoke')
      await workspace.guard(() => workspace.toggleShare(id))
    },
  },
  {
    name: 'unshare',
    group: 'sharing',
    help: 'revoke the share link',
    run: async () => {
      const node = workspace.target
      const id = node?.id
      if (!id) return workspace.say('nothing selected', 'error')
      if (!node?.shared) return workspace.say('not shared')
      await workspace.guard(() => workspace.toggleShare(id))
    },
  },
  {
    name: 'link',
    group: 'sharing',
    help: 'mint a new share link for the document and copy it',
    run: async () => {
      const node = workspace.target
      const id = node?.id
      if (!id) return workspace.say('nothing selected', 'error')
      if (!node?.shared) return workspace.say('not shared — :share first', 'error')
      await workspace.guard(async () => {
        const url = await workspace.mintShareLink(id)
        workspace.say(`new link → ${url} (copied) — the previous one is dead`)
      })
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
    help: 'download the open document as a .md file',
    run: () => {
      if (!requireOpen()) return
      const { name, body, created_at } = workspace.draft
      const text = name.trim() ? `# ${name.trim()}\n\n${body}` : body
      const url = URL.createObjectURL(new Blob([text], { type: 'text/markdown' }))
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = exportName(name.trim(), created_at)
      anchor.click()
      URL.revokeObjectURL(url)
      workspace.say(`wrote ${anchor.download}`)
    },
    bang: {
      help: 'download the whole workspace — every space, document and file — as a zip',
      run: async () => {
        // A plain navigation, so the browser streams the archive straight to
        // disk instead of the tab holding all of it in memory first.
        await workspace.flush()
        workspace.say('building the archive — the download starts on its own')
        location.href = '/api/export'
      },
    },
  },
  {
    name: 'upload',
    aliases: ['up'],
    group: 'editing',
    help: 'while writing, attach files at the cursor — otherwise put files into this folder',
    run: () => (workspace.editing ? hooks.pickFiles() : hooks.pickTreeFiles(false)),
    bang: {
      help: 'put files into this folder sealed in the vault — encrypted before they touch the disk',
      run: () =>
        workspace.guard(async () => {
          if (await workspace.ensureUnlocked()) hooks.pickTreeFiles(true)
        }),
    },
  },
  {
    name: 'media',
    group: 'editing',
    help: 'browse everything you have uploaded',
    run: () => workspace.guard(async () => {
      await workspace.loadMedia()
      workspace.overlay = 'media'
    }),
  },
  {
    name: 'search',
    aliases: ['se'],
    args: '[text]',
    group: 'documents',
    help: 'full-text search in this space (empty clears)',
    run: (arg) => workspace.guard(() => workspace.search(arg, false)),
    bang: {
      help: 'full-text search across every space — Ctrl-P',
      run: (arg) => workspace.guard(() => workspace.search(arg, true)),
    },
  },
  {
    name: 'clear',
    aliases: ['noh'],
    group: 'documents',
    help: 'clear the search filter',
    run: () => workspace.guard(() => workspace.search('')),
  },
  {
    name: 'reload',
    aliases: ['r'],
    group: 'documents',
    help: 're-read the current folder from the server',
    run: () =>
      workspace.guard(async () => {
        await workspace.refresh()
        workspace.say(`${workspace.nodes.length} here`)
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
        `${open}${workspace.here || '/'} · ${workspace.nodes.length} node${workspace.nodes.length === 1 ? '' : 's'}${
          workspace.query ? ` matching "${workspace.query}"` : ''
        }`,
      )
    },
  },
  {
    name: 'set',
    args: '<option>',
    group: 'view',
    help: 'vim, novim, clipboard, noclipboard, board, noboard',
    run: (arg) => {
      const option = arg.trim()
      if (option === 'vim') return workspace.setVim(true), workspace.say('vim keys on')
      if (option === 'novim') return workspace.setVim(false), workspace.say('vim keys off')
      if (option === 'clipboard')
        return workspace.setClipboard(true), workspace.say('yanks go to the system clipboard')
      if (option === 'noclipboard')
        return workspace.setClipboard(false), workspace.say('yanks stay in vim registers')
      // A board is a property of the space, not of this browser, so unlike the
      // options above this one goes to the server.
      if (option === 'board') return workspace.guard(() => workspace.setBoard(true))
      if (option === 'noboard') return workspace.guard(() => workspace.setBoard(false))
      workspace.say(`unknown option: ${option}`, 'error')
    },
  },
  {
    name: 'backup',
    group: 'view',
    help: 'when the workspace was last mirrored to proton drive',
    run: () => workspace.guard(async () => describeBackup(await api.backupStatus())),
    bang: {
      help: 'back up to proton drive now, rather than when the workspace falls quiet',
      run: () =>
        workspace.guard(async () => {
          // Whatever is in the editor belongs in the backup that was just
          // asked for, so it goes to the server before the mirror runs.
          await workspace.flush()
          workspace.say('backing up …')
          const status = await api.backupNow()
          if (status.last_error) return workspace.say(`backup failed: ${status.last_error}`, 'error')
          const last = status.last
          workspace.say(
            last
              ? `backed up · ${last.uploaded} file${last.uploaded === 1 ? '' : 's'} uploaded, ${last.skipped} unchanged`
              : 'proton drive backups are off — run `narl-workspace proton-login` on the server',
            last ? 'info' : 'error',
          )
        }),
    },
  },
  {
    name: 'sshkey',
    args: '[add <name> <key> | rm <name>]',
    group: 'view',
    help: 'list, add or remove keys allowed to mount the workspace over sftp',
    run: (arg) =>
      workspace.guard(async () => {
        const trimmed = arg.trim()
        if (trimmed === '' || trimmed === 'ls' || trimmed === 'list') {
          const keys = await api.listSshKeys()
          if (keys.length === 0) return workspace.say('no keys registered — :sshkey add <name> <key>')
          return workspace.say(keys.map((k) => `${k.name} ${k.fingerprint}`).join(' · '))
        }
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
        workspace.say('usage: :sshkey [add <name> <key> | rm <name>]', 'error')
      }),
  },
  {
    name: 'vault',
    group: 'vault',
    help: 'whether the vault is open — or, the first time, set its password',
    run: () =>
      workspace.guard(async () => {
        await workspace.loadVault()
        const vault = workspace.vault
        if (vault?.configured) {
          const left = vault.locks_in ? ` · locks in ${Math.ceil(vault.locks_in / 60)}m unless used` : ''
          return workspace.say(vault.unlocked ? `vault open${left} — :lock closes it` : 'vault locked — :unlock opens it')
        }
        const password = await workspace.askSecret('new vault password, 10+ characters — it cannot be recovered')
        if (password === null) return
        const again = await workspace.askSecret('the same again')
        if (again === null) return
        if (again !== password) return workspace.say('the two passwords differ — nothing was set', 'error')
        workspace.vault = await api.setupVault(password)
        workspace.say('vault ready and open — :encrypt seals what is under the cursor, :upload! seals new files')
      }),
  },
  {
    name: 'unlock',
    group: 'vault',
    help: 'open the vault with its password, so sealed files can be read',
    run: () =>
      workspace.guard(async () => {
        await workspace.loadVault()
        if (workspace.vault?.unlocked) return workspace.say('the vault is already open')
        await workspace.ensureUnlocked()
      }),
  },
  {
    name: 'lock',
    group: 'vault',
    help: 'close the vault now — it also closes itself when left alone, and on logout',
    run: () =>
      workspace.guard(async () => {
        workspace.vault = await api.lockVault()
        if (workspace.open?.file?.encrypted) workspace.open = await api.getNode(workspace.open.id)
        workspace.say('vault locked')
      }),
  },
  {
    name: 'passwd',
    group: 'vault',
    help: "change the vault's password — no file is rewritten",
    run: () =>
      workspace.guard(async () => {
        const current = await workspace.askSecret('current vault password')
        if (current === null) return
        const password = await workspace.askSecret('new vault password, 10+ characters')
        if (password === null) return
        const again = await workspace.askSecret('the same again')
        if (again === null) return
        if (again !== password) return workspace.say('the two passwords differ — nothing was changed', 'error')
        workspace.vault = await api.changeVaultPassword(current, password)
        workspace.say('vault password changed')
      }),
  },
  {
    name: 'encrypt',
    aliases: ['seal'],
    group: 'vault',
    help: 'seal the file under the cursor in the vault — on a folder, every file inside it',
    run: () => sealTarget(true),
  },
  {
    name: 'decrypt',
    aliases: ['unseal'],
    group: 'vault',
    help: 'take the file under the cursor back out of the vault — on a folder, every file inside it',
    run: () => sealTarget(false),
  },
  {
    name: 'help',
    aliases: ['h'],
    group: 'view',
    help: 'show the key and command reference',
    run: () => {
      workspace.overlay = workspace.overlay === 'help' ? 'none' : 'help'
    },
  },
  {
    name: 'logout',
    group: 'view',
    help: 'end the session',
    run: () => workspace.guard(() => workspace.logout()),
  },
]

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
    await workspace.guard(() => workspace.search(input.slice(1)))
    return
  }

  const [head, ...rest] = input.split(/\s+/)
  const arg = rest.join(' ')

  // `:12` jumps to a node by id, the way `:12` jumps to a line in vim.
  if (/^\d+$/.test(head)) {
    await workspace.guard(() => workspace.openNode(Number(head)))
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
    return
  }
  await spec.run(arg)
}

/** Command names for the `:` completion menu. */
export function completions(prefix: string): CommandSpec[] {
  const head = prefix.trim().replace(/^:/, '').split(/\s+/)[0] ?? ''
  if (!head) return commands
  return commands.filter(
    (spec) => spec.name.startsWith(head) || spec.aliases?.some((a) => a.startsWith(head)),
  )
}
