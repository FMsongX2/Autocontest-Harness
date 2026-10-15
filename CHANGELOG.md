**English** · [한국어](CHANGELOG_ko.md)

# Changelog

All notable changes. Versions follow [Semantic Versioning](https://semver.org/); until 1.0, a minor version may change the CLI or file formats, and the entry says how.

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
