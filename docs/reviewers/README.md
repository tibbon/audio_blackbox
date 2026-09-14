# Reviewer briefs

These files are the review lanes the `/ship-ticket` workflow (`.claude/workflows/ship-ticket.js`)
runs on every branch. Each brief is written for this repo: its invariants, its ticket history,
and the mistakes a generic reviewer would make here. `docs/REVIEW-CHECKLIST.md` is the standard
they apply; the briefs say which parts of it each lane owns.

## How the workflow uses them

1. The workflow lists the files the branch changed against `origin/main` and picks lanes from
   the routing table in `ship-ticket.js` (the `LENSES` table). `baseline` and `guardrails`
   always run; the others run when their files change.
2. Each reviewer agent gets its brief, the diff, the ticket, and the plan. It reads the changed
   code and returns findings with `file:line` evidence. Reviewers do not edit, build, or test.
3. A skeptic agent reads the code behind every finding that could block and returns one verdict:
   **refuted** (a preference, handled elsewhere, or unprovable; the default when unsure),
   **duplicate** of a judged finding, **out-of-scope** (possibly true, but a capability,
   hardening or design the ticket did not ask for), or **confirmed** (the changed code does
   something wrong or breaks a documented contract).
4. One fixer applies the confirmed findings as small fixup commits and the gate
   (`scripts/check.sh`) runs again. A fix that needs a new stage, option, file or mechanism is
   deferred instead.
5. Round 1 reviews the whole branch. Later rounds review only the fix commits; older code blocks
   again only for a high or critical finding. Judged findings are not raised again within a
   run, except a fixed finding whose defect is still present at HEAD. Only refuted verdicts are
   kept across runs (the review log), so confirmed and out-of-scope findings can come back on a
   rerun; out-of-scope ones are then filed in the follow-up ticket.
6. The loop converges when a round confirms nothing to fix. It stops without converging when
   a round confirms more than half (rounded up) of the round before, when a fix pass grows the branch past the
   growth limit, or at the round cap. The last round never fixes, because nothing would
   re-review those fixes. Nothing is pushed unless review converged.

Out-of-scope findings and deferred mediums go into one grouped follow-up ticket. The first
live run (DOLL-654) showed why: reviewers kept finding real gaps in an orchestration script,
the fixer built features for them, and each round's new code produced the next round's findings.

## Severity rubric (canonical)

Every brief and every agent uses these four levels. A brief's "Severity in this lane" section
gives lane-specific examples; it never redefines a level.

- **critical**: a crash, undefined behavior, lost or corrupted audio or files, a security or
  privacy exposure, or a build, CI, or release that is broken for everyone.
- **high**: wrong behavior users hit in normal use; a real-time thread violation; a broken FFI
  contract; a regression of existing behavior; a silenced lint or a weakened test.
- **medium**: a real defect with a narrow trigger or a workaround; new behavior without a test;
  a swallowed error; a missing or false SAFETY, ownership, or thread-contract comment where the
  checklist requires one; contract drift (header, `RustBridge.swift`, `project.yml`, docs that
  must move with the change).
- **low**: polish, naming, readability, optional refactors, suggestions. Never blocks.

The default bar is **medium**. Findings at medium or above are verified and fixed. Low findings
are listed in the PR description as notes and are not verified.

A defect is code that does something wrong or breaks a documented contract (the ticket,
AGENTS.md, this checklist). A capability the ticket did not ask for, extra hardening against
unlikely misuse, or a different design is not a defect at any severity.

## Pre-existing issues

A pre-existing issue is in code the branch did not change and did not make worse. Report it with
`origin: "preexisting"`. It never blocks the branch. Confirmed pre-existing findings at high or
above become Linear follow-up tickets instead of fixes on this branch, so the branch stays
scoped to its ticket; lower ones are notes.

If the branch touched the code and made an existing problem worse (a new caller of a racy helper,
a second copy of a wrong constant), it is not pre-existing.

## What reviewers never report

- Anything `scripts/check.sh` fails on mechanically: rustfmt, clippy with `-D warnings`, rustdoc,
  swift-format, `swiftlint --strict` and its custom rules, FFI header parity, String Catalog sync,
  pbxproj parity, cargo deny, cargo machete. The gate runs before review and after every fix.
- Style preferences with no bug behind them, above low.
- Concerns another lane owns. Each brief lists its neighbors.

## Adding a lane

1. Write `docs/reviewers/<key>.md` in the same shape as the existing briefs: `Runs when`,
   `In your lane`, `Not your lane`, `Severity in this lane`, `Facts that prevent false positives`.
2. Add an entry for `<key>` to the `LENSES` table in `.claude/workflows/ship-ticket.js` with the
   path patterns that route to it.
3. Update the neighbors' "Not your lane" sections so two lanes do not report the same thing.
4. Run `/ship-ticket review` on a branch that touches the new lane's files to see it fire.
