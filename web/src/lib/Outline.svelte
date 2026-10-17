<script lang="ts">
  /*
   * The headings of a rendered document, beside it, with the one being read
   * lit — the section tracking narl.io's bar does, turned on its side. It
   * reads the article after it renders, so it works on any markdown without
   * the renderer knowing about it.
   */
  import { slugify } from './util'

  interface Props {
    /** The rendered article. */
    root: HTMLElement | undefined
    /** What scrolls: the pane, or the page in the share view. */
    scroller: HTMLElement | undefined
    /** Anything that changes the rendered headings, to read them again. */
    source: string
  }

  const { root, scroller, source }: Props = $props()

  type Heading = { id: string; text: string; level: number }

  let headings = $state<Heading[]>([])
  let current = $state('')

  $effect(() => {
    void source
    if (!root || !scroller) return

    const seen = new Map<string, number>()
    const found: Heading[] = []
    const elements = Array.from(root.querySelectorAll<HTMLElement>('h2, h3'))
    for (const element of elements) {
      const text = element.textContent?.trim() ?? ''
      const base = slugify(text) || 'section'
      const n = (seen.get(base) ?? 0) + 1
      seen.set(base, n)
      element.id = n === 1 ? base : `${base}-${n}`
      found.push({ id: element.id, text, level: element.tagName === 'H2' ? 2 : 3 })
    }
    headings = found
    current = found[0]?.id ?? ''

    // The section whose heading last crossed the upper third is the one being
    // read; a band rather than a line, so a short section still registers.
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) current = entry.target.id
        }
      },
      { root: scroller, rootMargin: '-10% 0px -70% 0px' },
    )
    for (const element of elements) observer.observe(element)
    return () => observer.disconnect()
  })

  function jump(id: string) {
    root?.querySelector(`#${CSS.escape(id)}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    current = id
  }
</script>

{#if headings.length >= 2}
  <nav class="outline" aria-label="outline">
    <h2 class="label rule">outline</h2>
    <ol>
      {#each headings as heading (heading.id)}
        <li class:sub={heading.level === 3}>
          <button class:on={heading.id === current} onclick={() => jump(heading.id)}>
            <span class="marker"></span>
            <span class="text">{heading.text}</span>
          </button>
        </li>
      {/each}
    </ol>
  </nav>
{/if}

<style>
  .outline { font-size: 12px; }
  h2 { margin: 0 0 10px; font-size: 10px; font-weight: 400; }

  ol { list-style: none; margin: 0; padding: 0; }
  li.sub { padding-left: 15px; }

  button {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    padding: 3px 6px;
    border: 0;
    border-radius: 4px;
    text-align: left;
    font-size: 12px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--muted);
  }
  button:hover { background: transparent; color: var(--light); }
  button:active { transform: none; }
  button.on { color: var(--light); }
  button.on .marker { background: var(--light); }
  button:focus-visible { background: var(--light); color: var(--ink); }

  .marker { width: 5px; height: 5px; transform: translateY(-2px); }
  .text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
