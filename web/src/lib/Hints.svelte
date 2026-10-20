<script lang="ts">
  /*
   * `F`: a letter or two over every link and button in view, the way vimium
   * draws them — typing one does what a click on it would. The labels are
   * read off the page once and pinned where the targets were; anything that
   * moves the page under them puts them away rather than leaving them wrong.
   */
  import { followLink } from './commands'
  import { workspace } from './store.svelte'

  interface Props {
    root: HTMLElement
    close: () => void
  }

  const { root, close }: Props = $props()

  /* The home row first, then the rows above and below: the labels a hand on
     the keys reaches without looking. */
  const LETTERS = 'asdfghjklqweruiopzxcvnm'

  type Hint = { label: string; element: HTMLElement; x: number; y: number }

  /** Every target that is on screen inside the root, top to bottom. */
  function targets(): HTMLElement[] {
    const view = root.getBoundingClientRect()
    return Array.from(root.querySelectorAll<HTMLElement>('a[href], button, summary')).filter((element) => {
      const box = element.getBoundingClientRect()
      return (
        box.width > 0 &&
        box.height > 0 &&
        box.bottom > Math.max(view.top, 0) &&
        box.top < Math.min(view.bottom, innerHeight) &&
        box.right > view.left &&
        box.left < view.right
      )
    })
  }

  /* Labels all of one length, so none is the start of another and a label is
     done the moment it is typed. */
  function labels(count: number): string[] {
    let length = 1
    while (LETTERS.length ** length < count) length++
    return Array.from({ length: count }, (_, n) => {
      let label = ''
      for (let i = 0; i < length; i++) {
        label = LETTERS[n % LETTERS.length] + label
        n = Math.floor(n / LETTERS.length)
      }
      return label
    })
  }

  const found = targets()
  const names = labels(found.length)
  const hints: Hint[] = found.map((element, i) => {
    const box = element.getBoundingClientRect()
    return { label: names[i], element, x: box.left, y: box.top }
  })

  let typed = $state('')

  if (hints.length === 0) {
    workspace.say('nothing to follow on screen')
    queueMicrotask(() => close())
  }

  /** What a click would do — except that a link out of the workspace gets a tab of its own. */
  function activate(element: HTMLElement) {
    if (element instanceof HTMLAnchorElement) {
      const href = element.getAttribute('href') ?? ''
      if (href.startsWith('#') || element.target || element.hasAttribute('download')) element.click()
      else followLink(href)
      return
    }
    element.click()
  }

  $effect(() => {
    // Ahead of everything else, the editor and the window handler included,
    // as the `[y/N]` prompt takes its key.
    const onkey = (event: KeyboardEvent) => {
      if (['Shift', 'Control', 'Alt', 'Meta', 'CapsLock'].includes(event.key)) return
      event.preventDefault()
      event.stopPropagation()
      if (event.key === 'Backspace') {
        typed = typed.slice(0, -1)
        return
      }
      const next = typed + event.key.toLowerCase()
      const matching = hints.filter((hint) => hint.label.startsWith(next))
      if (event.key.length !== 1 || matching.length === 0) return close()
      if (matching.length === 1 && matching[0].label === next) {
        close()
        activate(matching[0].element)
        return
      }
      typed = next
    }
    window.addEventListener('keydown', onkey, true)
    window.addEventListener('scroll', close, true)
    window.addEventListener('resize', close)
    return () => {
      window.removeEventListener('keydown', onkey, true)
      window.removeEventListener('scroll', close, true)
      window.removeEventListener('resize', close)
    }
  })
</script>

<div class="hints" aria-hidden="true">
  {#each hints as hint (hint.label)}
    {#if hint.label.startsWith(typed)}
      <span class="hint" style="left: {hint.x}px; top: {hint.y}px"
        ><span class="typed">{typed}</span>{hint.label.slice(typed.length)}</span
      >
    {/if}
  {/each}
</div>

<style>
  .hints { position: fixed; inset: 0; z-index: 35; pointer-events: none; }
  .hint {
    position: absolute;
    transform: translate(-4px, -6px);
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--light);
    color: var(--ink);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    line-height: 1.3;
    text-transform: lowercase;
    box-shadow: 0 1px 6px color-mix(in oklab, var(--ink) 60%, transparent);
  }
  .typed { opacity: 0.4; }
</style>
