#!/usr/bin/env node
// Drives .claude/workflows/ship-ticket.js with mocked agents (DOLL-654).
//
// The workflow runtime executes the script body with agent(), parallel(),
// pipeline(), phase(), log() and args in scope. This harness supplies fakes
// that return canned, schema-checked results per agent label, so every
// control-flow path (convergence, stop rules, gates, CI, merge) runs in
// milliseconds without spending a single real agent.
//
// Usage: node scripts/test-ship-ticket.mjs [path/to/ship-ticket.js]
// scripts/check.sh tooling runs it.

import fs from 'node:fs'

const scriptPath = process.argv[2] || '.claude/workflows/ship-ticket.js'
const src = fs.readFileSync(scriptPath, 'utf8')
const metaMatch = src.match(/^export const meta = (\{[\s\S]*?\n\})\n/)
if (!metaMatch) { console.error(`${scriptPath}: no leading "export const meta = {...}" block`); process.exit(1) }
const meta = Function(`return (${metaMatch[1]})`)()
const body = src.slice(metaMatch[0].length)
const AsyncFunction = (async () => {}).constructor

// ---------------------------------------------------------------- harness

function schemaErrors(schema, value, path = '$') {
  if (!schema) return []
  const errs = []
  switch (schema.type) {
    case 'object': {
      if (typeof value !== 'object' || value === null || Array.isArray(value)) return [`${path} is not an object`]
      for (const r of schema.required || []) {
        if (!(schema.properties || {})[r]) errs.push(`${path}: required "${r}" is not in properties`)
        if (!(r in value)) errs.push(`${path}.${r} missing`)
      }
      for (const [k, v] of Object.entries(value)) {
        if (!(schema.properties || {})[k]) errs.push(`${path}.${k} is not in the schema`)
        else errs.push(...schemaErrors(schema.properties[k], v, `${path}.${k}`))
      }
      break
    }
    case 'array':
      if (!Array.isArray(value)) return [`${path} is not an array`]
      value.forEach((v, i) => errs.push(...schemaErrors(schema.items, v, `${path}[${i}]`)))
      break
    case 'string':
      if (typeof value !== 'string') errs.push(`${path} is not a string`)
      else if (schema.enum && !schema.enum.includes(value)) errs.push(`${path} "${value}" is not one of ${schema.enum.join('|')}`)
      break
    case 'integer':
      if (!Number.isInteger(value)) errs.push(`${path} is not an integer`)
      break
    case 'boolean':
      if (typeof value !== 'boolean') errs.push(`${path} is not a boolean`)
      break
  }
  return errs
}

async function run(scenario) {
  const calls = []
  const prompts = {}
  const errors = []
  const phases = meta.phases.map((p) => p.title)
  const agent = async (prompt, opts = {}) => {
    const label = opts.label || '(no label)'
    calls.push(label)
    prompts[label] = prompt
    if (opts.phase && !phases.includes(opts.phase)) errors.push(`${label}: phase "${opts.phase}" is not in meta.phases`)
    const bad = typeof prompt !== 'string' ? 'not a string' : (prompt.match(/.{0,50}(undefined(?! behavior)|\[object Object\]).{0,30}/) || [])[0]
    if (bad) errors.push(`${label}: suspicious prompt text: ${bad}`)
    let r
    try { r = scenario.agents(label, prompt, calls) } catch (e) { errors.push(`${label}: ${e.message}`); return null }
    if (r === undefined) { errors.push(`no mock for agent "${label}"`); return null }
    if (r !== null && opts.schema) for (const e of schemaErrors(opts.schema, r)) errors.push(`${label}: mock violates schema: ${e}`)
    return r
  }
  const parallel = async (thunks) => Promise.all(thunks.map((t) => t().catch((e) => { errors.push(`thunk threw: ${e.message}`); return null })))
  const pipeline = async (items, ...stages) => Promise.all(items.map(async (item, i) => {
    let v = item
    for (const stage of stages) {
      try { v = await stage(v, item, i) } catch (e) { errors.push(`stage threw: ${e.message}`); return null }
    }
    return v
  }))
  const logs = []
  let result
  try {
    const fn = new AsyncFunction('agent', 'parallel', 'pipeline', 'phase', 'log', 'args', 'budget', 'workflow', body)
    result = await fn(agent, parallel, pipeline, () => {}, (m) => logs.push(m), scenario.args,
      { total: null, spent: () => 0, remaining: () => Infinity }, async () => { throw new Error('nested workflow() is not used') })
  } catch (e) {
    errors.push(`script threw: ${e.stack}`)
  }
  if (!errors.length && scenario.check) {
    const msg = scenario.check(result, calls, prompts)
    if (msg) errors.push(`check: ${msg}`)
  }
  return { result, calls, errors, logs }
}

// ---------------------------------------------------------------- fixtures

const GREEN = { green: true, exitCode: 0, failingStep: '', errorExcerpt: '', skipped: [], tail: '==> all checks green' }
const RED = (step) => ({ green: false, exitCode: 1, failingStep: step, errorExcerpt: 'error[E0308]: mismatched types', skipped: [], tail: 'exit=1' })
const OK = { ok: true, detail: 'done', head: 'def456' }
const scope = (files, extra = {}) => ({ branch: 'tibbon/doll-9-make-x', head: 'abc123', dirtyFiles: [], files, filesSince: files, touchesUnsafe: false, commits: ['aaa feat: x (DOLL-9)'], diffLines: 400, priorLog: [], ...extra })
const finding = (title, severity, origin = 'branch', file = 'src/writer_thread.rs', line = 10) => ({ title, severity, origin, file, line, evidence: 'e', whyWrong: 'w', suggestedFix: 'f', checklistRef: '' })
const findings = (...fs) => ({ findings: fs, checked: ['looked'] })
const NONE = findings()
const verdict = (v, severity = 'medium', origin = 'branch', extra = {}) => ({ verdict: v, duplicateOf: '', stillPresent: false, severity, origin, reasoning: 'read src/x.rs:1', fix: 'do y', ...extra })
const fixed = (ids, extra = {}) => ({ results: ids.map((id) => ({ id, status: 'fixed', commit: 'f1', note: '' })), filesChanged: ['src/writer_thread.rs'], linesChanged: 12, ...extra })
const intake = (extra = {}) => ({ ok: true, stopReason: '', candidates: [], ticketId: 'DOLL-9', ticketTitle: 'Make X', ticketUrl: 'https://linear.app/x/DOLL-9', ticketState: 'Todo', ticketText: 'Do X', branchName: 'tibbon/doll-9-make-x', currentBranch: 'main', dirtyFiles: [], branchExists: false, branchCommits: [], filesChanged: [], prNumber: 0, ...extra })
const reviewIntake = (extra = {}) => intake({ currentBranch: 'tibbon/doll-9-make-x', branchExists: true, branchCommits: ['aaa feat: x (DOLL-9)'], filesChanged: ['src/writer_thread.rs'], ...extra })
const plan = (extra = {}) => ({ needsDecision: false, questions: [], alreadyDone: false, summary: 'S', approach: 'A', filesToTouch: ['src/writer_thread.rs'], testsToAdd: ['t'], risks: [], commitPlan: ['feat: x (DOLL-9)'], userVisible: true, ...extra })
const impl = (extra = {}) => ({ status: 'done', blockedReason: '', branch: 'tibbon/doll-9-make-x', commits: ['aaa feat: x (DOLL-9)'], filesChanged: ['src/writer_thread.rs', 'src/tests/writer_thread_tests.rs'], summary: 'Did X.', apiChanges: [], ffiChanges: [], layoutChanges: [], newDependencies: [], lintExpectations: [], testsAdded: ['writer_thread_tests::x'], deviationsFromPlan: [], releaseNote: 'X is faster.', ...extra })
const published = (n = 42) => ({ ok: true, detail: '', prNumber: n, prUrl: `https://github.com/tibbon/audio_blackbox/pull/${n}`, followUps: [] })
const CI_GREEN = { state: 'green', failures: [] }

// Table-driven mock: exact labels first, then prefix rules, else no mock.
const mocks = (table, rules = []) => (label, prompt, calls) => {
  if (label in table) {
    const v = table[label]
    return typeof v === 'function' ? v(prompt, calls) : v
  }
  for (const [prefix, v] of rules) if (label.startsWith(prefix)) return typeof v === 'function' ? v(label, prompt, calls) : v
  return undefined
}
const quietReviewers = ['review:', NONE]
const has = (calls, label) => calls.includes(label)

// ---------------------------------------------------------------- scenarios

const scenarios = [
  {
    name: 'review mode: gate runs first, clean round converges, low finding becomes a note',
    args: 'review',
    agents: mocks({ 'intake:review': reviewIntake(), 'gate:review-start-1': GREEN, 'scope:r1': scope(['src/writer_thread.rs']) },
      [['review:baseline:r1', findings(finding('nit', 'low'))], quietReviewers]),
    check: (r, calls) => calls[1] !== 'gate:review-start-1' ? 'gate did not run before review'
      : r.outcome !== 'review-converged' ? `outcome ${r.outcome}` : r.notes.length !== 1 ? 'low finding not noted' : '',
  },
  {
    name: 'review mode: a header change gates Rust and Swift together',
    args: 'review',
    agents: mocks({
      'intake:review': reviewIntake({ filesChanged: ['src/ffi.rs', 'include/blackbox_ffi.h'] }),
      'gate:review-start-1': (prompt) => { if (!prompt.includes('Run: ./scripts/check.sh >')) throw new Error('a header change is not gated with the default section'); return GREEN },
      'scope:r1': scope(['src/ffi.rs', 'include/blackbox_ffi.h']),
    }, [quietReviewers]),
    check: (r) => r.outcome !== 'review-converged' ? `outcome ${r.outcome}` : '',
  },
  {
    name: 'review mode: red gate stops before any reviewer',
    args: 'review',
    agents: mocks({ 'intake:review': reviewIntake(), 'gate:review-start-1': RED('tests'), 'gate-fix:review-start-1': OK, 'gate:review-start-2': RED('tests') }),
    check: (r, calls) => r.stage !== 'gate' ? `stage ${r.stage}` : calls.some((c) => c.startsWith('review:')) ? 'reviewers ran on a red gate' : '',
  },
  {
    name: 'review mode: round 2 reviews only the fixes; a late branch medium is a note, not a blocker',
    args: 'review',
    agents: mocks({
      'intake:review': reviewIntake(), 'gate:review-start-1': GREEN,
      'scope:r1': scope(['src/writer_thread.rs', 'src/ffi.rs'], { touchesUnsafe: true }),
      'review:baseline:r1': findings(finding('off by one', 'high'), finding('style', 'low', 'branch', 'src/writer_thread.rs', 200)),
      'review:realtime-audio:r1': findings(finding('alloc in callback', 'high', 'branch', 'src/cpal_processor.rs', 5)),
      'verify:R1.1': verdict('confirmed', 'high'),
      'verify:R1.3': verdict('refuted', 'high'),
      'fix:r1': fixed(['R1.1']),
      'gate:review-r1-1': GREEN,
      'scope:r2': scope(['src/writer_thread.rs', 'src/ffi.rs'], { filesSince: ['src/writer_thread.rs'] }),
      'review:guardrails:r2': findings(finding('late medium elsewhere', 'medium', 'branch', 'src/ffi.rs', 99)),
      'review-log': OK,
    }, [['review:', (label, prompt) => {
      if (label.endsWith(':r2') && !prompt.includes('R1.3 [refuted')) throw new Error('refuted finding missing from the round 2 seen list')
      if (label.endsWith(':r2') && !prompt.includes('Review those fix commits: git diff abc123..HEAD')) throw new Error('round 2 prompt does not scope to the fix commits')
      return NONE
    }]]),
    check: (r, calls) => has(calls, 'verify:R2.1') ? 'a late branch medium was verified as a blocker'
      : r.outcome !== 'review-converged' ? `outcome ${r.outcome}: ${r.reason}`
      : !r.notes.some((n) => n.startsWith('late medium')) ? 'late medium not kept as a note' : '',
  },
  {
    name: 'review mode: out-of-scope findings become suggestions and do not block',
    args: 'review',
    agents: mocks({
      'intake:review': reviewIntake(), 'gate:review-start-1': GREEN, 'scope:r1': scope(['scripts/check.sh']),
      'review:build-release:r1': findings(finding('should also lint fastlane', 'medium', 'branch', 'scripts/check.sh', 3), finding('not a bug', 'medium', 'branch', 'scripts/check.sh', 40)),
      'verify:R1.1': verdict('out-of-scope', 'medium'), 'verify:R1.2': verdict('refuted', 'medium'),
      'review-log': (prompt) => {
        if (!prompt.includes('refuted scripts/check.sh:40')) throw new Error('refuted verdict not logged')
        if (prompt.includes('scripts/check.sh:3 ')) throw new Error('out-of-scope verdict logged, so a rerun would never file it')
        return OK
      },
    }, [quietReviewers]),
    check: (r, calls) => has(calls, 'fix:r1') ? 'fixer ran for an out-of-scope finding'
      : !has(calls, 'review-log') ? 'review log not written'
      : r.outcome !== 'review-converged' ? `outcome ${r.outcome}` : r.suggestions.length !== 1 ? 'suggestion not recorded' : '',
  },
  {
    name: 'review mode: findings that do not halve stop the loop',
    args: 'review',
    agents: mocks({
      'intake:review': reviewIntake(), 'gate:review-start-1': GREEN,
      'scope:r1': scope(['src/config.rs']),
      'review:baseline:r1': findings(finding('a', 'medium', 'branch', 'src/config.rs', 10), finding('b', 'medium', 'branch', 'src/config.rs', 60)),
      'verify:R1.1': verdict('confirmed'), 'verify:R1.2': verdict('confirmed'),
      'fix:r1': fixed(['R1.1', 'R1.2'], { filesChanged: ['src/config.rs'] }),
      'gate:review-r1-1': GREEN,
      'scope:r2': scope(['src/config.rs']),
      'review:baseline:r2': findings(finding('c', 'medium', 'fix', 'src/config.rs', 110), finding('d', 'medium', 'fix', 'src/config.rs', 160)),
      'verify:R2.1': verdict('confirmed', 'medium', 'fix'), 'verify:R2.2': verdict('confirmed', 'medium', 'fix'),
    }, [quietReviewers]),
    check: (r, calls) => has(calls, 'fix:r2') ? 'fixed a round that did not halve'
      : r.outcome !== 'review-not-converged' || !/not settling/.test(r.reason) ? `outcome ${r.outcome}: ${r.reason}` : '',
  },
  {
    name: 'review mode: a fix that did not hold is refixed, then converges',
    args: { mode: 'review', maxRounds: 3 },
    agents: mocks({
      'intake:review': reviewIntake(), 'gate:review-start-1': GREEN,
      'scope:r1': scope(['src/config.rs']), 'scope:r2': scope(['src/config.rs']), 'scope:r3': scope(['src/config.rs']),
      'review:baseline:r1': findings(finding('bad parse', 'medium', 'branch', 'src/config.rs', 3)),
      'verify:R1.1': verdict('confirmed'),
      'fix:r1': fixed(['R1.1'], { filesChanged: ['src/config.rs'] }), 'gate:review-r1-1': GREEN,
      'review:baseline:r2': (prompt) => {
        if (!prompt.includes('except a finding marked fixed whose defect is still present')) throw new Error('seen list forbids re-reporting a fix that did not hold')
        return findings(finding('bad parse still', 'medium', 'fix', 'src/config.rs', 4))
      },
      'verify:R2.1': verdict('duplicate', 'medium', 'fix', { duplicateOf: 'R1.1', stillPresent: true }),
      'fix:r2': fixed(['R2.1'], { filesChanged: ['src/config.rs'] }), 'gate:review-r2-1': GREEN,
    }, [quietReviewers]),
    check: (r) => r.outcome !== 'review-converged' ? `outcome ${r.outcome}: ${r.reason}` : r.fixed.length !== 2 ? `fixed ${r.fixed.length}` : '',
  },
  {
    name: 'review mode: a duplicate of a fixed finding that is gone does not reopen it',
    args: 'review',
    agents: mocks({
      'intake:review': reviewIntake(), 'gate:review-start-1': GREEN,
      'scope:r1': scope(['src/config.rs']), 'scope:r2': scope(['src/config.rs']),
      'review:baseline:r1': findings(finding('bad parse', 'medium', 'branch', 'src/config.rs', 3)),
      'verify:R1.1': verdict('confirmed'),
      'fix:r1': fixed(['R1.1'], { filesChanged: ['src/config.rs'] }), 'gate:review-r1-1': GREEN,
      'review:baseline:r2': findings(finding('bad parse', 'medium', 'fix', 'src/config.rs', 3)),
      'verify:R2.1': verdict('duplicate', 'medium', 'fix', { duplicateOf: 'R1.1', stillPresent: false }),
    }, [quietReviewers]),
    check: (r, calls) => has(calls, 'fix:r2') ? 'reopened a fixed finding that is gone' : r.outcome !== 'review-converged' ? `outcome ${r.outcome}` : '',
  },
  {
    name: 'review mode: the last round reports findings instead of fixing them',
    args: 'review rounds=2',
    agents: mocks({
      'intake:review': reviewIntake(), 'gate:review-start-1': GREEN,
      'scope:r1': scope(['src/config.rs']), 'scope:r2': scope(['src/config.rs']),
      'review:baseline:r1': findings(finding('a', 'medium', 'branch', 'src/config.rs', 10), finding('b', 'medium', 'branch', 'src/config.rs', 60)),
      'verify:R1.1': verdict('confirmed'), 'verify:R1.2': verdict('confirmed'),
      'fix:r1': fixed(['R1.1', 'R1.2'], { filesChanged: ['src/config.rs'] }), 'gate:review-r1-1': GREEN,
      'review:baseline:r2': findings(finding('c', 'medium', 'fix', 'src/config.rs', 110)),
      'verify:R2.1': verdict('confirmed', 'medium', 'fix'),
    }, [quietReviewers]),
    check: (r, calls) => has(calls, 'fix:r2') ? 'fixed in the last round'
      : r.outcome !== 'review-not-converged' || !/round cap/.test(r.reason) ? `outcome ${r.outcome}: ${r.reason}`
      : !r.nextSteps.some((s) => /did not converge/.test(s)) ? 'next steps do not say it did not converge' : '',
  },
  {
    name: 'review mode: a fix pass over the growth limit stops the loop',
    args: 'review',
    agents: mocks({
      'intake:review': reviewIntake(), 'gate:review-start-1': GREEN, 'scope:r1': scope(['src/config.rs'], { diffLines: 200 }),
      'review:baseline:r1': findings(finding('a', 'medium', 'branch', 'src/config.rs', 1)),
      'verify:R1.1': verdict('confirmed'),
      'fix:r1': (prompt) => { if (!prompt.includes('under 80 changed lines')) throw new Error('growth limit missing from the fixer prompt'); return fixed(['R1.1'], { linesChanged: 300 }) },
    }, [quietReviewers]),
    check: (r, calls) => has(calls, 'gate:review-r1-1') ? 'gated an oversized fix pass'
      : r.outcome !== 'review-not-converged' || !/growth limit/.test(r.reason) ? `outcome ${r.outcome}: ${r.reason}` : '',
  },
  {
    name: 'review mode: a deferred medium becomes a suggestion; a deferred high stays open',
    args: 'review',
    agents: mocks({
      'intake:review': reviewIntake(), 'gate:review-start-1': GREEN, 'scope:r1': scope(['src/config.rs']),
      'review:baseline:r1': findings(finding('needs redesign', 'medium', 'branch', 'src/config.rs', 10), finding('big one', 'high', 'branch', 'src/config.rs', 60)),
      'verify:R1.1': verdict('confirmed'), 'verify:R1.2': verdict('confirmed', 'high'),
      'fix:r1': { results: [{ id: 'R1.1', status: 'deferred', commit: '', note: 'needs a new option' }, { id: 'R1.2', status: 'deferred', commit: '', note: 'too large' }], filesChanged: [], linesChanged: 0 },
    }, [quietReviewers]),
    check: (r) => r.outcome !== 'review-not-converged' ? `outcome ${r.outcome}` : r.suggestions.length !== 1 || r.openItems.length !== 1 ? `suggestions ${r.suggestions.length}, open ${r.openItems.length}` : '',
  },
  {
    name: 'review mode: a reviewer that returns nothing twice blocks convergence',
    args: 'review',
    agents: mocks({ 'intake:review': reviewIntake(), 'gate:review-start-1': GREEN, 'scope:r1': scope(['src/config.rs']) },
      [['review:rust-core', null], quietReviewers]),
    check: (r, calls) => !has(calls, 'review:rust-core:r1:retry') ? 'no retry' : r.outcome !== 'review-not-converged' ? `outcome ${r.outcome}` : '',
  },
  {
    name: 'review mode: a skeptic that returns nothing blocks convergence',
    args: 'review',
    agents: mocks({ 'intake:review': reviewIntake(), 'gate:review-start-1': GREEN, 'scope:r1': scope(['src/config.rs']),
      'review:baseline:r1': findings(finding('a', 'high', 'branch', 'src/config.rs', 1)), 'verify:R1.1': null }, [quietReviewers]),
    check: (r, calls) => has(calls, 'fix:r1') ? 'fixed without a verdict' : r.outcome !== 'review-not-converged' ? `outcome ${r.outcome}` : '',
  },
  {
    name: 'ship: no ticket lists candidates',
    args: '',
    agents: mocks({ intake: intake({ ok: false, stopReason: 'no ticket given', ticketId: '', candidates: ['DOLL-1 a [Todo, High]'] }) }),
    check: (r) => r.candidates.length !== 1 ? 'no candidates' : '',
  },
  {
    name: 'ship: a ticket that needs a decision stops before any branch work',
    args: 'DOLL-9',
    agents: mocks({ intake: intake(), plan: plan({ needsDecision: true, questions: ['Which format?'] }) }),
    check: (r, calls) => r.outcome !== 'needs-decision' || calls.length !== 2 ? `outcome ${r.outcome} after ${calls.join(', ')}` : '',
  },
  {
    name: 'ship: a stage word right after the ticket stops there and the rest is guidance',
    args: 'DOLL-9 plan then ask me about the format',
    agents: mocks({ intake: intake(), plan: (prompt) => { if (!prompt.includes('started this run: then ask me about the format')) throw new Error('guidance after the stage word lost'); return plan() } }),
    check: (r, calls) => r.outcome !== 'planned' || calls.length !== 2 ? `outcome ${r.outcome} after ${calls.join(', ')}` : '',
  },
  {
    name: 'review mode: reviewers wording one defect differently count as one finding',
    args: 'review',
    agents: mocks({
      'intake:review': reviewIntake(), 'gate:review-start-1': GREEN, 'scope:r1': scope(['.claude/workflows/ship-ticket.js', 'scripts/check.sh']),
      'review:baseline:r1': findings(finding('Tests corrected list is too broad', 'medium', 'branch', '.claude/workflows/ship-ticket.js', 1004)),
      'review:guardrails:r1': findings(finding('Every test commit is listed as corrected', 'medium', 'branch', '.claude/workflows/ship-ticket.js', 1004)),
      'review:build-release:r1': findings(finding('PR lists added tests as corrections', 'high', 'branch', '.claude/workflows/ship-ticket.js', 1006)),
      'verify:R1.1': (prompt) => { if (!prompt.includes('Other reviewers reported at the same place') || !prompt.includes('PR lists added tests as corrections')) throw new Error('skeptic does not see the merged reports'); return verdict('confirmed', 'medium') },
      'fix:r1': fixed(['R1.1'], { filesChanged: ['.claude/workflows/ship-ticket.js'] }), 'gate:review-r1-1': GREEN,
      'scope:r2': scope(['.claude/workflows/ship-ticket.js']),
    }, [quietReviewers]),
    check: (r, calls) => calls.some((c) => /^verify:R1\.[2-9]/.test(c)) ? 'one defect verified more than once'
      : r.outcome !== 'review-converged' ? `outcome ${r.outcome}: ${r.reason}` : r.rounds[0].raised !== 1 ? `raised ${r.rounds[0].raised}` : '',
  },
  {
    name: 'ship: happy path with a gate fix, a CI fix that is gated before push, and a behind branch',
    args: 'DOLL-9 keep the retry small',
    agents: mocks({
      intake: intake(),
      plan: (prompt) => { if (!prompt.includes('Guidance from the person who started this run: keep the retry small')) throw new Error('guidance lost'); return plan() },
      implement: (prompt) => { if (!prompt.includes('"test: correct <test name> (DOLL-9)"')) throw new Error('implementer is not told how to commit a corrected test'); return impl() },
      'gate:implement-1': RED('clippy --features=ffi'), 'gate-fix:implement-1': OK, 'gate:implement-2': GREEN,
      'scope:r1': scope(impl().filesChanged),
      tidy: (prompt) => { if (!prompt.includes('git fetch origin --quiet, then GIT_SEQUENCE_EDITOR')) throw new Error('first tidy does not rebase onto a fresh main'); return OK },
      'gate:final-1': GREEN,
      publish: (prompt) => {
        if (!prompt.includes('**Release note:** X is faster.')) throw new Error('release note missing from the PR body')
        if (!prompt.includes("<<'EOF'")) throw new Error('PR title not written through a quoted heredoc')
        if (!prompt.includes('someone pushed work this checkout lacks')) throw new Error('no remote safety check before push')
        if (!prompt.includes('HEAD..origin/tibbon/doll-9-make-x ^origin/main')) throw new Error('remote safety counts commits already on main as remote-only work')
        if (!prompt.includes('git log --no-merges --format="%h %s" HEAD..origin/')) throw new Error('remote safety counts a merge-from-main update commit as remote-only work')
        if (!prompt.includes('**Tests corrected**')) throw new Error('corrected tests are not explained in the PR body')
        return published()
      },
      'ci:1': { state: 'failed', failures: [{ check: 'Swift app', excerpt: 'boom' }] },
      'ci-fix:1': (prompt) => { if (/git push/.test(prompt.split('Do not squash')[1] || 'git push')) throw new Error('CI fixer is allowed to push'); return { ok: true, detail: 'fixed a Swift warning', head: 'x' } },
      'gate:ci-fix-1-1': GREEN,
      'ci-push:1': (prompt) => { if (!prompt.includes('Do not fetch.')) throw new Error('CI push rebases onto a newer main'); return OK },
      'ci:2': (prompt) => { if (!prompt.includes('Security audit')) throw new Error('CI watcher has no required checks'); return CI_GREEN },
      'merge:1': (prompt) => {
        if (!prompt.includes('git merge-base --is-ancestor origin/main')) throw new Error('merge trusts mergeStateStatus alone for up-to-date')
        if (!prompt.includes('never merge while it is UNKNOWN')) throw new Error('merge does not wait out an UNKNOWN merge state')
        return { merged: false, updatedBranch: true, detail: 'behind', mainHead: '' }
      },
      'ci:after-update-1': CI_GREEN,
      'merge:2': { merged: true, updatedBranch: false, detail: 'merged', mainHead: 'zzz' },
      linear: { ok: true, state: 'Done', detail: '' },
    }, [quietReviewers]),
    check: (r, calls) => {
      const order = ['implement', 'gate:implement-2', 'scope:r1', 'tidy', 'gate:final-1', 'publish', 'ci:1', 'ci-fix:1', 'gate:ci-fix-1-1', 'ci-push:1', 'ci:2', 'merge:1', 'ci:after-update-1', 'merge:2', 'linear']
      const idx = order.map((l) => calls.indexOf(l))
      if (idx.some((i) => i < 0)) return `missing calls: ${order.filter((_, i) => idx[i] < 0).join(', ')}`
      if (idx.some((v, i) => i && v < idx[i - 1])) return `out of order: ${calls.join(' > ')}`
      return r.outcome !== 'merged' ? `outcome ${r.outcome}` : !r.changedAfterReview.includes('fixed a Swift warning') ? 'CI fix not reported as changed after review' : ''
    },
  },
  {
    name: 'ship: review that does not converge pushes nothing',
    args: { ticket: 'DOLL-9', maxRounds: 1 },
    agents: mocks({ intake: intake(), plan: plan(), implement: impl(), 'gate:implement-1': GREEN, 'scope:r1': scope(impl().filesChanged),
      'review:baseline:r1': findings(finding('bug', 'high')), 'verify:R1.1': verdict('confirmed', 'high') }, [quietReviewers]),
    check: (r, calls) => calls.some((c) => /^(tidy|publish)/.test(c)) ? 'shipped without convergence' : r.stage !== 'review' ? `stage ${r.stage}` : '',
  },
  {
    name: 'ship: pre-existing highs and out-of-scope items become follow-ups; no-merge skips the merge',
    args: 'DOLL-9 no-merge',
    agents: mocks({
      intake: intake(), plan: plan(), implement: impl(), 'gate:implement-1': GREEN,
      'scope:r1': scope(impl().filesChanged),
      'review:rust-core:r1': findings(finding('old race', 'high', 'preexisting', 'src/utils.rs', 9), finding('add retries', 'medium', 'branch', 'src/utils.rs', 20), finding('old nit', 'medium', 'preexisting', 'src/utils.rs', 30)),
      'verify:R1.1': verdict('confirmed', 'high', 'preexisting'),
      'verify:R1.2': verdict('out-of-scope', 'medium'),
      'review-log': OK, tidy: OK, 'gate:final-1': GREEN,
      publish: (prompt) => {
        if (!prompt.includes('old race at src/utils.rs:9')) throw new Error('pre-existing defect not filed')
        if (!prompt.includes('Review suggestions from DOLL-9') || !prompt.includes('add retries')) throw new Error('suggestion not grouped')
        if (prompt.split('BODY START')[0].split('2. Protect')[0].includes('old nit')) throw new Error('a pre-existing medium was filed as a ticket')
        return published(43)
      },
      'ci:1': CI_GREEN, linear: { ok: true, state: 'In Review', detail: '' },
    }, [['merge', () => { throw new Error('merged despite no-merge') }], quietReviewers]),
    check: (r, calls) => has(calls, 'verify:R1.3') ? 'verified a pre-existing medium' : r.outcome !== 'pr-open' ? `outcome ${r.outcome}` : '',
  },
  {
    name: 'ship: missing tools stop the implement gate without a fixer',
    args: 'DOLL-9',
    agents: mocks({ intake: intake(), plan: plan(), implement: impl(),
      'gate:implement-1': { green: false, exitCode: 1, failingStep: '==> green, with skipped steps:', errorExcerpt: '', skipped: ['cargo-deny not installed'], tail: '' } }),
    check: (r, calls) => has(calls, 'gate-fix:implement-1') ? 'sent a fixer for missing tools' : !/missing tools/.test(r.reason) ? `reason ${r.reason}` : '',
  },
  {
    name: 'ship: a gate that makes no progress stops',
    args: 'DOLL-9',
    agents: mocks({ intake: intake(), plan: plan(), implement: impl(), 'gate:implement-1': RED('tests'), 'gate-fix:implement-1': OK, 'gate:implement-2': RED('tests') }),
    check: (r) => !/no progress/.test(r.reason) ? `reason ${r.reason}` : '',
  },
  {
    name: 'ship: final gate fixes squash in place and are listed in the PR; stop after pr',
    args: 'DOLL-9 pr',
    agents: mocks({
      intake: intake({ branchExists: true, branchCommits: ['aaa feat'], prNumber: 7 }), plan: plan(), implement: impl(), 'gate:implement-1': GREEN,
      'scope:r1': scope(impl().filesChanged), tidy: OK,
      'gate:final-1': RED('swiftlint'), 'gate-fix:final-1': { ok: true, detail: 'removed an unused import', head: 'y' }, 'gate:final-2': GREEN,
      'tidy:after-gate-fixes': (prompt) => { if (!prompt.includes('Do not fetch.')) throw new Error('re-tidy rebases onto a newer main'); return OK },
      publish: (prompt) => {
        if (!prompt.includes('Changed after review') || !prompt.includes('removed an unused import')) throw new Error('gate fix missing from the PR body')
        if (!prompt.includes('gh pr edit 7')) throw new Error('existing PR not reused')
        return published(7)
      },
    }, [quietReviewers]),
    check: (r) => r.outcome !== 'pr-open' || r.stage !== 'pr' ? `outcome ${r.outcome} at ${r.stage}` : '',
  },
  {
    name: 'ship: Swift sources add sanitizers and fastlane adds the App Store lint to the final gate',
    args: 'DOLL-9 pr',
    agents: mocks({
      intake: intake(), plan: plan(), implement: impl({ filesChanged: ['BlackBoxApp/BlackBoxApp/MeterView.swift', 'BlackBoxApp/fastlane/metadata/en-US/description.txt'] }),
      'gate:implement-1': (prompt) => { if (!prompt.includes('./scripts/check.sh swift >')) throw new Error('implement gate is not the swift section'); return GREEN },
      'scope:r1': scope(['BlackBoxApp/BlackBoxApp/MeterView.swift', 'BlackBoxApp/fastlane/metadata/en-US/description.txt']), tidy: OK,
      'gate:final-1': (prompt) => {
        if (!prompt.includes('./scripts/check.sh all;')) throw new Error('final gate does not run sanitizers')
        if (!prompt.includes('make check-app-store')) throw new Error('final gate skips the App Store lint')
        return GREEN
      },
      publish: published(),
    }, [quietReviewers]),
    check: (r) => r.outcome !== 'pr-open' ? `outcome ${r.outcome}` : '',
  },
  {
    name: 'ship: a flaky CI failure is re-run, not fixed',
    args: 'DOLL-9',
    agents: mocks({
      intake: intake(), plan: plan(), implement: impl(), 'gate:implement-1': GREEN, 'scope:r1': scope(impl().filesChanged),
      tidy: OK, 'gate:final-1': GREEN, publish: published(),
      'ci:1': { state: 'failed', failures: [{ check: 'Swift app', excerpt: 'runner lost' }] },
      'ci-fix:1': { ok: false, detail: 'flaky: runner lost connection', head: '' },
      'ci-rerun:1': OK, 'ci:rerun-1': CI_GREEN,
      'merge:1': { merged: true, updatedBranch: false, detail: 'merged', mainHead: 'zzz' },
      linear: { ok: true, state: 'Done', detail: '' },
    }, [quietReviewers]),
    check: (r, calls) => calls.some((c) => c.startsWith('ci-push')) ? 'pushed for a flaky failure' : r.outcome !== 'merged' ? `outcome ${r.outcome}` : '',
  },
]

// ---------------------------------------------------------------- run

let failed = 0
for (const s of scenarios) {
  const r = await run(s)
  if (r.errors.length) {
    failed++
    console.log(`FAIL  ${s.name}`)
    for (const e of r.errors) console.log(`      ${e}`)
    console.log(`      calls: ${r.calls.join(' > ')}`)
  } else {
    console.log(`ok    ${s.name}`)
  }
}
console.log(`\n${scenarios.length - failed}/${scenarios.length} ship-ticket scenarios passed`)
process.exit(failed ? 1 : 0)
