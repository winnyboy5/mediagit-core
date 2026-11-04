import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import type { LocalServerStatus } from '@/api'

const api = vi.hoisted(() => ({
  remoteList: vi.fn(),
  serverLocalStatus: vi.fn(),
  serverLocalSetup: vi.fn(),
  serverLocalStart: vi.fn(),
  serverLocalStop: vi.fn(),
  serverLocalLog: vi.fn(),
  serverLocalSetNetwork: vi.fn(),
  serverWhoami: vi.fn(),
  serverLogin: vi.fn(),
  serverUsers: vi.fn(),
  serverUserCreate: vi.fn(),
  serverUserSetRole: vi.fn(),
  serverUserDisable: vi.fn(),
  serverUserEnable: vi.fn(),
  serverUserResetPassword: vi.fn(),
  serverGrantSet: vi.fn(),
  serverUserGrants: vi.fn(),
  serverRepoAccess: vi.fn(),
  serverGrantRemove: vi.fn(),
  serverKeys: vi.fn(),
  serverKeyCreate: vi.fn(),
  serverKeyRevoke: vi.fn(),
  serverRepos: vi.fn(),
  serverRepoCreate: vi.fn(),
  serverRepoDelete: vi.fn(),
}))
vi.mock('@/api', async (orig) => ({ ...(await orig<typeof import('@/api')>()), api }))

import { useToast } from '@/components/useToast'
import { LOCAL, originOf, useServerStore } from '@/stores/server'

const URL_ = 'http://127.0.0.1:4100'
const status = (over: Partial<LocalServerStatus> = {}): LocalServerStatus => ({
  state: 'running',
  url: URL_,
  port: 4100,
  pid: 42,
  data_dir: '/data/server',
  log_path: '/data/server/server.log',
  lan: false,
  ...over,
})
const who = (role: string | null, auth = true) => ({
  action: 'status',
  server: URL_,
  auth_enabled: auth,
  logged_in: role !== null,
  credential_tier: null,
  identity: role ? { username: 'root', email: 'r@x.io', role, grants: [] } : null,
  removed: null,
})

beforeEach(() => {
  localStorage.clear()
  setActivePinia(createPinia())
  Object.values(api).forEach((f) => f.mockReset())
  api.remoteList.mockResolvedValue({ remotes: [] })
  api.serverLocalLog.mockResolvedValue('')
  useToast().items.value.forEach((t) => useToast().dismiss(t.id))
})

describe('originOf', () => {
  it('keeps http(s) origins and drops paths and junk', () => {
    expect(originOf('https://mediagit.studio.example/hero-film')).toBe('https://mediagit.studio.example')
    expect(originOf('http://10.0.0.5:3000/x/y')).toBe('http://10.0.0.5:3000')
    expect(originOf('/repos/local')).toBeNull()
    expect(originOf('ssh://host/x')).toBeNull()
    expect(originOf('')).toBeNull()
  })
})

describe('targets', () => {
  it('lists this computer first, then the open repository remotes and saved servers, once each', async () => {
    api.serverLocalStatus.mockResolvedValue(status({ state: 'stopped' }))
    const repo = (await import('@/stores/repo')).useRepoStore()
    repo.path = '/repos/hero'
    api.remoteList.mockResolvedValue({ remotes: [{ name: 'origin', url: 'https://a.example/hero' }, { name: 'b', url: 'https://a.example/other' }] })
    const s = useServerStore()
    await s.init()
    expect(s.addServer('https://b.example/whatever')).toBe(true)
    expect(s.addServer('https://a.example')).toBe(true)
    expect(s.targets.map((t) => t.id)).toEqual([LOCAL, 'https://a.example', 'https://b.example'])
    expect(JSON.parse(localStorage.getItem('mg.servers')!)).toEqual(['https://b.example'])
    expect(s.addServer('not a url')).toBe(false)
  })

  it('local has no address to talk to unless a server is up', async () => {
    api.serverLocalStatus.mockResolvedValue(status({ state: 'stopped' }))
    const s = useServerStore()
    await s.init()
    expect(s.url).toBeNull()
    expect(api.serverWhoami).not.toHaveBeenCalled()
  })
})

describe('local server', () => {
  it('first run: sets up, starts, then signs the admin in; the password is only an argument', async () => {
    api.serverLocalStatus.mockResolvedValue(status({ state: 'not_set_up', url: null, port: null, pid: null }))
    const s = useServerStore()
    await s.init()
    api.serverLocalSetup.mockResolvedValue(status({ state: 'stopped' }))
    api.serverLocalStart.mockResolvedValue(status())
    api.serverLogin.mockResolvedValue(who('admin'))
    api.serverWhoami.mockResolvedValue(who('admin'))
    api.serverLocalStatus.mockResolvedValue(status())
    expect(await s.setUp('root', 'r@x.io', 'pw-1')).toBe(true)
    expect(api.serverLocalSetup).toHaveBeenCalledWith('root', 'r@x.io', 'pw-1')
    expect(api.serverLogin).toHaveBeenCalledWith(URL_, 'root', 'pw-1')
    expect(s.access).toBe('admin')
    expect(JSON.stringify(s.$state)).not.toContain('pw-1')
  })

  it('a failed start keeps the server stopped and shows the reason', async () => {
    api.serverLocalStatus.mockResolvedValue(status({ state: 'stopped' }))
    const s = useServerStore()
    await s.init()
    api.serverLocalStart.mockRejectedValue({ code: 'port_in_use', message: 'port 4100 is used by another program' })
    await s.start()
    expect(s.actionError?.code).toBe('port_in_use')
    expect(s.local?.state).toBe('stopped')
    expect(s.busy).toBe(false)
  })

  it('stop forgets who you were', async () => {
    api.serverLocalStatus.mockResolvedValue(status())
    api.serverWhoami.mockResolvedValue(who('admin'))
    const s = useServerStore()
    await s.init()
    expect(s.access).toBe('admin')
    api.serverLocalStop.mockResolvedValue(status({ state: 'stopped', pid: null }))
    await s.stop()
    expect(s.local?.state).toBe('stopped')
    expect(s.me).toBeNull()
    expect(s.url).toBeNull()
  })

  it('network access is a plain setting call', async () => {
    api.serverLocalStatus.mockResolvedValue(status({ state: 'stopped' }))
    const s = useServerStore()
    await s.init()
    api.serverLocalSetNetwork.mockResolvedValue(status({ state: 'stopped', lan: true }))
    await s.setNetwork(true)
    expect(s.local?.lan).toBe(true)
  })
})

describe('access', () => {
  const cases: [string, ReturnType<typeof who>, string][] = [
    ['admin', who('admin'), 'admin'],
    ['non-admin', who('write'), 'member'],
    ['signed out', who(null), 'signed_out'],
    ['no accounts', who(null, false), 'no_accounts'],
  ]
  it.each(cases)('%s', async (_n, report, expected) => {
    api.serverLocalStatus.mockResolvedValue(status())
    api.serverWhoami.mockResolvedValue(report)
    const s = useServerStore()
    await s.init()
    expect(s.access).toBe(expected)
  })

  it('a server that cannot be reached is an error, not a permission state', async () => {
    api.serverLocalStatus.mockResolvedValue(status())
    api.serverWhoami.mockRejectedValue({ code: 'error', message: 'failed to reach server' })
    const s = useServerStore()
    await s.init()
    expect(s.meError?.message).toContain('reach')
    expect(s.access).toBe('unknown')
  })

  it('switching server drops the old one’s lists', async () => {
    api.serverLocalStatus.mockResolvedValue(status())
    api.serverWhoami.mockResolvedValue(who('admin'))
    api.serverUsers.mockResolvedValue({ users: [{ id: '1', username: 'ana', role: 'write' }] })
    const s = useServerStore()
    await s.init()
    await s.loadUsers()
    expect(s.users.data).toHaveLength(1)
    s.addServer('https://other.example')
    await vi.waitFor(() => expect(s.users.data).toHaveLength(0))
    expect(s.url).toBe('https://other.example')
  })
})

describe('writes', () => {
  async function admin() {
    api.serverLocalStatus.mockResolvedValue(status())
    api.serverWhoami.mockResolvedValue(who('admin'))
    const s = useServerStore()
    await s.init()
    return s
  }

  it('adds a person and refreshes the list; no password is kept', async () => {
    const s = await admin()
    api.serverUserCreate.mockResolvedValue({})
    api.serverUsers.mockResolvedValue({ users: [{ id: '2', username: 'ana', role: 'write' }] })
    await s.createUser('ana', 'a@x.io', 'write', 'pw-2')
    expect(api.serverUserCreate).toHaveBeenCalledWith(URL_, 'ana', 'a@x.io', 'write', 'pw-2')
    expect(s.users.data[0]?.username).toBe('ana')
    expect(JSON.stringify(s.$state)).not.toContain('pw-2')
  })

  it('disable and enable reload the list so the state shown is what the server reports', async () => {
    const s = await admin()
    api.serverUserDisable.mockResolvedValue({})
    api.serverUserEnable.mockResolvedValue({})
    api.serverUsers.mockResolvedValue({ users: [{ id: '2', username: 'ana', role: 'write', disabled: true }] })
    await s.disableUser('ana')
    expect(api.serverUserDisable).toHaveBeenCalledWith(URL_, 'ana')
    expect(s.users.data[0]?.disabled).toBe(true)
    api.serverUsers.mockResolvedValue({ users: [{ id: '2', username: 'ana', role: 'write', disabled: false }] })
    await s.enableUser('ana')
    expect(api.serverUserEnable).toHaveBeenCalledWith(URL_, 'ana')
    expect(s.users.data[0]?.disabled).toBe(false)
  })

  it('a failed write surfaces the error and still reloads', async () => {
    const s = await admin()
    api.serverUserSetRole.mockRejectedValue({ code: 'error', message: 'forbidden' })
    api.serverUsers.mockResolvedValue({ users: [] })
    expect(await s.setRole('ana', 'admin')).toBeNull()
    expect(s.actionError?.message).toBe('forbidden')
    expect(api.serverUsers).toHaveBeenCalled()
  })

  it('granting reloads the person and repository lists being shown', async () => {
    const s = await admin()
    api.serverUserGrants.mockResolvedValue({ grants: [] })
    api.serverRepoAccess.mockResolvedValue({ access: [] })
    await s.loadUserGrants('ana')
    await s.loadRepoAccess('film')
    api.serverGrantSet.mockResolvedValue({})
    api.serverUserGrants.mockResolvedValue({ grants: [{ repo: 'film', level: 'write' }] })
    api.serverRepoAccess.mockResolvedValue({ access: [{ user_id: '2', username: 'ana', level: 'write' }] })
    await s.grant('ana', 'film', 'write')
    expect(s.userGrants.data).toEqual([{ repo: 'film', level: 'write' }])
    expect(s.repoAccess.data[0]?.username).toBe('ana')
  })

  it('a new key is held once and cleared on dismiss', async () => {
    const s = await admin()
    api.serverKeyCreate.mockResolvedValue({ key: 'mgk_abc', keys: [] })
    api.serverKeys.mockResolvedValue({ keys: [{ id: 'k1', name: 'ci', created_at: 1 }] })
    await s.createKey('ci')
    expect(s.newKey).toEqual({ name: 'ci', key: 'mgk_abc' })
    expect(s.keys.data).toHaveLength(1)
    s.dismissKey()
    expect(s.newKey).toBeNull()
  })

  it('deleting a repository whose bucket keeps the data raises the retained note', async () => {
    const s = await admin()
    api.serverRepoDelete.mockResolvedValue({ deleted: true, storage_retained: true, backend: 's3' })
    api.serverRepos.mockResolvedValue({ repos: [] })
    await s.deleteRepo('film', 'film', false)
    expect(api.serverRepoDelete).toHaveBeenCalledWith(URL_, 'film', 'film', false)
    expect(s.retainedNote).toEqual({ name: 'film', backend: 's3' })
  })

  it('a clean delete does not', async () => {
    const s = await admin()
    api.serverRepoDelete.mockResolvedValue({ deleted: true, storage_retained: false, backend: 'local' })
    api.serverRepos.mockResolvedValue({ repos: [] })
    await s.deleteRepo('film', 'film', true)
    expect(s.retainedNote).toBeNull()
  })

  it('does nothing while another write runs', async () => {
    const s = await admin()
    let done!: () => void
    api.serverRepoCreate.mockReturnValue(new Promise((r) => (done = () => r({}))))
    api.serverRepos.mockResolvedValue({ repos: [] })
    const first = s.createRepo('a')
    expect(await s.createRepo('b')).toBeNull()
    done()
    await first
    expect(api.serverRepoCreate).toHaveBeenCalledTimes(1)
  })
})
