**English** · [한국어](CHANGELOG_ko.md)

# Changelog

All notable changes. Versions follow [Semantic Versioning](https://semver.org/); until 1.0, a minor version may change the CLI or file formats, and the entry says how.

## 0.9.1 - 2026-11-24

### Added
- `ach gate-criteria <id> --file <corrected gate.json> --expected-revision N [--expected-request TOKEN] [--dry-run]` fills in the judging criteria that a recorded gate left empty, for a contest already in progress (`running`, `plan_review`, `awaiting_approval`). A gate recorded with `criteria: []` cannot be recorded again after routing, and `ach evaluation prepare` refuses such an entry, so it could never get the independent final assessment a delegated approval needs. The file is the recorded gate with only `criteria` filled in. The command writes only when every condition holds: it runs outside contest sessions; the entry has no submission record and no unresolved submission attempt; the work revision and request token equal the ones given; the recorded criteria are empty and the file differs from the recorded gate in no other field; the rows equal the structured `{stage?, name, weight?}` rows of the accepted rules' `criteria` category, whose snapshots still hash as captured; every recorded score used the same rows; no submit approval was recorded in this approval wait; and no evaluation packet or assessment exists. It updates `gate.criteria`, the folder copy `.harness/gate.json`, appends the change to `extra.gate_amendments` (before, after, evidence, file hash, revision, request token), and adds a same-state history marker. State, work revision, request token, scores and the readiness record stay as they were. `--dry-run` runs the same checks and writes nothing.

### Migration
- None. Entries change only when the command is run for them. The contest session commits the updated `.harness/gate.json`.

## 0.9.0 - 2026-11-13

### Changed
- The score loop is bounded. Over 421 Judge runs in 26 contests, re-scoring the same packet moved the total by a median of 9 points (max 11.5), and the new-best gain per run was +3.9 at run 2, +1.7 at run 3, +2.6 at run 4, and at most 0.7 on average from run 5. Because the submission gate demanded `total >= target` on the latest `final`, sessions re-scored until a lucky sample passed (one contest took 41 draft/final runs and ended oscillating between 85.5 and 92). Plan scoring called a fresh Judge per candidate, made up 40% of all runs, and compared candidates across that noise.
- The ContestJudge verdict is findings-based: `reject` for a failed gated check other than persona traces or a rule or eligibility violation, `revise` for persona traces or any remaining CRITICAL/HIGH finding (including the Korean polish record HIGH), `ready` otherwise. A total below the target alone no longer makes the verdict `revise`. Independent-mode candidates follow the same rule.
- New config key `final_rounds` (integer, at least 1, default 3). `ach score --stage final` stamps the value in force on the record as `final_rounds` (a `final_rounds` key in the Judge output is ignored). The score gate (`ach transition … awaiting_approval`, `ach approval-status`, readiness and approval checks) still requires the latest `final` of the current work revision to be `ready` with matching artifact hashes; `final_below_target` is now reported only while the current work revision has fewer `final` records than the `final_rounds` stamped on the latest final, and the message shows the count (`최근 final 86/90 목표 미달(이번 작업 판 final 2/3회)`). Once that many finals exist, a `ready` final below the target passes. Recording a score is never refused because of the limit.
- Session loop rules (skill score loop, CONTRACT section 15, ContestHQ, level prompts, templates, playbooks): `draft` is scored once per checkpoint and a checkpoint is not re-scored to confirm fixes. `final` has a budget of `final_rounds` runs per work revision; after each run fix CRITICAL/HIGH first, and stop when the latest final is `ready` and reaches the target or the budget is used. Unchanged files are never re-scored for a higher number. If the budget is used and the verdict is still `revise` or `reject`, the session reports the remaining CRITICAL/HIGH findings and uses `--score-ok` with your approval. Level 1: one Judge run, plus one more after fixes when it says `revise`.
- Plan scoring runs one fresh ContestJudge per round on every candidate of that round (at most 3 rounds). The Judge scores each candidate separately against the same basis and rubric and lists them all in the new optional field `plan_candidates[{candidate, total, scores, plan_changes}]`; the top-level fields describe the highest-scoring candidate, so `ach score --stage plan` records the output unchanged. The session writes every candidate's total into the `RUBRIC.md` trend table under the same round number. Rounds stop early when the chosen candidate's predicted total reaches the target or the remaining `plan_changes` are below the noise.
- New config key `main_model` (string or `null`, default `null`): the model the main interface session (`ach chat`) opens with. Contest sessions and scheduled runs never use it.
- ContestEditor also removes AI-writing tells (repeated sentence patterns and endings, stock connectives and translationese, mechanical triplets and parallelism, paragraph-closing summaries, unsupported intensifiers) using the `humanize-korean` taxonomy, rewriting whole sentences so the text reads as human-written while content, numbers and claims stay unchanged.
- The Gemini polishing calls run through the Antigravity CLI instead of the session runner. New `scripts/agy-polish.sh` runs one step on `agy` (default model `gemini-4-hydrogen-high`) from an instruction file the caller writes, lets agy read and write files but not run commands, reports the SHA-256 change of the expected files, and retries a failed call only while none of them changed. ContestEditor writes the instruction file, runs the script, then reviews the diff against a copy of the original and reverts any change that breaks the preservation rules (it gains the `write` tool for `.private/editor/`). The humanize-korean role calls (diagnostician, monolith, finalizer) run through the same script instead of runner subagents. A failed call is reported, never replaced by another model.
- The window `max_running` (default 6) is now the only limit on starting a contest session (`launch`, `restart`, `resume`, `recover`, `tick`, running → after a revision request). Memory budget, memory pressure and a throttled token window no longer refuse a session; `ach capacity` reports them in the new `warnings` list and still uses them for `recommended_workers` and the scheduled run's gate count. `slots_by_level` stays and shows the same free count for every level. The config keys `max_running_by_level` and `ram_gb_by_level` are removed.

### Fixed
- `ach launch` and `ach restart` reused the workspace id recorded in the registry even after the backend had forgotten it (removed by hand or by a cleanup), so every retry failed with Orca's `selector_not_found`. A recorded Orca workspace that `orca worktree show` no longer knows is now dropped and a new one is created.

### Migration
- No registry migration. Existing score records stay valid; a `final` record without `final_rounds` counts as 3.
- Running sessions pick up the new rules when they re-read the skill.
- There is no setting that restores the unbounded loop. To allow more final runs, raise the budget with `ach config set final_rounds <n>`; a changed value applies from the next recorded `final`.
- Remove `max_running_by_level` and `ram_gb_by_level` from `data/config.json` (`ach config unset max_running_by_level`, `ach config unset ram_gb_by_level`); the config loader now refuses them as unknown keys. Set the window with `ach config set max_running <n>`.

## 0.8.5 - 2026-11-12

### Fixed
- `ach score` refused every score for a gate with a pass/fail screening step published as weight 0 (e.g. a first-round topic screen): the Judge could neither list it (weights must be above 0) nor omit it (every gate criterion had to be scored). Weight-0 gate criteria are now treated as unscored screens and are not required.

## 0.8.4 - 2026-11-07

### Changed
- ContestEditor polishing and the Korean prose polish (`humanize-korean`) now run once, right before submission: on the submission candidate after improvement scoring has settled its content, in the order ContestEditor → polish → page review → final score. They no longer run after the first draft or during improvement loops (skill score loop steps 3–5, level prompts, ContestHQ, ContestEditor, playbooks, CONTRACT).
- ContestJudge checks the polish record only on a `final` the caller marks as the polished submission candidate; `draft` and improvement-loop `final` scores no longer get a HIGH finding for a missing record.

## 0.8.3 - 2026-11-03

### Fixed
- `ach clean` run from inside the contest's own session (`ACH_SESSION` equal to its id) closed that session first, which stopped the command before the folder was moved and the registry was updated. It now archives, trashes the folder, and saves the ledger and registry first, and closes its own session (and workspace) as the very last step. Another unrecorded live terminal in the workspace still blocks the clean.
- A clean rerun after an interrupted one failed on files already archived (read-only rule snapshots). Archived files with the same bytes are now skipped; different bytes are never overwritten and block the clean.

## 0.8.2 - 2026-10-27

### Changed
- The contest basis is documented once, when the contest starts. `RUBRIC.md` gains a first section `## 0. 채점 근거`: organizer (주최), host (주관), purpose (개최 목적) and judging criteria (심사기준), each as written in the notice with its location, plus a short summary of what the organizer would pick as a winner. ContestHQ writes it right after reading the notice (level 3-5); a level 2 session writes it before research.
- Every plan-scoring Judge run uses that section as the shared yardstick. It checks the section against the notice and, where they differ, scores against the notice and reports the difference.
- `ach score --stage plan` refuses until the section exists with all four rows filled (content and notice location, no template placeholders), and stores the section's `{path, sha256}` as `basis.document`.

### Migration
- Running contests without the section must add it to `RUBRIC.md` before their next plan score. Plan scores recorded earlier are unchanged.

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
