---
name: chamfera-contributing
description: "Use when starting a task in Chamfera, choosing what to work on, committing, or handing work back: the workflow every change follows."
---

# Working on Chamfera

## Before you start
- Read `PLAN.md` for the current milestone and what "done" means for it.
- Read the skill for the area (the table in `AGENTS.md`).

## Branches
- `dev` is the default branch, where development happens. Start each topic from the latest `dev`: `git switch dev && git pull && git switch -c <area>/<topic>` (for example `kernel/k1-curves`, `ui/viewport`).
- Merge a topic back into `dev` through a pull request once `cargo xtask ci` passes.
- `main` is only for releases. A release is a merge of `dev` into `main` plus a tag `vX.Y.Z`. Never commit or push work directly to `main`, and never force-push `dev` or `main`.
- Remote: `origin` is `https://github.com/Open-Physical-Systems/Chamfera`.

## While you work
- One topic per commit. Commit after each piece that builds and passes `cargo xtask ci`.
- Commit messages: a first line that someone who never saw the chat understands ("Sketch: coincident constraint joins two points"), then why.
- Keep `PLAN.md` honest: when a milestone item lands, mark it.
- Never commit `target*/` or local `plan/` notes.
- Parallel work: dispatch one subagent per independent unit with a self-contained brief (what to change, what not to touch, what to report). No two units write the same file. Read their reports before continuing; a report is not a result until you check it.

## Showing a change
- For anything a user can see (from M1), capture a real screenshot or recording yourself.
- For geometry, show the measured numbers next to the expected ones.

## Before you say "done"
- `cargo xtask ci` passes. If something failed or was skipped, say so with the output.
