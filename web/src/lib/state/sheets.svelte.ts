import { api, type Backlink, type Revision, type RevisionSummary, type SpaceOverview, type TrashItem } from '../api'
import { formatStamp, type Heading } from '../util'
import type { Workspace } from '../store.svelte'

/** What floats over the tiles; one at a time. */
export type Sheet = 'help' | 'overview' | 'actions' | 'verify' | 'info' | 'trash' | 'settings'
/** The info sheet's parts: what the open document holds, and what is around it. */
export type InfoSection = 'outline' | 'history' | 'backlinks' | 'cards' | 'files' | 'share'

/** What floats over the tiles, and what each sheet lists. */
export class Sheets {
  readonly ws: Workspace

  constructor(ws: Workspace) {
    this.ws = ws
  }

  /** The sheet over everything, if any; `openSheet` and `closeSheet` move it. */
  shown = $state<'none' | Sheet>('none')
  /** The part of the info sheet on show; it stays for the next time it opens. */
  infoSection = $state<InfoSection>('outline')

  /** `gO`: the headings of what is on screen, to jump between from the keyboard. */
  outline = $state.raw<Heading[]>([])
  outlineCursor = $state(0)

  /** What `:trash` lists: everything deleted, the last first. */
  trash = $state<TrashItem[]>([])
  trashCursor = $state(0)

  /** `:history`: the open document's earlier versions, and the one shown. */
  history = $state<RevisionSummary[]>([])
  historyCursor = $state(0)
  revision = $state.raw<Revision | null>(null)
  /** `:backlinks`: the documents that link to the open one. */
  backlinks = $state<Backlink[]>([])
  backlinksCursor = $state(0)
  /** The info sheet's cards: where its cursor is in the open document's. */
  cardsCursor = $state(0)
  /** The file picked in the info sheet's files: Enter opens it, `i` embeds it while writing. */
  filesCursor = $state(0)

  overview = $state<SpaceOverview[]>([])
  overviewCursor = $state(0)

  /** `:history`: the open document's earlier versions, the newest first. */
  async loadHistory() {
    const open = this.ws.open
    if (!open) return
    // What is typed and not yet written is part of what a revision is
    // compared with, and of what a restore must not lose.
    await this.ws.drafts.flushTile()
    this.history = await api.revisions(open.id)
    this.historyCursor = 0
    this.revision = null
    await this.showRevision(0)
  }

  /** Read the revision at `index`, for the overlay to compare with the draft. */
  async showRevision(index: number) {
    const open = this.ws.open
    const summary = this.history[index]
    this.historyCursor = index
    if (!open || !summary) return
    const revision = await api.revision(open.id, summary.id)
    // A later j may have moved on while this one was being read.
    if (this.history[this.historyCursor]?.id === revision.id) this.revision = revision
  }

  /**
   * Bring a revision back: it becomes the document again, and what it
   * replaces is kept as a revision of its own, so a restore is undone the
   * same way. The editor reads the draft when it opens, so this leaves it.
   */
  async restoreRevision(revision: Revision) {
    const tile = this.ws.tile
    if (!tile.open) return
    await this.ws.drafts.flushTile(tile)
    tile.editing = false
    tile.draft = { ...tile.draft, name: revision.name, body: revision.body }
    tile.dirty = true
    await this.ws.drafts.save({ quiet: true, restore: true, tile })
    this.close()
    this.ws.say(`restored the version from ${formatStamp(revision.saved_at)} — :history has the one it replaced`)
  }

  /** `:backlinks`: what links to the open document, the last written first. */
  async loadBacklinks() {
    const open = this.ws.open
    if (!open) return
    // A link typed and not yet written is not one the server knows about.
    await this.ws.drafts.flush()
    this.backlinks = await api.backlinks(open.id)
    this.backlinksCursor = 0
  }

  async loadTrash() {
    this.trash = await api.trash()
    this.trashCursor = Math.min(this.trashCursor, Math.max(this.trash.length - 1, 0))
  }

  /** Delete one thing in the trash for good, files and all. */
  async purge(id: number) {
    await api.purge(id)
    if (this.ws.tree.lastDeleted === id) this.ws.tree.lastDeleted = null
    await this.loadTrash()
  }

  async emptyTrash() {
    const { removed } = await api.emptyTrash()
    this.ws.tree.lastDeleted = null
    if (this.shown === 'trash') await this.loadTrash()
    this.ws.say(removed === 1 ? 'the trash is empty — 1 thing deleted for good' : `the trash is empty — ${removed} things deleted for good`)
  }

  async loadOverview() {
    this.overview = await api.overview()
  }

  open(sheet: Sheet) {
    this.shown = sheet
  }

  /** Esc, q, a click beside it: the sheet away. */
  close() {
    this.shown = 'none'
  }
}
