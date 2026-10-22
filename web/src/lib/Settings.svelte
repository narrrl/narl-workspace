<script lang="ts" module>
  import { api, type ApiToken, type BackupStatus, type SignedIn, type SshKey } from './api'
  import { browserOf, describeBackup, describeVault, hooks, runCommand } from './commands'
  import { workspace } from './store.svelte'
  import { relative } from './util'

  /*
   * Everything about the workspace that is not in it: how the editor
   * behaves, the vault, the backup, who is signed in, and what may reach in
   * from outside. Each row runs the command a line would, so the sheet and
   * the line cannot drift apart, and the line still does all of it.
   */
  export type SettingsSection = 'options' | 'vault' | 'backup' | 'sessions' | 'keys' | 'tokens'
  export const settingsSections: SettingsSection[] = ['options', 'vault', 'backup', 'sessions', 'keys', 'tokens']

  /** What the sheet shows, as last read; the part on show stays for the next time. */
  export const settings = $state({
    section: 'options' as SettingsSection,
    cursor: 0,
    sessions: [] as SignedIn[],
    keys: [] as SshKey[],
    tokens: [] as ApiToken[],
    backup: null as BackupStatus | null,
  })

  export async function loadSettings() {
    const [sessions, keys, tokens, backup] = await Promise.all([
      api.sessions(),
      api.listSshKeys(),
      api.listTokens(),
      api.backupStatus(),
      workspace.vault.load(),
    ])
    Object.assign(settings, { sessions, keys, tokens, backup })
    settings.cursor = Math.min(settings.cursor, Math.max(settingRows().length - 1, 0))
  }

  /** The sheet, at `section` or where it was last left, read afresh. */
  export function openSettings(section?: SettingsSection) {
    return workspace.guard(async () => {
      if (section && section !== settings.section) {
        settings.section = section
        settings.cursor = 0
      }
      await loadSettings()
      workspace.sheets.open('settings')
    })
  }

  export interface Setting {
    name: string
    detail: string
    /** Enter, or a click on the row. */
    act?: () => unknown
    /** `x`, after a question. */
    remove?: () => unknown
  }

  const run = (line: string) => () => runCommand(line)
  const type = (line: string) => () => hooks.openCommandLine(line)
  const confirm = (question: string, line: string) => async () => {
    if (await workspace.prompts.ask(question)) await runCommand(line)
  }
  const used = (at: number | null) => (at === null ? 'never used' : `used ${relative(at)}`)

  /** The rows of a part of the sheet, from what was last read. */
  export function settingRows(section = settings.section): Setting[] {
    switch (section) {
      case 'options': {
        const space = workspace.tree.space
        return [
          {
            name: 'vim keys',
            detail: workspace.vimEnabled ? 'on — the editor reads vim' : 'off — a plain editor',
            act: run(workspace.vimEnabled ? 'set novim' : 'set vim'),
          },
          {
            name: 'clipboard',
            detail: workspace.clipboard ? 'yanks go to the system clipboard' : 'yanks stay in vim registers',
            act: run(workspace.clipboard ? 'set noclipboard' : 'set clipboard'),
          },
          ...(space
            ? [
                {
                  name: 'board',
                  detail: space.has_board ? `${space.slug} has a board` : `${space.slug} has none`,
                  act: run(space.has_board ? 'set noboard' : 'set board'),
                },
              ]
            : []),
        ]
      }
      case 'vault': {
        const vault = workspace.vault.status
        if (!vault?.configured) {
          return [{ name: 'set a password', detail: 'files sealed under it open only with it — it cannot be recovered', act: run('vault') }]
        }
        return [
          {
            name: vault.unlocked ? 'lock' : 'unlock',
            detail: `${describeVault()} · ${vault.sealed} sealed`,
            act: run(vault.unlocked ? 'vault lock' : 'vault unlock'),
          },
          { name: 'change the password', detail: 'every sealed file is re-sealed under a new key', act: run('vault passwd') },
        ]
      }
      case 'backup': {
        const status = settings.backup
        if (!status?.configured) {
          return [{ name: 'log in to proton drive', detail: 'mirror the workspace there once it falls quiet', act: type(':backup login ') }]
        }
        return [
          { name: 'back up now', detail: describeBackup(status), act: run('backup!') },
          { name: 'log out', detail: 'forget the session — what is mirrored stays on drive', act: run('backup logout') },
        ]
      }
      case 'sessions':
        return settings.sessions.map((session, index) => ({
          name: session.current ? 'this browser' : browserOf(session.user_agent),
          detail: `seen ${relative(session.last_seen)} · signed in ${relative(session.created_at)}`,
          remove: session.current
            ? undefined
            : confirm(`sign out ${browserOf(session.user_agent)}? the vault locks too`, `sessions rm ${index + 1}`),
        }))
      case 'keys':
        return [
          ...settings.keys.map((key) => ({
            name: key.name,
            detail: `${key.fingerprint} · ${key.vault ? 'reads sealed files' : 'no sealed files'} · ${used(key.last_used_at)}`,
            act: run(`sshkey vault ${key.name} ${key.vault ? 'off' : 'on'}`),
            remove: confirm(`remove the key ${key.name}? it mounts nothing from now on`, `sshkey rm ${key.name}`),
          })),
          { name: 'add a key', detail: ':sshkey add <name> <public key>', act: type(':sshkey add ') },
        ]
      case 'tokens':
        return [
          ...settings.tokens.map((token) => ({
            name: token.name,
            detail: `made ${relative(token.created_at)} · ${used(token.last_used_at)}`,
            remove: confirm(`retire the token ${token.name}? it captures nothing from now on`, `token rm ${token.name}`),
          })),
          { name: 'add a token', detail: 'shown once and copied — the server keeps only its hash', act: type(':token add ') },
        ]
    }
  }

  /** Tab and Shift-Tab: the next part along, round the ends. */
  export function turnSettings(by: number) {
    const at = settingsSections.indexOf(settings.section)
    settings.section = settingsSections[(at + by + settingsSections.length) % settingsSections.length]
    settings.cursor = 0
  }

  export function moveSettings(by: number) {
    settings.cursor = Math.min(Math.max(settings.cursor + by, 0), Math.max(settingRows().length - 1, 0))
  }

  export const actSetting = (index = settings.cursor) => void settingRows()[index]?.act?.()
  export const removeSetting = (index = settings.cursor) => void settingRows()[index]?.remove?.()
</script>

<script lang="ts">
  import Sheet from './Sheet.svelte'

  const rows = $derived(settingRows())

  const tabs = $derived<[SettingsSection, string][]>([
    ['options', 'options'],
    ['vault', workspace.vault.status?.configured ? `vault ${workspace.vault.status.unlocked ? 'open' : 'locked'}` : 'vault'],
    ['backup', settings.backup?.configured ? `backup${settings.backup.last_error ? ' failing' : ''}` : 'backup off'],
    ['sessions', `sessions ${settings.sessions.length}`],
    ['keys', `ssh keys ${settings.keys.length}`],
    ['tokens', `tokens ${settings.tokens.length}`],
  ])

  const turn = (section: SettingsSection) => {
    settings.section = section
    settings.cursor = 0
  }
</script>

<Sheet title="settings" width={100} cursor={settings.cursor} keys={['settings', 'sheet']} hide={rows[settings.cursor]?.remove ? [] : ['remove']}>
  <nav class="tabs">
    {#each tabs as [name, text] (name)}
      <button class:on={name === settings.section} onclick={() => turn(name)}>{text}</button>
    {/each}
  </nav>

  <ul class="items">
    {#each rows as row, index (index)}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
      <li
        class="item"
        class:at={index === settings.cursor}
        data-index={index}
        onpointerenter={() => (settings.cursor = index)}
        onclick={() => actSetting(index)}
      >
        <span class="text">
          <span class="name">{row.name}</span>
          <span class="muted">{row.detail}</span>
        </span>
        {#if row.remove}
          <button class="danger" onclick={(event) => (event.stopPropagation(), removeSetting(index))}>remove</button>
        {/if}
      </li>
    {:else}
      <li class="muted">nothing here.</li>
    {/each}
  </ul>
</Sheet>

<style>
  /* A row with a button beside its two lines. */
  .items .item { flex-direction: row; align-items: center; gap: 12px; }
  .text { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0; }
</style>
