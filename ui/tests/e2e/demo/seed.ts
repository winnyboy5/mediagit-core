import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync, rmSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { CLI, CLI_ENV, CORPUS } from './demo-env'

// Deterministic seeding through the CLI (same env as qa-live: fixed author and CDC seed).

/** Run the release CLI, throw on a non-zero exit, return stdout. */
export function cli(cwd: string, args: string[]): string {
  return execFileSync(CLI, args, { cwd, env: { ...process.env, ...CLI_ENV }, encoding: 'utf8', windowsHide: true })
}

const PROJECTS = process.env.MG_DEMO_PROJECTS ?? 'D:\\Studio'

/** A fresh, empty project folder PROJECTS\name (shown on screen, so keep it short). Wipes only that folder. */
export function folder(name: string): string {
  if (!/^[A-Za-z0-9._ -]+$/.test(name) || name === '.' || name === '..') throw new Error(`bad project folder name: ${name}`)
  const d = resolve(PROJECTS, name)
  if (resolve(d, '..').toLowerCase() !== resolve(PROJECTS).toLowerCase()) throw new Error(`refusing to wipe outside ${PROJECTS}: ${d}`)
  rmSync(d, { recursive: true, force: true })
  mkdirSync(d, { recursive: true })
  return d
}

/** Copy fixtures from test-files/ (MG_QA_TESTFILES) into `dir`. */
export function copyMedia(dir: string, ...names: string[]) {
  for (const n of names) copyFileSync(join(CORPUS, n), join(dir, n))
}

/** init + add + save in one call. */
export function seedRepo(dir: string, message: string, branch = 'main') {
  cli(dir, ['init', '--initial-branch', branch, dir])
  cli(dir, ['add', '--all'])
  cli(dir, ['commit', '-m', message])
}
