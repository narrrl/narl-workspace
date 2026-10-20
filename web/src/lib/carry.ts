/*
 * Rows dragged out of the list onto a folder row, a crumb or a space pill —
 * the pointer's way of `m` and `p`. A marked row carries every marked node,
 * any other row only itself. The nodes stay here; the drag carries a type of
 * its own, so files from the desktop, or text, are never taken for them.
 */
import type { Attachment } from 'svelte/attachments'
import type { NodeSummary } from './api'
import { workspace } from './store.svelte'

const NODES = 'application/x-workspace-nodes'

let carried: NodeSummary[] = []

/** `ondragstart` of a row. */
export function carry(event: DragEvent, node: NodeSummary) {
  if (!event.dataTransfer) return
  carried = workspace.isMarked(node.id) ? workspace.marked : [node]
  event.dataTransfer.setData(NODES, JSON.stringify(carried.map((n) => n.id)))
  event.dataTransfer.effectAllowed = 'move'
}

/** `ondragend` of a row: dropped or not, nothing is carried any more. */
export function release() {
  carried = []
}

/** Make an element a place the carried nodes can be dropped into `parent`. */
export function into(parent: number | null, where: string): Attachment<HTMLElement> {
  return (element) => {
    if (parent === null) return
    // A folder does not take itself, or a drag that is not of rows.
    const takes = (event: DragEvent) =>
      !!event.dataTransfer?.types.includes(NODES) && carried.length > 0 && !carried.some((n) => n.id === parent)
    const over = (event: DragEvent) => {
      if (!takes(event)) return
      event.preventDefault()
      event.stopPropagation()
      event.dataTransfer!.dropEffect = 'move'
      element.classList.add('carry-over')
    }
    const leave = () => element.classList.remove('carry-over')
    const drop = (event: DragEvent) => {
      leave()
      if (!takes(event)) return
      event.preventDefault()
      event.stopPropagation()
      const nodes = carried
      carried = []
      void workspace.guard(() => workspace.moveNodes(nodes, parent, where))
    }
    element.addEventListener('dragover', over)
    element.addEventListener('dragleave', leave)
    element.addEventListener('drop', drop)
    return () => {
      element.removeEventListener('dragover', over)
      element.removeEventListener('dragleave', leave)
      element.removeEventListener('drop', drop)
    }
  }
}
