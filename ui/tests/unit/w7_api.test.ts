import { beforeEach, describe, expect, it, vi } from 'vitest'

const core = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: core.invoke,
  convertFileSrc: (p: string) => `asset://${p}`,
  Channel: class {
    onmessage: unknown
  },
}))
const listen = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/event', () => ({ listen }))

import { CONTRACT_VERSION, api, isApiError, toApiError } from '@/api'

const R = '/r'
const P = () => {}

beforeEach(() => {
  core.invoke.mockReset()
  core.invoke.mockResolvedValue({})
})

// [label, call, command, subset of the arguments the backend must receive]
type Case = [string, () => unknown, string, Record<string, unknown>]
const cases: Case[] = [
  ['engineInfo', () => api.engineInfo(), 'engine_info', {}],
  ['repoStatus', () => api.repoStatus(R), 'repo_status', { repo: R }],
  ['add', () => api.add(R, ['a'], undefined, P), 'repo_add', { paths: ['a'], opts: null }],
  ['add opts', () => api.add(R, ['a'], { all: true } as never), 'repo_add', { opts: { all: true } }],
  ['lsFiles', () => api.lsFiles(R), 'repo_ls_files', { paths: null, ignored: false }],
  ['lsFiles ignored', () => api.lsFiles(R, true, ['x']), 'repo_ls_files', { paths: ['x'], ignored: true }],
  ['unstage', () => api.unstage(R, ['a']), 'repo_unstage', { paths: ['a'] }],
  ['commit', () => api.commit(R, 'm'), 'repo_commit', { message: 'm', opts: null }],
  ['init', () => api.init('/p'), 'repo_init', { path: '/p', initialBranch: null }],
  ['init branch', () => api.init('/p', 'dev'), 'repo_init', { initialBranch: 'dev' }],
  ['clone', () => api.clone('u', 'd', 'op', P), 'repo_clone', { url: 'u', dest: 'd', branch: null, opId: 'op' }],
  ['cancelOp', () => api.cancelOp('op'), 'cancel_op', { opId: 'op' }],
  ['allowRepo', () => api.allowRepo(R), 'repo_allow', { repo: R }],
  ['log', () => api.log(R, 0, 5, false), 'repo_log', { skip: 0, max: 5, all: false, path: null }],
  ['log filter', () => api.log(R, 0, 5, true, 'p', { author: 'a' }), 'repo_log', { path: 'p', filter: { author: 'a' } }],
  ['show', () => api.show(R, 'HEAD'), 'repo_show', { rev: 'HEAD', tree: false }],
  ['extract', () => api.extract(R, 'HEAD', 'f', 3, P), 'repo_extract', { path: 'f', expected: 3 }],
  ['mediaInfo', () => api.mediaInfo(R, 'f'), 'media_info', { path: 'f' }],
  ['branchList', () => api.branchList(R), 'branch_list', { repo: R }],
  ['branchList remote', () => api.branchList(R, true), 'branch_list', { remote: true }],
  ['branchCreate', () => api.branchCreate(R, 'b', { startPoint: 's' }), 'branch_create', { name: 'b', opts: { startPoint: 's' } }],
  ['branchSwitch', () => api.branchSwitch(R, 'b', P, { force: true }), 'branch_switch', { name: 'b', opts: { force: true } }],
  ['branchDelete', () => api.branchDelete(R, 'b', { remote: true }), 'branch_delete', { opts: { remote: true } }],
  ['branchRename', () => api.branchRename(R, 'a', 'b'), 'branch_rename', { from: 'a', to: 'b' }],
  ['branchRename overwrite', () => api.branchRename(R, 'a', 'b', true), 'branch_rename', { force: true, confirmed: true }],
  ['branchShow', () => api.branchShow(R, null), 'branch_show', { name: null }],
  ['branchProtect', () => api.branchProtect(R, 'b', true, true), 'branch_protect', { requireReviews: true, unprotect: true, confirmed: true }],
  ['tagList', () => api.tagList(R), 'tag_list', { pattern: null }],
  ['tagShow', () => api.tagShow(R, 't'), 'tag_show', { name: 't' }],
  ['tagVerify', () => api.tagVerify(R, 't'), 'tag_verify', { name: 't' }],
  ['tagCreate', () => api.tagCreate(R, 't', { message: 'm' }), 'tag_create', { options: { message: 'm' } }],
  ['tagDelete', () => api.tagDelete(R, 't'), 'tag_delete', { name: 't' }],
  ['lockList', () => api.lockList(R), 'lock_list', { remote: null }],
  ['lockCreate', () => api.lockCreate(R, 'f', 'me', 'o'), 'lock_create', { path: 'f', owner: 'me', remote: 'o' }],
  ['lockUnlock', () => api.lockUnlock(R, 'id', true), 'lock_unlock', { lockId: 'id', force: true, remote: null }],
  ['remoteList', () => api.remoteList(R), 'remote_list', { repo: R }],
  ['remoteAdd', () => api.remoteAdd(R, 'o', 'u'), 'remote_add', { fetch: false, token: null }],
  ['remoteAdd token', () => api.remoteAdd(R, 'o', 'u', { fetch: true, token: 'tk' }), 'remote_add', { fetch: true, token: 'tk' }],
  ['remoteRename', () => api.remoteRename(R, 'o', 'p'), 'remote_rename', { old: 'o', new: 'p' }],
  ['remoteSetUrl', () => api.remoteSetUrl(R, 'o', 'u', true), 'remote_set_url', { push: true }],
  ['remoteShow', () => api.remoteShow(R, 'o'), 'remote_show', { name: 'o' }],
  ['remoteRemove', () => api.remoteRemove(R, 'o'), 'remote_remove', { name: 'o' }],
  ['fetch', () => api.fetch(R, 'o', 'op', P), 'repo_fetch', { remote: 'o', opts: null }],
  ['pull', () => api.pull(R, null, 'op', P), 'repo_pull', { remote: null }],
  ['push', () => api.push(R, 'o', 'op', P), 'repo_push', { remote: 'o', opId: 'op' }],
  ['merge', () => api.merge(R, 'b', 'ours', { squash: true }, P), 'repo_merge', { strategy: 'ours', opts: { squash: true } }],
  ['mergeControl', () => api.mergeControl(R, 'abort', P), 'repo_merge_control', { action: 'abort' }],
  ['mergeResolve', () => api.mergeResolve(R, 'f', 'ours', 'c'), 'merge_resolve', { side: 'ours', theirs: 'c' }],
  ['diff', () => api.diff(R, null, null), 'repo_diff', { from: null, to: null }],
  ['diff cached', () => api.diff(R, null, null, true), 'repo_diff', { cached: true }],
  ['stashList', () => api.stashList(R), 'stash_list', { repo: R }],
  ['stashShow', () => api.stashShow(R, 2), 'stash_show', { index: 2 }],
  ['stashSave', () => api.stashSave(R, null, true, ['a'], P), 'stash_save', { includeUntracked: true, paths: ['a'] }],
  ['stashApply', () => api.stashApply(R, 0, { index: true }, P), 'stash_apply', { restoreIndex: true, force: false, confirmed: false }],
  ['stashPop force', () => api.stashPop(R, 0, { force: true }, P), 'stash_pop', { force: true, confirmed: true }],
  ['stashDrop', () => api.stashDrop(R, 1), 'stash_drop', { index: 1 }],
  ['stashClear', () => api.stashClear(R), 'stash_clear', { repo: R }],
  ['reset', () => api.reset(R, 'x', 'soft'), 'repo_reset', { mode: 'soft' }],
  ['resetHard', () => api.resetHard(R, 'x', true, P), 'repo_reset_hard', { rev: 'x', confirmed: true }],
  ['revert', () => api.revert(R, ['a']), 'repo_revert', { revs: ['a'] }],
  ['revert opts', () => api.revert(R, ['a'], { noCommit: true }), 'repo_revert', { opts: { noCommit: true } }],
  ['revertControl', () => api.revertControl(R, 'skip'), 'repo_revert_control', { action: 'skip' }],
  ['restoreFile', () => api.restoreFile(R, 'x', 'f'), 'repo_restore_file', { path: 'f' }],
  ['remoteLsTree', () => api.remoteLsTree(R, 'o', 'd', null), 'remote_ls_tree', { dir: 'd', gitRef: null }],
  ['downloadPreview', () => api.downloadPreview(R, 'p', null, 'op', P), 'repo_download_preview', { remotePath: 'p' }],
  ['download', () => api.download(R, 'p', 'main', 'out', 'op', P), 'repo_download', { output: 'out', gitRef: 'main' }],
  ['unlockRepo', () => api.unlockRepo(R, 'pw'), 'unlock_repo', { passphrase: 'pw' }],
  ['lockRepo', () => api.lockRepo(R), 'lock_repo', { repo: R }],
  ['stats', () => api.stats(R), 'repo_stats', { passphrase: null }],
  ['gc', () => api.gc(R, true, false, { auto: true }, undefined, P), 'repo_gc', { dryRun: true, options: { auto: true }, passphrase: null }],
  ['fsck', () => api.fsck(R, { deep: true }, 'pw', P), 'repo_fsck', { options: { deep: true }, passphrase: 'pw' }],
  ['verify', () => api.verify(R, { quick: true }, undefined, P), 'repo_verify', { options: { quick: true } }],
  ['authLogin', () => api.authLogin(null, 's', 'u', 'pw'), 'auth_login', { server: 's', password: 'pw' }],
  ['authRun', () => api.authRun(R, 'status', null), 'auth_run', { action: 'status', all: false }],
  ['serverLocalStatus', () => api.serverLocalStatus(), 'server_local_status', {}],
  ['serverLocalSetup', () => api.serverLocalSetup('u', 'e', 'p'), 'server_local_setup', { username: 'u', email: 'e' }],
  ['serverLocalSetFolder', () => api.serverLocalSetFolder('/f'), 'server_local_set_folder', { path: '/f' }],
  ['serverStorageTest', () => api.serverStorageTest({} as never), 'server_storage_test', {}],
  ['serverLocalStart', () => api.serverLocalStart(), 'server_local_start', {}],
  ['serverLocalStop', () => api.serverLocalStop(), 'server_local_stop', {}],
  ['serverLocalLog', () => api.serverLocalLog(50), 'server_local_log', { lines: 50 }],
  ['serverLocalSetNetwork', () => api.serverLocalSetNetwork(true), 'server_local_set_network', { enabled: true }],
  ['serverWhoami', () => api.serverWhoami('s'), 'auth_run', { repo: null, action: 'status', server: 's' }],
  ['serverLogin', () => api.serverLogin('s', 'u', 'p'), 'auth_login', { repo: null, username: 'u' }],
  ['serverUsers', () => api.serverUsers('s'), 'server_users_list', { server: 's' }],
  ['serverUserCreate', () => api.serverUserCreate('s', 'u', 'e', 'admin', 'p'), 'server_user_create', { role: 'admin' }],
  ['serverUserSetRole', () => api.serverUserSetRole('s', 'u', 'r'), 'server_user_set_role', { role: 'r' }],
  ['serverUserResetPassword', () => api.serverUserResetPassword('s', 'u', 'p'), 'server_user_reset_password', { password: 'p' }],
  ['serverUserDisable', () => api.serverUserDisable('s', 'u'), 'server_user_disable', { confirm: true }],
  ['serverUserEnable', () => api.serverUserEnable('s', 'u'), 'server_user_enable', { user: 'u' }],
  ['serverUserGrants', () => api.serverUserGrants('s', 'u'), 'server_user_grants', { user: 'u' }],
  ['serverRepoAccess', () => api.serverRepoAccess('s', 'r'), 'server_repo_access', { repo: 'r' }],
  ['serverGrantSet', () => api.serverGrantSet('s', 'u', 'r', 'write'), 'server_grant_set', { level: 'write' }],
  ['serverGrantRemove', () => api.serverGrantRemove('s', 'u', 'r'), 'server_grant_remove', { confirm: true }],
  ['serverKeys', () => api.serverKeys('s'), 'server_keys_list', { server: 's' }],
  ['serverKeyCreate', () => api.serverKeyCreate('s', 'n', ['read']), 'server_key_create', { permissions: ['read'] }],
  ['serverKeyRevoke', () => api.serverKeyRevoke('s', 'id'), 'server_key_revoke', { id: 'id', confirm: true }],
  ['serverRepos', () => api.serverRepos('s'), 'server_repos_list', { server: 's' }],
  ['serverRepoCreate', () => api.serverRepoCreate('s', 'n'), 'server_repo_create', { name: 'n' }],
  ['serverRepoDelete', () => api.serverRepoDelete('s', 'n', 't', true), 'server_repo_delete', { typedName: 't', force: true, confirm: true }],
  ['authRegister', () => api.authRegister(null, 'u', 'e', 'p'), 'auth_register', { username: 'u' }],
  ['authPasswd', () => api.authPasswd(null, 'old', 'new'), 'auth_passwd', { current: 'old', new: 'new' }],
  ['keyRotateMaster', () => api.keyRotateMaster(R, 'kf', null, null), 'key_rotate_master', { newKeyfile: 'kf' }],
  ['keyStatus', () => api.keyStatus(R), 'key_status', { passphrase: null }],
  ['keyInit', () => api.keyInit(R, 'pw'), 'key_init', { passphrase: 'pw' }],
  ['keyRecover', () => api.keyRecover(R, 'code', null), 'key_recover', { code: 'code' }],
  ['configList', () => api.configList(R), 'config_list', { repo: R }],
  ['configSet', () => api.configSet(R, 'k', 'v', true), 'config_set', { key: 'k', value: 'v', secret: true }],
  ['configSchema', () => api.configSchema(), 'config_schema', {}],
  ['configUnset', () => api.configUnset(R, 'k'), 'config_unset', { key: 'k' }],
  ['sparseList', () => api.sparseList(R), 'sparse_list', { repo: R }],
  ['sparseSet', () => api.sparseSet(R, ['a'], true, P), 'sparse_set', { patterns: ['a'], patternMode: true }],
  ['sparseDisable', () => api.sparseDisable(R, P), 'sparse_disable', { repo: R }],
  ['rebaseStart', () => api.rebaseStart(R, 'up', P, { keepEmpty: true }), 'rebase_start', { upstream: 'up', keepEmpty: true }],
  ['sequencerControl', () => api.sequencerControl(R, 'rebase', 'abort', P), 'sequencer_control', { command: 'rebase', action: 'abort' }],
  ['pickStart', () => api.pickStart(R, ['c'], P), 'pick_start', { commits: ['c'] }],
  ['pickStart opts', () => api.pickStart(R, ['c'], P, { noCommit: true }), 'pick_start', { opts: { noCommit: true } }],
  ['bisectStart', () => api.bisectStart(R, 'b', null, P), 'bisect_start', { bad: 'b', good: null }],
  ['bisectStart restart', () => api.bisectStart(R, 'b', 'g', P, true), 'bisect_start', { restart: true, confirmed: true }],
  ['bisectMark', () => api.bisectMark(R, 'good', null, P), 'bisect_mark', { verdict: 'good' }],
  ['bisectReset', () => api.bisectReset(R, P), 'bisect_reset', { repo: R }],
  ['bisectReset commit', () => api.bisectReset(R, P, 'c'), 'bisect_reset', { commit: 'c', confirmed: true }],
  ['bisectReplay', () => api.bisectReplay(R, 'log', P), 'bisect_replay', { logfile: 'log' }],
  ['bisectLog', () => api.bisectLog(R), 'bisect_log', { repo: R }],
  ['reflog', () => api.reflog(R, 'HEAD', 5), 'reflog_list', { reference: 'HEAD', count: 5 }],
  ['reflogAll', () => api.reflogAll(R, 5), 'reflog_all', { count: 5 }],
  ['reflogDelete', () => api.reflogDelete(R, 'HEAD'), 'reflog_delete', { confirmed: true }],
  ['reflogExpire', () => api.reflogExpire(R, null, 3), 'reflog_expire', { reference: null, keep: 3, confirmed: true }],
]

describe('api wrappers send the right command and arguments', () => {
  it.each(cases)('%s', async (_label, call, cmd, subset) => {
    await call()
    expect(core.invoke).toHaveBeenCalledTimes(1)
    const [c, args] = core.invoke.mock.calls[0]!
    expect(c).toBe(cmd)
    expect(args ?? {}).toMatchObject(subset)
  })

  it('optional flags are left out of the arguments, not sent as undefined', async () => {
    await api.branchList(R)
    await api.diff(R, null, null)
    await api.bisectReset(R, P)
    for (const [, args] of core.invoke.mock.calls) {
      expect(Object.keys(args)).not.toEqual(expect.arrayContaining(['remote', 'cached', 'commit']))
    }
  })

  it('fileSrc and onRepoChanged go to the Tauri helpers', () => {
    expect(api.fileSrc('/a')).toBe('asset:///a')
    const cb = () => {}
    api.onRepoChanged(cb)
    expect(listen).toHaveBeenCalledWith('repo-changed', cb)
  })

  it('a progress callback is wired to the channel sent to the backend', async () => {
    const seen: number[] = []
    await api.add(R, ['a'], undefined, (p) => seen.push(p.pos))
    const ch = core.invoke.mock.calls[0]![1].onProgress as { onmessage: (p: { pos: number }) => void }
    ch.onmessage({ pos: 7 })
    expect(seen).toEqual([7])
    // Without a callback the channel still swallows messages.
    await api.add(R, ['a'])
    const quiet = core.invoke.mock.calls[1]![1].onProgress as { onmessage: (p: unknown) => void }
    expect(() => quiet.onmessage({})).not.toThrow()
  })
})

describe('error helpers', () => {
  it('recognises an ApiError by code and message', () => {
    expect(isApiError({ code: 'x', message: 'y' })).toBe(true)
    expect(isApiError({ code: 'x' })).toBe(false)
    expect(isApiError(null)).toBe(false)
    expect(isApiError('boom')).toBe(false)
  })

  it('wraps anything else as an unknown error with a readable message', () => {
    expect(toApiError(new Error('bad'))).toEqual({ code: 'unknown', message: 'bad' })
    expect(toApiError('text')).toEqual({ code: 'unknown', message: 'text' })
    const e = { code: 'c', message: 'm' }
    expect(toApiError(e)).toBe(e)
  })

  it('exposes the contract version the UI was written against', () => {
    expect(CONTRACT_VERSION).toBe(1)
  })
})
