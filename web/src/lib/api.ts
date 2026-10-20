/** What a node is: a top-level space, a folder inside one, a document, or a file. */
export type NodeKind = 'space' | 'folder' | 'document' | 'file'

/** Whether a node can hold other nodes — the ones `l` walks into. */
export const isContainer = (node: { kind: NodeKind }) => node.kind === 'space' || node.kind === 'folder'

/** The bytes behind a file node. */
export interface FileInfo {
  media_id: string
  mime: string
  size: number
  url: string
  /** Sealed in the vault: it opens only while the vault is unlocked. */
  encrypted: boolean
}

export interface NodeSummary {
  id: number
  parent_id: number | null
  kind: NodeKind
  name: string
  slug: string
  excerpt: string
  position: number
  has_board: boolean
  /** Spaces the app has to tell apart: `diary`, or nothing. */
  role: string | null
  child_count: number
  created_at: number
  updated_at: number
  shared: boolean
  share_token: string | null
  /** Folders and spaces only: whatever lands here is sealed in the vault. */
  sealed: boolean
  /** Files only. */
  file: FileInfo | null
  /** Search results only: where the match sits, `work/notes`. */
  location?: string
}

/** One step of a node's path, the space first and the node itself last. */
export interface Crumb {
  id: number
  kind: NodeKind
  name: string
  slug: string
}

export interface WorkspaceNode extends NodeSummary {
  body: string
  path: Crumb[]
}

/** A space as the overview shows it: itself, and what was last written in it. */
export interface SpaceOverview {
  space: NodeSummary
  recent: NodeSummary[]
}

/** A browser that is signed in. */
export interface SignedIn {
  id: string
  created_at: number
  last_seen: number
  user_agent: string
  /** The session asking. */
  current: boolean
}

/** A public key allowed to mount the workspace over SFTP. */
export interface SshKey {
  id: number
  name: string
  fingerprint: string
  created_at: number
  last_used_at: number | null
  /** May read sealed files over the mount while the vault is open. */
  vault: boolean
}

export interface SharedDocument {
  name: string
  body: string
  created_at: number
  updated_at: number
  token: string
}

/** What an import did, as `POST /api/nodes/{id}/import` reports it. */
export interface ImportReport {
  /** New spaces, made by an import at the root. */
  spaces: number
  folders: number
  documents: number
  files: number
  links: number
  cards: number
  skipped: string[]
}

/** A card on a space's board. `node_id` is the document it is about, if any. */
export interface Card {
  id: number
  list_id: number
  title: string
  body: string
  node_id: number | null
  node_name: string | null
  due_at: number | null
  done_at: number | null
  position: number
  created_at: number
  updated_at: number
}

/** An open card as the agenda lists it, with the board it lives on. */
export interface AgendaCard extends Card {
  space_id: number
  space_slug: string
  list_name: string
  role: 'inbox' | 'doing'
}

export interface Agenda {
  cards: AgendaCard[]
  /** Whether the diary has an entry today; null when there is no diary. */
  diary_today: boolean | null
}

export interface BoardList {
  id: number
  name: string
  position: number
  cards: Card[]
}

/** One space's board. There is one per space; the diary has none. */
export interface Board {
  space_id: number
  space_name: string
  lists: BoardList[]
}

export interface MediaFile {
  id: string
  filename: string
  mime: string
  size: number
  created_at: number
  url: string
  encrypted: boolean
  /** Every document that embeds this file — a file may be used by several. */
  node_ids: number[]
  /** The file node it is, when it sits in the tree; deleted from there. */
  file_node: number | null
}

/** One thing a certificate, key or keystore file holds, as `/inspect` describes it. */
export type CertItem =
  | {
      kind: 'certificate'
      subject: string
      issuer: string
      serial: string
      not_before: number
      not_after: number
      /** `RSA 2048`, `EC P-256`, `Ed25519`. */
      key: string
      names: string[]
      usage: string[]
      ca: boolean
      sha256: string
      sha1: string
      label: string | null
    }
  | { kind: 'request'; subject: string; key: string; names: string[] }
  | {
      kind: 'private_key'
      /** Null while it is still encrypted. */
      key: string | null
      encrypted: boolean
      /** The index of its certificate among the items. */
      matches: number | null
      label: string | null
    }
  | { kind: 'public_key'; key: string }

export interface Inspection {
  items: CertItem[]
  /** Something in the file is behind a password that was not given. */
  needs_password: boolean
}

/** The vault, as `/api/vault` reports it. */
export interface VaultStatus {
  configured: boolean
  unlocked: boolean
  /** Seconds until it locks itself if nothing uses it. */
  locks_in: number | null
  /** How many files are sealed: what an export needs the vault open for. */
  sealed: number
}

/** The Proton Drive mirror, as `/api/backup` reports it. */
export interface BackupStatus {
  configured: boolean
  device: string | null
  running: boolean
  pending: boolean
  last_run_at: number | null
  last_success_at: number | null
  last_error: string | null
  last: { uploaded: number; skipped: number; pruned: number; bytes: number } | null
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message)
  }
}

/** No answer at all: offline, or the server is down. Status 0, as XHR has it. */
export const unreachable = (error: unknown) =>
  error instanceof ApiError && (error.status === 0 || error.status === 502 || error.status === 503 || error.status === 504)

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  let response: Response
  try {
    response = await fetch(`/api${path}`, {
      credentials: 'same-origin',
      ...init,
      headers:
        init.body instanceof FormData
          ? init.headers
          : { 'content-type': 'application/json', ...init.headers },
    })
  } catch {
    // fetch says "Failed to fetch" and nothing else; this says what it means.
    throw new ApiError(0, 'the server cannot be reached')
  }

  if (!response.ok) {
    let message = response.statusText
    try {
      message = (await response.json()).error ?? message
    } catch {
      /* the body was not JSON; the status text will do */
    }
    throw new ApiError(response.status, message)
  }

  return response.status === 204 ? (undefined as T) : await response.json()
}

const body = (data: unknown) => JSON.stringify(data)

/** The size of a page of a long list, the most the server hands out at once. */
const PAGE = 2000

/** Every page of a list, read until one comes back short. */
async function pages<T>(read: (limit: number, offset: number) => Promise<T[]>): Promise<T[]> {
  const all: T[] = []
  for (;;) {
    const page = await read(PAGE, all.length)
    all.push(...page)
    if (page.length < PAGE) return all
  }
}

/**
 * Files up as multipart, by XHR rather than fetch, because a gigabyte going
 * up needs to say how far along it is.
 */
function send<T>(url: string, files: File[], progress?: (fraction: number) => void) {
  return new Promise<T>((resolve, reject) => {
    const form = new FormData()
    for (const file of files) form.append('file', file)
    const xhr = new XMLHttpRequest()
    xhr.open('POST', url)
    xhr.responseType = 'json'
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable) progress?.(event.loaded / event.total)
    }
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) resolve(xhr.response as T)
      else reject(new ApiError(xhr.status, xhr.response?.error ?? xhr.statusText))
    }
    xhr.onerror = () => reject(new ApiError(0, 'the upload did not reach the server'))
    xhr.send(form)
  })
}

export const api = {
  me: () => request<{ user: string }>('/me'),

  login: (username: string, password: string) =>
    request<{ user: string }>('/login', { method: 'POST', body: body({ username, password }) }),

  /** `everywhere`: every device's session ends, not only this one. */
  logout: (everywhere = false) =>
    request<{ ok: true }>(`/logout${everywhere ? '?everywhere=true' : ''}`, { method: 'POST' }),

  listSpaces: () => request<NodeSummary[]>('/spaces'),

  overview: () => request<SpaceOverview[]>('/spaces/overview'),

  createSpace: (name: string, has_board = true) =>
    request<WorkspaceNode>('/spaces', { method: 'POST', body: body({ name, has_board }) }),

  /**
   * The children of a node, or — with `q` — a search across the whole tree.
   * A folder is read a page at a time until a page comes back short, so one
   * that grows past a page is listed whole rather than cut off; a search
   * stops at its first page, the best matches.
   */
  listNodes: async (params: { parent?: number; q?: string; space?: number }) => {
    const query = new URLSearchParams()
    if (params.q) query.set('q', params.q)
    else if (params.parent !== undefined) query.set('parent', String(params.parent))
    if (params.q && params.space !== undefined) query.set('space', String(params.space))
    if (params.q) return request<NodeSummary[]>(`/nodes?${query}`)
    return pages((limit, offset) => {
      query.set('limit', String(limit))
      query.set('offset', String(offset))
      return request<NodeSummary[]>(`/nodes?${query}`)
    })
  },

  getNode: (id: number) => request<WorkspaceNode>(`/nodes/${id}`),

  /** `work/einarbeiten/01-was-ist-eebus` — the node behind a `/n/...` address. */
  resolvePath: (path: string) => request<WorkspaceNode>(`/resolve?path=${encodeURIComponent(path)}`),

  /**
   * Upload a folder into a node. The relative path is passed as each part's
   * file name, which is the only place a multipart body has to put it, and is
   * what the server rebuilds the directory tree from.
   */
  importFolder: (id: number, files: File[]) => {
    const form = new FormData()
    for (const file of files) form.append('file', file, file.webkitRelativePath || file.name)
    return request<ImportReport>(`/nodes/${id}/import`, { method: 'POST', body: form })
  },

  /** An `:export!` poured back in whole: its directories become spaces. */
  importRoot: (files: File[]) => {
    const form = new FormData()
    for (const file of files) form.append('file', file, file.webkitRelativePath || file.name)
    return request<ImportReport>('/import', { method: 'POST', body: form })
  },

  createNode: (input: {
    parent_id: number
    kind?: NodeKind
    name?: string
    body?: string
    created_at?: number
  }) => request<WorkspaceNode>('/nodes', { method: 'POST', body: body(input) }),

  /** `base_updated_at`: refused with 409 if the node was saved since then. */
  updateNode: (
    id: number,
    input: { name: string; body: string; created_at?: number; has_board?: boolean; base_updated_at?: number },
  ) => request<WorkspaceNode>(`/nodes/${id}`, { method: 'PUT', body: body(input) }),

  deleteNode: (id: number) =>
    request<{ ok: true; removed: number }>(`/nodes/${id}`, { method: 'DELETE' }),

  moveNode: (id: number, parent_id: number, position?: number) =>
    request<WorkspaceNode>(`/nodes/${id}/move`, { method: 'POST', body: body({ parent_id, position }) }),
  /** All of them into `parent_id`, or none when one cannot go. */
  moveNodes: (ids: number[], parent_id: number) =>
    request<{ moved: number; unchanged: number }>('/nodes/move', { method: 'POST', body: body({ ids, parent_id }) }),

  /** Publishes the document, or mints a fresh key for one already published. */
  share: (id: number) =>
    request<{ token: string; key: string; path: string; sealed: number }>(`/nodes/${id}/share`, {
      method: 'POST',
    }),

  unshare: (id: number) => request<{ ok: true }>(`/nodes/${id}/share`, { method: 'DELETE' }),

  /** Open cards across every board: due within a week, overdue, or in hand. */
  agenda: () => request<Agenda>('/agenda'),

  /** The board of a space. Fails for a space that has none, such as the diary. */
  board: (spaceId: number) => request<Board>(`/spaces/${spaceId}/board`),

  createList: (spaceId: number, name: string) =>
    request<Board>(`/spaces/${spaceId}/lists`, { method: 'POST', body: body({ name }) }),

  renameList: (listId: number, name: string) =>
    request<Board>(`/lists/${listId}`, { method: 'PUT', body: body({ name }) }),

  deleteList: (listId: number) => request<Board>(`/lists/${listId}`, { method: 'DELETE' }),

  createCard: (
    listId: number,
    input: { title: string; body?: string; node_id?: number | null; due_at?: number | null },
  ) => request<Card>(`/lists/${listId}/cards`, { method: 'POST', body: body(input) }),

  /*
   * Only what changed is sent. `due_at: null` clears the date while leaving it
   * out keeps it, which is why the server reads those fields as double options.
   */
  updateCard: (
    id: number,
    patch: {
      title?: string
      body?: string
      node_id?: number | null
      due_at?: number | null
      done?: boolean
    },
  ) => request<Card>(`/cards/${id}`, { method: 'PUT', body: body(patch) }),

  deleteCard: (id: number) => request<{ ok: true }>(`/cards/${id}`, { method: 'DELETE' }),

  moveCard: (id: number, list_id: number, position?: number) =>
    request<Card>(`/cards/${id}/move`, { method: 'POST', body: body({ list_id, position }) }),

  listMedia: () => pages((limit, offset) => request<MediaFile[]>(`/media?limit=${limit}&offset=${offset}`)),

  /**
   * Upload files into a space or folder as file nodes. XHR rather than fetch,
   * because a gigabyte going up needs to say how far along it is.
   */
  putFiles: (parentId: number, files: File[], encrypt: boolean, progress?: (fraction: number) => void) =>
    send<WorkspaceNode[]>(`/api/nodes/${parentId}/files${encrypt ? '?encrypt=true' : ''}`, files, progress),

  /** Files to embed in a document, with the same progress as `putFiles`. */
  upload: (files: File[], progress?: (fraction: number) => void) =>
    send<MediaFile[]>('/api/media', files, progress),

  deleteMedia: (id: string) => request<{ ok: true }>(`/media/${id}`, { method: 'DELETE' }),

  vaultStatus: () => request<VaultStatus>('/vault'),

  setupVault: (password: string) =>
    request<VaultStatus>('/vault', { method: 'POST', body: body({ password }) }),

  changeVaultPassword: (current: string, password: string) =>
    request<VaultStatus>('/vault', { method: 'PUT', body: body({ current, password }) }),

  unlockVault: (password: string) =>
    request<VaultStatus>('/vault/unlock', { method: 'POST', body: body({ password }) }),

  lockVault: () => request<VaultStatus>('/vault/lock', { method: 'POST' }),

  /**
   * Seal a file into the vault, or a folder with everything in it and all
   * that lands there later — or take it back out.
   */
  seal: (id: number, encrypt: boolean) =>
    request<{ files: number; unchanged: number; failed: number; shared: number; sealed: boolean }>(`/nodes/${id}/${encrypt ? 'encrypt' : 'decrypt'}`, {
      method: 'POST',
    }),

  /** What a certificate, key or keystore holds; the password opens it for this answer only. */
  inspect: (id: number, password?: string) =>
    request<Inspection>(`/nodes/${id}/inspect`, { method: 'POST', body: body({ password }) }),

  backupStatus: () => request<BackupStatus>('/backup'),

  backupNow: () => request<BackupStatus>('/backup', { method: 'POST' }),

  readShared: (token: string, key: string) =>
    request<SharedDocument>(`/share/${encodeURIComponent(token)}`, { method: 'POST', body: body({ key }) }),

  sessions: () => request<SignedIn[]>('/sessions'),
  revokeSession: (id: string) =>
    request<{ ok: true }>(`/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  listSshKeys: () => request<SshKey[]>('/ssh-keys'),

  addSshKey: (name: string, public_key: string) =>
    request<SshKey>('/ssh-keys', { method: 'POST', body: body({ name, public_key }) }),

  removeSshKey: (id: number) => request<{ ok: true }>(`/ssh-keys/${id}`, { method: 'DELETE' }),

  setSshKeyVault: (id: number, vault: boolean) =>
    request<{ ok: true }>(`/ssh-keys/${id}`, { method: 'PATCH', body: body({ vault }) }),
}
