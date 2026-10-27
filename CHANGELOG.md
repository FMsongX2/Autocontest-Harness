**English** · [한국어](CHANGELOG_ko.md)

# Changelog

All notable changes. Versions follow [Semantic Versioning](https://semver.org/); until 1.0, a minor version may change the CLI or file formats, and the entry says how.

## 0.8.1 - 2026-10-27

### Changed
- Plan scoring is tied to the contest at hand. Every `ContestJudge` plan call is a new run told to score against this contest's organizer and host, its stated purpose, and its judging criteria. The Judge reports them as `basis {organizer, purpose, criteria_source}` with notice locations, and each criterion rationale is written against them.
- `ach score --stage plan` refuses a score without the three `basis` fields or without `evaluator {id, model, run_id}`, and refuses a `run_id` already recorded for the contest, so a plan cannot be re-scored by reusing an earlier Judge run. Both objects are stored with the score.

### Migration
- Plan scores recorded before 0.8.1 stay valid. New plan scores from running sessions need `basis` and `evaluator`; the refusal message names the missing field.

## 0.8.0 - 2026-10-27

### Added
- Runtime checkpoints with verified artifacts, observational `ach reconcile [--apply]`, and conservative `ach recover` from the latest checkpoint. Unknown runtime is never treated as missing, ambiguous terminals are never adopted, and routed work is never started by recovery.
- Official source snapshots (`ach rules capture|review|acknowledge|status`) with semantic, category-based invalidation of gate, plan, final and submission evidence; byte-only changes stay pending until reviewed.
- Blinded independent evaluation packets and measured reporting (`ach evaluation prepare|record|report|outcome|calibration`), kept separate from improvement scoring, with no invented probabilities.
- Durable submission transactions (`ach submission intent|start|unknown|resolve|status`) bound to the approved destination and artifacts, plus receipt confirmation through `ach submit <id> --attempt A --file proof.json`. An unknown external result blocks retries until proven.
- Managed controllers under `controllers/` (`ach controller approve|queue|briefs|preset`) with contract checks, lifetime locks, durable alert deduplication, and the `new_work_frozen` setting.
- Board columns for runtime, next action, blockers, evidence time, and a user-attention section, built from stored data without probing backends.

### Changed
- Delegated approval additionally requires tracked official rules and, for scored non-video work, a verified independent final assessment. Known pending or invalidating rule changes block every approval path, including `--score-ok`.
- `ach submit` no longer accepts direct receipt flags; `ach transition … submitted` remains refused. Transitions are blocked while a dispatched submission is unresolved.

### Fixed
- Restore managed controller preset-name validation so queued launches and explicit preset switches do not fail with a missing validator.
- Resolve alerts for removed contests even when they were raised earlier in the current cycle.
- Orca sessions are judged by `terminal show` `connected`, so a closed terminal (whose history Orca keeps) no longer blocks `restart`/`session-close`; disconnected terminals are not counted as live.
- omp restores no longer pass a prompt (`omp --restore` refuses one) and name the preset's default model explicitly, so a restored session switches to the new preset. `restart` reports that the message was not delivered; `recover` also writes the checkpoint turn to `.harness/recovery-<seq>.md`.
- `.gitignore` now keeps everything under `data/` local except the committed examples (controller state, rule snapshots and briefs included).
- Update `rustls-webpki` to 0.103.16 and `webpki-roots` to 1.0.11 (RUSTSEC-2026-0382, RUSTSEC-2026-0381: distrusted intermediates could be accepted).

### Migration
- Build from source with `scripts/install.sh --from-source`. Old controller scripts under `data/` moved to `controllers/` (`ach controller …`); restart the local controller on the new CLI.
- Existing contests gain no rules evidence, independent evaluations or submission transactions automatically; delegated approvals stay blocked until those records exist.

## 0.7.0 - 2026-10-25

### Changed
- Submission readiness is now structured (`ach readiness`): explicit blockers, questions, follow-up work and verified artifact SHA-256 values. `ach approval-status` is the single approval-validity/eligibility API used by the local controller; prose is never treated as readiness.
- Final scores and submission approvals are bound to the current work revision and artifact manifest. Revisions, new scores (including reject), changed files and new readiness requests invalidate old approvals. Score totals, positive weights, duplicate criteria and published gate criteria are validated.
- Scoped, revocable submission delegation is explicit configuration and defaults off. `approve --delegated --expected-request` rechecks the request under the registry lock; contest sessions cannot grant delegation or approve their own work. Personalization uses separately reviewed final copies linked to preserved originals.
- `ach restart` retains the runner environment, arguments and saved conversation; `ach resume` checks capacity and opens a session before marking follow-up work running. `ach session-close` compares state/history under lock and retains failed closes for retry. Direct transitions to `submitted`, and from submitted/follow-up to `running`, are refused.
- Stale Orca handles are not proof that a workspace is empty: restart and cleanup inspect its complete current terminal inventory and refuse to replace or remove unowned live terminals, including shells whose activity cannot be verified.
- Korean prose polishing is performed by the calling session. Editor and Judge consume the recorded preservation checks and hashes; missing polishing evidence is a HIGH/revise finding, not a contradictory failed hard gate. Brief refresh replaces only managed sections, detects manual edits and preserves explicitly adopted legacy sections in a backup.
- Paginated report, proposal, paper, and application deliverables require direct visual inspection of every rendered page. Metadata, style values, extracted text, OCR, and successful image generation do not replace page-by-page inspection.
- Authoring playbooks, launch/HQ instructions, ContestEditor, ContestJudge, and submission templates require file hashes and page-specific evidence; edits, signing, saving, or conversion invalidate the previous visual review. ContestJudge rejects incomplete final-page inspection. This is an agent-review requirement, not image analysis performed by `ach`.
- Submitted contests waiting for a future round can use `followup_pending`: release the session slot without deleting the folder or changing submission receipts. Unsubmitted entries cannot enter it; actual follow-up work resumes through `ach resume`.

### Migration
- Build/install from this checkout with `scripts/install.sh --from-source`. Historical scores, receipts and approvals remain readable, but old unbound approvals/final scores cannot authorize new submissions. Re-review current artifacts, record a current final and structured readiness, then obtain a new approval; never bulk-promote legacy notes to ready.
- Record the user's existing delegation through `ach config` only after confirming its scope. Restart the local controller on the new CLI, and refresh existing controller briefs with a dry run before explicitly adopting legacy sections. No migration command submits contests or closes their sessions.

## 0.6.0 - 2026-10-16

You talk to the main session; the agents call the harness core, now named `ach`, which is no longer installed on your `PATH`.

### Changed
- **The binary is `ach`** (was `harness`). Release assets are `ach-<version>-<target>.tar.gz`. `scripts/install.sh` installs `bin/ach` in the clone and links nothing onto your `PATH`; it removes the `~/.local/bin/harness` link and `bin/harness` that earlier versions installed.
- **Environment variables:** `HARNESS_HOME` is now `ACH_HOME`, `HARNESS_SESSION` is `ACH_SESSION`, and the omp extension's `HARNESS_BIN` is `ACH_BIN`.
- **How agents find `ach`:** every session the harness starts (contest sessions, the main session `bin/ach chat`, scheduled runs) runs with the clone's `bin/` first on its `PATH`. The omp extension adds it to the omp sessions that load it, the Orca automation's prompt names the binary's path, and the skill tells any other session to call `<clone>/bin/ach`.
- **Scheduled jobs:** `ach schedule status` and `ach doctor` report jobs written by 0.5 or earlier (they run `harness` with `HARNESS_HOME`) and an Orca automation with an older prompt as not in sync. `ach schedule install` rewrites them, and now also updates the prompt of an existing Orca automation.
- **Documentation:** the README is agent-first: install, ask your agent to read `INIT.md`, open the main session with `bin/ach chat`, and talk to it. The CLI reference, configuration, sources, scheduling, routing, ledger, cleanup, profile, local rules, and project layout moved to `docs/reference.md` and `docs/reference_ko.md`. Figures are redrawn around the main session and `ach`.

### Migration
Run `git pull && scripts/install.sh`, then `bin/ach schedule install` once so the scheduled run calls `ach`. Contest sessions started by 0.5 still call `harness`, which is gone: launch them again, or tell their agent to use `<clone>/bin/ach`. Registry, ledger, and config files are unchanged.

## 0.5.2 - 2026-10-15

### Fixed
- `harness init` narrows an existing `data/` to 0700. A clone already has `data/` (it holds the committed examples) with the umask's mode, so other users could list it; with umask 002, as on Ubuntu, it was group-writable. The JSON report has a new `tightened` list.

## 0.5.1 - 2026-10-15

### Changed
- The Korean documentation moved to its own files: `README_ko.md`, `CONTRIBUTING_ko.md`, `SECURITY_ko.md`, `CHANGELOG_ko.md`. The English files no longer carry a Korean section.

### Fixed
- `harness doctor` no longer warns about the binary's commit when the checkout moved past it with documentation-only commits, so a fresh clone of `main` installed from the latest release passes the version check.

## 0.5.0 - 2026-10-15

A second runner adapter: Claude Code, written by Claude Code itself from `CONTRIBUTING.md`.

### Added
- `claude` runner adapter for Claude Code, written by Claude Code from `CONTRIBUTING.md`: converts the agents into subagent files under `~/.claude/agents/contest-harness/`, links the skill into `~/.claude/skills/`, applies `agent_models` through the `model` field of those files, passes the instructions as an `@<path>` mention (as text when the path has whitespace), and defaults `runner_args.claude` to `--permission-mode auto`. A scheduled run (`harness autorun`) on Claude Code 2.1.296 collected a contest and recorded the gate decision through the `ContestGate` subagent. Interactive contest sessions, `harness chat`, per-agent models, and Orca automations have not run on it.
- `Spec.prompt` for runner adapters: the instructions in memory, which `runners::prompt_text` returns before it reads `prompt_file`.

### Changed
- `INIT.md` asks which runner to use and covers model choices on Claude Code.

### Fixed
- `harness launch --dry-run` works with adapters that pass the instructions as text. It built the command from a file the dry run never writes; it now passes the text in `Spec.prompt`.

## 0.4.0 - 2026-10-15

Agent-driven setup: point your agent at `INIT.md` and it sets the harness up with you.

### Added
- `INIT.md`: the setup procedure an agent follows: install, scaffold, one batch of questions with recommended defaults, apply them through the CLI, the user profile, model choices, install, schedule, doctor. omp: `/contest-init`.
- `harness init`: creates `data/`, `data/config.json` and `data/profile.md` from the examples (private permissions), keeps existing files, and runs the doctor.
- `harness config path|get|set|unset`: dotted keys, values read as JSON when they parse, the whole config validated before anything is written.
- `agent_models` and `harness agents`: a model per harness agent. `harness install` applies the choices to the runner (omp: `task.agentModelOverrides`, touching only the harness agents); `harness doctor` checks names and drift.
- Runner adapter methods `apply_agent_models` and `agent_models`.

### Changed
- The main session starts with `harness doctor` and runs the `INIT.md` procedure when the checkout is not set up.
- The profile can be filled by dictating it to the setup agent (`INIT.md` step 4). Writing the personal-details section yourself stays the recommendation, because dictated values pass through the model provider and the session transcript.

## 0.3.0 - 2026-10-15

Production hardening: installs and runs on a new machine, survives failures halfway through, and keeps unattended agents inside the user's approvals.

### Added
- Prebuilt binaries for macOS (arm64, x86_64) and Linux (x86_64, arm64, static musl) on GitHub Releases, with `SHA256SUMS`. `scripts/install.sh` installs the one matching the checkout's version (or builds from source) and links it into `~/.local/bin`.
- `harness doctor`: checks the checkout, binary/checkout version drift, config, data files, git identity, gh login, runner and workspace, profile, memory probe and the scheduled run, each with the fix.
- `harness approve <id>`: the user's approval, required by `harness submit` and `harness profile show --for <id>`, and the only way out of a Level 5 `plan_review`. Contest sessions run with `HARNESS_SESSION=<id>` and cannot approve themselves.
- `harness add --from-file`, `dedupe-check --id` without a title, `--note-file` / `--receipt-file`: agents pass fetched text through files, never on a shell command line.
- `cron` scheduler (macOS and Linux) next to Orca automations and launchd.
- Linux support: memory from `/proc/meminfo`, pressure from PSI, XDG trash.
- `harness --version` includes the git commit.
- `SECURITY.md` with the threat model; private vulnerability reporting. `scripts/check.sh` and `scripts/release.sh`. An end-to-end test of the whole lifecycle.

### Changed
- Config: unknown keys are rejected and values are validated when loaded; errors name the file.
- `route` checks gh, git identity and the workspace before writing anything, can be re-run after any failure, refuses an existing public or foreign repository, disables GitHub Actions on new repositories, and leaves the contest folder clean in git.
- `route`, `launch` and `tick` do network work outside the registry lock; a failed route leaves the entry untouched; launches are serialised so limits hold.
- `clean` fetches before locking, keeps the records of entries that succeeded when another fails, rejects archive paths that leave the folder, and uses the XDG trash on Linux.
- Collection keeps a listing whose detail page fails (with a note) instead of dropping the whole source, never merges two listings of the same site by title, treats titles that differ by a number as different contests, and retries transient HTTP errors.
- Ledger: manual records with similar titles no longer overwrite each other; a new edition of a submitted contest is no longer a duplicate.
- Registry and ledger files carry a format version, keep unknown fields, and are written 0600 with a directory fsync; `data/` is created 0700.
- External commands run with timeouts. Headless sessions are closed by process group only, never by a reused pid.
- launchd jobs are labelled per checkout. Release builds unwind on panic instead of aborting.
- YAML parsing uses `serde-saphyr` (no unmaintained dependencies in `cargo audit`).

### Migration
- A Level 5 contest whose plan review ended with `transition ... running` before this version needs `running -> plan_review` again and `harness approve`.
- Contest sessions now ask for `harness approve <id>` (in your terminal or through the main session) before submitting.
- Remove unknown or misspelled keys from `data/config.json`; `harness doctor` names them.
- Reinstall the binary with `scripts/install.sh` after `git pull`.

## 0.2.0 - 2026-10-15

Rust rewrite of the harness: single binary, 11 sources, five execution levels, score loop, submission ledger, persona guard, cleanup with archive, Orca/tmux/headless workspaces, omp adapter.
