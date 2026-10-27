**English** · [한국어](reference_ko.md)

# Reference

Operator and contributor reference for Autocontest-Harness. You do not need it to use the harness: you talk to the main session, and the agents call `ach`, the harness core. Commands below are written as `ach <command>`; from a terminal in the clone, run them as `bin/ach <command>`. The interface contract is [CONTRACT.md](CONTRACT.md); the overview is the [README](../README.md).

## The `ach` binary

`ach` is the harness core: one binary that owns the registry, the ledger, and the scheduled jobs, collects listings, routes and launches contests, and enforces the approval, score, and persona gates. The agents call it; you normally only run `bin/ach chat`.

- **Install.** `scripts/install.sh` downloads the binary for your platform from the GitHub release whose tag is `v` followed by the version in `Cargo.toml`, checks it against the release's `SHA256SUMS`, and installs it to `bin/ach` in the clone (git-ignored). Prebuilt targets are `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-musl`, and `aarch64-unknown-linux-musl`. When no asset fits, the script builds from source with `cargo build --release --locked`; `scripts/install.sh --from-source` forces that. It does not touch your `PATH`, and it removes the `$BIN_DIR/harness` link (default `~/.local/bin`) and `bin/harness` that versions up to 0.5 installed.
- **Not on your PATH.** Every session the harness starts (contest sessions, the main session, scheduled runs) runs as `env [ACH_SESSION=<id>] PATH=<clone>/bin:<PATH> <command>`, so its agents find `ach`. The omp extension puts `<clone>/bin` first on the `PATH` of the omp sessions that load it. The Orca automation's prompt names the binary's path, and launchd and cron jobs call it by path. Any other agent session, such as Claude Code opened in the clone, follows the skill's rule: when `ach` is not found, call `<clone>/bin/ach`.
- **Which checkout.** `ACH_HOME` when it is set, otherwise the nearest directory above the real binary path (symlinks resolved) that contains `harness/prompts/launch.md`, otherwise the checkout it was built from. It reads `data/`, prompts, agents, and the skill from there. Every command except `doctor`, `--help`, and `--version` refuses to run when that directory has no `harness/prompts/launch.md`, and creates nothing there.

## Running steps by hand

The main session runs all of this when you ask it. From a terminal in the clone:

```bash
bin/ach init                                 # data/ (0700), data/config.json and data/profile.md from the examples (0600), data/logs/; existing files are kept; then the doctor
bin/ach config set workspace tmux            # dotted keys; the whole config is validated before writing
bin/ach config set schedule.cron '0 9,21 * * *'
bin/ach config get                           # the effective config (defaults + file); see Configuration
$EDITOR data/profile.md                      # your profile: eligibility facts and the details submission forms ask for
bin/ach doctor                               # fix every [실패] (failure) line; [경고] (warning) lines are advisory
bin/ach install                              # agents, skill and extension for the configured runner (omp: omp plugin link), plus agent_models
bin/ach collect                              # all sources; or: bin/ach collect --source dacon --pages 1
bin/ach list --state discovered
# Gate: ask the main session (or any omp session) to run ContestGate on an id, or use
# the /contest-gate <id> prompt template in omp. The agent writes gate.json and records
# it with: ach gate <id> --file gate.json
bin/ach show wevity-111674
bin/ach route wevity-111674                  # folder <contests_root>/<Folder>, private repo, workspace, CONTEST.md
bin/ach capacity                             # free slots per level, memory budget, recommended workers
bin/ach launch wevity-111674 --dry-run       # print the command, prompt, and warnings without starting or writing anything
bin/ach launch wevity-111674
bin/ach peek wevity-111674                   # tail of the session's screen or log
```

Use `--no-repo` or `--no-workspace` on `route` to skip the GitHub repository or the workspace. To launch everything the limits allow, use `ach tick --auto-launch`. It routes and launches `gate_pass` and `routed` contests in priority order. Contests that do not fit are reported as `waiting`. Rules for your own environment go in `data/local-rules.md` (see [Local rules](#local-rules)).

## Setup check (`ach doctor`)

`ach doctor [--json]` checks that this checkout, its configuration, the tools it calls, and the scheduled run are ready. It only reads, and it runs even where other commands refuse because no checkout was found. Every problem comes with a fix. `ach init` runs it at the end, and the main session runs it at the start.

- **checkout**: the checkout root and its `harness/prompts/launch.md`.
- **version**: the commit the binary was built from against the checkout's `HEAD`. Commits that change documentation only do not count.
- **os**: macOS or Linux; any other system fails.
- **config**: `data/config.json` parses and passes validation. A missing file is a warning, and the defaults are used.
- **registry and ledger**: `data/registry.json` and `data/submissions.json` load.
- **data permissions**: a warning when group or others can read `data/`.
- **git identity**: `git var GIT_AUTHOR_IDENT` succeeds (failure otherwise).
- **gh**: on `PATH`, and `gh auth status` succeeds (warning).
- **runner**: its binary is on `PATH` (failure), and it is installed for this checkout (for omp, `omp plugin list --json`; warning).
- **workspace**: its binary is on `PATH` (failure). For `orca`, `orca_repo_path` must be an Orca project in `orca repo list` (failure; the fix is `orca repo add --path <path>`).
- **agent_models**: every name in `agent_models` is an agent in `agents/` (failure otherwise; the fix is `ach config unset agent_models.<name>`). When the runner reports the models it applies, a difference from the config is a warning whose fix is `ach install`. An empty map is ok: every agent uses its default role model.
- **contests_root**: exists and is writable.
- **profile**: exists, has mode 0600, and has no example placeholders left (warning).
- **memory**: available memory can be read (warning).
- **schedule**: the scheduled run is installed and in sync on the configured scheduler, and not also installed on another one (warning).

Text output has one `[ok]`, `[경고]` (warning), or `[실패]` (failure) line per check, a fix line under each problem, and a closing count `실패 n건, 경고 m건`. JSON is `{"checks": [{name, level, detail, fix?}]}` with `level` `ok`, `warn`, or `fail`. The exit code is 1 when any check fails; warnings alone leave it at 0.

## omp extension

The repository is an omp extension package. `ach install` with the `omp` runner runs `omp plugin link <repo>`, which loads the extension file and, by omp's conventions, the repository's skills, prompt templates (`commands/`), and agents.

| Name | Kind | What it does |
|---|---|---|
| `/contest` | Command | Board summary: counts by state, contests awaiting submission approval or plan review, hold questions, in-progress deadlines within 7 days, and capacity |
| `/contest <id>` | Command | Summary of one contest |
| `/contest-chat` | Command | How to open the main interface session, and the binary version |
| `/contest-gate <id>` | Prompt template (`commands/contest-gate.md`) | Runs `ContestGate` on one registry entry and summarizes the result |
| `/contest-init` | Prompt template (`commands/contest-init.md`) | Follows [INIT.md](../INIT.md) to set up the harness with you, then reports the `ach doctor` result and what is left to do |

<p>Table 1. Commands registered by the extension</p>

The extension gets all data from `ach ... --json`. It looks for the binary at `ACH_BIN`, then `bin/ach`, `target/release/ach`, and `target/debug/ach` in the checkout, then `ach` on `PATH`. When omp loads it, it puts the checkout's `bin/` first on the omp process's `PATH`, so agents in that session can call `ach`. It does not add the persona.

## Sources

Each source is one module under `src/sources/`.

| Name | Site | Method | Notes |
|---|---|---|---|
| `linkareer` | Linkareer (링커리어) | HTML | Reads the Next.js `__NEXT_DATA__` on the list page, including deadline times. First page only |
| `wevity` | Wevity (위비티) | HTML | The list has no deadline, so only new items open the detail page |
| `thinkcontest` | Thinkgood (씽굿) | API | Internal JSON list API. Reads "open" and "upcoming" separately |
| `allcon` | Allcon (올콘) | API + HTML | Internal JSON list, HTML detail for new items |
| `contestkorea` | Contest Korea (콘테스트코리아) | HTML | List and detail pages. Filters out closed rows |
| `detizen` | Detizen (대티즌) | API | Supabase REST with the site's public publishable key |
| `campuspick` | Campuspick (캠퍼스픽) | API + HTML | Internal list API, detail `__INITIAL_STATE__` for new items |
| `allforyoung` | Allforyoung (요즘것들) | API | Public JSON API. Deadline comes from the detail API |
| `dacon` | DACON (데이콘) | HTML + API | Nuxt data on the list page, detail API for registration and submission deadlines |
| `devpost` | Devpost | API + HTML | Public JSON API, online hackathons only. Detail pages are often blocked by Cloudflare and are not bypassed |
| `kaggle` | Kaggle | API | Internal list API with prize and UTC deadline. Excludes Getting Started and Community competitions |

<p>Table 2. Built-in sources</p>

Two sources were left out on purpose. Campusmon's robots.txt has `Disallow: /`, and AIConnect shut down on 2026-07-31. All deadlines are stored in KST (`+09:00`). When a source only gives a date, the deadline is set to 23:59 KST that day, and the gate checks the exact time against the official notice.

Collection rules:

- A failed detail page does not drop the listing. The list row is kept (with the list's deadline, or none) and gets the note `상세 실패: <error>`, so the gate re-checks it.
- Listings from the same site are never merged by title, because one site with two ids means two contests. A URL match still merges.
- Titles whose remaining number tokens differ (ARC-AGI-2 and ARC-AGI-3, Track 1 and Track 2) are not similar. A number on one side only (such as a prize amount), years, and edition numbers do not split a match.
- Collected ids must match `[a-z0-9-]+`; others are skipped and reported in `errors`. Titles and organizers are kept on one line: control and invisible characters are removed and whitespace is collapsed, in the registry as well.
- HTTP requests time out after 30 s, and requests to one site are at least 1 s apart. Timeouts, I/O, connection, and DNS errors, 429, and 5xx responses are retried after 2 s and then 6 s (three attempts in all). A numeric `Retry-After` of up to 30 s is honored; a longer one means no retry. Other 4xx responses fail at once. Error messages start with the URL.
- A source that panics is reported as `<name>: 수집 중 패닉: <message>`, and the other sources keep going.

## Configuration

Settings are read from `data/config.json` on top of the defaults in [`src/config.rs`](../src/config.rs). Any key you leave out keeps its default. For map-valued keys (`preset_by_level`, `model_by_level`, `max_running_by_level`, `ram_gb_by_level`, `runner_args`, `persona`, `schedule`, `agent_models`), only the entries you list are overridden; `{"max_running_by_level": {"5": 0}}` changes Level 5 alone. Level keys are the strings `"1"` to `"5"`.

`ach config` reads and changes the file without opening it. Keys are dotted paths (`schedule.cron`, `agent_models.ContestJudge`, `model_by_level.3`). `set` reads the value as JSON when it parses (numbers, `true`/`false`, `null`, `"..."`, `[...]`, `{...}`) and as a string otherwise, validates the whole config, and only then writes the file (atomically, mode 0600); an invalid value is refused and the file stays as it was. `unset` removes the key from the file so its default applies again. `get` shows the effective value (defaults plus file), or the whole config without a key.

```bash
ach config path                          # the config file path
ach config get schedule                  # effective value
ach config set max_running_by_level.5 0  # read as the number 0
ach config set github_org my-org         # not JSON, so read as a string
ach config unset max_running_by_level.5  # back to the default
```

The file is validated when it is loaded, and every config error starts with its path. Unknown keys are rejected (`unknown field ...`), also inside `persona` and `schedule`. `runner` must name a compiled-in adapter; `workspace` must be `orca`, `tmux`, `headless`, or `terminal`; `schedule.scheduler` must be `orca`, `launchd`, or `cron`; `schedule.cron` and `schedule.scout_hours` (a cron hour field, 0-23, empty allowed) must parse; by-level keys must be `"1"` to `"5"`; memory values must be finite and 0 or more, `ram_gb_per_worker` above 0, and `collect_pages` 1 or more.

| Key | Default | Description |
|---|---|---|
| `github_org` | `""` | GitHub organization or user where `ach route` creates a private repository per contest via `gh`. Empty means the account `gh` is logged in as |
| `contests_root` | `"~/contests"` | Directory that holds one folder per contest. `dedupe-check` also reads `<contests_root>/*/README.md` and `<contests_root>/.shared/existing_contests.md` |
| `orca_repo_path` | `null` (= `contests_root`) | Git repository registered in Orca. Contest workspaces are created as its worktrees |
| `runner` | `"omp"` | Runner adapter (agent harness) for contest sessions, scheduled runs, and the main session. The core has `omp` and `claude` (Claude Code); see [Adding a runner adapter](../README.md#adding-a-runner-adapter) |
| `workspace` | `"orca"` | Where contest sessions run: `orca`, `tmux`, or `headless` |
| `preset_by_level` | `{"1": null, "2": null, "3": null, "4": null, "5": null}` | Model preset per level, for adapters with presets. For omp, for example `{"3": "ResearchPreset"}` is taken from `modelPresets` in `~/.omp/agent/config.yml` and passed as a per-launch `--config` overlay, so your global omp config is not modified. `null` uses omp's default roles. A named preset must exist in your omp config, otherwise `launch` fails |
| `model_by_level` | `{}` | Model id per level, used when no preset applies |
| `runner_args` | `{"omp": [], "claude": ["--permission-mode", "auto"]}` | Extra arguments added to every command of that adapter, keyed by runner name |
| `max_running` | `6` | Maximum number of contests in progress at once, across all levels. `0` pauses all launches |
| `max_running_by_level` | `{"1": 3, "2": 2, "3": 2, "4": 1, "5": 1}` | Maximum contests in progress per level. `0` means contests at that level stay in `gate_pass` and are never launched |
| `ram_reserve_gb` | `8` | Memory held back for your own apps. The budget is available memory (macOS: `vm_stat` free + inactive + speculative; Linux: `MemAvailable` in `/proc/meminfo`) minus this value |
| `ram_gb_by_level` | `{"1": 2, "2": 3, "3": 4, "4": 6, "5": 8}` | Memory one session at each level is assumed to use. These are estimates, not measurements |
| `ram_gb_per_worker` | `1.5` | Memory per worker inside a session. It caps the recommended worker count |
| `max_workers` | `6` | Upper bound on recommended parallel workers per session |
| `primary_provider` | `"anthropic"` | Model provider whose usage windows drive the token pace calculation |
| `collect_pages` | `3` | Default number of list pages per source for `collect` and `tick` (1 or more) |
| `collect_skip` | `[]` | Source names to turn off, for example `["devpost", "kaggle"]` |
| `auto_launch` | `false` | When `true`, `ach tick` routes and launches passed contests as capacity allows. Leave it off until you trust the gate's decisions |
| `gate_batch` | `16` | Maximum number of `discovered` contests one scheduled run gates in parallel. The actual number depends on the token pace of `primary_provider`: all of it when the mode is maxing, half when steady, 4 when conserve, 0 when throttle |
| `persona.enabled` | `false` | Add the operator persona to the main session. Contest sessions never get it |
| `persona.file` | `"data/persona.md"` | Persona text file (git-ignored). Example: `data/persona.example.md` |
| `persona.call_user` | `""` | How the persona addresses you. Also scanned by the persona guard |
| `persona.guard_terms` | `[]` | Terms that must not appear in deliverables or commit messages: the persona's name, form of address, and speech markers |
| `schedule.cron` | `"0 */6 * * *"` | When the scheduled run starts: a 5-field cron expression (minute hour day month weekday) in the machine's local time. Applied by `ach schedule install` |
| `schedule.scheduler` | `"orca"` | What starts the scheduled run: `orca` (an Orca automation running the runner's Orca agent), `launchd` (a launchd job running `ach autorun`, macOS only), or `cron` (a user crontab entry running `ach autorun`) |
| `schedule.scout_hours` | `"0-5"` | Local hours in which a scheduled run also runs the `ContestScout` web search, written as a cron hour field (`0-5`, `0,12`, `*`). Empty means never. `ach tick` reports the decision as `scout` |
| `agent_models` | `{}` | Model per harness agent, keyed by agent name (`agents/<Name>.md`), in the runner's selector syntax (omp: `provider/model:effort` or `@role`). Applied to the runner by `ach install`; `null` removes a choice the harness applied before. Unknown agent names are refused by `install` and fail `doctor`. See [Per-agent models](../README.md#per-agent-models) |
| `delegation` | `{"enabled": false, "granted_at": "", "note": "", "contest_ids": [], "allow_personalization": false, "video_score_exempt": false, "concept_selection": false}` | Standing delegation of submission approval, off by default. `enabled: true` needs a non-empty `granted_at` and `note`. An empty `contest_ids` covers every contest; otherwise only those ids. `allow_personalization` lets a delegated approval fill existing personal details and an existing signature into separate final copies, `video_score_exempt` waives the score condition for video contests under delegated approval, and `concept_selection` delegates the choice of a creative concept card. Fees, new accounts, new consents, new signatures, and Level 5 plan approval are never delegated. A delegated approval additionally needs tracked official rules and, from Level 2 up unless the video exemption applies, a final score bound to a verified independent assessment (see [Durable workflows](#durable-workflows-080)). Contest sessions cannot change it. Revoke with `ach config set delegation.enabled false`; a delegated approval not yet used then no longer authorizes a submission intent, `submission start`, or `ach profile show`, while records of submissions already started or confirmed stay. See [Approval in the CLI](../README.md#approval-in-the-cli) |
| `new_work_frozen` | `false` | New-work freeze. When `true`, `collect`, `add`, `gate`, `route`, `launch` (even with `--force`), the collect and auto-launch parts of `tick`, and new starts by the queue controller are refused. Checkpoints, reconcile, recovery of contests already in progress, approvals, submission transactions, and cleanup still work. Only you (or the main session on your direct instruction) change it with `ach config set new_work_frozen true\|false`; unfreezing never starts queued contests by itself |

<p>Table 3. Configuration keys</p>

How capacity is computed (`ach capacity`):

- **Token pace.** For each usage window reported by `omp usage --json`, the harness computes pace = share of quota remaining ÷ share of time remaining. In the tightest window, a pace of 1.5 or more allows `max_workers` (maxing), 1 to 1.5 allows half of that (steady), and below 1 scales down (conserve). If 10% or less of the quota remains, the harness allows one worker and starts no new sessions (throttle). Without omp, only the memory and count limits apply.
- **Memory.** New sessions start only if their `ram_gb_by_level` fits within the memory budget. Available memory comes from `vm_stat` on macOS and from `MemAvailable` in `/proc/meminfo` on Linux. No new session starts while memory pressure is `warning` or higher: the macOS memory-pressure level, or on Linux `some avg10` in `/proc/pressure/memory` (10 or more is `warning`, 40 or more `critical`, unknown when the file is absent). When memory cannot be read, the memory limit is skipped (the count and token limits still apply) and `ach capacity --json` reports `ram.basis: "unavailable"`, with `ram.available_gb` and `ram.budget_gb` null and a `ram.warning`. Otherwise `ram.basis` is `"measured"`.
- **Count limits.** `max_running` and `max_running_by_level` apply on top of the token and memory limits. `running` and `plan_review` contests count as in progress.

## CLI reference

Every command accepts `--json`. CLI messages and `BOARD.md` are in Korean. Exit codes: `0` success, `1` error (the reason goes to stderr; this includes refusals by the persona and score gates and by the readiness, delegated-approval, and submit checks), `2` usage error (bad arguments, reported by the argument parser), `3` possible duplicate (`dedupe-check`, and `add` without `--force`), `4` persona traces found (`check`).

| Command | What it does |
|---|---|
| `ach --version` | Prints the version and the commit the binary was built from: `<version> (<12-character sha>)` |
| `ach doctor` | Checks the checkout, config, tools, and scheduled run. Exit 1 if any check fails. See [Setup check](#setup-check-ach-doctor) |
| `ach init` | Creates what a new checkout needs: `data/` (0700; an existing `data/` that other users can open is narrowed to 0700), `data/config.json` and `data/profile.md` from the committed examples (0600), and `data/logs/`. Existing files are kept. Then runs the doctor. JSON is `{created: [...], kept: [...], tightened: [...], checks: [...], next: "<root>/INIT.md"}`. Exit 0 |
| `ach config path` | Prints the config file path (`data/config.json`) |
| `ach config get [KEY]` | Prints the effective value of a dotted key (defaults + file), or the whole config |
| `ach config set <KEY> <VALUE>` | Sets a dotted key. The value is read as JSON when it parses, otherwise as a string. The whole config is validated before the file is written (atomic, 0600); an invalid value is refused and the file is unchanged |
| `ach config unset <KEY>` | Removes a key from the file so its default applies again. A key that is not in the file is an error |
| `ach agents` | Lists the agents in `agents/*.md`: name, description, default model (frontmatter `model`), the `agent_models` choice, and what the runner applies now. JSON is `[{name, description, default_model: [...], configured, applied}]` |
| `ach collect [--source <name>\|all] [--pages N]` | Collects new listings, merges cross-site duplicates, and adds new contests as `discovered`. `--pages` (default `collect_pages`) must be 1 or more |
| `ach add --title T --url U [--org O] [--deadline D] [--source scout\|manual] [--note N] [--force]` | Adds one contest. The URL must be http(s). `--deadline` takes `YYYY-MM-DD` (stored as `YYYY-MM-DDT23:59+09:00`) or `YYYY-MM-DDTHH:MM[:SS]+09:00` (`24:00` is read as 23:59; other offsets are rejected). An id already taken gets `-2`, `-3`, and so on. If the contest looks like a duplicate, prints the candidates instead |
| `ach add --from-file <json> [--source scout\|manual] [--force]` | Adds a JSON object or an array of `{title, url, org?, deadline?, source?, note?, categories?}`. Every item is validated before anything is written; a bad item fails the file with `<path>: [i] ...`. JSON output is `{"added": [...], "duplicates": [{title, url, matches}]}`. Exit 3 if any item was held back as a possible duplicate |
| `ach dedupe-check ["<title>"] [--id ID] [--url U] [--org O] [--deadline D]` | Looks for the same contest in the submission ledger, the registry, and the contest folders. Ledger matches come first as `kind: submitted`. With `--id`, that entry is left out and its title, URL, organizer, and deadline are used (flags override), so the title is optional |
| `ach list [--state S] [--type T]` | Lists contests by deadline |
| `ach show <id>` | Prints one registry entry |
| `ach gate <id> --file gate.json` | Records a gate result and moves the contest to `gate_pass`, `rejected`, or `hold` |
| `ach route <id> [--folder NAME] [--no-repo] [--no-workspace]` | Runs the preflight checks, then creates the folder, repository, workspace, and `CONTEST.md` and moves the contest to `routed`. Running it again after a failure resumes. Refuses a contest already in the submission ledger. See [Routing and launching](#routing-and-launching) |
| `ach launch <id> [--preset NAME] [--dry-run] [--force]` | Starts the configured runner in the configured workspace with the level prompt and moves the contest to `running`. Refuses if the level has no free slot or memory (`--force` overrides). `--preset` is omp only. `--dry-run` writes nothing; it prints the command and warnings for missing runner or workspace binaries, and its JSON has `prompt` (the path a real launch writes), `prompt_text`, and `warnings` |
| `ach peek <id> [--lines N]` | Prints the last N lines (default 40) of a session's screen or log |
| `ach check <id>` | Scans a contest folder and its commit messages for persona traces |
| `ach transition <id> <state> [--note N \| --note-file F] [--by WHO] [--persona-ok REASON] [--score-ok REASON]` | Changes state, enforcing the transition rules, the approval rules, the score gates, and the persona guard. Refuses `plan_review → running` (only `approve`), `→ submitted` (only `submit`), and `submitted`/`followup_pending → running` (only `resume`). `--score-ok` adds a typed override flag to the history entry; only that flag counts, and only for your own approval, never a delegated one. Note text cannot bypass the score gate |
| `ach readiness <id> [--file F] [--personalized]` | Without `--file`, prints the current readiness record. With `--file`, validates the five fields `submission_ready`, `needs_user_input`, `blockers`, `artifacts` (`[{path, sha256}]`, regular files inside the contest folder whose hashes match now; from Level 2 up, within the latest `final` score's `artifacts`), and `followup_required`, plus `destination` (the exact official submission URL or email address; required since 0.8.0 for any new submission intent), and records them for the current work revision. Only in `running` or `awaiting_approval`; a new record invalidates an approval given for an earlier one. `--personalized` records approved final copies after a valid submit approval: the file adds `personalization_of` (the approved `request_token`); each personalized copy in `artifacts` adds `source` (the relative path of the approved original it personalizes), while an unchanged original may be listed with its own path and hash and no `source`; together they must cover every approved original exactly once, and no copy may overwrite an original. `followup_required` and `destination` must equal the approved record's values (to change them, record an ordinary readiness and get a new approval). A delegated approval also needs `delegation.allow_personalization` |
| `ach approval-status <id>` | Prints `{id, revision, request_token, submit_approval_valid, eligible, reasons, readiness, latest_final}` with the persona and delegation policy checks. Since 0.8.0 `reasons` may hold rule codes (`rules_pending_review`, `rules_gate_invalidated`, `rules_plan_invalidated`, `rules_final_invalidated`; `rules_untracked` for delegated approval only), independent-assessment codes for delegated approval (`independent_final_missing`, `independent_final_stale`), and submission codes (`submission_intent_pending`, `submission_dispatching`, `submission_unknown`) |
| `ach approve <id> (--note N \| --note-file F) [--by WHO]` | Records your approval: in `plan_review`, a `plan` approval and the move to `running`; in `awaiting_approval`, a `submit` approval bound to the current work revision, readiness record, and scores (the state stays). Refused while `ACH_SESSION` is set, that is, inside contest sessions |
| `ach approve <id> --delegated --expected-request TOKEN (--note N \| --note-file F)` | A delegated `submit` approval within the configured `delegation`, run by the main session with the `request_token` from `approval-status`. Under the registry lock it re-checks eligibility, the persona guard, the delegation scope, and the token, and records nothing if any fails. Never a plan approval; no `--score-ok` override (video contests only with `video_score_exempt`). Refused inside contest sessions |
| `ach submission intent <id> --file F` | Records a submission intent: `{"method": "form\|email\|site", "destination": "<equal to the approved readiness destination>", "expected_request": "<request_token>"}`. Rechecks the approval, rules, and file hashes and binds the approved `artifacts`, revision, and rules binding to a new `attempt_id` (`a1`, `a2`, …). The same request returns the same attempt; a different request, or any attempt in `dispatching` or `unknown`, is refused |
| `ach submission start <id> --attempt A` | Moves the intent to `dispatching` and saves it before the external send, after checking the approval, token, destination, files, rules, and revision again. A second start always fails (exit 1) and never authorizes another send |
| `ach submission unknown <id> --attempt A --file F` | Records that the outcome of a send is unknown (`{"attempt_id", "reason": "timeout\|response_lost\|crash\|ambiguous_response\|other", "observed_at", "evidence"?}`), only from `dispatching`. Blocks new intents and resends |
| `ach submission resolve <id> --attempt A --file F` | Records positive evidence that nothing was submitted (`{"attempt_id", "verified_not_submitted": true, "source": "portal_listing\|organizer_reply\|mailbox_search", "observed_at", "artifacts"}` with at least one file under `.private/`), from `dispatching` or `unknown`. Only this reopens a new intent; there is no `--force` |
| `ach submission status <id>` | `{tracked, unresolved, latest, attempts, blockers, review_needed}`, without receipt text, destination, or proof contents |
| `ach submit <id> --attempt A --file <proof.json>` | Confirms a dispatched attempt with receipt evidence and moves the contest to `submitted` (the only way there); adds it to the submission ledger in the same write. Proof: `{"attempt_id", "receipt": "<one-line receipt number or reference>", "source": "confirmation_page\|receipt_email\|portal_listing\|organizer_reply", "observed_at", "destination", "submitted_sha256": [...], "artifacts": [{path, sha256}]}`. Accepts `dispatching` or `unknown` attempts, never a bare `intent`. Proof files are copied read-only to `.private/submissions/<A>/`; metadata keeps only the receipt reference and hashes. The same proof again changes nothing but restores a missing ledger record; different proof is refused. If the workflow moved on meanwhile, the receipt and ledger record are kept, the state is left alone, and `review_needed` is reported |
| `ach score <id> --stage plan\|draft\|final --file <judge.json> [--note N \| --note-file F]` | Records a `ContestJudge` score for the current work revision. Criteria must match `gate.json` exactly (round, name, published weight), weights must be above 0, round and name may not repeat, and `total` may differ from the normalized sum by at most 0.5. A `final` score needs `artifacts: [{path, sha256}]` whose hashes match the current files; with `evaluation_id` and `assessment_id` it must equal that independent assessment exactly and stores the binding. Every score stores the current `rules_binding`, and a new `plan` or `final` score clears that stage's rule invalidation. Recording a new `final` after approval invalidates that approval. Without `--stage`, prints the score history |
| `ach submitted` | Lists the submission ledger: date, title, method, receipt |
| `ach submitted add --title T [--url U] [--org O] [--date D] [--method M] [--receipt R] [--folder F] [--note N]` | Records a contest you entered outside the harness |
| `ach submitted import` | One-time import of submitted entries from the registry into the ledger. Existing keys are skipped |
| `ach clean <id> [--dry-run] [--purge] [--force]` | After submission, closes the contest's session and workspace and moves its local folder to the trash (`--purge` deletes it). Refuses unless every cleanup condition holds (`--force` skips the checks, except that an unresolved submission attempt always blocks cleanup; a missing folder is then a problem, not an abort) |
| `ach clean --all [--dry-run]` | Runs `clean` over every `submitted` or `closed` contest that still has its folder. A contest that fails becomes a failed outcome; the others are still cleaned and saved |
| `ach restart <id> [--preset P] [--restore UUID] [--dry-run]` | Reopens a `running`, `awaiting_approval`, or `plan_review` session with the same conversation history, for example to change the preset. Without `--restore`, uses the stored session history or the runner's only one, and refuses rather than discarding history. State and work revision do not change |
| `ach resume <id> [--preset P] [--restore UUID] [--dry-run]` | Resumes real follow-up work on a `submitted` or `followup_pending` contest. Checks capacity under `data/launch.lock` and moves the contest to `running` (a new work revision) only after the session has opened |
| `ach session-close <id> --expected-state S --expected-history N [--dry-run]` | Closes only the session of a finished contest (`submitted`, `followup_pending`, `closed`, `skipped`, `duplicate`) when its state and history length still match. Does not delete the folder. Refused inside contest sessions |
| `ach profile path` | Prints the profile file path (`data/profile.md`) and whether it exists |
| `ach profile eligibility [--json]` | Prints only the `## 참가 자격` section, the eligibility facts the gate needs, without identifying values. JSON is `{path, eligibility}`. This command and `profile show` warn on stderr when the profile is readable by group or others or its eligibility section still has example placeholders |
| `ach profile show --for <id> [--json]` | Prints the whole profile, personal details included. Only for a contest in `awaiting_approval` with a valid submit approval, or one already `submitted`; inside a contest session, only for its own id. JSON is `{path, text}` |
| `ach budget` | Shows per-provider remaining quota, pace, and the recommended worker count |
| `ach capacity` | Shows how many sessions can start now. JSON fields: `slots`, `slots_by_level`, `recommended_workers`, `running`, `running_by_level`, `ram` (`basis`, `available_gb`, `reserve_gb`, `budget_gb`, `pressure`, `warning`), `limits`, `providers` |
| `ach board` | Writes the action-oriented `BOARD.md` from stored records only (no network or backend calls); see [Board](#board) |
| `ach tick [--no-collect] [--auto-launch]` | Runs one control loop: collect, list contests waiting for the gate, optionally route and launch within capacity, then update `BOARD.md`. With `new_work_frozen` it neither collects nor launches. Exits 0 so the scheduled run can read its JSON; the text starts with the error count, and JSON adds `warnings` and `scout` (whether the current local hour is in `schedule.scout_hours`) |
| `ach contract [--json]` | The interface contract of this binary: `{protocol: 2, version: "0.8.0", capabilities: [...]}`. Controllers check `protocol == 2` before any side effect |
| `ach checkpoint <id> [--file F]` | Records a stage checkpoint, or prints the latest one without `--file` |
| `ach reconcile [<id>] [--apply]` | Observes recorded sessions and workspaces. Read-only by default; `--apply` stores the observation only |
| `ach recover <id> --expected-observation TOKEN [--dry-run]` | Reopens a missing session with its saved conversation only when every recovery condition holds |
| `ach rules capture\|review\|acknowledge\|status <id> ...` | Official rule snapshots, structured review, stage re-review, status |
| `ach evaluation prepare\|record\|report\|outcome <id> ...`, `ach evaluation calibration` | Blinded evaluation packets, independent assessments, dispersion reports, official outcomes, observed-sample summary |
| `ach controller <approve\|queue\|briefs\|preset> [args...]` | Runs `python3 -m controllers ...` from the checkout without a shell |
| `ach install [--runner R ...] [--all]` | Installs agents, the skill, and the extension for the configured runner, the given runners, or every compiled-in adapter whose CLI is on `PATH`, and applies `agent_models` to the configured runner. Refuses unknown agent names in `agent_models` |
| `ach chat [--runner R] [--workspace W]` | Opens the main interface session at the repository root |
| `ach autorun` | Runs the scheduled automation once with the configured runner's one-shot command (what the launchd and cron jobs start) |
| `ach schedule install [--scheduler orca\|launchd\|cron]` | Creates or updates the scheduled run with `schedule.cron` on the scheduler (default `schedule.scheduler`). Warns if any other scheduler also has the job |
| `ach schedule remove [--scheduler orca\|launchd\|cron]` | Removes the scheduled run from that scheduler |
| `ach schedule status` | Shows the configured schedule and what Orca, launchd, and cron actually have: cron, enabled or loaded, next and last run, last exit code and log, and whether each matches the config (`in_sync`, null for schedulers other than the configured one). Without `--json`, prints a short summary |

<p>Table 4. Commands</p>

## Scheduled runs

Each scheduled run follows [harness/prompts/automation.md](../harness/prompts/automation.md):

1. `ach tick` collects listings and updates the board (with `new_work_frozen`, it skips collection and launching, and steps 2 to 5 are skipped). Then `ach reconcile --json` observes every recorded session read-only; contests whose runtime is `missing`, `unknown`, or `ambiguous` are only reported, never relaunched by the scheduled run.
2. A first-pass filter moves clear misses among `discovered` posts to `skipped`. Anything ambiguous is left for the gate. The filter catches three kinds of post:
   - Posts that are not contests, such as supporter programs, reporter programs, courses, product-testing groups, recruiting, event notices, and comment events.
   - Posts whose title or note alone shows you are not eligible, judged against your profile's eligibility facts (`ach profile eligibility --json`). Examples are contests only for school students or for residents, employees, or members of a specific group, contests that require on-site attendance, filming, or an entry fee, and overseas contests that exclude participants from Korea.
   - Posts whose deadline has passed.
3. The run decides how many contests to gate. It reads the `mode` of the `limits.basis` provider in `ach capacity --json` and gates `gate_batch` contests when the mode is maxing, half of that when steady, 4 when conserve, and none when throttle. It picks the remaining `discovered` contests nearest the deadline and runs one `ContestGate` agent per contest in parallel.
4. Only when step 1 reported `scout: true` (the hour is in `schedule.scout_hours`, by default 00:00 to 05:59), `ContestScout` runs a web search.
5. `ach tick --no-collect` runs again. If `auto_launch` is `true`, contests that passed are routed and launched within the global limit, the per-level limits, and the memory budget. The rest are reported as `waiting`.
6. `ach clean --all` cleans up submitted contests whose work is safely pushed (see [Cleanup after submission](#cleanup-after-submission)). Contests with an unresolved submission attempt are never cleaned.
7. The session reports in five lines or fewer: newly passed contests (deadline, level, priority), contests on hold with their questions, contests waiting for plan review (`plan_review`), what was launched, what was cleaned up or blocked, and errors.

### When it runs

Set the time in `data/config.json` and apply it:

```json
"schedule": {"cron": "0 9,21 * * *", "scheduler": "orca", "scout_hours": "21"}
```

```bash
ach schedule install     # create or update the scheduled run with schedule.cron
ach schedule status      # what Orca, launchd, and cron have, and whether it matches the config
ach schedule remove      # remove it (add --scheduler orca|launchd|cron for another one)
```

`cron` has five fields (minute, hour, day, month, weekday) in the machine's local time. Each field takes `*`, a number, a range `a-b`, a step `*/n`, `a/n` or `a-b/n`, or a comma list of those; weekday is 0-7 with 0 and 7 both Sunday. Names (`MON`, `JAN`) and macros (`@daily`) are not accepted, so every scheduler reads the same expression. As in cron, when both day and weekday are restricted, a day that matches either one runs. Run `schedule install` again after changing `cron` (with Orca, also after changing `runner`); until then `status` shows `in_sync: false`.

**Orca automation** (`scheduler: "orca"`, the default). The harness looks for the Orca automation whose prompt names this checkout's `harness/prompts/automation.md` and updates its trigger, agent (the runner's Orca agent, `omp`), and enabled flag. If there is none, it creates an Orca workspace named `Autocontest-Harness` in `orca_repo_path` and an automation of the same name in your local time zone. `remove` deletes the automation and leaves the workspace. Run it now with `orca automations run <id>`.

**launchd** (`scheduler: "launchd"`, macOS only; it refuses elsewhere). `ach autorun` runs the prompt once with the configured runner's one-shot command from the repository root and writes the output to `data/logs/autorun-<timestamp>.log`. `schedule install` writes `~/Library/LaunchAgents/<label>.plist`, with `StartCalendarInterval` entries converted from the cron, and loads it. The label is `io.github.fmsongx2.autocontest-harness.<8 hex>`, where the suffix is a hash of the checkout path, so two clones do not collide. A plist with the old fixed label is reported under `launchd.legacy` by `status` and retired by `install` if it belongs to this checkout. The job pins `ACH_HOME` to the checkout and `PATH` to the value at install time, so run `schedule install` again after moving tools. launchd cannot express a starred day step combined with a weekday (`0 8 */2 * 1`), and such an expression is refused.

**cron** (`scheduler: "cron"`, macOS and Linux). `schedule install --scheduler cron` writes this checkout's block into your user crontab between `# BEGIN autocontest-ach <root>` and `# END autocontest-ach <root>`, replacing the previous block and leaving every other line untouched. The job is `<cron> cd <root> && ACH_HOME=<root> PATH=<install-time PATH> <harness binary> autorun >> <root>/data/logs/cron.log 2>&1`. `remove` deletes only that block. `status` reports it as `crontab: {available, installed, cron, in_sync, next_run, log}`.

Without `--json`, `schedule status` prints a short summary: a settings line, then for Orca, launchd, and cron whether the job is installed, whether it matches the config, and the next run.

Keep one scheduler. Two would start two runs that gate and launch the same contests; `schedule install` warns when any other scheduler also has the job, and `ach doctor` reports it.

## Routing and launching

`ach route <id>` turns a `gate_pass` contest into a folder `<contests_root>/<Folder>` with `CONTEST.md`, a private GitHub repository, and a workspace, then moves it to `routed`.

- **Preflight.** Before writing anything, `route` checks the workspace backend (and that `orca_repo_path` exists) unless `--no-workspace` is given, `gh` on `PATH` and `gh auth status` unless `--no-repo` is given, and the git commit identity. Each error names the fix.
- **Resumable.** `.harness/contest.json` is written right after `.harness/` is created, so running `route` again after any failure resumes. A failed route leaves the entry as it was (`gate_pass`, no folder).
- **`.gitignore`.** The contest folder's `.gitignore` gets `.DS_Store`, `.venv/`, `__pycache__/`, `.private/`, `.harness/*-preset.*`, `.harness/launch-prompt.md`, `.harness/session.log`, and `.harness/contest.json`. Missing lines are appended to an existing `.gitignore`. `contest.json` is written before the first commit, so the working tree is clean after `route`.
- **Existing repository.** A GitHub repository that already has the folder name must be private and either empty or this contest's (its `.harness/contest.json`, or the `| 하네스 id | <id> |` row of its `CONTEST.md`). A remote `main` with a different history is refused.
- **GitHub Actions off.** After creating a repository, `route` disables GitHub Actions on it (`gh api -X PUT repos/<owner>/<repo>/actions/permissions -F enabled=false`). A failure there is a warning; route JSON lists warnings under `warnings`.
- The title written to `existing_contests.md` and to the commit message is kept on one line.

`ach launch <id>` starts the configured runner in the configured workspace with the level prompt. Every contest session runs as `env ACH_SESSION=<id> PATH=<clone>/bin:… <command>` (see [Approval in the CLI](../README.md#approval-in-the-cli)). `data/launch.lock` serializes the capacity check and the launch, so two launches cannot both pass the limits. `--dry-run` writes nothing.

`route`, `launch`, and `tick` do their network and session work on a copy of the entry outside the exclusive registry lock, and store the result only if the entry's state and history did not change in the meantime. Concurrent edits, such as a deadline update from `collect` or a new score, are kept. If `route` succeeds and the launch after it fails, the entry stays `routed`.

A `headless` session is recorded as `pid:<pid>/<start time>:<log>`. Closing it signals only its process group, and only while that pid still has the recorded start time, so a reused pid is left alone.

## Durable workflows (0.8.0)

The normative text, with every field and code, is [CONTRACT.md](CONTRACT.md) sections 17 to 22 (Korean). This section is the English summary with working examples. Every input goes through a JSON file written with `write` (contest folder `.private/`, or a `mktemp -d` folder); only ids, `request_token`, `attempt_id`, and observation tokens appear on command lines. `ach` never interprets file contents as shell commands.

### Checkpoints, reconcile, and recovery

The registry `state` is the workflow state; whether a session is actually alive is observed separately, and an unobserved session is never reported as healthy.

```json
{"stage": "draft", "completed_artifacts": [{"path": "drafts/proposal.md", "sha256": "<64 hex>"}], "next_action": "Strengthen the evidence table for criterion 3, then run a draft score", "blocked_reason": null, "needs_user_input": false, "next_action_at": "2026-10-26T09:00+09:00"}
```

- `ach checkpoint <id> --file .private/checkpoint.json` (in `running`, `awaiting_approval`, or `plan_review`) records it for the current revision after rehashing every file. `stage` is `research`, `plan`, `draft`, `review`, or `submission`; unknown keys are refused; `next_action` (up to 1000 characters) is never executed and never copied to the board. `ach checkpoint <id>` prints the latest one.
- `ach reconcile [<id>] [--apply]` returns `{entries: [{id, workflow_state, runtime_state, observed_at, observation_token, checkpoint_valid, checkpoint_seq, artifact_integrity, approval_integrity, blocked_reasons, next_action, next_action_at, needs_user_input, recoverable}], changed, conflicts, applied}`. `runtime_state` is `running`, `missing`, `ambiguous`, `unknown`, `not_started`, or `stopped`; `unknown` is not `missing`. It never launches, kills, or adopts an unrecorded terminal. `--apply` stores the observation under a compare-and-swap on the entry's state, history, and session, and never completes work or rewrites approvals.
- `ach recover <id> --expected-observation <token> [--dry-run]` reopens the saved conversation only when the state is `running` or `awaiting_approval` (never `plan_review`), the token still matches, the runtime is confirmed `missing`, the current-revision checkpoint is valid, no submission attempt is unresolved, nothing is blocked or waiting for you, `next_action_at` is due, exactly one saved conversation is known, and capacity (counted without this contest) allows it. The resumed prompt quotes the checkpoint. Refusals are stable codes such as `observation_changed`, `runtime_unknown`, `checkpoint_stale_revision`, `submission_unresolved`, `saved_conversation_ambiguous`, `next_action_not_due`, `human_plan_review`, and `no_capacity`. Run `--dry-run` first; the revision does not change.

### Official rule evidence

```json
{"sources": [{"id": "notice", "url": "https://contest.example.org/notice/123"}, {"id": "guide", "url": "https://contest.example.org/files/guide.pdf", "path": ".private/rules/inbox/guide.pdf"}]}
```

- `ach rules capture <id> --file F` stores the actual bytes as immutable content-addressed files (`.private/rules/sha256/<hex>`, or `data/rules/<id>/sha256/<hex>` before routing). With `path` it copies your local file; without it, a bounded GET without cookies or login fetches the URL. A changed hash becomes a pending review (`changed_pending`); an identical capture is `unchanged` and invalidates nothing.
- `ach rules review <id> --file F` accepts the pending `version` against `base_version`, with typed rules: `{"category": "eligibility|deadline|ai_policy|forms|criteria|submission", "key"?, "value", "source": {"id", "sha256", "locator"}}`. A missing locator or hash is refused; `"ambiguous": true` or `"decision": "unresolved"` keeps it pending. The CLI compares values: eligibility or AI policy changes invalidate gate, plan, and final; criteria invalidate plan and final; forms invalidate final; deadline and submission (destination or method) changes invalidate only submission readiness and approval. Identical values (a banner-only edit) are accepted without any invalidation.
- `ach rules acknowledge <id> --stage gate|plan|final --file F` (`{"rules_version", "epoch", "decision": "pass", "reviewer", "rationale", "evidence": [{path, sha256}]}`) clears one stage after an explicit re-review; a new `plan` or `final` score also clears its stage. `ach rules status <id>` shows `tracked`, `pending`, `invalidated`, and `binding`.
- Approval codes: `rules_pending_review` and `rules_*_invalidated` block every approval; `rules_untracked` blocks only delegated approval. Upgrading never fabricates rule evidence for existing contests.

### Independent final assessment and outcomes

Plan, draft, and improvement scoring may see the working context. The final score that a delegated approval relies on comes from a blinded packet instead:

1. Make sanitized copies of the deliverables (and rubric) with identifying text removed, then `ach evaluation prepare <id> --file F` with `{"purpose": "independent_final", "stage": "final", "candidates": [{"artifacts": [{"source": {path, sha256}, "sanitized": {path, sha256}}]}], "rubric"?, "references"?, "attestation": {"by", "identity_removed": true, "statement"}}`. It refuses `.private/`, `.harness/`, `JUDGE.md`, and profile files, copies only the sanitized files into `.harness/evaluation/<packet_id>/packet/` under neutral labels, and binds the revision, rules binding, and source hashes privately. The CLI blinds metadata only; removing identity inside documents is your responsibility, which the attestation records.
2. A fresh `ContestJudge` run reads only that packet. Record it with `ach evaluation record <id> --file F` (`{"packet_id", "evaluator": {"id", "model", "run_id"}, "mode": "independent", "candidates": [{"label", "total", "target", "verdict", "scores", "artifacts": [{"file", "sha256"}]}]}`).
3. Write the same total, target, verdict, and criteria into `.harness/judge/final-<n>.json` with `evaluation_id` and `assessment_id`, and run `ach score <id> --stage final`. Any mismatch is refused; `"independent": true` alone counts for nothing.

`ach evaluation report <id>` groups assessments of the same packet and reports `n`, mean, sample standard deviation, min, max, and verdict disagreement; with `n < 2` dispersion is unavailable, not zero. After results are announced, `ach evaluation outcome <id> --file F` records `{"result": "won|finalist|lost|disqualified|withdrawn", "source_url", "announced_at", "evidence": {path, sha256}, "official_score"?, ...}`, and `ach evaluation calibration` shows observed counts and score-versus-outcome comparisons labelled `inadequate_data` when samples are few. Nothing produces win probabilities.

### Submission transactions

The harness does not automate arbitrary websites: the approved session performs the form or email submission, and the CLI records durable intent and evidence around it. An unknown outcome is not a failure and is never retried automatically.

1. Readiness includes `"destination": "<official URL or email>"`; the approval binds it.
2. `ach submission intent <id> --file .private/submission-intent.json` with `{"method": "form", "destination": "<same>", "expected_request": "<request_token>"}` returns `attempt_id` `a1`.
3. `ach submission start <id> --attempt a1` saves `dispatching` before anything is sent.
4. Submit the approved files to that destination only.
5. Confirm with `ach submit <id> --attempt a1 --file .private/submission-proof.json` (receipt evidence saved under `.private/`). On a timeout, lost response, or crash, run `ach submission unknown <id> --attempt a1 --file ...` instead and do not resend. Later, confirm with a receipt found in the portal or mailbox, or record `ach submission resolve` with positive evidence that nothing arrived; only that reopens a new intent.

Attempt states: `intent`, `dispatching`, `unknown`, `verified_not_submitted`, `confirmed`, `expired` (an unstarted intent whose approval went stale; nothing was sent). `ach submission status <id>` never prints receipt text or the destination. Unresolved attempts block new intents, recovery, and cleanup.

### Managed controllers

Long-running automation lives in the `controllers/` package; `data/` holds only state and logs.

```bash
ach controller approve --once --dry-run     # delegated-approval loop, one dry cycle
ach controller queue                        # queue loop; prints QUEUE_READY
ach controller briefs refresh --dry-run --id <id>
ach controller preset --pick
```

- `approve [--once] [--dry-run] [--interval 120]`: `ach reconcile --apply` (observe only), `approval-status`, `approve --delegated --expected-request`, `session-close` and `clean` for finished contests. Ready line `AUTOAPPROVE_READY`.
- `queue [--once] [--dry-run] [--interval 60]`: reloads `data/queue-master/queue.json`, `preset.json`, and `new_work_frozen` every cycle and launches only `routed` contests listed in `queue.json` when not frozen. It never recovers sessions. Ready line `QUEUE_READY`.
- `briefs create <id>...` / `briefs refresh [--dry-run] [--adopt-legacy] (--id ... | --file ...)` maintain `.private/controller-brief.md` (exit 3 means review needed). Operator-specific text goes in `data/queue-master/brief-overrides.json`, never in the public package.
- `preset <Preset> (<id>... | --all | --video) [--resume] [--dry-run]`, or `preset --pick` (`"non_video": "auto"` in `preset.json` needs `auto_providers` and `auto_default`).
- Every subcommand checks `ach contract --json` for `protocol: 2` before any write (exit 2 otherwise). `approve` and `queue` hold a non-blocking lock (`data/controller-approve.lock`, `data/controller-queue.lock`) for their lifetime; a second instance exits 75. A failed `--once` cycle exits 1. `--dry-run` takes no lock and writes nothing. `ach controller` passes its own path as `ACH_BIN`.
- Alerts are written to `data/auto-approve/events.jsonl` and `data/queue-master/events.jsonl` only when a fingerprint (code, sorted reason codes, deadline band `none|invalid|expired|lt24h|lt72h|lt7d|gte7d`) is new, changes, or resolves; state persists in `alerts.json` (0600) so restarts do not repeat a warning.

### Board

`BOARD.md` starts with `## 사용자 확인 필요 (N)` (`| id | 상태 | 이유 | 다음 조치 | 기한 |`), the contests that need you, listed with codes only. The active sections (`awaiting_approval`, `plan_review`, `running`) have `| 마감 | 공모전 | 유형·레벨 | 폴더 | id | 런타임 | 다음 조치 | 막힘 | 근거 시각 |`; the runtime shows the last observation or `unobserved` when there is none or it is stale. The board reads stored records only and never copies private blocker prose, checkpoint text, or receipt text.

## Submission ledger

`data/submissions.json` (git-ignored) records every contest you have already entered, so the harness never assigns or submits the same contest again. Each entry keeps the registry id (or a `manual-<slug>` key), title, organizer, URL and alternative URLs, folder, submission time in KST, method, receipt, files, a note, and its source (`harness`, `manual`, or `import`).

- `ach submit <id> --attempt <A> --file <proof>` writes the ledger inside the same registry write, so a ledger failure leaves the registry unchanged. The key is the registry id; an earlier record with the same key (an earlier round or a manual record) keeps its fields and gets the new round appended under `extra.rounds`. Only the receipt reference and hashes go into the ledger, never the proof text.
- `ach submitted add --title T ...` records a contest you entered outside the harness. It is an archival import only: it never starts, confirms, or resolves a harness submission attempt. An existing manual record is replaced only by one with the same title and date; otherwise the new key gets `-2`, `-3`, and so on. A title without letters or digits gets a hash key.
- `ach submitted import` copies past submissions from the registry once: entries with submissions or in state `submitted`, and `closed` entries whose title or history note mentions a submission (제출).
- `ach dedupe-check` matches the ledger by URL key, alternative URLs, or a similar title, and lists those matches first as `{"kind": "submitted", ...}`. A title match requires the same year and edition number (제N회, N기, Nth) when both titles have one; URL matches do not. `ContestGate`, `ContestScout`, and the main session mark such a contest `duplicate` (note "already submitted: <title, date>") instead of gating it.
- `ach route` refuses a contest that is already in the ledger.

## Cleanup after submission

Once a contest is submitted, the session decides cleanup from `followup_required` in its readiness record. When it is `false`, run `ach clean <id>`; scheduled cleanup uses `ach clean --all`. When it is `true`, because only a future final round or presentation remains, record what is left and move to `followup_pending`. This requires an existing submission, preserves the folder and ledger, releases the session slot, and prevents cleanup. Return to `running` only with `ach resume <id>`, which checks capacity, when actual follow-up work resumes. Automatic cleanup from the main session only cleans contests recorded with `followup_required: false`; contests with `true`, or without a structured record (from before 0.7.0), are cleaned only when you ask. Close a finished contest's leftover session with `ach session-close`.

`clean` removes nothing unless all five conditions hold; otherwise it lists the problems. `--force` skips conditions 1 to 4, never 5; a missing folder is then reported as a problem for that contest instead of stopping the run.

1. The state is `submitted` or `closed`.
2. The contest is in the submission ledger.
3. The folder is a git repository with an upstream, a clean working tree (no changes and no untracked files that are not ignored), and no unpushed commits (checked after `git fetch`).
4. Every file listed in the registry's `submissions[].files` exists in git or in the folder. Signed copies and other files kept out of git on purpose are fine; they are archived in the next step.
5. No submission attempt is unresolved (`intent`, `dispatching`, or `unknown`).

Then it:

1. Closes the session (Orca terminal, tmux window, or headless process) and removes the Orca workspace if there is one. A failure here is reported as a warning and cleanup continues.
2. Copies what git does not hold and cannot be rebuilt into `data/archive/<id>/` (git-ignored, mode 0700): the submission files that are not tracked, and everything under the contest folder's `.private/` (receipts, form edit links, copies with personal details). Paths with `..`, absolute paths, and symlinks that lead outside the folder are refused. Other ignored files (renders, caches, virtual environments) are only counted and go with the folder.
3. Moves the folder to the trash: `~/.Trash/<name>-<time>` on macOS; on Linux the XDG trash `$XDG_DATA_HOME/Trash/files` (default `~/.local/share/Trash/files`, with a `.trashinfo` entry) when it exists; otherwise `data/trash/`. `--purge` deletes it instead.
4. Sets `session` to null and records `folder_removed_at` in the registry, with a history note in the same state. The state does not change.
5. Records the remote `repo` URL, `folder_removed_at` and the `archive` path in the ledger entry, so the folder can be restored with `git clone`.
6. Keeps the line in `<contests_root>/.shared/existing_contests.md`, which dedupe still uses.

`--dry-run` prints the checks and the planned actions only. JSON output is `{id, ok, problems, actions, trash, archive}` (an array with `--all`).

`git fetch` and the checks run under a read lock; each contest is then re-checked and cleaned under the write lock. The ledger is written before the folder moves and restored if the move fails. With `--all`, a contest that fails becomes a failed outcome and the others are still cleaned and saved.

## User profile

`data/profile.md` (git-ignored) is a Markdown file with your details. `ach init` copies it from the committed `data/profile.example.md`, which has empty fields only, with mode 0600; keep it at 0600. Harness commands only read it and never change it. You fill it in yourself, or, during setup ([INIT.md](../INIT.md) step 4), you dictate the values to your agent:

- Writing it yourself is recommended, especially for the personal details section.
- If you dictate, the agent writes exactly the values you give into the matching section. For the personal details section it first warns you that dictated values pass through the model provider and the session transcript and recommends editing the file directly. It runs `chmod 600 data/profile.md`, does not repeat the values back to you, and does not copy them anywhere else (other files, commits, logs, notes). Outside that step agents do not edit the file.
- Files such as a signature, seal, or certificate are put in `data/profile/` by you; the profile only holds their paths.

The file has two sections:

- `## 참가 자격` (or `## Eligibility`): the facts the gate checks, such as participation (individual or team), student status, school, department, level, year, birth year (for age limits, instead of a birth date), region of residence, nationality, whether you have proof of enrollment, strengths, and conditions you avoid. No identifying values go here.
- `## 제출용 개인정보` (or `## Submission details`): the details submission forms ask for: name (Korean, English, hanja), birth date, phone, email, the official name of your school and department, paths to signature, seal, and enrollment certificate files (for example under `data/profile/`, git-ignored), and your Hancom Docs account. It is read only after you approve a submission.

- `ContestGate` reads only `ach profile eligibility --json`, the `## 참가 자격` section.
- Identifying values are read only at an approved submission step, with `ach profile show --for <id>` (see [Approval in the CLI](../README.md#approval-in-the-cli)).
- If the file or its `## 참가 자격` section is missing, the commands fail with a message that points to `data/profile.example.md`.

## Local rules

Rules that only make sense on your machine, such as your own helper scripts, compute servers, document tools, or folder conventions, go in `data/local-rules.md`. It is git-ignored, so it never reaches the public repository. Start from the committed example `data/local-rules.example.md`. When the file exists, agents and the skill read it first and follow it.

## Extending: adding a source

1. Create `src/sources/<name>.rs` that implements the module contract in the header comment of [`src/sources/mod.rs`](../src/sources/mod.rs):
   - `NAME`: the source name and registry id prefix, lowercase ASCII with no hyphens.
   - `collect(pages, known_ids) -> Result<Vec<Item>>`: every contest on the first `pages` list pages. Ids already in `known_ids` are returned from list data only, without opening detail pages. An empty result is treated as a sign that the site structure changed or nothing is open.
   - `url_key(url) -> Option<String>`: `Some("<NAME>:<unique id>")` for this source's detail URLs and `None` for any other URL. Dedupe uses it.
   - Each `Item` has `entry_id` (`"<NAME>-<unique id>"`), `title`, `url`, `organizer`, `deadline` (KST `YYYY-MM-DDTHH:MM+09:00` or `None`), and `categories`, plus optional `note` and `alt_url` (the official site).
2. Declare the module in `src/sources/mod.rs` and add it to `SOURCES`. `--source` picks it up automatically.
3. Keep parsing separate from fetching. Shared HTTP, text, and date helpers are in `src/sources/common.rs` and `src/http.rs`. Requests to the same site are at least one second apart.
4. Add `tests/source_<name>.rs`. The tests parse real responses saved under `tests/fixtures/<name>_*.html|json` and never touch the network. Cover the list parser, the deadline conversion to KST, and `url_key`.
5. Respect `robots.txt` and the site's terms. Do not bypass blocks. The Devpost module, for example, falls back to list data when Cloudflare blocks a detail page.

Run `scripts/check.sh` before you open a pull request (see [CONTRIBUTING.md](../CONTRIBUTING.md)).

## Project layout

```text
Contest-Harness/
├── Cargo.toml                  Rust crate (binary `ach`)
├── src/
│   ├── main.rs                 CLI entry point
│   ├── config.rs, paths.rs     settings and file locations
│   ├── registry.rs, board.rs   registry, state machine, BOARD.md
│   ├── store.rs                versioned, atomic JSON storage
│   ├── collect.rs, dedupe.rs   collection and duplicate matching
│   ├── route.rs, launch.rs     folders, repositories, sessions, main interface
│   ├── budget.rs               token pace, memory, capacity
│   ├── guard.rs                persona guard
│   ├── doctor.rs               ach doctor checks
│   ├── process.rs              external commands with timeouts and process groups
│   ├── install.rs, schedule.rs adapter installs, agent_models; cron, Orca automation, launchd, crontab, autorun
│   ├── init.rs                 ach init
│   ├── runners/                runner adapter interface (mod.rs) and the omp adapter
│   ├── workspaces/             orca, tmux, headless, terminal
│   ├── runtime.rs, rules.rs    checkpoints, reconcile, recover; official rule snapshots and review
│   ├── evaluation.rs           blinded packets, independent assessments, outcomes
│   ├── submission.rs           submission transactions (intent, start, unknown, resolve, confirm)
│   └── sources/                one module per listing site
├── controllers/                managed controllers (python3 -m controllers approve|queue|briefs|preset)
├── scripts/                    install.sh, check.sh (required local checks), release.sh
├── bin/ach                     the harness core, installed by scripts/install.sh (git-ignored, not on your PATH)
├── package.json                omp extension package manifest
├── extensions/contest-harness.ts  /contest commands for omp
├── commands/                   /contest-gate and /contest-init prompt templates
├── agents/                     agents (ContestScout, ContestGate, ContestHQ, DomainResearcher,
│                               OpenSourceScout, ContestJudge, ContestEditor, ReferenceScout,
│                               UxReviewer)
├── skills/contest-harness/     lifecycle skill and per-type playbooks
├── harness/
│   ├── prompts/                launch.md, level_1.md … level_5.md, automation.md, main.md
│   └── templates/              SPEC, PLAN, TASKS, JUDGE, RUBRIC, SUBMISSION, LEDGER
├── data/
│   ├── config.example.json     settings example (ach init copies it to config.json)
│   ├── config.json             your settings (git-ignored)
│   ├── profile.example.md      user profile example (ach init copies it to profile.md)
│   ├── profile.md              your profile (git-ignored)
│   ├── profile/                signature, seal, and proof files (git-ignored)
│   ├── logs/                   scheduled-run logs (git-ignored)
│   ├── local-rules.example.md  local rules example
│   ├── local-rules.md          your local rules (git-ignored)
│   ├── persona.example.md      optional operator persona example
│   ├── persona.md              your persona (git-ignored)
│   ├── submissions.json        submission ledger (git-ignored)
│   └── registry.json           contest registry (git-ignored)
├── INIT.md                     setup procedure an agent follows
├── docs/CONTRACT.md            commands, states, file formats, backends
├── docs/reference*.md          this reference (English and Korean)
├── docs/diagrams/              README figures: HTML sources (diagram-design) and the exported SVGs
├── CONTRIBUTING.md             adding a runner adapter or a source
├── SECURITY.md                 threat model, vulnerability reporting
├── CHANGELOG.md                release notes
├── *_ko.md                     Korean versions of README, CONTRIBUTING, SECURITY, CHANGELOG
├── references/REFERENCES.md    reference projects (clones are git-ignored)
├── tests/                      integration tests (e2e.rs runs the whole lifecycle offline) and saved source fixtures
└── BOARD.md                    generated board (git-ignored)
```
