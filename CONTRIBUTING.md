**English** · [한국어](CONTRIBUTING_ko.md)

# Contributing

Pull requests are welcome. Two kinds of extension have a fixed shape: a **runner adapter**, which lets the harness drive another agent harness, and a **source**, which collects listings from another site. This file covers both. The interface contract is [docs/CONTRACT.md](docs/CONTRACT.md); section 10 has the runner interface.

## Ground rules

- Run `scripts/check.sh` before you open the pull request. It runs `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, and `cargo audit` (skipped with a note when `cargo-audit` is not installed). Pull requests and releases must pass it. There is no CI, so paste the output into the pull request.
- Never commit personal data, operator-specific paths, or anything from `data/` other than the `*.example.*` files.
- Agent prompts, the skill, playbooks, and templates are written in Korean in a report style. Code comments and this file are in English.
- Only the CLI writes the registry. Agents and extensions call `ach`.

## Adding a runner adapter

The core ships two adapters: `omp`, the one that has run contests end to end, and `claude` (Claude Code), which Claude Code wrote from this guide and which has run the scheduled job (collection and a gate decision by the `ContestGate` subagent) but no interactive contest session. An adapter for another harness (Codex, pi, or your own) is a Rust module that implements the `Runner` trait in [src/runners/mod.rs](src/runners/mod.rs).

Earlier versions carried unverified adapters for pi, Claude Code, Codex, and dsh. Claude Code has since been rewritten in the core and the rest were removed, but their code is a reasonable starting point:

```bash
git show bf05366:src/runners/codex.rs    # also pi.rs, dsh.rs
git show bf05366:tests/backends.rs       # their command-line and install tests
```

### 1. Implement `Runner`

Create `src/runners/<name>.rs` with a unit struct and `impl Runner for <Struct>`. [src/runners/omp.rs](src/runners/omp.rs) is the reference implementation.

| Method | Default | What it must do |
|---|---|---|
| `name()` | none | The config value `runner` that selects the adapter. Lowercase and unique |
| `binary()` | none | The executable looked up on `PATH`. Every command line starts with it |
| `available()` | `binary()` is on `PATH` | Override only if the harness needs more than the binary |
| `subagents()` | none | `true` if the harness can run the harness agents (`agents/*.md`) as subagents through a task or agent tool. With `false`, `ach launch` tells the session to run those steps one after another itself |
| `presets()` | `false` | `true` if the harness has named model presets (`preset_by_level`, `ach launch --preset`) |
| `preset_file(name, dir)` | error | Write the per-run file for preset `name` under `dir` (the contest's `.harness/`), named `<runner>-preset.<ext>`, and return its path. It reaches the command as `Spec::preset_file` |
| `orca_agent()` | `None` | The agent id Orca uses for this harness (`orca automations create --provider`). Needed for `schedule.scheduler = "orca"` |
| `interactive(spec)` | none | A command for a session with a TTY that starts by reading the instructions |
| `oneshot(spec)` | none | A command that runs the instructions once without a TTY and exits. The `headless` workspace and `ach autorun` use it |
| `install(root)` | none | Install the agents, the skill, and, where the harness has one, an extension package from the checkout at `root`. Idempotent. One human-readable line per action |
| `installed(root)` | `None` | Whether the harness agents, skill, and extension are installed for the checkout at `root`: `Some(true)` or `Some(false)`, or `None` when the adapter cannot tell. `ach doctor` reports it. The omp adapter checks that `omp plugin list --json` has an enabled `contest-harness` package that resolves to `root` |
| `apply_agent_models(models)` | refuses any `Some`; all `None` is a no-op | Apply `agent_models` from the config (agent name to `Some(selector)` in this harness's model syntax, or `None` to remove a choice the harness applied before) where the harness reads per-agent models. `ach install` calls it for the configured runner only. Touch only the agents named in `models`, leave every other setting alone, and return one human-readable line per action. Idempotent. The omp adapter merges the names into omp's global `task.agentModelOverrides` (`omp config get`, then `omp config set`) |
| `agent_models()` | `None` | The per-agent models the harness applies now (agent name to selector), or `None` when the adapter cannot tell. `ach agents` shows it and `ach doctor` compares it with `agent_models` |

<p>Table 1. The <code>Runner</code> trait</p>

`Spec` is the input to both command builders:

| Field | Meaning | How to pass it |
|---|---|---|
| `cwd` | The contest folder or the repository root | Workspaces already run `cd <cwd> && <command>`. Pass it as well if the harness has a working-directory flag |
| `prompt_file` | The instructions the agent reads before anything else | As a file reference if the harness supports one (`claude` passes an `@<path>` mention), otherwise inline with `runners::prompt_text`. Under `ach launch --dry-run` the file may not exist |
| `prompt` | The same instructions in memory, when the caller has them (`ach launch`) | Do not read it directly: `runners::prompt_text` returns it and reads `prompt_file` only when it is `None` |
| `first_message` | The first user message, telling the agent to follow the instructions | May be left out when the instructions themselves are the first message |
| `system_append` | The persona and main role, for the main session (`ach chat`) only | As a system-prompt file or text (`runners::system_text`). Contest sessions and scheduled runs always get `None`, so nothing persona-related may appear then |
| `model` | `model_by_level` for the contest's level | The harness's model flag. If it has none, say so in the pull request |
| `preset_file` | The file `preset_file()` wrote | However the harness reads presets |
| `extra_args` | `runner_args.<name>` from the config | Unchanged and in order |

<p>Table 2. <code>Spec</code> fields</p>

### 2. Register it

In `src/runners/mod.rs`, add `pub mod <name>;` and append `&<name>::<Struct>` to `ALL`. If the harness needs fixed flags to work unattended (a permission mode, a sandbox setting), add a default `runner_args` entry in `defaults()` in [src/config.rs](src/config.rs).

### 3. Agents, skill, and extension

- The agents in `agents/*.md` are omp task-agent files: YAML frontmatter (`name`, `description`, `tools`, a model role in `model`, an optional `output` schema in JSON Type Definition) and a Korean body. `runners::agent_docs(root)` parses all of them (`AgentDoc::model` holds the default model selectors), and `AgentDoc::instructions()` returns the body with a closing section that asks for the output schema. Map omp tool names to the harness's own and drop tools it lacks.
- The skill `skills/contest-harness` is a plain Agent Skills directory. Link it where the harness looks for skills.
- Use `runners::link` and `runners::write_file`: they are idempotent and refuse to overwrite a real file. Remove agent files your previous install wrote that no longer exist in `agents/`.
- An extension package for the harness (commands like `/contest`) goes next to the adapter. It must get every value from `ach ... --json` and must not add the persona.

### 4. Tests

- `every_adapter_keeps_the_runner_contract` in [tests/backends.rs](tests/backends.rs) runs on every adapter in `ALL`, so it covers yours as soon as it is registered. It checks that:
  - both commands start with `binary()`;
  - the instructions reach the agent (the prompt path or its text is in the command);
  - `runner_args` are passed unchanged and in order;
  - the system-prompt addition appears only when `system_append` is set;
  - the one-shot command differs from the interactive one;
  - `preset_file()` refuses when `presets()` is `false`.
- Add a test with the exact interactive and one-shot command lines for a full and a bare `Spec`, like `omp_commands`.
- If `install` writes files, test it against a scratch home directory: a second run changes nothing, and a stale agent file is removed.
- If you implement `apply_agent_models`, keep the merge in a pure function and test it without the harness CLI, like `merge_overrides` in [tests/setup.rs](tests/setup.rs): named agents are set or removed, every other entry stays.
- Tests never start the real harness and never touch the network.
- [tests/e2e.rs](tests/e2e.rs) drives the real binary through one whole lifecycle offline (add, gate, route, a headless session on a stand-in runner, approval, submission, cleanup) with a temporary home, git config, and `ACH_HOME`. If your change touches that path, keep it passing and extend it rather than adding a second end-to-end test.

### 5. Pull request checklist

- [ ] `scripts/check.sh` passes (output pasted).
- [ ] `ach install --runner <name>` output, from two runs in a row.
- [ ] `ach launch <id> --dry-run` output for a routed test contest.
- [ ] One real session on the harness (a contest session, `ach chat`, or `ach autorun`), showing that the agent read the instructions and called `ach`. Name the harness version.
- [ ] If `orca_agent()` is set: `ach schedule install --scheduler orca` with a test cron, `ach schedule status`, then `ach schedule remove`.
- [ ] A row in Table 1 of README.md and README_ko.md, and in CONTRACT section 10.
- [ ] Known gaps (no subagents, no model flag, untested tool mapping) listed in the pull request and in the limitations of README.md and README_ko.md.

## Adding a source

A source is one module under `src/sources/` that implements the module contract in the header comment of [src/sources/mod.rs](src/sources/mod.rs). The steps are in the reference section [Extending: adding a source](docs/reference.md#extending-adding-a-source). Tests parse saved responses under `tests/fixtures/` and never touch the network. Respect `robots.txt` and the site's terms, and do not bypass blocks.
