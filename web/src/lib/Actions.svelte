<script lang="ts">
  /*
   * What the keyboard does to the row under the cursor, as buttons: the way a
   * phone renames, moves, shares, seals or deletes a node. Every button runs
   * the same command a key would, on the same target, so the two can never
   * drift apart.
   */
  import { isContainer } from './api'
  import { hooks, runCommand } from './commands'
  import { workspace } from './store.svelte'

  const node = $derived(workspace.selected)

  type Action = { label: string; run: () => unknown; danger?: boolean }

  const actions = $derived.by((): Action[] => {
    if (!node) return []
    const sealed = node.file?.encrypted ?? node.sealed
    const holdsFiles = isContainer(node) || node.kind === 'file'
    const list: Action[] = [
      { label: isContainer(node) ? 'open' : 'read', run: () => workspace.guard(() => workspace.openSelected()) },
      { label: 'rename', run: () => hooks.openCommandLine(`:name ${node.name}`) },
      workspace.isMarked(node.id)
        ? { label: 'put it back — not moving', run: () => workspace.mark(node, false) }
        : { label: 'move — then open a folder and drop it there', run: () => workspace.mark(node, false) },
    ]
    if (workspace.marked.length && workspace.parentId !== null && !workspace.isMarked(node.id)) {
      const count = workspace.marked.length
      list.push({
        label: `drop the ${count === 1 ? 'marked node' : `${count} marked nodes`} here`,
        run: () => workspace.guard(() => workspace.drop()),
      })
    }
    if (node.kind === 'document') {
      list.push(
        node.shared
          ? { label: 'stop sharing', run: () => runCommand('unshare') }
          : { label: 'share — copies the link', run: () => runCommand('share') },
      )
      if (node.shared) list.push({ label: 'new share link — the old one stops working', run: () => runCommand('link') })
    }
    // Inside a sealed folder only that folder opens, as a whole.
    if (holdsFiles && node.kind !== 'space' && !(sealed && workspace.parentSealed)) {
      list.push(
        sealed
          ? { label: node.kind === 'file' ? 'take out of the vault' : 'unseal — take every file out', run: () => runCommand('decrypt') }
          : { label: node.kind === 'file' ? 'seal in the vault' : 'seal it — now and what lands later', run: () => runCommand('encrypt') },
      )
    }
    list.push({ label: 'delete', run: () => runCommand('delete'), danger: true })
    return list
  })

  function act(action: Action) {
    workspace.overlay = 'none'
    // The list is what the commands act on while it has the focus.
    workspace.pane = 'list'
    void action.run()
  }
</script>

<div class="sheet">
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">{node ? node.name.trim() || node.slug : 'nothing'}</span></span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>
  <ul>
    {#each actions as action (action.label)}
      <li><button class:danger={action.danger} onclick={() => act(action)}>{action.label}</button></li>
    {/each}
  </ul>
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 14px;
    width: min(100%, 48ch);
    max-height: 100%;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 18px 20px;
    animation: resolve var(--t-slow) var(--ease-snap) both;
  }
  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; min-width: 0; }
  .title { color: var(--light); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; overflow-y: auto; }
  li button {
    width: 100%;
    padding: 12px 14px;
    text-align: left;
    text-transform: none;
    letter-spacing: 0;
    font-family: var(--font-sans);
    font-size: 15px;
  }
  .danger { color: var(--alarm); }
</style>
