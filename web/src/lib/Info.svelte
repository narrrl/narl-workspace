<script lang="ts" module>
  import type { Backlink, NodeSummary, RevisionSummary } from './api'
  import { workspace, type InfoSection } from './store.svelte'
  import { formatStamp } from './util'

  /*
   * What the keys in actions.ts do here too, so a key and a click are the
   * same thing. The info sheet holds what is known about the open document
   * besides its text: its headings, its earlier versions, what links to it,
   * and whether it is shared.
   */
  export const infoSections: InfoSection[] = ['outline', 'history', 'backlinks', 'share']

  /** Tab and Shift-Tab: the next part along, round the ends. */
  export function turnInfo(by: number) {
    const at = infoSections.indexOf(workspace.infoSection)
    workspace.infoSection = infoSections[(at + by + infoSections.length) % infoSections.length]
  }

  /** Enter on a heading: the sheet away, and the text scrolled there. */
  export function goHeading(index = workspace.outlineCursor) {
    const heading = workspace.outline[index]
    workspace.closeSheet()
    heading?.go()
  }

  /** What Enter and the button do: the shown revision becomes the document again. */
  export async function restoreShown() {
    const revision = workspace.revision
    if (!revision) return
    if (!(await workspace.ask(`bring back the version from ${formatStamp(revision.saved_at)}? the text now is kept in the history`))) return
    await workspace.guard(() => workspace.restoreRevision(revision))
  }

  export function moveHistory(by: number) {
    const last = Math.max(workspace.history.length - 1, 0)
    const index = Math.min(Math.max(workspace.historyCursor + by, 0), last)
    if (index !== workspace.historyCursor) void workspace.guard(() => workspace.showRevision(index))
  }

  /** Enter on a backlink: the linking document is opened. */
  export function openBacklink(link: Backlink) {
    workspace.closeSheet()
    return workspace.guard(() => workspace.openPath(link.path))
  }

  // Sharing is a key away; taking a link back from its readers asks first.
  export async function toggleSharing(node: NodeSummary | null | undefined) {
    if (!node) return workspace.say('nothing selected', 'error')
    if (node.shared && !(await workspace.ask(`stop sharing "${node.name.trim() || node.slug}"? the link stops working`))) return
    await workspace.guard(() => workspace.toggleShare(node.id))
  }

  // A new link retires the one readers have, so it asks first.
  export async function renewLink(node: NodeSummary | null | undefined) {
    if (!node) return workspace.say('nothing selected', 'error')
    if (!node.shared) return workspace.say('not shared — s shares it', 'error')
    if (!(await workspace.ask(`new share link for "${node.name.trim() || node.slug}"? the current one stops working`))) return
    await workspace.guard(() => workspace.renewShareLink(node.id))
  }

  const REASONS: Record<string, string> = {
    edit: 'saved over',
    force: 'written over by :w!',
    sftp: 'saved over the mount',
    restore: 'replaced by a restore',
  }
  const reason = (summary: RevisionSummary) => REASONS[summary.reason] ?? summary.reason
</script>

<script lang="ts">
  import Sheet from './Sheet.svelte'
  import { diffLines, folded } from './diff'
  import { relative } from './util'

  const doc = $derived(workspace.open)
  const section = $derived(workspace.infoSection)

  // The shallowest level in the document is the left edge, so a text that
  // starts at `##` is not indented by a level it never uses.
  const top = $derived(Math.min(...workspace.outline.map((heading) => heading.level)))

  // Against the text as it is now: what restoring would take out and put back.
  const rows = $derived(section === 'history' && workspace.revision ? folded(diffLines(workspace.draft.body, workspace.revision.body)) : [])
  const changed = $derived(rows.some((row) => row.kind === 'gone' || row.kind === 'added'))

  /*
   * Only the token half of a share link is recoverable — the key lives in the
   * fragment, and the server kept nothing but its hash — so the sheet shows
   * the link's shape and offers a fresh one rather than a copy of the old.
   */
  const sharePath = $derived(doc?.share_token ? `${location.origin}/s/${doc.share_token}#…` : null)

  const tabs = $derived<[InfoSection, string][]>([
    ['outline', `outline ${workspace.outline.length}`],
    ['history', `history ${workspace.history.length}`],
    ['backlinks', `linked from ${workspace.backlinks.length}`],
    ['share', doc?.shared ? 'shared' : 'private'],
  ])

  const cursor = $derived(
    { outline: workspace.outlineCursor, history: workspace.historyCursor, backlinks: workspace.backlinksCursor, share: undefined }[section],
  )
</script>

<Sheet title="info" detail={doc?.name.trim() || doc?.slug || ''} width={120} fill={section === 'history'} {cursor} keys={[section, 'info', 'sheet']}>
  {#snippet actions()}
    {#if section === 'history' && workspace.revision}
      <button onclick={restoreShown}>restore</button>
    {/if}
  {/snippet}

  <nav class="tabs">
    {#each tabs as [name, text] (name)}
      <button class:on={name === section} onclick={() => (workspace.infoSection = name)}>{text}</button>
    {/each}
  </nav>

  {#if section === 'outline'}
    <ol class="items">
      {#each workspace.outline as heading, index (index)}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <li
          class="item"
          class:at={index === workspace.outlineCursor}
          style:padding-left="{8 + (heading.level - top) * 18}px"
          data-index={index}
          onclick={() => goHeading(index)}
          onpointerenter={() => (workspace.outlineCursor = index)}
        >
          <span class="name">{heading.text}</span>
        </li>
      {:else}
        <li class="muted">no headings in this document — a line that starts with # is one.</li>
      {/each}
    </ol>
  {:else if section === 'history'}
    <div class="split">
      <ul class="items versions">
        {#each workspace.history as summary, index (summary.id)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
          <li class="item" class:at={index === workspace.historyCursor} data-index={index} onclick={() => workspace.guard(() => workspace.showRevision(index))}>
            <span class="when">{formatStamp(summary.saved_at)}</span>
            <span class="muted">{relative(summary.saved_at)} · {reason(summary)} · {summary.size} chars</span>
          </li>
        {:else}
          <li class="muted">no earlier versions yet — one is kept when a save replaces the text.</li>
        {/each}
      </ul>

      <div class="diff">
        {#if workspace.revision}
          {#if workspace.revision.name !== workspace.draft.name}
            <div class="row gone">- # {workspace.draft.name}</div>
            <div class="row added">+ # {workspace.revision.name}</div>
          {/if}
          {#each rows as row, index (index)}
            {#if row.kind === 'fold'}
              <div class="row fold">··· {row.count} unchanged</div>
            {:else}
              <div class="row {row.kind}">{row.kind === 'gone' ? '-' : row.kind === 'added' ? '+' : ' '} {row.text}</div>
            {/if}
          {/each}
          {#if !changed}
            <div class="row fold">the text is the same as now</div>
          {/if}
        {/if}
      </div>
    </div>
  {:else if section === 'backlinks'}
    <ul class="items">
      {#each workspace.backlinks as link, index (link.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <li
          class="item"
          class:at={index === workspace.backlinksCursor}
          data-index={index}
          onpointerenter={() => (workspace.backlinksCursor = index)}
          onclick={() => openBacklink(link)}
        >
          <span class="name">{link.name.trim() || link.slug}</span>
          <span class="muted">{link.path}</span>
          {#if link.context}<span class="context">{link.context}</span>{/if}
        </li>
      {:else}
        <li class="muted">nothing links here yet — a link is [text](/n/…) or [[title]].</li>
      {/each}
    </ul>
  {:else}
    <div class="share">
      {#if sharePath}
        <div class="address">
          <span class="ring"></span>
          <span class="label public">public</span>
          <span class="link">{sharePath}</span>
        </div>
        <p class="muted">anyone with the link can read it. the link exists in full only when it is made — a new one is copied, and the old one stops working.</p>
        <div class="buttons">
          <button onclick={() => renewLink(doc)}>new link</button>
          <button class="danger" onclick={() => toggleSharing(doc)}>stop sharing</button>
        </div>
      {:else}
        <p class="muted">private — only you can read it. sharing makes a link that cannot be guessed, and copies it.</p>
        <div class="buttons">
          <button onclick={() => toggleSharing(doc)}>share</button>
        </div>
      {/if}
    </div>
  {/if}

  {#snippet footer()}
    {#if section === 'history'}<span class="muted">- goes, + comes back</span>{/if}
  {/snippet}
</Sheet>

<style>
  .tabs { display: flex; flex-wrap: wrap; gap: 4px; flex: none; }
  .tabs button { text-transform: none; letter-spacing: 0; }
  .tabs .on { background: var(--light); border-color: var(--light); color: var(--ink); }

  .split { display: flex; gap: 18px; min-height: 0; flex: 1; }
  .versions { width: 34ch; flex: none; }
  .when { color: var(--fg); font-size: 13px; }

  .diff {
    flex: 1;
    min-width: 0;
    overflow: auto;
    border: 1px solid var(--rim);
    border-radius: var(--radius-s);
    padding: 8px 0;
    font-size: 12px;
  }
  .row { white-space: pre-wrap; overflow-wrap: anywhere; padding: 0 10px; }
  .row.gone { background: color-mix(in oklab, var(--alarm) 16%, transparent); }
  .row.added { background: color-mix(in oklab, var(--light) 14%, transparent); }
  .row.fold { color: var(--muted); }

  .context { font-size: 12px; color: var(--subtle); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .share { display: flex; flex-direction: column; gap: 12px; font-size: 12px; }
  .share p { margin: 0; }
  .address {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 6px 12px;
    border: 1px solid var(--rim);
    border-radius: var(--radius-m);
    background: var(--surface);
    overflow-x: auto;
    white-space: nowrap;
  }
  .ring { width: 8px; height: 8px; flex: none; border: 1.5px solid var(--light); border-radius: 999px; }
  .public { color: var(--light); }
  .address .link { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; color: var(--muted); }
  .buttons { display: flex; gap: 6px; }

  @media (max-width: 767px) {
    .split { flex-direction: column; }
    .versions { width: auto; max-height: 30%; }
  }
</style>
