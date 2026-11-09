import { defineStore } from 'pinia'
import { computed, ref, shallowRef, type Ref } from 'vue'
import {
  api,
  toApiError,
  type ApiError,
  type AuthReport,
  type HostedRepo,
  type LocalServerStatus,
  type StorageOpts,
  type ServerKey,
  type ServerUser,
} from '@/api'
import { useToast } from '@/components/useToast'
import { load, save } from '@/lib/persist'
import { useRepoStore } from './repo'

/** "This computer" is the app's own server; every other target is a server address. */
export const LOCAL = 'local'

export interface Target {
  id: string
  label: string
  /** Where the CLI talks to it; null while the local server is not set up. */
  url: string | null
}

/** http(s)://host[:port] of a remote's address, or null for a path or an unusable address. */
export function originOf(remoteUrl: string): string | null {
  try {
    const u = new URL(remoteUrl)
    return u.protocol === 'http:' || u.protocol === 'https:' ? u.origin : null
  } catch {
    return null
  }
}

/** What the signed-in person may do on the selected server. */
export type Access = 'unknown' | 'signed_out' | 'no_accounts' | 'member' | 'admin'

interface Slice<T> {
  data: T
  loading: boolean
  error: ApiError | null
}
const empty = <T>(data: T): Slice<T> => ({ data, loading: false, error: null })

/**
 * The Server view: the local server's lifecycle, plus people, access, API keys and hosted
 * repositories on any server. Passwords are arguments of one call and are never kept here;
 * the only secret held is a freshly made API key, until the person confirms they saved it.
 */
export const useServerStore = defineStore('server', () => {
  const repo = useRepoStore()
  const toast = useToast()

  const local = ref<LocalServerStatus | null>(null)
  const localError = ref<ApiError | null>(null)
  const folderError = ref<ApiError | null>(null)
  const log = ref('')
  const saved = ref<string[]>(load<string[]>('mg.servers', []))
  const remotes = ref<string[]>([])
  const selected = ref<string>(load<string>('mg.server.target', LOCAL))

  const me = shallowRef<AuthReport | null>(null)
  const meError = ref<ApiError | null>(null)
  const users = ref<Slice<ServerUser[]>>(empty([]))
  const keys = ref<Slice<ServerKey[]>>(empty([]))
  const repos = ref<Slice<HostedRepo[]>>(empty([]))
  const userGrants = ref<Slice<{ repo: string; level: string }[]>>(empty([]))
  const repoAccess = ref<Slice<{ user_id: string; username: string; level: string }[]>>(empty([]))
  /** The person and repository the Access lists are showing, so a write can reload them. */
  let lastUser: string | null = null
  let lastRepo: string | null = null

  const busy = ref(false)
  const actionError = ref<ApiError | null>(null)
  /** A new API key, shown once. */
  const newKey = ref<{ name: string; key: string } | null>(null)
  /** A hosted repository was removed but the storage bucket kept its data. */
  const retainedNote = ref<{ name: string; backend: string | null } | null>(null)

  const targets = computed<Target[]>(() => {
    const out: Target[] = [{ id: LOCAL, label: 'This computer', url: local.value?.url ?? null }]
    for (const url of [...remotes.value, ...saved.value]) {
      if (!out.some((t) => t.id === url)) out.push({ id: url, label: url.replace(/^https?:\/\//, ''), url })
    }
    return out
  })
  const target = computed(() => targets.value.find((t) => t.id === selected.value) ?? targets.value[0]!)
  const isLocal = computed(() => target.value.id === LOCAL)
  /** The address admin calls go to; null until there is a server to talk to. */
  const url = computed(() => (isLocal.value && local.value?.state !== 'running' && local.value?.state !== 'elsewhere' ? null : target.value.url))

  const access = computed<Access>(() => {
    const m = me.value
    if (!m) return 'unknown'
    if (!m.auth_enabled) return 'no_accounts'
    if (!m.logged_in || !m.identity) return 'signed_out'
    return m.identity.role === 'admin' ? 'admin' : 'member'
  })

  let token = 0

  async function run<T>(fn: () => Promise<T>, ok?: (r: T) => string | undefined): Promise<T | null> {
    if (busy.value) return null
    busy.value = true
    actionError.value = null
    try {
      const r = await fn()
      const msg = ok?.(r)
      if (msg) toast.push({ kind: 'success', title: msg })
      return r
    } catch (e) {
      actionError.value = toApiError(e)
      return null
    } finally {
      busy.value = false
    }
  }

  // -- targets ----------------------------------------------------------------
  async function loadRemotes() {
    if (!repo.path) {
      remotes.value = []
      return
    }
    try {
      const r = await api.remoteList(repo.path)
      remotes.value = [...new Set(r.remotes.map((x) => originOf(x.url)).filter((o): o is string => !!o))]
    } catch {
      remotes.value = []
    }
  }

  function addServer(address: string): boolean {
    const origin = originOf(address.trim())
    if (!origin) return false
    if (!saved.value.includes(origin) && !remotes.value.includes(origin)) {
      saved.value = [...saved.value, origin]
      save('mg.servers', saved.value)
    }
    void select(origin)
    return true
  }

  function forgetServer(origin: string) {
    saved.value = saved.value.filter((s) => s !== origin)
    save('mg.servers', saved.value)
    if (selected.value === origin) void select(LOCAL)
  }

  async function select(id: string) {
    selected.value = id
    save('mg.server.target', id)
    token++
    me.value = null
    meError.value = null
    users.value = empty([])
    keys.value = empty([])
    repos.value = empty([])
    userGrants.value = empty([])
    repoAccess.value = empty([])
    lastUser = lastRepo = null
    actionError.value = null
    await refreshMe()
  }

  // -- local server -------------------------------------------------------------
  async function refreshLocal() {
    try {
      local.value = await api.serverLocalStatus()
      localError.value = null
    } catch (e) {
      localError.value = toApiError(e)
    }
  }

  async function refreshLog() {
    try {
      log.value = await api.serverLocalLog(40)
    } catch {
      log.value = ''
    }
  }

  async function init() {
    await Promise.all([loadRemotes(), refreshLocal()])
    if (!targets.value.some((t) => t.id === selected.value)) selected.value = LOCAL
    await Promise.all([refreshMe(), refreshLog()])
  }

  /** First run: create the server and its first admin, start it, and sign that admin in. */
  async function setUp(username: string, email: string, password: string, storage?: StorageOpts) {
    const st = await run(async () => {
      await api.serverLocalSetup(username, email, password, storage)
      const up = await api.serverLocalStart()
      local.value = up
      // The CLI keeps the credential; this store keeps nothing.
      if (up.url) await api.serverLogin(up.url, username, password)
      return up
    }, () => 'Local server is running')
    await refreshLocal()
    if (st) await Promise.all([refreshMe(), refreshLog()])
    return !!st
  }

  /** Move the local server to another folder (no path = the default). Errors land in `folderError`. */
  async function setFolder(path?: string) {
    folderError.value = null
    const st = await run(() => api.serverLocalSetFolder(path), () => 'Server folder changed')
    if (st) local.value = st
    else {
      folderError.value = actionError.value
      actionError.value = null
    }
    return !!st
  }

  /** Try the storage credentials. The result never throws: failures come back as `{ok:false}`. */
  async function testStorage(storage: StorageOpts) {
    try {
      return await api.serverStorageTest(storage)
    } catch (e) {
      return { ok: false, error: toApiError(e).message }
    }
  }

  async function start() {
    const st = await run(() => api.serverLocalStart(), () => 'Local server started')
    if (st) local.value = st
    else await refreshLocal()
    await Promise.all([refreshMe(), refreshLog()])
  }

  async function stop() {
    const st = await run(() => api.serverLocalStop(), () => 'Local server stopped')
    if (st) local.value = st
    else await refreshLocal()
    me.value = null
    await refreshLog()
  }

  async function setNetwork(enabled: boolean) {
    const st = await run(() => api.serverLocalSetNetwork(enabled), () => (enabled ? 'Other computers can connect once it starts' : 'Only this computer can connect'))
    if (st) local.value = st
  }

  // -- who am I -----------------------------------------------------------------
  async function refreshMe() {
    const u = url.value
    me.value = null
    meError.value = null
    if (!u) return
    const mine = ++token
    try {
      const r = await api.serverWhoami(u)
      if (mine === token) me.value = r
    } catch (e) {
      if (mine === token) meError.value = toApiError(e)
    }
  }

  async function login(username: string, password: string) {
    const u = url.value
    if (!u) return false
    const r = await run(() => api.serverLogin(u, username, password), () => 'Signed in')
    await refreshMe()
    return !!r
  }

  // -- lists --------------------------------------------------------------------
  async function fill<T>(slice: Ref<Slice<T[]>>, fetch: (u: string) => Promise<T[]>) {
    const u = url.value
    if (!u) return
    slice.value = { ...slice.value, loading: true, error: null }
    try {
      const data = await fetch(u)
      if (u === url.value) slice.value = { data, loading: false, error: null }
    } catch (e) {
      if (u === url.value) slice.value = { data: [], loading: false, error: toApiError(e) }
    }
  }
  const loadUsers = () => fill(users, async (u) => (await api.serverUsers(u)).users)
  const loadKeys = () => fill(keys, async (u) => (await api.serverKeys(u)).keys)
  const loadUserGrants = (user: string) => ((lastUser = user), fill(userGrants, async (u) => (await api.serverUserGrants(u, user)).grants ?? []))
  const loadRepoAccess = (repo: string) => ((lastRepo = repo), fill(repoAccess, async (u) => (await api.serverRepoAccess(u, repo)).access ?? []))
  const reloadAccess = async () => {
    await Promise.all([refreshMe(), lastUser ? loadUserGrants(lastUser) : null, lastRepo ? loadRepoAccess(lastRepo) : null])
  }
  const loadRepos = () => fill(repos, async (u) => (await api.serverRepos(u)).repos)

  // -- writes -------------------------------------------------------------------
  /** `then` runs after a success so the list reflects it. */
  async function write<T>(fn: (u: string) => Promise<T>, ok: (r: T) => string | undefined, then: () => Promise<unknown>) {
    const u = url.value
    if (!u) return null
    const r = await run(() => fn(u), ok)
    await then()
    return r
  }

  const createUser = (user: string, email: string, role: string, password: string) =>
    write((u) => api.serverUserCreate(u, user, email, role, password), () => `Added ${user}`, loadUsers)
  const setRole = (user: string, role: string) => write((u) => api.serverUserSetRole(u, user, role), () => `${user} is now ${role}`, loadUsers)
  const resetPassword = (user: string, password: string) =>
    write((u) => api.serverUserResetPassword(u, user, password), () => `New password set for ${user}`, async () => {})
  const disableUser = (user: string) => write((u) => api.serverUserDisable(u, user), () => `${user} can no longer sign in`, loadUsers)
  const enableUser = (user: string) => write((u) => api.serverUserEnable(u, user), () => `${user} can sign in again`, loadUsers)
  const grant = (user: string, repoName: string, level: string) =>
    write((u) => api.serverGrantSet(u, user, repoName, level), () => `${user} can ${level === 'read' ? 'read' : level === 'write' ? 'change' : 'manage'} ${repoName}`, reloadAccess)
  const revokeGrant = (user: string, repoName: string) =>
    write((u) => api.serverGrantRemove(u, user, repoName), () => `${user} no longer has access to ${repoName}`, reloadAccess)

  /** `permissions` empty = everything you can do yourself; otherwise only those (`repo:read`, `repo:write`, `repo:admin`). */
  async function createKey(name: string, permissions: string[] = []) {
    const r = await write((u) => api.serverKeyCreate(u, name, permissions), () => undefined, loadKeys)
    if (r?.key) newKey.value = { name, key: r.key }
    return !!r
  }
  const revokeKey = (id: string, name: string) => write((u) => api.serverKeyRevoke(u, id), () => `Key ${name} revoked`, loadKeys)
  const dismissKey = () => (newKey.value = null)

  const createRepo = (name: string) => write((u) => api.serverRepoCreate(u, name), () => `Created ${name}`, loadRepos)
  async function deleteRepo(name: string, typedName: string, force: boolean) {
    const r = await write((u) => api.serverRepoDelete(u, name, typedName, force), (x) => (x.storage_retained ? undefined : `Deleted ${name}`), loadRepos)
    if (r?.storage_retained) retainedNote.value = { name, backend: r.backend }
    return !!r
  }

  return {
    local, localError, folderError, setFolder, testStorage, log, selected, targets, target, isLocal, url, access,
    me, meError, users, keys, repos, userGrants, repoAccess, busy, actionError, newKey, retainedNote,
    init, select, addServer, forgetServer, refreshLocal, refreshLog, setUp, start, stop, setNetwork,
    refreshMe, login, loadUsers, loadKeys, loadRepos, loadUserGrants, loadRepoAccess,
    createUser, disableUser, enableUser, setRole, resetPassword, grant, revokeGrant, createKey, revokeKey, dismissKey, createRepo, deleteRepo,
  }
})
