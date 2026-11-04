import { expect, test, type Page } from '@playwright/test'
import { expectNoAxeViolations } from './helpers'
import { healthy, ipcCalls, mockIpc, type Scenario } from './ipc-mock'

const LOCAL_URL = 'http://127.0.0.1:4100'
const STUDIO = 'https://mediagit.studio.example'
const PW = 'Zebra-pw-Quiet-91!'

const status = (state: string, extra: Record<string, unknown> = {}) => ({
  state,
  url: state === 'not_set_up' ? null : LOCAL_URL,
  port: state === 'not_set_up' ? null : 4100,
  pid: state === 'running' ? 4242 : null,
  data_dir: 'C:\\Users\\mira\\AppData\\Roaming\\dev.mediagit.desktop\\server',
  log_path: 'C:\\Users\\mira\\AppData\\Roaming\\dev.mediagit.desktop\\server\\server.log',
  lan: false,
  ...extra,
})
const me = (role: string | null, server = LOCAL_URL, auth = true) => ({
  action: 'status',
  server,
  auth_enabled: auth,
  logged_in: role !== null,
  credential_tier: 'stored login',
  identity: role ? { username: 'mira', email: 'mira@studio.example', role, grants: [{ repo: 'hero-film', level: 'write' }] } : null,
  removed: null,
})
const op = (extra: Record<string, unknown> = {}) => ({ action: 'x', server: LOCAL_URL, id: null, name: null, role: null, repo: null, level: null, key: null, permissions: [], keys: [], users: [], note: null, ...extra })
const USERS = op({
  users: [
    { id: 'u1', username: 'mira', role: 'admin' },
    { id: 'u2', username: 'ana', role: 'write' },
    { id: 'u3', username: 'omar', role: 'read', disabled: true },
  ],
})
const KEYS = op({ keys: [{ id: 'k1', name: 'build machine', created_at: 1793000000 }] })
const REPOS = {
  server: LOCAL_URL,
  repos: [
    { name: 'hero-film', size_bytes: 4_800_000_000, object_count: 1200, last_modified: Math.floor(Date.now() / 1000) - 7200 },
    { name: 'trailer-cut', size_bytes: 0, object_count: 0, last_modified: 0 },
  ],
}

type Cmds = Record<string, unknown>
async function boot(page: Page, commands: Cmds = {}, extra: Partial<Scenario> = {}, tab?: string) {
  await mockIpc(page, { commands: healthy(commands), ...extra })
  await page.goto('/#/server', { waitUntil: 'domcontentloaded' })
  await expect(page.getByRole('heading', { name: 'Server', level: 1 })).toBeVisible()
  if (tab) await page.getByRole('tab', { name: tab }).click()
}

/** A running local server with a signed-in admin. */
const admin = (more: Cmds = {}): Cmds => ({
  server_local_status: status('running'),
  server_local_log: 'listening on 127.0.0.1:4100',
  auth_run: me('admin'),
  server_users_list: USERS,
  server_keys_list: KEYS,
  server_repos_list: REPOS,
  ...more,
})

const secretsGone = async (page: Page, ...secrets: string[]) => {
  const seen = await page.evaluate(() => document.body.innerText + JSON.stringify({ ...localStorage }) + JSON.stringify({ ...sessionStorage }))
  for (const s of secrets) expect(seen).not.toContain(s)
}

test.describe('Server: Overview, this computer', () => {
  test('not set up: invitation to set up; no axe violations', async ({ page }) => {
    await boot(page, { server_local_status: status('not_set_up') })
    await expect(page.getByRole('heading', { name: 'Run a server on this computer' })).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('loading: skeleton until the status arrives', async ({ page }) => {
    await boot(page, {}, { scripts: { server_local_status: [{ hold: true }, { resolve: status('stopped') }] } })
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await page.evaluate(() => (window as unknown as { __ipc: { release(): void } }).__ipc.release())
    await expect(page.getByTestId('local-state')).toHaveText('Stopped')
  })

  test('first run: set up, start, then sign in with the admin password', async ({ page }) => {
    const state = `(a, calls) => calls.some((c) => c.cmd === 'server_local_start') ? ${JSON.stringify(status('running'))} : calls.some((c) => c.cmd === 'server_local_setup') ? ${JSON.stringify(status('stopped'))} : ${JSON.stringify(status('not_set_up'))}`
    await boot(
      page,
      { server_local_setup: status('stopped'), server_local_start: status('running'), auth_login: me('admin'), auth_run: me('admin') },
      { handlers: { server_local_status: state } },
    )
    await page.getByRole('button', { name: 'Set up local server…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Set up local server' })
    await dlg.getByLabel('Admin username').fill('mira')
    await dlg.getByLabel('Admin email').fill('mira@studio.example')
    await dlg.getByLabel('Password', { exact: true }).fill(PW)
    await dlg.getByLabel('Repeat password').fill(PW + 'x')
    await expect(dlg.getByText('The passwords do not match')).toBeVisible()
    await expect(dlg.getByRole('button', { name: 'Set up and start' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Repeat password').fill(PW)
    await dlg.getByRole('button', { name: 'Set up and start' }).click()
    await expect(page.getByTestId('local-state')).toHaveText('Running')
    const names = (await ipcCalls(page)).map((c) => c.cmd).filter((c) => /^(server_local_setup|server_local_start|auth_login)$/.test(c))
    expect(names).toEqual(['server_local_setup', 'server_local_start', 'auth_login'])
    const [login] = await ipcCalls(page, 'auth_login')
    expect(login!.args).toMatchObject({ server: LOCAL_URL, username: 'mira', repo: null })
    await expect(page.getByTestId('me-line')).toContainText('Signed in as mira')
    await secretsGone(page, PW)
  })

  test('first run failure keeps the dialog open with the reason', async ({ page }) => {
    await boot(page, { server_local_status: status('not_set_up'), server_local_setup: { __error: { code: 'setup_failed', message: 'password too weak' } } })
    await page.getByRole('button', { name: 'Set up local server…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Set up local server' })
    await dlg.getByLabel('Admin username').fill('mira')
    await dlg.getByLabel('Admin email').fill('mira@studio.example')
    await dlg.getByLabel('Password', { exact: true }).fill('abc')
    await dlg.getByLabel('Repeat password').fill('abc')
    await dlg.getByRole('button', { name: 'Set up and start' }).click()
    await expect(dlg.getByRole('alert')).toContainText('password too weak')
    await expectNoAxeViolations(page)
  })

  test('stopped: start it; running: address, folder, log, copy', async ({ page }) => {
    const state = `(a, calls) => calls.some((c) => c.cmd === 'server_local_start') ? ${JSON.stringify(status('running'))} : ${JSON.stringify(status('stopped'))}`
    await boot(page, { server_local_start: status('running'), auth_run: me('admin'), server_local_log: 'listening on 127.0.0.1:4100' }, { handlers: { server_local_status: state } })
    await expect(page.getByTestId('local-state')).toHaveText('Stopped')
    await expect(page.getByTestId('local-card')).toContainText('This computer only')
    await expectNoAxeViolations(page)
    await page.getByRole('button', { name: 'Start server' }).click()
    await expect(page.getByTestId('local-state')).toHaveText('Running')
    await expect(page.getByTestId('local-card')).toContainText(LOCAL_URL)
    await expect(page.getByTestId('local-card')).toContainText('server')
    await expect(page.getByRole('region', { name: 'What the server printed' }).or(page.getByTestId('log-card'))).toContainText('listening on 127.0.0.1:4100')
    await expect(page.getByRole('checkbox', { name: 'Let other computers on my network connect' })).toBeDisabled()
    await expectNoAxeViolations(page)
  })

  test('stop asks first', async ({ page }) => {
    await boot(page, admin({ server_local_stop: status('stopped') }))
    await page.getByRole('button', { name: 'Stop server…' }).click()
    await expect(page.getByRole('alertdialog')).toContainText('People connected to it lose access')
    await expectNoAxeViolations(page)
    await page.getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'server_local_stop')).toHaveLength(0)
    await page.getByRole('button', { name: 'Stop server…' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Stop server' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'server_local_stop')).length).toBe(1)
  })

  test('network access can be switched on while stopped', async ({ page }) => {
    await boot(page, { server_local_status: status('stopped'), server_local_set_network: status('stopped', { lan: true }) })
    await page.getByRole('checkbox', { name: 'Let other computers on my network connect' }).click()
    const [call] = await ipcCalls(page, 'server_local_set_network')
    expect(call!.args).toEqual({ enabled: true })
    await expect(page.getByTestId('local-card')).toContainText('Other computers on your network')
  })

  test('another server already answers: no start or stop is offered', async ({ page }) => {
    await boot(page, { server_local_status: status('elsewhere'), auth_run: me('admin') })
    await expect(page.getByTestId('local-state')).toHaveText('Running, started elsewhere')
    await expect(page.getByTestId('local-card')).toContainText('did not start it')
    await expect(page.getByRole('button', { name: /Start server|Stop server/ })).toHaveCount(0)
    await expectNoAxeViolations(page)
  })

  test('start failure shows the reason', async ({ page }) => {
    await boot(page, { server_local_status: status('stopped'), server_local_start: { __error: { code: 'port_in_use', message: 'port 4100 is used by another program' } } })
    await page.getByRole('button', { name: 'Start server' }).click()
    await expect(page.getByRole('alert')).toContainText('port 4100 is used by another program')
    await expect(page.getByTestId('local-state')).toHaveText('Stopped')
    await expectNoAxeViolations(page)
  })

  test('status failure is an error state with retry', async ({ page }) => {
    await boot(page, { server_local_status: { __error: { code: 'spawn_failed', message: 'could not read the server state' } } })
    await expect(page.getByText('could not read the server state')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('running but signed out: sign in offered', async ({ page }) => {
    await boot(page, admin({ auth_run: me(null), auth_login: me('admin') }))
    await expect(page.getByTestId('me-card')).toContainText('not signed in')
    await page.getByTestId('me-card').getByRole('button', { name: 'Sign in' }).click()
    const dlg = page.getByRole('dialog', { name: 'Sign in' })
    await dlg.getByLabel('Username or email').fill('mira')
    await dlg.getByLabel('Password').fill(PW)
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Sign in' }).click()
    const [login] = await ipcCalls(page, 'auth_login')
    expect(login!.args).toMatchObject({ server: LOCAL_URL, username: 'mira' })
    await secretsGone(page, PW)
  })
})

test.describe('Server: other servers', () => {
  const storage = { 'mg.servers': JSON.stringify([STUDIO]), 'mg.server.target': JSON.stringify(STUDIO) }
  const remote = (role: string | null, auth = true) => ({ server_local_status: status('stopped'), auth_run: me(role, STUDIO, auth) })

  test('overview: address, sign-in and your role', async ({ page }) => {
    await boot(page, remote('admin'), { storage })
    await expect(page.getByTestId('remote-card')).toContainText(STUDIO)
    await expect(page.getByTestId('me-line')).toHaveText('mira, Can manage (admin)')
    await expectNoAxeViolations(page)
  })

  test('the server picker switches between this computer and a saved server', async ({ page }) => {
    await boot(page, remote('admin'), { storage })
    await page.getByRole('combobox', { name: 'Managing' }).click()
    await expect(page.getByRole('option')).toHaveText(['This computer', 'mediagit.studio.example'])
    await page.getByRole('option', { name: 'This computer' }).click()
    await expect(page.getByTestId('local-card')).toBeVisible()
  })

  test('add a server by address', async ({ page }) => {
    await boot(page, { server_local_status: status('stopped'), auth_run: me('admin', 'https://other.example') })
    await page.getByRole('button', { name: 'Add server…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Add a server' })
    await dlg.getByLabel('Server address').fill('ftp://nope')
    await dlg.getByRole('button', { name: 'Add server' }).click()
    await expect(dlg.getByText('Use an http(s) address')).toBeVisible()
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Server address').fill('https://other.example/some/repo')
    await dlg.getByRole('button', { name: 'Add server' }).click()
    await expect(page.getByTestId('remote-card')).toContainText('https://other.example')
  })

  test('not signed in: People asks to sign in', async ({ page }) => {
    await boot(page, remote(null), { storage }, 'People')
    await expect(page.getByTestId('gate-signed_out')).toContainText('Sign in to manage people')
    await expectNoAxeViolations(page)
  })

  test('not an admin: People is a clear read-only state; own keys still work', async ({ page }) => {
    await boot(page, { ...remote('write'), server_keys_list: KEYS }, { storage }, 'People')
    await expect(page.getByTestId('gate-member')).toContainText('You need admin rights')
    await expect(page.getByTestId('gate-member')).toContainText('Your role: Can change files (write)')
    await expectNoAxeViolations(page)
    expect(await ipcCalls(page, 'server_users_list')).toHaveLength(0)
    await page.getByRole('tab', { name: 'API keys' }).click()
    await expect(page.getByTestId('keys')).toContainText('build machine')
    await page.getByRole('tab', { name: 'Repositories' }).click()
    await expect(page.getByTestId('gate-member')).toContainText('repositories')
    await page.getByRole('tab', { name: 'Access' }).click()
    await expect(page.getByTestId('gate-member')).toContainText('access')
  })

  test('a server without sign-in: accounts tabs say so; repositories still work', async ({ page }) => {
    await boot(page, { ...remote(null, false), server_repos_list: { ...REPOS, server: STUDIO } }, { storage }, 'People')
    await expect(page.getByTestId('gate-open')).toContainText('does not use accounts')
    await expectNoAxeViolations(page)
    await page.getByRole('tab', { name: 'Repositories' }).click()
    await expect(page.getByTestId('repos')).toContainText('hero-film')
  })

  test('unreachable: an error with retry, not a permission message', async ({ page }) => {
    await boot(page, { server_local_status: status('stopped'), auth_run: { __error: { code: 'error', message: 'failed to reach the server' } } }, { storage }, 'People')
    await expect(page.getByTestId('gate-error')).toContainText('failed to reach the server')
    await expectNoAxeViolations(page)
  })

  test('the local server being down is explained on every admin tab', async ({ page }) => {
    await boot(page, { server_local_status: status('stopped') }, {}, 'Repositories')
    await expect(page.getByTestId('gate-down')).toContainText('The local server is not running')
    await page.getByRole('button', { name: 'Go to Overview' }).click()
    await expect(page.getByTestId('local-card')).toBeVisible()
  })
})

test.describe('Server: People', () => {
  test('list with roles; no axe violations', async ({ page }) => {
    await boot(page, admin(), {}, 'People')
    const t = page.getByTestId('people')
    await expect(t.getByRole('row')).toHaveCount(4)
    await expect(t).toContainText('ana')
    await expect(t).toContainText('Can change files')
    await expectNoAxeViolations(page)
  })

  test('loading, empty and error states', async ({ page }) => {
    await boot(page, admin(), { scripts: { server_users_list: [{ hold: true }, { resolve: op() }] } }, 'People')
    await expect(page.getByRole('status', { name: 'Loading' })).toBeVisible()
    await expectNoAxeViolations(page)
    await page.evaluate(() => (window as unknown as { __ipc: { release(): void } }).__ipc.release())
    await expect(page.getByText('No one has an account yet')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('list failure', async ({ page }) => {
    await boot(page, admin({ server_users_list: { __error: { code: 'error', message: 'the server refused to list people' } } }), {}, 'People')
    await expect(page.getByText('the server refused to list people')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('add a person: password goes as an argument, never stays on the page', async ({ page }) => {
    await boot(page, admin({ server_user_create: op() }), {}, 'People')
    await page.getByRole('button', { name: 'Add person…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Add person' })
    await dlg.getByLabel('Username').fill('lena')
    await dlg.getByLabel('Email').fill('lena@studio.example')
    await dlg.getByLabel('Starting password').fill(PW)
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Add person' }).click()
    await expect(dlg).toBeHidden()
    const [c] = await ipcCalls(page, 'server_user_create')
    expect(c!.args).toEqual({ server: LOCAL_URL, user: 'lena', email: 'lena@studio.example', role: 'write', password: PW })
    await secretsGone(page, PW)
  })

  test('add person failure keeps the dialog and shows why', async ({ page }) => {
    await boot(page, admin({ server_user_create: { __error: { code: 'error', message: 'that username is taken' } } }), {}, 'People')
    await page.getByRole('button', { name: 'Add person…' }).click()
    const dlg = page.getByRole('dialog', { name: 'Add person' })
    await dlg.getByLabel('Username').fill('ana')
    await dlg.getByLabel('Email').fill('ana@studio.example')
    await dlg.getByLabel('Starting password').fill(PW)
    await dlg.getByRole('button', { name: 'Add person' }).click()
    await expect(dlg.getByRole('alert')).toContainText('that username is taken')
  })

  test('change role', async ({ page }) => {
    await boot(page, admin({ server_user_set_role: op() }), {}, 'People')
    await page.getByRole('button', { name: 'Change what ana can do' }).click()
    const dlg = page.getByRole('dialog', { name: 'Change what ana can do' })
    await dlg.getByRole('combobox', { name: 'What they can do' }).click()
    await page.getByRole('option', { name: 'Can manage (admin)' }).click()
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Save' }).click()
    const [c] = await ipcCalls(page, 'server_user_set_role')
    expect(c!.args).toEqual({ server: LOCAL_URL, user: 'ana', role: 'admin' })
  })

  test('a disabled person is marked in words and can be enabled', async ({ page }) => {
    await boot(page, admin({ server_user_enable: op() }), {}, 'People')
    await expect(page.getByTestId('people').getByRole('row', { name: /omar/ })).toContainText('Cannot sign in')
    await expect(page.getByRole('button', { name: 'Stop omar signing in' })).toHaveCount(0)
    await page.getByRole('button', { name: 'Let omar sign in again' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'server_user_enable')).length).toBe(1)
    const [c] = await ipcCalls(page, 'server_user_enable')
    expect(c!.args).toEqual({ server: LOCAL_URL, user: 'omar' })
  })

  test('disable asks first', async ({ page }) => {
    await boot(page, admin({ server_user_disable: op() }), {}, 'People')
    await page.getByRole('button', { name: 'Stop ana signing in' }).click()
    await expect(page.getByRole('alertdialog')).toContainText('cannot sign in until you enable them again')
    await expectNoAxeViolations(page)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Cancel' }).click()
    expect(await ipcCalls(page, 'server_user_disable')).toHaveLength(0)
    await page.getByRole('button', { name: 'Stop ana signing in' }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: 'Disable' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'server_user_disable')).length).toBe(1)
    const [c] = await ipcCalls(page, 'server_user_disable')
    expect(c!.args).toEqual({ server: LOCAL_URL, user: 'ana', confirm: true })
  })

  test('reset password', async ({ page }) => {
    await boot(page, admin({ server_user_reset_password: op() }), {}, 'People')
    await page.getByRole('button', { name: 'Reset password for omar' }).click()
    const dlg = page.getByRole('dialog', { name: 'Reset password for omar' })
    await dlg.getByLabel('New password').fill(PW)
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Reset password' }).click()
    const [c] = await ipcCalls(page, 'server_user_reset_password')
    expect(c!.args).toEqual({ server: LOCAL_URL, user: 'omar', password: PW })
    await secretsGone(page, PW)
  })
})

test.describe('Server: Access', () => {
  test('give and remove access to a repository', async ({ page }) => {
    await boot(
      page,
      admin({
        server_grant_set: op(),
        server_grant_remove: op(),
        server_user_grants: op({ grants: [{ repo: 'trailer-cut', level: 'read' }] }),
        server_repo_access: op({ access: [{ user_id: 'u2', username: 'ana', level: 'write' }] }),
      }),
      {},
      'Access',
    )
    await expect(page.getByRole('list', { name: 'Your access' })).toContainText('hero-film')
    await expectNoAxeViolations(page)
    const form = page.getByTestId('grant-form')
    await expect(form.getByRole('button', { name: 'Give access' })).toBeDisabled()
    await form.getByRole('combobox', { name: 'Person' }).click()
    await page.getByRole('option', { name: /^ana/ }).click()
    await form.getByRole('combobox', { name: 'Repository' }).click()
    await page.getByRole('option', { name: 'hero-film' }).click()
    await expect(page.getByTestId('user-grants')).toContainText('trailer-cut')
    await expect(page.getByTestId('repo-access')).toContainText('ana')
    await expect(page.getByTestId('user-grants').getByRole('list')).toContainText('Can look (read)')
    await expect(page.getByTestId('current-level')).toContainText('ana has no access of their own to hero-film')
    await expectNoAxeViolations(page)
    await form.getByRole('button', { name: 'Give access' }).click()
    const [g] = await ipcCalls(page, 'server_grant_set')
    expect(g!.args).toEqual({ server: LOCAL_URL, user: 'ana', repo: 'hero-film', level: 'write' })
    await form.getByRole('button', { name: 'Remove access…' }).click()
    await expect(page.getByRole('alertdialog')).toContainText("Remove ana's access to hero-film?")
    await expectNoAxeViolations(page)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Remove access' }).click()
    const [r] = await ipcCalls(page, 'server_grant_remove')
    expect(r!.args).toEqual({ server: LOCAL_URL, user: 'ana', repo: 'hero-film', confirm: true })
  })

  test('no repositories yet', async ({ page }) => {
    await boot(page, admin({ server_repos_list: { server: LOCAL_URL, repos: [] } }), {}, 'Access')
    await expect(page.getByText('No repositories yet')).toBeVisible()
    await expectNoAxeViolations(page)
  })
})

test.describe('Server: API keys', () => {
  test('list, make (shown once, kept until saved), revoke', async ({ page }) => {
    await boot(page, admin({ server_key_create: op({ key: 'mgk_live_7QK2M9XD4TVBHE8A', name: 'ci' }), server_key_revoke: op() }), {}, 'API keys')
    await expect(page.getByTestId('keys')).toContainText('build machine')
    await expectNoAxeViolations(page)
    await page.getByRole('button', { name: 'New key…' }).click()
    const dlg = page.getByRole('dialog', { name: 'New API key' })
    await dlg.getByLabel('Name').fill('ci')
    await expectNoAxeViolations(page)
    await dlg.getByRole('button', { name: 'Make key' }).click()
    const once = page.getByRole('dialog', { name: 'Copy your new key' })
    await expect(once.getByTestId('new-key-value')).toHaveText('mgk_live_7QK2M9XD4TVBHE8A')
    await expect(once.getByRole('button', { name: 'Done' })).toBeDisabled()
    await page.keyboard.press('Escape')
    await expect(once).toBeVisible()
    await expectNoAxeViolations(page)
    await once.getByRole('checkbox', { name: 'I saved it' }).click()
    await once.getByRole('button', { name: 'Done' }).click()
    await expect(once).toBeHidden()
    await secretsGone(page, 'mgk_live_7QK2M9XD4TVBHE8A')
    await page.getByRole('button', { name: 'Revoke key build machine' }).click()
    await expect(page.getByRole('alertdialog')).toContainText('Anything using this key stops working at once')
    await expectNoAxeViolations(page)
    await page.getByRole('alertdialog').getByRole('button', { name: 'Revoke key' }).click()
    await expect.poll(async () => (await ipcCalls(page, 'server_key_revoke')).length).toBe(1)
    const [c] = await ipcCalls(page, 'server_key_revoke')
    expect(c!.args).toEqual({ server: LOCAL_URL, id: 'k1', confirm: true })
  })

  test('empty and error states', async ({ page }) => {
    await boot(page, admin({ server_keys_list: op() }), {}, 'API keys')
    await expect(page.getByText('You have no API keys')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('list failure', async ({ page }) => {
    await boot(page, admin({ server_keys_list: { __error: { code: 'error', message: 'could not read your keys' } } }), {}, 'API keys')
    await expect(page.getByText('could not read your keys')).toBeVisible()
    await expectNoAxeViolations(page)
  })
})

test.describe('Server: Repositories', () => {
  test('list with size and last change; no axe violations', async ({ page }) => {
    await boot(page, admin(), {}, 'Repositories')
    const t = page.getByTestId('repos')
    await expect(t).toContainText('hero-film')
    await expect(t).toContainText('4.5 GB')
    await expect(t).toContainText('2h ago')
    await expect(t).toContainText('Nothing pushed yet')
    await expectNoAxeViolations(page)
  })

  test('empty and error states', async ({ page }) => {
    await boot(page, admin({ server_repos_list: { server: LOCAL_URL, repos: [] } }), {}, 'Repositories')
    await expect(page.getByText('No repositories yet')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('list failure', async ({ page }) => {
    await boot(page, admin({ server_repos_list: { __error: { code: 'error', message: 'the server could not list repositories' } } }), {}, 'Repositories')
    await expect(page.getByText('the server could not list repositories')).toBeVisible()
    await expectNoAxeViolations(page)
  })

  test('create validates the name', async ({ page }) => {
    await boot(page, admin({ server_repo_create: { action: 'create', server: LOCAL_URL, name: 'new-film', repo: null, deleted: false, storage_retained: false, backend: null } }), {}, 'Repositories')
    await page.getByRole('button', { name: 'New repository…' }).click()
    const dlg = page.getByRole('dialog', { name: 'New repository' })
    await dlg.getByLabel('Name').fill('bad name!')
    await expect(dlg.getByText('Use letters, numbers')).toBeVisible()
    await expect(dlg.getByRole('button', { name: 'Create repository' })).toBeDisabled()
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Name').fill('new-film')
    await dlg.getByRole('button', { name: 'Create repository' }).click()
    const [c] = await ipcCalls(page, 'server_repo_create')
    expect(c!.args).toEqual({ server: LOCAL_URL, name: 'new-film' })
  })

  test('delete needs the name typed back', async ({ page }) => {
    await boot(page, admin({ server_repo_delete: { action: 'delete', server: LOCAL_URL, name: 'hero-film', repo: null, deleted: true, storage_retained: false, backend: 'local' } }), {}, 'Repositories')
    await page.getByRole('button', { name: 'Delete hero-film' }).click()
    const dlg = page.getByRole('dialog', { name: 'Delete hero-film?' })
    const go = dlg.getByRole('button', { name: 'Delete repository' })
    await expect(go).toBeDisabled()
    await expect(dlg).toContainText('Files already in the storage bucket stay there until you remove them')
    await dlg.getByLabel('Type hero-film to confirm').fill('hero-fil')
    await expect(go).toBeDisabled()
    await expectNoAxeViolations(page)
    await dlg.getByLabel('Type hero-film to confirm').fill('hero-film')
    await dlg.getByRole('checkbox', { name: 'Delete even though it holds 4.5 GB of versions' }).click()
    await go.click()
    const [c] = await ipcCalls(page, 'server_repo_delete')
    expect(c!.args).toEqual({ server: LOCAL_URL, name: 'hero-film', typedName: 'hero-film', force: true, confirm: true })
  })

  test('a bucket-backed server keeps the data: said plainly', async ({ page }) => {
    await boot(page, admin({ server_repo_delete: { action: 'delete', server: LOCAL_URL, name: 'hero-film', repo: null, deleted: true, storage_retained: true, backend: 's3' } }), {}, 'Repositories')
    await page.getByRole('button', { name: 'Delete hero-film' }).click()
    const dlg = page.getByRole('dialog', { name: 'Delete hero-film?' })
    await dlg.getByLabel('Type hero-film to confirm').fill('hero-film')
    await dlg.getByRole('button', { name: 'Delete repository' }).click()
    await expect(page.getByTestId('retained')).toContainText('the data in the storage bucket (s3) is kept')
    await expectNoAxeViolations(page)
  })

  test('delete failure keeps the dialog and shows why', async ({ page }) => {
    await boot(page, admin({ server_repo_delete: { __error: { code: 'error', message: 'repository is not empty; delete it with data to confirm' } } }), {}, 'Repositories')
    await page.getByRole('button', { name: 'Delete hero-film' }).click()
    const dlg = page.getByRole('dialog', { name: 'Delete hero-film?' })
    await dlg.getByLabel('Type hero-film to confirm').fill('hero-film')
    await dlg.getByRole('button', { name: 'Delete repository' }).click()
    await expect(dlg.getByRole('alert')).toContainText('repository is not empty')
  })
})

test('Welcome offers the Server view without a repository', async ({ page }) => {
  await mockIpc(page, { commands: healthy({ server_local_status: status('not_set_up') }) })
  await page.goto('/#/welcome', { waitUntil: 'domcontentloaded' })
  await page.getByRole('button', { name: 'Host a server…' }).click()
  await expect(page.getByRole('heading', { name: 'Server', level: 1 })).toBeVisible()
})
