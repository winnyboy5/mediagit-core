// Verdict for one live-QA run: node summarize.mjs <runDir> [playwright.json]
// Reads every <ID>-<slug>.json the Recorder wrote in <runDir>, writes <runDir>/gates.tsv
// (case, result, checks, failed, detail) and exits 1 on any FAIL or any missing case.
// Playwright's exit code is not the verdict: checks are soft, so it exits 0 with failed checks.
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const dir = process.argv[2]
if (!dir || !existsSync(dir)) { console.error('usage: node summarize.mjs <runDir> [playwright.json]'); process.exit(2) }
const pwFile = process.argv[3] ?? join(dir, 'playwright.json')

// Cases the Recorder finished. A case counts failed when ANY check or ambient check is not ok,
// whatever its own `result` field says.
const rows = []
for (const f of readdirSync(dir).filter((f) => /\.json$/.test(f) && f !== 'playwright.json').sort()) {
  let j
  try { j = JSON.parse(readFileSync(join(dir, f), 'utf8')) } catch (e) { rows.push({ id: f, result: 'FAIL', n: 0, bad: 1, detail: `unreadable JSON: ${e.message}` }); continue }
  if (typeof j.id !== 'string' || !Array.isArray(j.checks)) continue // not a case file
  const all = [...j.checks, ...(j.ambient ?? [])]
  const failed = all.filter((c) => !c.ok)
  let result = failed.length ? 'FAIL' : j.result === 'SKIP' ? 'SKIP' : 'PASS'
  // Only ambient checks = the case asserted nothing of its own: not a pass.
  if (result === 'PASS' && j.checks.length === 0) { result = 'FAIL'; failed.push({ what: 'the case recorded no checks of its own (only ambient ones)' }) }
  if (j.result === 'FAIL' && !failed.length) result = 'FAIL' // result says FAIL but nothing to show: still red
  const detail = failed.length ? failed.map((c) => c.what + (c.detail ? ` [${String(c.detail).slice(0, 120)}]` : '')).join(' ; ') : j.skipReason ?? ''
  rows.push({ id: j.id, result, n: all.length, bad: failed.length, detail })
}

// Cases Playwright ran (not skipped) that left no JSON: crashed before finish(), or never reached it.
const missing = []
let pwTests = 0
const coveredIds = new Set() // every id token of a covered shared-title test ("R-05 + R-06" covers both)
if (existsSync(pwFile)) {
  const walk = (s) => {
    for (const sp of s.specs ?? []) for (const t of sp.tests ?? []) {
      const results = t.results ?? []
      if (t.status === 'skipped' && results.every((r) => r.status === 'skipped')) continue
      pwTests++
      // "S-06a + S-03 Title": every " + " segment starts with an id. A test is covered when ANY of its ids
      // equals a case JSON id exactly (a multi-id test writes one JSON); no prefix matching.
      const toks = sp.title.split(' + ').map((x) => x.split(' ')[0])
      const tok = toks.find((t) => rows.some((r) => r.id === t)) ?? toks[0]
      const has = rows.some((r) => r.id === tok)
      if (has) for (const t of toks) coveredIds.add(t.toLowerCase())
      const failed = results.some((r) => r.status !== 'passed')
      if (!has) missing.push({ id: tok, detail: `Playwright ran "${sp.title.slice(0, 80)}" but wrote no case JSON${failed ? ' (the test failed/crashed)' : ''}` })
      else if (failed) rows.push({ id: tok, result: 'FAIL', n: 0, bad: 1, detail: `Playwright test "${sp.title.slice(0, 80)}" threw: ${(results.find((r) => r.status !== 'passed')?.error?.message ?? '').split('\n')[0].slice(0, 150)}` })
    }
    for (const c of s.suites ?? []) walk(c)
  }
  for (const s of JSON.parse(readFileSync(pwFile, 'utf8')).suites ?? []) walk(s)
} else {
  missing.push({ id: '(playwright)', detail: `no Playwright JSON report at ${pwFile}; cannot tell which cases ran` })
}
for (const m of missing) rows.push({ id: m.id, result: 'FAIL', n: 0, bad: 1, detail: m.detail })

// Redact anything token-shaped before it reaches gates.tsv or the console.
// QALIVE_IDS (set by the runner from -Cases): every requested id must have produced a case.
for (const id of (process.env.QALIVE_IDS ?? '').split(',').filter(Boolean)) {
  if (!rows.some((r) => r.id.toLowerCase() === id.toLowerCase()) && !coveredIds.has(id.toLowerCase())) rows.push({ id, result: 'FAIL', n: 0, bad: 1, detail: `requested case ${id} produced no result` })
}
const clean = (s) => String(s)
  .replace(/mgk_[A-Za-z0-9_-]+/g, 'mgk_****')
  .replace(/(Bearer\s+)[A-Za-z0-9._~+/=-]+/gi, '$1****')
  .replace(/(AccountKey=|sig=|X-Amz-Signature=|X-Amz-Credential=|X-Amz-Security-Token=|X-Goog-Signature=)[^;&\s]+/gi, '$1****')
  .replace(/AKIA[0-9A-Z]{16}/g, 'AKIA****')
  .replace(/eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_.-]+/g, 'jwt_****')
  .replace(/[\t\r\n]+/g, ' ')
const tsv = ['case\tresult\tchecks\tfailed\tdetail', ...rows.map((r) => [r.id, r.result, r.n, r.bad, clean(r.detail)].join('\t'))].join('\n') + '\n'
writeFileSync(join(dir, 'gates.tsv'), tsv)
const count = (k) => rows.filter((r) => r.result === k).length
console.log(`qa-live verdict: ${count('PASS')} pass, ${count('FAIL')} fail, ${count('SKIP')} skip (${rows.length} cases, ${pwTests} Playwright tests)`)
for (const r of rows.filter((r) => r.result === 'FAIL')) console.log(`  FAIL ${r.id}: ${clean(r.detail).slice(0, 200)}`)
if (count('FAIL') === 0 && count('PASS') === 0) console.log('  no case passed (nothing ran, or every case was skipped): not a green run')
process.exit(count('FAIL') > 0 || count('PASS') === 0 ? 1 : 0)
