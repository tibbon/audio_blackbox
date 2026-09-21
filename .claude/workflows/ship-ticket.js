export const meta = {
  name: 'ship-ticket',
  description: 'Ship a Linear ticket the same way every time: plan, branch, implement, make check, review until it converges, PR, CI, merge, Linear',
  whenToUse: 'Any change to audio_blackbox. "/ship-ticket DOLL-N" runs the whole loop. "/ship-ticket review" runs only the review loop on the current branch. A stage word right after the ticket (plan | implement | review | pr) stops after that stage; no-merge, rounds=N and bar=low|medium|high|critical tune the run; any other text is guidance for the planner.',
  phases: [
    { title: 'Intake', detail: 'read the ticket and the working tree; nothing is changed' },
    { title: 'Plan', detail: 'read-only plan; stops with questions when the ticket needs a decision' },
    { title: 'Implement', detail: 'branch, implement, commit, scoped make check until green' },
    { title: 'Review', detail: 'round 1 reviews the branch; later rounds review only the fixes' },
    { title: 'Verify', detail: 'a skeptic refutes, confirms, or sends each finding to a follow-up' },
    { title: 'Fix', detail: 'one fixer applies small confirmed fixes as fixup commits, then the gate' },
    { title: 'Ship', detail: 'autosquash, full make check, PR, CI, merge, Linear' },
  ],
}

// ============================================================================
// How this workflow is organised
// ============================================================================
// The script owns control flow; agents own every side effect (git, gh, cargo,
// Linear). The script cannot read files, so anything it branches on comes back
// from an agent as schema-validated JSON. scripts/test-ship-ticket.mjs drives
// this file with mocked agents; scripts/check.sh tooling runs it.
//
// Invariants:
// - Only one agent edits the checkout at a time. Reviewers and skeptics are
//   read-only and run in parallel; implementer and fixers run alone.
// - An agent that returns null (blocked, stopped, API failure) is a failure,
//   never an empty result. A missing reviewer or skeptic blocks convergence.
// - Every judged finding is remembered, refuted ones included, so rejected
//   findings cannot come back.
// - Fixes are `git commit --fixup` commits, squashed before anything is pushed.
//   A corrected test is the exception: its own commit, reason in the message
//   and, when published, in the PR description (checklist §0).
// - Nothing is pushed unless review converged and the full gate passed on the
//   exact tree being pushed.
//
// Why review converges (the first live run did not, DOLL-654):
// - Round 1 reviews the whole branch. Later rounds review only the fix commits;
//   older code blocks again only for a high or critical finding.
// - The skeptic separates defects from requests. A missing capability or extra
//   hardening the ticket did not ask for is "out-of-scope" and becomes one
//   grouped follow-up ticket, not a fix.
// - The fixer makes small fixes only. Anything needing a new stage, option,
//   file or mechanism is deferred to a follow-up. A fix pass that grows the
//   branch past the growth limit stops the loop.
// - Confirmed findings must at least halve (rounded up) each round, or the loop stops and
//   reports instead of spending another round. The last round never fixes,
//   because nothing would re-review those fixes.
//
// Reviewer briefs live in docs/reviewers/<key>.md; routing lives in LENSES.

// ---------------------------------------------------------------- options

const SEVERITIES = ['low', 'medium', 'high', 'critical']
const STAGES = ['plan', 'implement', 'review', 'pr']

function parseOptions(raw) {
  const o = {
    mode: 'ship',
    ticket: '',
    stopAfter: '',
    merge: true,
    maxRounds: 3,
    bar: 'medium',
    base: 'origin/main',
    guidance: '',
  }
  if (raw && typeof raw === 'object' && !Array.isArray(raw)) {
    for (const k of Object.keys(o)) if (raw[k] !== undefined) o[k] = raw[k]
  } else {
    const words = (Array.isArray(raw) ? raw.join(' ') : String(raw || '')).split(/\s+/).filter(Boolean)
    const rest = []
    for (const w of words) {
      const lw = w.toLowerCase()
      if (!o.ticket && /^doll-\d+$/.test(lw)) o.ticket = w
      else if (lw === 'no-merge' || lw === 'nomerge') o.merge = false
      else if (/^rounds=\d+$/.test(lw)) o.maxRounds = Number(lw.split('=')[1])
      else if (/^bar=(low|medium|high|critical)$/.test(lw)) o.bar = lw.split('=')[1]
      else rest.push(w)
    }
    // A stage word first in line always stops there and the rest is guidance.
    // Stopping early is the safe mistake: "/ship-ticket DOLL-12 plan then ask
    // me" must never run on to merge.
    const first = rest.length ? rest[0].toLowerCase() : ''
    if (first === 'review' && !o.ticket) { o.mode = 'review'; rest.shift() }
    else if (STAGES.includes(first)) { o.stopAfter = first; rest.shift() }
    o.guidance = rest.join(' ')
  }
  if (o.mode === 'review' && o.ticket) { o.mode = 'ship'; o.stopAfter = 'review' }
  o.ticket = String(o.ticket || '').toUpperCase()
  o.maxRounds = Math.min(Math.max(Number(o.maxRounds) || 3, 1), 6)
  if (!SEVERITIES.includes(o.bar)) o.bar = 'medium'
  if (o.stopAfter && !STAGES.includes(o.stopAfter)) o.stopAfter = ''
  return o
}

const opts = parseOptions(args)
const rank = (s) => Math.max(0, SEVERITIES.indexOf(String(s || 'low').toLowerCase()))
const atOrAbove = (s, level) => rank(s) >= rank(level)
const maxLevel = (a, b) => (rank(a) >= rank(b) ? a : b)
const stopsAt = (stage) => opts.stopAfter === stage

// Whether a finding has to be fixed on this branch before it can ship.
// Round 1: anything the branch introduced at or above the bar. Later rounds:
// what the fix commits introduced at or above the bar, and anything else only
// when it is high or critical. Pre-existing problems never block.
function blocks(severity, origin, round, bar) {
  if (origin === 'preexisting') return false
  if (round > 1 && origin !== 'fix') return atOrAbove(severity, maxLevel(bar, 'high'))
  return atOrAbove(severity, bar)
}

// ---------------------------------------------------------------- reviewer routing

const LENSES = [
  { key: 'baseline', always: true },
  { key: 'guardrails', always: true },
  {
    key: 'realtime-audio',
    paths: [
      /^src\/(cpal_processor|writer_thread|raw_wav_writer|silence_check_worker|audio_recorder|audio_processor|constants|macos_sample_rate_listener)\.rs$/,
      /^src\/tests\/(alloc|ring_buffer|writer_thread|silence_gate|silence|shutdown)_tests\.rs$/,
    ],
  },
  {
    key: 'ffi-unsafe',
    unsafeFlag: true,
    paths: [
      /^src\/ffi\.rs$/, /^include\//, /^BlackBoxApp\/bridge\//, /RustBridge\.swift$/, /GlobalHotkeyManager\.swift$/,
      /^src\/macos_sample_rate_listener\.rs$/, /^src\/lib\.rs$/, /^src\/tests\/ffi_tests\.rs$/,
    ],
  },
  { key: 'rust-core', paths: [/^src\/(?!tests\/).*\.rs$/, /^Cargo\.toml$/] },
  {
    key: 'swift-app',
    paths: [
      /^BlackBoxApp\/BlackBoxApp\//, /^BlackBoxApp\/project\.yml$/, /^BlackBoxApp\/Guardrails\.xcconfig$/,
      /^BlackBoxApp\/.*\.entitlements$/,
    ],
  },
  { key: 'tests', paths: [/^src\/tests\//, /^BlackBoxApp\/BlackBoxAppTests\//, /^src\/(test_utils|mock_processor)\.rs$/] },
  {
    key: 'build-release',
    paths: [
      /^\.github\//, /^scripts\//, /^Makefile$/, /^Cargo\.(toml|lock)$/, /^(deny|clippy)\.toml$/,
      /^\.swiftlint\.yml$/, /^\.swift-format$/, /^BlackBoxApp\/(fastlane|Gemfile)/, /^\.claude\//,
      /^docs\/reviewers\//, /^SETUP\.md$/, /^rust-toolchain/, /^BlackBoxApp\/project\.yml$/,
      /^BlackBoxApp\/BlackBoxApp\/(Info\.plist|PrivacyInfo\.xcprivacy)$/, /^BlackBoxApp\/BlackBoxApp\.xcodeproj\//,
    ],
  },
]

function pickLenses(files, touchesUnsafe, alsoKeys) {
  return LENSES.filter((l) =>
    l.always ||
    (alsoKeys || []).includes(l.key) ||
    (l.unsafeFlag && touchesUnsafe) ||
    (l.paths || []).some((re) => files.some((f) => re.test(f))))
}

// scripts/check.sh section for a set of changed files during the loop.
// include/ counts as both: the Swift app compiles the header through
// BlackBoxApp/bridge, so a header change must also build RustBridge.swift.
function sectionFor(files) {
  const rust = files.some((f) => /^(src\/|include\/|Cargo\.|clippy\.toml|deny\.toml|benches\/)/.test(f))
  const swift = files.some((f) => /^(BlackBoxApp\/|include\/|\.swiftlint\.yml|\.swift-format)/.test(f))
  const build = files.some((f) => /^(scripts\/|Makefile|\.github\/)/.test(f))
  if ((rust && swift) || build) return 'default'
  if (swift) return 'swift'
  if (rust) return 'rust'
  return 'tooling'
}

// The gate before pushing is the full run. Swift source changes add the
// sanitizer section (checklist §6.2 wants a TSan-clean run for Swift
// concurrency code); App Store metadata changes add make check-app-store,
// which scripts/check.sh does not cover.
function finalGateFor(files) {
  return {
    section: files.some((f) => /^BlackBoxApp\/BlackBoxApp(Tests)?\/.*\.swift$/.test(f)) ? 'all' : 'default',
    appStore: files.some((f) => /^BlackBoxApp\/fastlane\//.test(f)),
  }
}

const REQUIRED_CHECKS = ['Format', 'Clippy', 'Test (macos-latest)', 'Swift app', 'Security audit']
const CI_GREEN_RULE = `CI is green only when every one of these required checks exists on the PR's current head commit and passed: ${REQUIRED_CHECKS.join(', ')}. Every other check must be pass or skipping. Checks can take a few minutes to register after a push; "no checks yet" is pending, never green.`

// ---------------------------------------------------------------- schemas

const str = { type: 'string' }
const strs = { type: 'array', items: { type: 'string' } }
const bool = { type: 'boolean' }
const int = { type: 'integer' }
const obj = (properties) => ({ type: 'object', properties, required: Object.keys(properties) })
const ORIGINS = ['branch', 'fix', 'preexisting']

const INTAKE_SCHEMA = obj({
  ok: bool,
  stopReason: { type: 'string', description: 'why the run must stop now, or ""' },
  candidates: { ...strs, description: 'only when no ticket was given: up to 8 lines "DOLL-N title [state, priority]"' },
  ticketId: str,
  ticketTitle: str,
  ticketUrl: str,
  ticketState: str,
  ticketText: { type: 'string', description: 'description plus any comments that change scope, as markdown' },
  branchName: { type: 'string', description: 'the ticket branch name' },
  currentBranch: str,
  dirtyFiles: { ...strs, description: 'git status --porcelain entries ignoring target/; empty when clean' },
  branchExists: bool,
  branchCommits: { ...strs, description: '"<sha> <subject>" for commits on the branch not on origin/main, oldest first' },
  filesChanged: { ...strs, description: 'git diff --name-only origin/main...<branch>, or empty' },
  prNumber: { type: 'integer', description: 'open PR for the branch, or 0' },
})

const PLAN_SCHEMA = obj({
  needsDecision: bool,
  questions: strs,
  alreadyDone: { type: 'boolean', description: 'main already satisfies the ticket' },
  summary: { type: 'string', description: 'two or three sentences: what changes and why' },
  approach: { type: 'string', description: 'markdown: ordered steps naming files and functions' },
  filesToTouch: strs,
  testsToAdd: strs,
  risks: strs,
  commitPlan: { ...strs, description: 'planned commit subjects in the repo style' },
  userVisible: { type: 'boolean', description: 'true when an app or CLI user would notice the change' },
})

const IMPLEMENT_SCHEMA = obj({
  status: { type: 'string', enum: ['done', 'blocked'] },
  blockedReason: str,
  branch: str,
  commits: { ...strs, description: '"<sha> <subject>" for every branch commit, oldest first' },
  filesChanged: { ...strs, description: 'git diff --name-only origin/main...HEAD' },
  summary: { type: 'string', description: 'what changed and why, for the PR body, 2-6 sentences' },
  apiChanges: strs,
  ffiChanges: strs,
  layoutChanges: { ...strs, description: 'WAV/file format, buffer layout, StatusFlags changes' },
  newDependencies: { ...strs, description: 'each with a one-line justification' },
  lintExpectations: { ...strs, description: 'every #[expect] or swiftlint disable added, with its reason' },
  testsAdded: strs,
  deviationsFromPlan: strs,
  releaseNote: { type: 'string', description: 'one user-facing line for the next CHANGELOG, or "" when nothing user-visible changed' },
})

const GATE_SCHEMA = obj({
  green: { type: 'boolean', description: 'true only when the log contains "==> all checks green" and ends with exit=0' },
  exitCode: int,
  failingStep: { type: 'string', description: 'the last "==> " step header before the failure, or ""' },
  errorExcerpt: { type: 'string', description: 'the first real error lines, up to 60, ANSI stripped; "" when green or when only tools were skipped' },
  skipped: { ...strs, description: 'tools check.sh listed as skipped or not installed' },
  tail: { type: 'string', description: 'the last 15 lines of the log, ANSI stripped' },
})

const SCOPE_SCHEMA = obj({
  branch: str,
  head: str,
  dirtyFiles: strs,
  files: { ...strs, description: 'files changed between the merge base and HEAD' },
  filesSince: { ...strs, description: 'files changed between SINCE and HEAD; same as files when SINCE is empty' },
  touchesUnsafe: { type: 'boolean', description: 'see the prompt' },
  commits: { ...strs, description: '"<sha> <subject>" for branch commits, oldest first' },
  diffLines: { type: 'integer', description: 'added plus removed lines between the merge base and HEAD' },
  priorLog: { ...strs, description: 'lines of the review log file if it exists, else empty' },
})

const FINDINGS_SCHEMA = obj({
  findings: {
    type: 'array',
    items: obj({
      title: { type: 'string', description: 'one line naming the defect' },
      severity: { type: 'string', enum: SEVERITIES },
      origin: { type: 'string', enum: ORIGINS, description: 'branch: introduced by the branch; fix: introduced or left broken by the fix commits under review; preexisting: already on the base and not made worse' },
      file: str,
      line: int,
      evidence: { type: 'string', description: 'what the code does, quoting the relevant lines' },
      whyWrong: str,
      suggestedFix: str,
      checklistRef: { type: 'string', description: 'docs/REVIEW-CHECKLIST.md section such as "§2", or ""' },
    }),
  },
  checked: { ...strs, description: 'what you examined and found sound, one line each' },
})

const VERDICT_SCHEMA = obj({
  verdict: { type: 'string', enum: ['confirmed', 'refuted', 'duplicate', 'out-of-scope'] },
  duplicateOf: { type: 'string', description: 'id of the judged finding this repeats, or ""' },
  stillPresent: { type: 'boolean', description: 'for a duplicate of a finding marked fixed: true only if the defect is still present at HEAD' },
  severity: { type: 'string', enum: SEVERITIES },
  origin: { type: 'string', enum: ORIGINS },
  reasoning: { type: 'string', description: 'cite each file:line you actually read' },
  fix: { type: 'string', description: 'the smallest correct fix for a confirmed finding, or ""' },
})

const FIX_SCHEMA = obj({
  results: {
    type: 'array',
    items: obj({
      id: str,
      status: { type: 'string', enum: ['fixed', 'deferred', 'disputed', 'not-fixed'] },
      commit: { type: 'string', description: 'short sha of the commit holding the fix, or ""' },
      note: str,
    }),
  },
  filesChanged: { ...strs, description: 'files this fix pass modified' },
  linesChanged: { type: 'integer', description: 'insertions plus deletions from git diff --shortstat <start head>..HEAD' },
})

const STEP_SCHEMA = obj({
  ok: bool,
  detail: { type: 'string', description: 'what happened, or why it failed' },
  head: { type: 'string', description: 'HEAD sha after the step, or ""' },
})

const PUBLISH_SCHEMA = obj({
  ok: bool,
  detail: str,
  prNumber: int,
  prUrl: str,
  followUps: { ...strs, description: '"DOLL-N title" for each follow-up ticket created or already present' },
})

const CI_SCHEMA = obj({
  state: { type: 'string', enum: ['green', 'failed', 'timeout'] },
  failures: {
    type: 'array',
    items: obj({ check: str, excerpt: { type: 'string', description: 'the failing step and its first error lines, up to 60' } }),
  },
})

const MERGE_SCHEMA = obj({
  merged: bool,
  updatedBranch: { type: 'boolean', description: 'true when the PR was behind main and was rebased with gh pr update-branch' },
  detail: str,
  mainHead: { type: 'string', description: 'local main HEAD after pulling, or ""' },
})

const CLOSEOUT_SCHEMA = obj({ ok: bool, state: str, detail: str })

// ---------------------------------------------------------------- shared prompt text

const REPO = `Repository: audio_blackbox, BlackBox Audio Recorder. A Rust core crate (\`blackbox\`, repo root, edition 2024) exposes a C ABI from src/ffi.rs through the hand-maintained include/blackbox_ffi.h to a SwiftUI/AppKit macOS menu-bar app in BlackBoxApp/. The audio callback only pushes f32 samples into an rtrb ring; a writer thread does all IO.
Before acting, read AGENTS.md (Workflow, Guardrails, Invariants) and the sections of docs/REVIEW-CHECKLIST.md your task touches.`

const GIT_RULES = `Git rules: work only on the ticket branch. Never commit to or push main. Never use --no-verify, git reset --hard, git clean, git stash, or git checkout -- on files you did not change in this task. Do not push unless the task says to.`

const CODE_RULES = `Code rules (docs/REVIEW-CHECKLIST.md §0): never silence a lint to get green. #[allow] is a compile error; #[expect(lint, reason = "...")] only when the reason states a true invariant. Swift silences use "// swiftlint:disable:next rule - reason". Never weaken, skip or delete a test to make a change pass. No new dependency to save twenty lines. Any public API, FFI signature, StatusFlags, buffer layout or WAV format change updates include/blackbox_ffi.h and RustBridge.swift in the same commit. Unsafe, FFI, real-time and concurrency code carries SAFETY / ownership / thread-contract comments. Match the surrounding idioms (thiserror + BlackboxError, @Observable).`

const COMMIT_STYLE = `Commit style (see git log): "<area>: <imperative summary> (DOLL-N)" with area one of fix, feat, perf, refactor, test, docs, build, ci, deps, lint, swift, ffi, or fix(<scope>). The body says why, wrapped at 72 columns. One logical change per commit.`

const FIXUP_RULES = (commits) => `Commit each fix with git commit --fixup=<sha> onto the branch commit it belongs to. Branch commits:
${(commits || []).join('\n') || '(ask git: git log --format="%h %s" origin/main..HEAD)'}
If no branch commit fits, make a normal commit. ${COMMIT_STYLE}
Exception: when an existing test's expectation was wrong, change it in its own normal commit with the subject "test: correct <test name> (DOLL-N)" and a body that says why the old expectation was wrong; never fold a test change into a fixup (checklist §0). Commits that only add tests do not use that subject.`

const RUBRIC = `Severity (canonical definitions in docs/reviewers/README.md):
- critical: crash, undefined behavior, audio or file data loss or corruption, security or privacy exposure, broken build/CI/release for everyone.
- high: wrong behavior users hit in normal use, real-time thread violation, FFI contract break, regression, silenced lint or weakened test.
- medium: a real defect in what the branch does, with a narrow trigger or a workaround; missing test for new behavior; swallowed error; missing or false SAFETY/ownership/thread-contract comment the checklist requires; contract drift (header, RustBridge, project.yml, docs that must move with the change).
- low: polish, naming, readability, optional refactor. Never blocks.
Scope: a defect is code that does something wrong or breaks a documented contract (the ticket, AGENTS.md, docs/REVIEW-CHECKLIST.md). A capability the ticket did not ask for, extra hardening against unlikely misuse, or a different design is not a defect at any severity.`

const LOG_FILE = 'target/ship-ticket/<current branch with every / replaced by _>/review-log.txt'

// ---------------------------------------------------------------- the gate

async function runGate(section, label, phaseName, appStore) {
  const cmd = section && section !== 'default' ? `./scripts/check.sh ${section}` : './scripts/check.sh'
  const logFile = `target/ship-ticket/gate-${label}.log`
  const full = appStore
    ? `{ ${cmd}; rc=$?; if [ $rc -eq 0 ]; then make check-app-store; rc=$?; fi; echo "exit=$rc"; } > ${logFile} 2>&1`
    : `${cmd} > ${logFile} 2>&1; echo "exit=$?" >> ${logFile}`
  const r = await agent(
    `${REPO}

Run the repo gate and report its result exactly. Do not edit, fix, stage or commit anything.

1. mkdir -p target/ship-ticket
2. Run: ${full}
   It can take 20 minutes or more (clippy on three feature sets, tests, benches, xcodebuild${section === 'all' ? ', then the Swift tests twice more under sanitizers' : ''}). Run it in the background and wait for it to finish; never let a tool timeout kill it. If another scripts/check.sh is already running (pgrep -f scripts/check.sh), wait for it to end first.
3. Read ${logFile}. green is true only if it contains "==> all checks green" and the last line is exit=0.
4. failingStep is the last line starting with "==> " before the first failure. errorExcerpt holds the first real error lines (compiler errors, clippy lints, failed tests, "error:" lines, lint violations), ANSI escapes stripped. When the only problem is that tools were skipped ("green, with skipped steps"), errorExcerpt is "".
5. skipped lists anything check.sh reported as skipped or not installed.`,
    { label: `gate:${label}`, phase: phaseName, schema: GATE_SCHEMA, effort: 'low' },
  )
  if (!r) return { green: false, exitCode: -1, failingStep: 'gate agent returned nothing (blocked or failed)', errorExcerpt: '', skipped: [], tail: '', agentFailed: true }
  return r
}

// Gate, fix, gate again. Stops when green, when the environment is the
// problem (tools missing), or when an attempt makes no progress. Fixes here
// are mechanical; their descriptions are returned so the PR can list code
// that changed after review.
async function gateUntilGreen({ section, label, phaseName, commits, attempts, context, appStore }) {
  const fixNotes = []
  let prevSig = ''
  let last = null
  for (let i = 1; i <= attempts; i++) {
    const g = await runGate(section, `${label}-${i}`, phaseName, appStore)
    if (g.green || g.agentFailed) return { ...g, attemptsUsed: i, fixNotes }
    if (g.skipped.length && !g.errorExcerpt.trim()) {
      return { ...g, attemptsUsed: i, fixNotes, environment: true, failingStep: `missing tools: ${g.skipped.join(', ')}` }
    }
    const sig = `${g.failingStep}|${g.errorExcerpt.slice(0, 400)}`
    if (sig === prevSig) return { ...g, attemptsUsed: i, fixNotes, noProgress: true }
    prevSig = sig
    last = g
    if (i === attempts) break
    const fix = await agent(
      `${REPO}

The gate (scripts/check.sh${section && section !== 'default' ? ' ' + section : ''}${appStore ? ' and make check-app-store' : ''}) failed.
Failing step: ${g.failingStep}
Errors:
${g.errorExcerpt}
${context ? `\nContext: ${context}\n` : ''}
Make the smallest change that fixes the root cause. Reproduce first with the narrowest command for that step (for example cargo clippy --all-targets --features ffi -- -D warnings, cargo test --no-default-features <name> -- --test-threads=1, swiftlint lint --strict, ./scripts/check-ffi-header.sh), then confirm it passes. If a new lint fires on code this branch did not touch, fix it there or park it in the Cargo.toml lint backlog block with a count and a ticket (AGENTS.md invariant); do not #[expect] your way through unrelated files. If the only real fix changes behavior beyond what the failing check demands, change nothing and return ok=false with the reason.
${CODE_RULES}
${GIT_RULES}
${FIXUP_RULES(commits)}
Do not run the full scripts/check.sh; the gate reruns it. In detail, describe each change in one line.`,
      { label: `gate-fix:${label}-${i}`, phase: phaseName, schema: STEP_SCHEMA },
    )
    if (!fix || !fix.ok) return { ...g, attemptsUsed: i, fixNotes, fixFailed: true, fixDetail: fix ? fix.detail : 'fixer returned nothing' }
    fixNotes.push(fix.detail)
  }
  return { ...last, attemptsUsed: attempts, fixNotes }
}

// ---------------------------------------------------------------- review loop

function norm(s) {
  return String(s || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim().split(' ').slice(0, 10).join(' ')
}

async function reviewLoop({ base, bar, maxRounds, ticketText, planText, ticketId }) {
  const judged = []
  const notes = []
  const followUps = []    // confirmed pre-existing defects: one ticket each
  const suggestions = []  // out-of-scope requests and deferred fixes: one grouped ticket
  const openItems = []
  const roundsLog = []
  let since = ''
  let rerunKeys = []
  let converged = false
  let reason = ''
  let lastScope = null
  let priorLog = []
  let prevBlocking = 0

  for (let round = 1; round <= maxRounds; round++) {
    const scope = await agent(
      `${REPO}

Report the review scope for the current branch. Read-only: do not edit anything.
BASE: ${base}
SINCE: ${since || '(empty)'}
1. git fetch origin --quiet. merge base = git merge-base ${base} HEAD.
2. files = git diff --name-only <merge base>..HEAD. commits = git log --format="%h %s" --reverse <merge base>..HEAD. diffLines = insertions plus deletions from git diff --shortstat <merge base>..HEAD.
3. filesSince = git diff --name-only ${since || '<merge base>'}..HEAD.
4. touchesUnsafe: true when any line of git diff -U8 ${since || '<merge base>'}..HEAD (added, removed or context) contains unsafe, SAFETY:, extern "C", no_mangle, repr(C), Unmanaged, withUnsafe, @unchecked, nonisolated(unsafe) or assumeIsolated.
5. dirtyFiles = git status --porcelain entries, ignoring target/.
6. priorLog: ${round === 1 ? `if ${LOG_FILE} exists, its lines; else empty.` : 'always empty.'}`,
      { label: `scope:r${round}`, phase: 'Review', schema: SCOPE_SCHEMA, effort: 'low' },
    )
    if (!scope) { reason = 'the scope agent returned nothing'; break }
    lastScope = scope
    if (round === 1) priorLog = scope.priorLog || []
    if (scope.dirtyFiles.length) { reason = `uncommitted changes in the working tree: ${scope.dirtyFiles.join(', ')}`; break }
    if (!scope.files.length) { converged = true; reason = 'the branch has no changes against the base'; break }

    const routeFiles = round === 1 ? scope.files : scope.filesSince
    const lenses = pickLenses(routeFiles, scope.touchesUnsafe, rerunKeys)
    log(`Review round ${round}/${maxRounds}: ${lenses.map((l) => l.key).join(', ')} over ${routeFiles.length} files`)

    const seen = [
      ...priorLog.map((l) => `(earlier run) ${l}`),
      ...judged.map((j) => `${j.id} [${j.verdict}${j.status ? ', ' + j.status : ''}, ${j.severity}] ${j.file}:${j.line} ${j.title}`),
    ]
    const seenText = seen.length ? seen.join('\n') : '(none yet)'

    const roundTask = round === 1
      ? `Review the whole branch: git diff ${base}...HEAD. Set origin to "branch" for defects the branch introduced and "preexisting" for problems in code the branch did not change or make worse.`
      : `Round ${round}. Earlier rounds reviewed the whole branch and their confirmed findings were fixed in the commits after ${since}. Review those fix commits: git diff ${since}..HEAD, reading the surrounding code as needed.
Report with origin "fix": a defect the fix commits introduced, or a fix that did not hold.
Report with origin "branch" only a high or critical defect elsewhere in the branch that earlier rounds missed. Do not report medium or low findings outside the fix commits; the earlier rounds owned that code.`

    const reviewerPrompt = (lens) => `${REPO}

You are the "${lens.key}" reviewer for this branch. Read docs/reviewers/${lens.key}.md first; it defines your lane, what is not your lane, and facts that prevent false positives. docs/reviewers/README.md defines severity.
Read-only: do not edit files, commit, build or run tests. Use git, Read and Grep.

Ticket ${ticketId || '(none)'}:
${ticketText || '(no ticket context; infer intent from the commits)'}
${planText ? `\nPlan the branch was built from:\n${planText}\n` : ''}
Branch commits:
${scope.commits.join('\n')}

${roundTask}

Already judged. Do not report these again, reworded or not, except a finding marked fixed whose defect is still present at HEAD: report that with origin "fix" and name its id in the title.
${seenText}

${RUBRIC}

Report only defects you can show with code evidence and a file:line. Read enough surrounding code and callers to be sure. The gate (scripts/check.sh) already passed and covers formatting, clippy, swiftlint, header parity and the string catalog; do not report those.
Return at most your 6 strongest findings. Zero findings is the right answer for sound code. List what you checked in "checked".`

    let results = await parallel(lenses.map((l) => () =>
      agent(reviewerPrompt(l), { label: `review:${l.key}:r${round}`, phase: 'Review', schema: FINDINGS_SCHEMA })))
    const retry = lenses.filter((_, i) => !results[i])
    if (retry.length) {
      log(`Retrying reviewers that returned nothing: ${retry.map((l) => l.key).join(', ')}`)
      const again = await parallel(retry.map((l) => () =>
        agent(reviewerPrompt(l), { label: `review:${l.key}:r${round}:retry`, phase: 'Review', schema: FINDINGS_SCHEMA })))
      results = results.map((r, i) => r || again[retry.indexOf(lenses[i])] || null)
    }
    const failedLenses = lenses.filter((_, i) => !results[i]).map((l) => l.key)

    // Cross-reviewer dedup within the round, by location: reviewers word the
    // same defect differently, and counting it once per reviewer would trip
    // the halving rule (the second live run saw one defect confirmed 4 times).
    // Two defects within 3 lines share one skeptic, who sees every title.
    const ORIGIN_WEIGHT = { preexisting: 0, branch: 1, fix: 2 }
    const candidates = []
    let n = 0
    results.forEach((r, i) => {
      if (!r) return
      for (const f of r.findings) {
        const prev = candidates.find((c) => c.file === f.file && Math.abs(c.line - f.line) <= 3)
        if (prev) {
          if (rank(f.severity) > rank(prev.severity)) prev.severity = f.severity
          if (ORIGIN_WEIGHT[f.origin] > ORIGIN_WEIGHT[prev.origin]) prev.origin = f.origin
          if (!prev.lenses.includes(lenses[i].key)) prev.lenses.push(lenses[i].key)
          prev.alsoReported.push(`${f.title} (${lenses[i].key}): ${f.whyWrong}`)
        } else {
          candidates.push({ ...f, id: `R${round}.${++n}`, lens: lenses[i].key, lenses: [lenses[i].key], alsoReported: [] })
        }
      }
    })
    // Verify what could block, plus pre-existing claims serious enough for a
    // follow-up ticket. Everything else is a note and is not verified.
    const worthVerifying = (f) => blocks(f.severity, f.origin, round, bar) ||
      (f.origin === 'preexisting' && atOrAbove(f.severity, maxLevel(bar, 'high')))
    const toVerify = candidates.filter(worthVerifying)
    for (const f of candidates.filter((c) => !worthVerifying(c))) {
      if (!notes.some((x) => x.file === f.file && norm(x.title) === norm(f.title))) notes.push({ ...f, verified: false, round })
    }

    const verdicts = await pipeline(toVerify, (f) =>
      agent(
        `${REPO}

You are a skeptic. A "${f.lens}" reviewer claims the finding below on this branch. Decide what it really is by reading the actual code, its callers and its tests. Read docs/reviewers/${f.lens}.md, especially "Facts that prevent false positives".
Read-only: do not edit, commit, build or run tests.

CLAIM ${f.id}: ${f.title}
Severity claimed: ${f.severity}
Origin claimed: ${f.origin}
Location: ${f.file}:${f.line}
Evidence: ${f.evidence}
Why wrong: ${f.whyWrong}
Suggested fix: ${f.suggestedFix}
${f.alsoReported.length ? `Other reviewers reported at the same place (judge them together as one finding):\n${f.alsoReported.map((a) => `- ${a}`).join('\n')}\n` : ''}
Ticket ${ticketId || '(none)'}: ${ticketText ? ticketText.slice(0, 1500) : '(no ticket context)'}

Already judged findings:
${seenText}

Verdicts:
- duplicate: the same defect as a judged finding; give its id. If that finding is marked fixed, set stillPresent to whether the defect is still there at HEAD.
- refuted: a style preference, handled elsewhere, contradicted by a repo fact, speculative, or not provable from the code. Default to refuted when unsure.
- out-of-scope: it may be true, but it asks for something the ticket did not ask for: a new capability, extra hardening against unlikely misuse, a different design, or coverage beyond the change. Those become a follow-up ticket, not a fix on this branch.
- confirmed: the changed code does something wrong or breaks a documented contract, and you proved it with file:line evidence you read yourself.

Set severity to what the evidence supports:
${RUBRIC}
origin: "preexisting" when the base branch (git show ${base}:<path>) already had the problem and this branch did not make it worse${round > 1 ? `; "fix" when the commits after ${since} introduced it or failed to fix it; otherwise "branch"` : '; otherwise "branch"'}. For a confirmed finding, give the smallest correct fix.`,
        { label: `verify:${f.id}`, phase: 'Verify', schema: VERDICT_SCHEMA, effort: 'high' },
      ).then((v) => ({ f, v })))

    const toFix = []
    const unverified = []
    let outOfScope = 0
    verdicts.forEach((item, i) => {
      const f = toVerify[i]
      if (!item || !item.v) { unverified.push(f); return }
      const v = item.v
      const whyWrong = [f.whyWrong, ...f.alsoReported].join('\nAlso reported: ')
      const entry = { id: f.id, round, lens: f.lenses.join('+'), title: f.title, file: f.file, line: f.line, severity: v.severity, origin: v.origin, verdict: v.verdict, reasoning: v.reasoning, fix: v.fix, whyWrong, evidence: f.evidence, status: '' }
      judged.push(entry)
      if (v.verdict === 'duplicate') {
        const orig = judged.find((j) => j.id === v.duplicateOf)
        if (orig && orig.status === 'fixed' && v.stillPresent && atOrAbove(v.severity, bar)) {
          entry.verdict = 'confirmed'
          entry.origin = 'fix'
          entry.title = `${f.title} (fix for ${orig.id} did not hold)`
          toFix.push(entry)
        }
        return
      }
      if (v.verdict === 'refuted') return
      if (v.verdict === 'out-of-scope') {
        outOfScope++
        if (atOrAbove(v.severity, bar)) suggestions.push({ ...entry, why: 'out of scope for the ticket' })
        return
      }
      if (v.origin === 'preexisting') {
        if (atOrAbove(v.severity, maxLevel(bar, 'high'))) followUps.push(entry)
        else notes.push({ ...f, severity: v.severity, verified: true, round })
        return
      }
      if (blocks(v.severity, v.origin, round, bar)) toFix.push(entry)
      else notes.push({ ...f, severity: v.severity, verified: true, round })
    })

    const roundEntry = { round, lenses: lenses.map((l) => l.key), failedLenses, raised: candidates.length, verified: toVerify.length, outOfScope, confirmed: toFix.length, fixed: 0 }
    roundsLog.push(roundEntry)
    log(`Round ${round}: ${candidates.length} raised, ${toVerify.length} verified, ${outOfScope} out of scope, ${toFix.length} confirmed to fix`)

    if (failedLenses.length) { reason = `reviewers returned nothing twice: ${failedLenses.join(', ')}`; openItems.push(...toFix.map((t) => ({ ...t, why: 'not fixed: the round was incomplete' }))); break }
    if (unverified.length) { reason = `${unverified.length} findings could not be verified (the skeptic returned nothing)`; openItems.push(...unverified.map((f) => ({ ...f, why: 'unverified' })), ...toFix.map((t) => ({ ...t, why: 'not fixed: the round was incomplete' }))); break }
    if (!toFix.length) {
      converged = true
      reason = round === 1 ? 'round 1 confirmed nothing to fix' : `round ${round} confirmed nothing new in the fixes`
      break
    }
    if (round > 1 && toFix.length > Math.ceil(prevBlocking / 2)) {
      reason = `review is not settling: confirmed findings went ${roundsLog.map((r) => r.confirmed).join(' then ')}; each round must confirm at most half (rounded up) of the round before`
      openItems.push(...toFix.map((t) => ({ ...t, why: 'not fixed: the loop stopped because findings were not halving' })))
      break
    }
    if (round === maxRounds) {
      reason = `hit the ${maxRounds}-round cap with ${toFix.length} confirmed findings in the last round; they were not fixed because nothing would re-review the fixes`
      openItems.push(...toFix.map((t) => ({ ...t, why: 'not fixed: round cap' })))
      break
    }
    prevBlocking = toFix.length

    const growthLimit = Math.max(80, Math.round(scope.diffLines * 0.25))
    const fixRes = await agent(
      `${REPO}

Apply these review findings. Each was confirmed by an independent skeptic. You are the only agent editing the checkout.

${toFix.map((t) => `### ${t.id} [${t.severity}] ${t.title}
Location: ${t.file}:${t.line}
Evidence: ${t.evidence}
Why wrong: ${t.whyWrong}
Skeptic's reasoning: ${t.reasoning}
Fix: ${t.fix}`).join('\n\n')}

First record the start head: git rev-parse HEAD.
Fix each finding with the smallest correct change, and add or extend a test when the finding is about behavior. Run the narrowest checks that prove it (the specific cargo test, clippy for the touched feature set, swiftlint on the touched files).
Stay small. Do not add stages, agents, options, schemas, files or new mechanisms, and do not refactor around a fix. If the correct fix needs any of those, or more than about 40 changed lines, leave it: status "deferred" with a note on what the real fix needs. The whole pass must stay under ${growthLimit} changed lines.
If you are certain a finding is wrong after reading the code closely, change nothing for it: status "disputed" with the evidence. "not-fixed" means you tried and could not; say why.
${CODE_RULES}
${GIT_RULES}
${FIXUP_RULES(scope.commits)}
Do not run the full scripts/check.sh; the gate runs next. Report linesChanged from git diff --shortstat <start head>..HEAD.`,
      { label: `fix:r${round}`, phase: 'Fix', schema: FIX_SCHEMA },
    )
    if (!fixRes) { reason = 'the fixer returned nothing'; openItems.push(...toFix.map((t) => ({ ...t, why: 'fixer returned nothing' }))); break }

    rerunKeys = [...failedLenses]
    for (const t of toFix) {
      const r = fixRes.results.find((x) => x.id === t.id)
      if (!r) { openItems.push({ ...t, why: 'the fixer did not report on it' }); continue }
      t.status = r.status
      t.commit = r.commit
      t.note = r.note
      if (r.status === 'fixed') {
        roundEntry.fixed++
        for (const k of t.lens.split('+')) if (!rerunKeys.includes(k)) rerunKeys.push(k)
      } else if (r.status === 'deferred' && !atOrAbove(t.severity, 'high')) {
        suggestions.push({ ...t, why: `deferred: ${r.note}` })
      } else {
        openItems.push({ ...t, why: `${r.status}: ${r.note}` })
      }
    }
    if (openItems.length) { reason = `${openItems.length} confirmed findings were not fixed`; break }
    if (fixRes.linesChanged > growthLimit) {
      reason = `the round ${round} fixes changed ${fixRes.linesChanged} lines, over the ${growthLimit}-line growth limit; review them by hand before shipping`
      break
    }
    if (!roundEntry.fixed) {
      converged = true
      reason = `round ${round} findings were all deferred to a follow-up`
      break
    }

    const g = await gateUntilGreen({
      section: sectionFor(fixRes.filesChanged.length ? fixRes.filesChanged : scope.files),
      label: `review-r${round}`,
      phaseName: 'Fix',
      commits: scope.commits,
      attempts: 3,
      context: `This failure appeared after applying review fixes for ${toFix.map((t) => t.id).join(', ')}.`,
    })
    if (!g.green) { reason = g.environment ? g.failingStep : `the gate is red after round ${round} fixes at "${g.failingStep}"`; break }

    since = scope.head
  }

  // Remember refuted verdicts so a rerun does not re-judge them. Confirmed
  // findings are not logged: if a fix did not hold, a rerun must be able to
  // raise it again. Out-of-scope findings are not logged either: a rerun
  // must re-raise them so publish files them in the follow-up ticket.
  const logLines = judged
    .filter((j) => j.verdict === 'refuted')
    .map((j) => `${j.verdict} ${j.file}:${j.line} ${j.title}`)
  if (logLines.length) {
    const wrote = await agent(
      `Append these lines to ${LOG_FILE} (create directories as needed; target/ is ignored by git). Change nothing else.

${logLines.join('\n')}`,
      { label: 'review-log', phase: 'Review', schema: STEP_SCHEMA, effort: 'low' },
    )
    if (!wrote || !wrote.ok) log('Could not write the review log; a rerun may re-judge refuted findings.')
  }

  return { converged, reason, rounds: roundsLog, judged, notes, followUps, suggestions, openItems, scope: lastScope, bar }
}

// ---------------------------------------------------------------- reporting

function list(items, empty) {
  return items && items.length ? items.map((x) => `- ${x}`).join('\n') : `- ${empty}`
}

function summarizeReview(review) {
  return {
    converged: review.converged,
    reason: review.reason,
    rounds: review.rounds,
    fixed: review.judged.filter((j) => j.status === 'fixed').map((j) => `${j.title} (${j.file}:${j.line}, ${j.commit || 'no sha'})`),
    refuted: review.judged.filter((j) => j.verdict === 'refuted').map((j) => `${j.title} (${j.file}:${j.line})`),
    openItems: review.openItems.map((x) => `${x.title} (${x.file}:${x.line}): ${x.why}`),
    followUps: review.followUps.map((x) => `${x.title} (${x.file}:${x.line}, ${x.severity})`),
    suggestions: review.suggestions.map((x) => `${x.title} (${x.file}:${x.line}): ${x.why}`),
    notes: review.notes.map((x) => `${x.title} (${x.file}:${x.line})`),
  }
}

function reviewSection(review) {
  const fixed = review.judged.filter((j) => j.status === 'fixed')
  const refuted = review.judged.filter((j) => j.verdict === 'refuted').length
  const lines = [
    `Converged after ${review.rounds.length} round${review.rounds.length === 1 ? '' : 's'} at bar **${review.bar}**. ${fixed.length} confirmed findings fixed, ${refuted} refuted, ${review.suggestions.length} sent to a follow-up as out of scope or deferred.`,
    '',
    '| Round | Reviewers | Raised | Verified | Out of scope | Confirmed | Fixed |',
    '| --- | --- | --- | --- | --- | --- | --- |',
    ...review.rounds.map((r) => `| ${r.round} | ${r.lenses.join(', ')} | ${r.raised} | ${r.verified} | ${r.outOfScope} | ${r.confirmed} | ${r.fixed} |`),
  ]
  if (fixed.length) lines.push('', '**Fixed**', ...fixed.map((j) => `- ${j.title} (${j.severity}, \`${j.file}:${j.line}\`)`))
  if (review.notes.length) lines.push('', '**Notes, not changed**', ...review.notes.map((x) => `- ${x.title} (\`${x.file}:${x.line}\`)${x.verified ? '' : ', unverified'}`))
  return lines.join('\n')
}

function prBody({ ticket, impl, review, gateTail, afterReview, followUpMarker }) {
  return `Closes ${ticket.ticketId}.

${impl.summary}
${impl.releaseNote ? `\n**Release note:** ${impl.releaseNote}\n` : ''}
## Commits
<!-- COMMITS -->

## Contract changes (checklist §6.4)
**Public API**
${list(impl.apiChanges, 'None')}

**FFI and header**
${list(impl.ffiChanges, 'None')}

**File format, buffer layout, StatusFlags**
${list(impl.layoutChanges, 'None')}

**New dependencies**
${list(impl.newDependencies, 'None')}

**Lint expectations added**
${list(impl.lintExpectations, 'None')}

**Tests added**
${list(impl.testsAdded, 'None')}
${impl.deviationsFromPlan && impl.deviationsFromPlan.length ? `\n**Deviations from the plan**\n${list(impl.deviationsFromPlan, 'None')}\n` : ''}
## Review
${reviewSection(review)}
${afterReview.length ? `\n**Changed after review** (gate fixes, not re-reviewed)\n${list(afterReview, 'None')}\n` : ''}${followUpMarker}
## Local verification
\`\`\`
${gateTail}
\`\`\`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
`
}

// Squash fixups. The first tidy also rebases onto a freshly fetched main; any
// later tidy squashes in place so the tree the gate passed is the tree pushed.
function tidyPrompt(branch, rebaseOntoMain) {
  return `${REPO}

Tidy the ticket branch ${branch} before pushing. Do not push unless a later step says to.
1. Confirm you are on ${branch} with a clean tree (ignoring target/).
2. ${rebaseOntoMain
    ? 'git fetch origin --quiet, then GIT_SEQUENCE_EDITOR=: git rebase -i --autosquash origin/main'
    : 'Do not fetch. GIT_SEQUENCE_EDITOR=: git rebase -i --autosquash $(git merge-base origin/main HEAD)'}
3. On conflicts: git rebase --abort and return ok=false with the conflicting files.
4. Confirm no "fixup!" commits remain: git log --format="%h %s" origin/main..HEAD
Return head = the new HEAD sha.`
}

const REMOTE_SAFETY = (branch) => `Protect work that exists only on the remote: git fetch origin --quiet; if origin/${branch} exists, list git log --no-merges --format="%h %s" HEAD..origin/${branch} ^origin/main (commits already on main, and the merge commit gh pr update-branch or the Update branch button adds, are not remote-only work). Ignore "fixup!" commits and commits whose subject also appears in git log --format="%s" origin/main..HEAD (this workflow squashes and rewrites its own commits). If anything else is left, someone pushed work this checkout lacks.`

// ---------------------------------------------------------------- review-only mode

if (opts.mode === 'review') {
  phase('Intake')
  const where = await agent(
    `${REPO}

Read-only. Report the current branch for a review-only run.
1. ok=false with stopReason when the current branch is main or detached, or when git status --porcelain (ignoring target/) is not empty.
2. If the branch name contains doll-<number>, set ticketId to DOLL-<number> and read that issue from Linear (title, description, comments that change scope) into ticketTitle, ticketUrl, ticketState and ticketText. Otherwise leave them "".
3. git fetch origin --quiet. branchCommits = git log --format="%h %s" --reverse origin/main..HEAD. filesChanged = git diff --name-only origin/main...HEAD.
Leave candidates empty; branchName = currentBranch; branchExists = true; prNumber = the open PR for this branch or 0.`,
    { label: 'intake:review', phase: 'Intake', schema: INTAKE_SCHEMA, effort: 'low' },
  )
  if (!where) return { outcome: 'stopped', stage: 'intake', reason: 'the intake agent returned nothing' }
  if (!where.ok) return { outcome: 'stopped', stage: 'intake', reason: where.stopReason, dirtyFiles: where.dirtyFiles }

  // Reviewers skip anything the gate catches, so the gate must be green first.
  const gate = await gateUntilGreen({
    section: sectionFor(where.filesChanged),
    label: 'review-start',
    phaseName: 'Intake',
    commits: where.branchCommits,
    attempts: 3,
    context: `Gate before a review-only run on ${where.currentBranch}.`,
  })
  if (!gate.green) {
    return { outcome: 'stopped', stage: 'gate', branch: where.currentBranch, reason: gate.environment ? gate.failingStep : `scripts/check.sh is red at "${gate.failingStep}"; review needs a green gate`, errors: gate.errorExcerpt }
  }

  const review = await reviewLoop({ base: opts.base, bar: opts.bar, maxRounds: opts.maxRounds, ticketText: where.ticketText, planText: '', ticketId: where.ticketId })
  const madeCommits = review.judged.some((j) => j.status === 'fixed') || gate.fixNotes.length > 0
  const nextSteps = []
  if (!review.converged) {
    nextSteps.push('Review did not converge. Resolve the open items before pushing, then run /ship-ticket review again.')
  } else if (madeCommits) {
    nextSteps.push('Fixes are fixup! commits. Squash them in place before pushing: GIT_SEQUENCE_EDITOR=: git rebase -i --autosquash $(git merge-base origin/main HEAD)')
    nextSteps.push(where.prNumber ? `PR #${where.prNumber} is open, so the push needs --force-with-lease; check origin for commits this checkout lacks first.` : 'Then push and open the PR, or run /ship-ticket with the ticket to publish the standard way.')
  }
  if (review.followUps.length || review.suggestions.length) nextSteps.push('File the follow-ups and suggestions below as Linear tickets, or run /ship-ticket with the ticket, which files them.')
  return {
    outcome: review.converged ? 'review-converged' : 'review-not-converged',
    branch: where.currentBranch,
    gateFixes: gate.fixNotes,
    ...summarizeReview(review),
    nextSteps,
  }
}

// ---------------------------------------------------------------- ship mode

phase('Intake')
const ticket = await agent(
  `${REPO}

Read-only intake for /ship-ticket. Do not change Linear, git or files.
Ticket requested: ${opts.ticket || '(none)'}

If no ticket was requested: set ok=false, stopReason "no ticket given", and fill candidates with up to 8 open issues from the Linear "Audio Blackbox" project (team Dollhouse): In Progress or Todo first, then Backlog, highest priority first, as "DOLL-N title [state, priority]". Fill the git fields and leave the ticket fields "".

Otherwise:
1. Read the issue with its relations and comments. Fill ticketId, ticketTitle, ticketUrl, ticketState, branchName (Linear's gitBranchName) and ticketText (description plus comments that change scope).
2. ok=false when the state is Done, Canceled or Duplicate, or when an issue that blocks it is not Done. Say which in stopReason.
3. git fetch origin --quiet. currentBranch; dirtyFiles from git status --porcelain ignoring target/. ok=false when dirtyFiles is not empty.
4. The ticket branch is any local or origin branch whose name contains "${opts.ticket.toLowerCase()}-" or equals branchName; report its actual name in branchName. branchExists; branchCommits = git log --format="%h %s" --reverse origin/main..<branch>; filesChanged = git diff --name-only origin/main...<branch>; prNumber = the open PR from that branch (gh pr list --head <branch> --json number) or 0.`,
  { label: 'intake', phase: 'Intake', schema: INTAKE_SCHEMA, effort: 'low' },
)
if (!ticket) return { outcome: 'stopped', stage: 'intake', reason: 'the intake agent returned nothing (Linear or git access may be blocked)' }
if (!ticket.ok) {
  return { outcome: 'stopped', stage: 'intake', reason: ticket.stopReason, candidates: ticket.candidates, dirtyFiles: ticket.dirtyFiles, usage: 'Run /ship-ticket DOLL-N' }
}
log(`${ticket.ticketId}: ${ticket.ticketTitle}${ticket.branchExists ? ` (resuming ${ticket.branchName}, ${ticket.branchCommits.length} commits)` : ''}`)

phase('Plan')
const plan = await agent(
  `${REPO}

Plan ${ticket.ticketId} before any code changes. Read-only: do not edit files, branches or Linear.

# ${ticket.ticketTitle}
${ticket.ticketUrl}
${ticket.ticketText}
${opts.guidance ? `\nGuidance from the person who started this run: ${opts.guidance}\n` : ''}${ticket.branchExists ? `\nWork already exists on ${ticket.branchName}:\n${ticket.branchCommits.join('\n') || '(no commits yet)'}\nRead it (git diff origin/main...${ticket.branchName}) and plan only what is left.\n` : ''}
Read the code the ticket touches, ARCHITECTURE.md where it applies, docs/decisions.md for the DOLL references involved, and the tests that cover the area.

Set needsDecision=true with specific questions when the ticket is ambiguous in a way that changes the implementation, conflicts with an AGENTS.md invariant or docs/REVIEW-CHECKLIST.md rule, or needs a product call. Do not guess on those. Set alreadyDone=true when main already satisfies the ticket.
Otherwise give a concrete plan: steps naming files and functions, the tests that will prove it (Rust tests in src/tests/, Swift tests in BlackBoxApp/BlackBoxAppTests/), risks, and commit subjects in this style: ${COMMIT_STYLE}
userVisible is true when a user of the app or CLI would notice the change. CHANGELOG.md is written by the release PR, not per change. Keep scope to the ticket; list anything else you notice under risks.`,
  { label: 'plan', phase: 'Plan', schema: PLAN_SCHEMA, effort: 'high' },
)
if (!plan) return { outcome: 'stopped', stage: 'plan', reason: 'the planner returned nothing', ticket: ticket.ticketId }
const planText = `${plan.summary}\n\n${plan.approach}\n\nTests: ${plan.testsToAdd.join('; ') || 'none'}\nCommits: ${plan.commitPlan.join('; ')}`
if (plan.needsDecision) return { outcome: 'needs-decision', stage: 'plan', ticket: ticket.ticketId, questions: plan.questions, plan: planText, next: `Answer the questions in the ticket or as guidance, then run /ship-ticket ${ticket.ticketId} <guidance>` }
if (plan.alreadyDone) return { outcome: 'already-done', stage: 'plan', ticket: ticket.ticketId, detail: plan.summary }
if (stopsAt('plan')) return { outcome: 'planned', stage: 'plan', ticket: ticket.ticketId, plan: planText, risks: plan.risks, filesToTouch: plan.filesToTouch }

phase('Implement')
const impl = await agent(
  `${REPO}

Implement ${ticket.ticketId}: ${ticket.ticketTitle}
${ticket.ticketUrl}

Ticket:
${ticket.ticketText}
${opts.guidance ? `\nGuidance from the person who started this run: ${opts.guidance}\n` : ''}
Plan (follow it; record any deviation and why in deviationsFromPlan):
${planText}
Risks: ${plan.risks.join('; ') || 'none noted'}

Setup:
1. Set the Linear issue to In Progress if it is not already, assigned to the current user if unassigned.
2. ${ticket.branchExists
    ? `Switch to ${ticket.branchName}. ${REMOTE_SAFETY(ticket.branchName)} If so, return status "blocked" listing those commits. If the branch has no open PR and is behind origin/main, rebase it onto origin/main; on conflicts, git rebase --abort and return status "blocked".`
    : `git fetch origin, then git switch -c ${ticket.branchName} origin/main.`}

Work:
- Write the tests the plan names, and make them fail for the right reason before the fix where that is practical.
- When an existing test's expectation was wrong, change it in its own commit with the subject "test: correct <test name> (${ticket.ticketId})" and a body that says why the old expectation was wrong (checklist §0). Commits that only add tests do not use that subject.
- Implement in small commits that follow the commit plan. ${COMMIT_STYLE}
${plan.userVisible ? '- Do not edit CHANGELOG.md (the release PR writes it). Put a one-line user-facing release note in releaseNote.\n' : ''}- Keep docs that must move with the change in the same commit: include/blackbox_ffi.h, RustBridge.swift, AGENTS.md invariants, docs/decisions.md, SETUP.md.
- project.yml edits need make xcodegen and the regenerated project in the same commit.
- Run the narrow checks as you go (cargo test for the touched module, cargo clippy --all-targets with the relevant feature set -- -D warnings, swiftlint lint --strict on touched Swift). Do not run the full scripts/check.sh; the gate runs next.
${CODE_RULES}
${GIT_RULES}
Do not push or open a PR. Leave the working tree clean (everything committed).
Return status "blocked" with a reason instead of guessing when you hit a decision the plan did not settle.`,
  { label: 'implement', phase: 'Implement', schema: IMPLEMENT_SCHEMA },
)
if (!impl) return { outcome: 'stopped', stage: 'implement', reason: 'the implementer returned nothing', ticket: ticket.ticketId }
if (impl.status !== 'done') return { outcome: 'stopped', stage: 'implement', reason: impl.blockedReason, ticket: ticket.ticketId, branch: impl.branch, commits: impl.commits }

const firstGate = await gateUntilGreen({
  section: sectionFor(impl.filesChanged),
  label: 'implement',
  phaseName: 'Implement',
  commits: impl.commits,
  attempts: 4,
  context: `Implementing ${ticket.ticketId}: ${ticket.ticketTitle}.`,
})
if (!firstGate.green) {
  return { outcome: 'stopped', stage: 'implement-gate', ticket: ticket.ticketId, branch: impl.branch, reason: firstGate.environment ? firstGate.failingStep : `scripts/check.sh stayed red at "${firstGate.failingStep}"${firstGate.noProgress ? ' (no progress between attempts)' : ''}`, errors: firstGate.errorExcerpt }
}
if (stopsAt('implement')) return { outcome: 'implemented', stage: 'implement', ticket: ticket.ticketId, branch: impl.branch, commits: impl.commits, gate: firstGate.tail }

const review = await reviewLoop({ base: opts.base, bar: opts.bar, maxRounds: opts.maxRounds, ticketText: `${ticket.ticketTitle}\n${ticket.ticketText}`, planText, ticketId: ticket.ticketId })
const reviewSummary = summarizeReview(review)
if (!review.converged) {
  return { outcome: 'stopped', stage: 'review', ticket: ticket.ticketId, branch: impl.branch, reason: `review did not converge: ${review.reason}. Nothing was pushed.`, review: reviewSummary, next: `Resolve the open items, then run /ship-ticket ${ticket.ticketId} again; the review log keeps refuted findings from repeating.` }
}
if (stopsAt('review')) return { outcome: 'reviewed', stage: 'review', ticket: ticket.ticketId, branch: impl.branch, review: reviewSummary }

phase('Ship')
const tidy = await agent(tidyPrompt(impl.branch, true), { label: 'tidy', phase: 'Ship', schema: STEP_SCHEMA, effort: 'low' })
if (!tidy || !tidy.ok) return { outcome: 'stopped', stage: 'tidy', ticket: ticket.ticketId, branch: impl.branch, reason: tidy ? tidy.detail : 'the tidy agent returned nothing', review: reviewSummary }

const branchFiles = review.scope ? review.scope.files : impl.filesChanged
const branchCommits = review.scope ? review.scope.commits : impl.commits
const finalSpec = finalGateFor(branchFiles)
const finalGate = await gateUntilGreen({
  section: finalSpec.section,
  appStore: finalSpec.appStore,
  label: 'final',
  phaseName: 'Ship',
  commits: branchCommits,
  attempts: 3,
  context: `Final full gate for ${ticket.ticketId} after squashing review fixups and rebasing onto origin/main.`,
})
if (!finalGate.green) {
  return { outcome: 'stopped', stage: 'final-gate', ticket: ticket.ticketId, branch: impl.branch, reason: finalGate.environment ? finalGate.failingStep : `the full gate stayed red at "${finalGate.failingStep}". Nothing was pushed.`, errors: finalGate.errorExcerpt, review: reviewSummary }
}
const afterReview = [...finalGate.fixNotes]
if (finalGate.fixNotes.length) {
  const retidy = await agent(tidyPrompt(impl.branch, false), { label: 'tidy:after-gate-fixes', phase: 'Ship', schema: STEP_SCHEMA, effort: 'low' })
  if (!retidy || !retidy.ok) return { outcome: 'stopped', stage: 'tidy', ticket: ticket.ticketId, branch: impl.branch, reason: retidy ? retidy.detail : 'the tidy agent returned nothing', review: reviewSummary }
}

const FOLLOWUP_MARKER = '<!-- FOLLOWUPS -->'
const body = prBody({ ticket, impl, review, gateTail: finalGate.tail, afterReview, followUpMarker: FOLLOWUP_MARKER })
const defectSpecs = review.followUps.map((f) => `- [${f.severity}] ${f.title} at ${f.file}:${f.line}. ${f.reasoning}${f.fix ? ` Suggested fix: ${f.fix}` : ''}`)
const suggestionSpecs = review.suggestions.map((f) => `- [${f.severity}] ${f.title} at ${f.file}:${f.line} (${f.why}). ${f.reasoning}`)

const published = await agent(
  `${REPO}

Publish ${impl.branch} for ${ticket.ticketId}.

1. Follow-up tickets (team Dollhouse, project Audio Blackbox, state Backlog, related to ${ticket.ticketId}). Before creating any, search the project for an open issue that already covers it and reuse that instead.
${defectSpecs.length ? `   a. One ticket per confirmed pre-existing defect, with this evidence and "Found by /ship-ticket review of ${ticket.ticketId}":\n${defectSpecs.join('\n')}` : '   a. No pre-existing defects.'}
${suggestionSpecs.length ? `   b. One ticket titled "Review suggestions from ${ticket.ticketId}" listing these out-of-scope or deferred items:\n${suggestionSpecs.join('\n')}` : '   b. No suggestions.'}
   Put "DOLL-N title" for each ticket in followUps.
2. ${REMOTE_SAFETY(impl.branch)} If so, do not push: return ok=false listing those commits.
3. Push: git push ${ticket.prNumber || ticket.branchExists ? '--force-with-lease ' : ''}-u origin ${impl.branch}
4. Build the PR body from the markdown between the BODY markers:
   - Replace "<!-- COMMITS -->" with a numbered list of git log --format="%s" --reverse origin/main..HEAD.
   - Replace "${FOLLOWUP_MARKER}" with "\\n**Follow-up tickets**\\n" plus one "- DOLL-N title" line per follow-up, or with nothing when there are none.
   - Checklist §6.4 must be complete for the final diff, not only what the implementer reported. Scan git diff origin/main...HEAD for added #[expect( or swiftlint:disable lines, changes to [dependencies] in Cargo.toml, changes to src/ffi.rs or include/, new pub items, and test files; add anything missing to the matching list, replacing "None".
   - Checklist §0 wants every corrected test explained in the PR. Read git log --format="%s%n%b" origin/main..HEAD. For each commit whose subject starts with "test: correct", add a "**Tests corrected**" list after "Tests added" with one "- <subject>: <why the old expectation was wrong, from the commit body>" line each. Omit the list when there are none.
   Write it to target/ship-ticket/pr-body.md.
5. Write the PR title to target/ship-ticket/pr-title.txt with a quoted heredoc (cat > file <<'EOF'), never inline in a command, because titles can contain quotes or backticks: ${ticket.ticketTitle} (${ticket.ticketId})
6. ${ticket.prNumber
    ? `gh pr edit ${ticket.prNumber} --title "$(cat target/ship-ticket/pr-title.txt)" --body-file target/ship-ticket/pr-body.md`
    : `gh pr create --base main --head ${impl.branch} --title "$(cat target/ship-ticket/pr-title.txt)" --body-file target/ship-ticket/pr-body.md`}
7. Attach the PR URL to the Linear issue as a link.
Return prNumber and prUrl.

BODY START
${body}
BODY END`,
  { label: 'publish', phase: 'Ship', schema: PUBLISH_SCHEMA, effort: 'low' },
)
if (!published || !published.ok) return { outcome: 'stopped', stage: 'publish', ticket: ticket.ticketId, branch: impl.branch, reason: published ? published.detail : 'the publish agent returned nothing', review: reviewSummary }
log(`PR #${published.prNumber} open: ${published.prUrl}`)
if (stopsAt('pr')) return { outcome: 'pr-open', stage: 'pr', ticket: ticket.ticketId, pr: published.prUrl, followUps: published.followUps, review: reviewSummary }

async function watchCi(label) {
  const r = await agent(
    `Watch CI for PR #${published.prNumber} until it is decided. Read-only apart from gh queries.
${CI_GREEN_RULE}
Poll gh pr checks ${published.prNumber} --json name,bucket,link about every 60 seconds using background waits, so no single command hits a tool timeout. The Swift app job is the slowest (often 10-20 minutes). Stop waiting after 60 minutes: state "timeout".
state "green" per the rule above. "failed" as soon as any check fails: for each failing check pull the failing step's log (gh run view <run id> --log-failed, or gh api repos/{owner}/{repo}/actions/jobs/<job id>/logs) and put the step name and its first error lines in excerpt.`,
    { label: `ci:${label}`, phase: 'Ship', schema: CI_SCHEMA, effort: 'low' },
  )
  return r || { state: 'timeout', failures: [{ check: 'ci watcher', excerpt: 'the CI watch agent returned nothing' }] }
}

let ci = await watchCi('1')
for (let attempt = 1; ci.state === 'failed' && attempt <= 2; attempt++) {
  log(`CI failed (${ci.failures.map((f) => f.check).join(', ')}); fix attempt ${attempt}/2`)
  const ciFix = await agent(
    `${REPO}

CI failed on PR #${published.prNumber} (${impl.branch}) although the local gate was green. Find why the local gate missed it.

${ci.failures.map((f) => `### ${f.check}\n${f.excerpt}`).join('\n\n')}

Reproduce each failure locally with the command the CI job runs (.github/workflows/rust.yml), make the smallest fix for the root cause, and confirm the reproduction passes. If the failure is a flaky runner problem with no code cause (network, runner image, cache), change nothing and return ok=false with detail starting "flaky:" and the evidence. If scripts/check.sh does not cover what failed, say so in detail.
${CODE_RULES}
${GIT_RULES}
${FIXUP_RULES(branchCommits)}
Do not squash, run the full scripts/check.sh, or push; the workflow does those next. In detail, describe each change in one line.`,
    { label: `ci-fix:${attempt}`, phase: 'Ship', schema: STEP_SCHEMA },
  )
  if (!ciFix) break
  if (!ciFix.ok) {
    if (!/^flaky:/i.test(ciFix.detail)) break
    await agent(`Re-run the failed jobs of the latest CI run for PR #${published.prNumber}: find the run id from gh pr checks ${published.prNumber} --json link, then gh run rerun <run id> --failed. Report ok and the run id.`, { label: `ci-rerun:${attempt}`, phase: 'Ship', schema: STEP_SCHEMA, effort: 'low' })
    ci = await watchCi(`rerun-${attempt}`)
    continue
  }
  const ciGate = await gateUntilGreen({ section: finalSpec.section, appStore: finalSpec.appStore, label: `ci-fix-${attempt}`, phaseName: 'Ship', commits: branchCommits, attempts: 2, context: `After a CI fix for PR #${published.prNumber}.` })
  if (!ciGate.green) {
    return { outcome: 'pr-open', stage: 'ci-fix-gate', ticket: ticket.ticketId, pr: published.prUrl, reason: `the local gate is red after the CI fix at "${ciGate.failingStep}"; nothing new was pushed`, errors: ciGate.errorExcerpt, review: reviewSummary }
  }
  const changes = [ciFix.detail, ...ciGate.fixNotes]
  afterReview.push(...changes)
  const pushed = await agent(
    `${tidyPrompt(impl.branch, false)}

Then:
5. ${REMOTE_SAFETY(impl.branch)} If so, do not push: return ok=false listing those commits.
6. git push --force-with-lease origin ${impl.branch}
7. Comment on PR #${published.prNumber} with gh pr comment, heading "Changed after review (CI fix, gated locally, not re-reviewed)", listing:
${changes.map((c) => `- ${c}`).join('\n')}`,
    { label: `ci-push:${attempt}`, phase: 'Ship', schema: STEP_SCHEMA, effort: 'low' },
  )
  if (!pushed || !pushed.ok) return { outcome: 'pr-open', stage: 'ci-push', ticket: ticket.ticketId, pr: published.prUrl, reason: pushed ? pushed.detail : 'the push agent returned nothing', review: reviewSummary }
  ci = await watchCi(String(attempt + 1))
}
if (ci.state !== 'green') {
  return { outcome: 'pr-open', stage: 'ci', ticket: ticket.ticketId, pr: published.prUrl, reason: `CI is ${ci.state}`, failures: ci.failures, followUps: published.followUps, review: reviewSummary }
}

let merged = null
if (opts.merge) {
  for (let attempt = 1; attempt <= 3; attempt++) {
    const m = await agent(
      `Merge PR #${published.prNumber} per the repo merge policy (rebase merges only; the branch must be up to date with main; auto-merge is off).
1. gh pr view ${published.prNumber} --json state,mergeStateStatus,headRefOid and gh pr checks ${published.prNumber}. If mergeStateStatus is UNKNOWN, GitHub is still computing it: re-query about every 15 seconds for up to 2 minutes, and never merge while it is UNKNOWN; if it stays UNKNOWN, merged=false with detail, and stop.
2. --admin bypasses the up-to-date rule, so check it yourself: git fetch origin main pull/${published.prNumber}/head --quiet, then git merge-base --is-ancestor origin/main <headRefOid>. If that fails, or mergeStateStatus is BEHIND: gh pr update-branch ${published.prNumber} --rebase, set updatedBranch=true and merged=false, and stop; CI must run again first.
3. ${CI_GREEN_RULE} If CI is not green by that rule, merged=false with detail, and stop.
4. Otherwise: gh pr merge ${published.prNumber} --rebase --admin --delete-branch. Confirm gh pr view reports MERGED.
5. git switch main && git pull --ff-only; mainHead = git rev-parse --short HEAD.`,
      { label: `merge:${attempt}`, phase: 'Ship', schema: MERGE_SCHEMA, effort: 'low' },
    )
    if (!m) break
    merged = m
    if (m.merged || !m.updatedBranch) break
    ci = await watchCi(`after-update-${attempt}`)
    if (ci.state !== 'green') break
  }
}

const didMerge = Boolean(merged && merged.merged)
const closeout = await agent(
  `Update Linear issue ${ticket.ticketId} after /ship-ticket.
- Make sure PR ${published.prUrl} is attached as a link (add it if missing).
- ${didMerge ? 'Set the state to Done.' : 'The PR is not merged. If the team has an "In Review" state, set it; otherwise leave the state unchanged.'}
Report the final state.`,
  { label: 'linear', phase: 'Ship', schema: CLOSEOUT_SCHEMA, effort: 'low' },
)

return {
  outcome: didMerge ? 'merged' : 'pr-open',
  ticket: ticket.ticketId,
  title: ticket.ticketTitle,
  pr: published.prUrl,
  branch: impl.branch,
  mainHead: didMerge ? merged.mainHead : '',
  mergeDetail: opts.merge ? (merged ? merged.detail : 'the merge agent returned nothing') : 'merge skipped (no-merge)',
  ci: ci.state,
  linear: closeout ? closeout.state : 'not updated',
  followUps: published.followUps,
  review: reviewSummary,
  changedAfterReview: afterReview,
  gate: finalGate.tail,
}
