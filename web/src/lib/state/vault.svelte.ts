import { api, type VaultStatus } from '../api'
import type { Workspace } from '../store.svelte'

/** The vault, as last heard from the server, and opening it when something needs it. */
export class Vault {
  readonly ws: Workspace

  constructor(ws: Workspace) {
    this.ws = ws
  }

  /** The vault, as last heard from the server; null until asked. */
  status = $state<VaultStatus | null>(null)

  async load() {
    this.status = await api.vaultStatus()
  }

  /**
   * Whatever needs the vault asks for the password itself rather than failing
   * with "locked": opening it is the obvious next step anyway. False when
   * there is no vault, or the question was declined.
   */
  async ensureUnlocked(): Promise<boolean> {
    await this.load()
    if (!this.status?.configured) {
      this.ws.say('there is no vault yet — :vault sets one up', 'error')
      return false
    }
    if (this.status.unlocked) return true
    const password = await this.ws.prompts.askSecret('vault password')
    if (password === null) return false
    // The key is stretched on purpose, which takes a moment worth saying.
    this.ws.say('unlocking …')
    this.status = await api.unlockVault(password)
    this.ws.say('vault unlocked')
    return true
  }

  /** Seal what is under the cursor, or take it back out, and show the result. */
  async seal(id: number, encrypt: boolean) {
    const { files, unchanged, failed, shared, sealed } = await api.seal(id, encrypt)
    const verb = encrypt ? 'encrypted' : 'decrypted'
    const already = unchanged > 0 ? `, ${unchanged} already were` : ''
    const broken = failed > 0 ? `, ${failed} could not be` : ''
    const later = sealed ? ' — what lands in the folder from now on is sealed too' : ''
    const readers =
      shared > 0 ? ` — ${shared} shared document${shared === 1 ? '' : 's'} embed it, and their readers no longer see it` : ''
    this.ws.say(
      `${verb} ${files} file${files === 1 ? '' : 's'}${already}${broken}${later}${readers}`,
      failed > 0 || shared > 0 ? 'error' : undefined,
    )
    await this.ws.tree.refresh()
    for (const tile of this.ws.tiles.every) if (tile.open?.kind === 'file') tile.open = await api.getNode(tile.open.id)
  }
}
