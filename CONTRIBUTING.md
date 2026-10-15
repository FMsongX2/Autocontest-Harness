**English** · [한국어](#한국어)

# Contributing

Pull requests are welcome. Two kinds of extension have a fixed shape: a **runner adapter**, which lets the harness drive another agent harness, and a **source**, which collects listings from another site. This file covers both. The interface contract is [docs/CONTRACT.md](docs/CONTRACT.md); section 10 has the runner interface.

## Ground rules

- Run `scripts/check.sh` before you open the pull request. It runs `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, and `cargo audit` (skipped with a note when `cargo-audit` is not installed). Pull requests and releases must pass it. There is no CI, so paste the output into the pull request.
- Never commit personal data, operator-specific paths, or anything from `data/` other than the `*.example.*` files.
- Agent prompts, the skill, playbooks, and templates are written in Korean in a report style. Code comments and this file are in English.
- Only the CLI writes the registry. Agents and extensions call `harness`.

## Adding a runner adapter

The core ships one adapter, `omp`, because it is the one that has run contests end to end. An adapter for another harness (Claude Code, Codex, pi, or your own) is a Rust module that implements the `Runner` trait in [src/runners/mod.rs](src/runners/mod.rs).

Earlier versions carried unverified adapters for pi, Claude Code, Codex, and dsh. They were removed from the core; their code is a reasonable starting point:

```bash
git show bf05366:src/runners/claude.rs   # also pi.rs, codex.rs, dsh.rs
git show bf05366:tests/backends.rs       # their command-line and install tests
```

### 1. Implement `Runner`

Create `src/runners/<name>.rs` with a unit struct and `impl Runner for <Struct>`. [src/runners/omp.rs](src/runners/omp.rs) is the reference implementation.

| Method | Default | What it must do |
|---|---|---|
| `name()` | none | The config value `runner` that selects the adapter. Lowercase and unique |
| `binary()` | none | The executable looked up on `PATH`. Every command line starts with it |
| `available()` | `binary()` is on `PATH` | Override only if the harness needs more than the binary |
| `subagents()` | none | `true` if the harness can run the harness agents (`agents/*.md`) as subagents through a task or agent tool. With `false`, `harness launch` tells the session to run those steps one after another itself |
| `presets()` | `false` | `true` if the harness has named model presets (`preset_by_level`, `harness launch --preset`) |
| `preset_file(name, dir)` | error | Write the per-run file for preset `name` under `dir` (the contest's `.harness/`), named `<runner>-preset.<ext>`, and return its path. It reaches the command as `Spec::preset_file` |
| `orca_agent()` | `None` | The agent id Orca uses for this harness (`orca automations create --provider`). Needed for `schedule.scheduler = "orca"` |
| `interactive(spec)` | none | A command for a session with a TTY that starts by reading the instructions |
| `oneshot(spec)` | none | A command that runs the instructions once without a TTY and exits. The `headless` workspace and `harness autorun` use it |
| `install(root)` | none | Install the agents, the skill, and, where the harness has one, an extension package from the checkout at `root`. Idempotent. One human-readable line per action |
| `installed(root)` | `None` | Whether the harness agents, skill, and extension are installed for the checkout at `root`: `Some(true)` or `Some(false)`, or `None` when the adapter cannot tell. `harness doctor` reports it. The omp adapter checks that `omp plugin list --json` has an enabled `contest-harness` package that resolves to `root` |

<p>Table 1. The <code>Runner</code> trait</p>

`Spec` is the input to both command builders:

| Field | Meaning | How to pass it |
|---|---|---|
| `cwd` | The contest folder or the repository root | Workspaces already run `cd <cwd> && <command>`. Pass it as well if the harness has a working-directory flag |
| `prompt_file` | The instructions the agent reads before anything else | As a file reference if the harness supports one, otherwise inline with `runners::prompt_text` |
| `first_message` | The first user message, telling the agent to follow the instructions | May be left out when the instructions themselves are the first message |
| `system_append` | The persona and main role, for the main session (`harness chat`) only | As a system-prompt file or text (`runners::system_text`). Contest sessions and scheduled runs always get `None`, so nothing persona-related may appear then |
| `model` | `model_by_level` for the contest's level | The harness's model flag. If it has none, say so in the pull request |
| `preset_file` | The file `preset_file()` wrote | However the harness reads presets |
| `extra_args` | `runner_args.<name>` from the config | Unchanged and in order |

<p>Table 2. <code>Spec</code> fields</p>

### 2. Register it

In `src/runners/mod.rs`, add `pub mod <name>;` and append `&<name>::<Struct>` to `ALL`. If the harness needs fixed flags to work unattended (a permission mode, a sandbox setting), add a default `runner_args` entry in `defaults()` in [src/config.rs](src/config.rs).

### 3. Agents, skill, and extension

- The agents in `agents/*.md` are omp task-agent files: YAML frontmatter (`name`, `description`, `tools`, a model role, an optional `output` schema in JSON Type Definition) and a Korean body. `runners::agent_docs(root)` parses all of them, and `AgentDoc::instructions()` returns the body with a closing section that asks for the output schema. Map omp tool names to the harness's own and drop tools it lacks.
- The skill `skills/contest-harness` is a plain Agent Skills directory. Link it where the harness looks for skills.
- Use `runners::link` and `runners::write_file`: they are idempotent and refuse to overwrite a real file. Remove agent files your previous install wrote that no longer exist in `agents/`.
- An extension package for the harness (commands like `/contest`) goes next to the adapter. It must get every value from `harness ... --json` and must not add the persona.

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
- Tests never start the real harness and never touch the network.
- [tests/e2e.rs](tests/e2e.rs) drives the real binary through one whole lifecycle offline (add, gate, route, a headless session on a stand-in runner, approval, submission, cleanup) with a temporary home, git config, and `HARNESS_HOME`. If your change touches that path, keep it passing and extend it rather than adding a second end-to-end test.

### 5. Pull request checklist

- [ ] `scripts/check.sh` passes (output pasted).
- [ ] `harness install --runner <name>` output, from two runs in a row.
- [ ] `harness launch <id> --dry-run` output for a routed test contest.
- [ ] One real session on the harness, either a contest session or `harness chat`, showing that the agent read the instructions and called `harness`. Name the harness version.
- [ ] If `orca_agent()` is set: `harness schedule install --scheduler orca` with a test cron, `harness schedule status`, then `harness schedule remove`.
- [ ] A row in README Table 1 (English and Korean) and in CONTRACT section 10.
- [ ] Known gaps (no subagents, no model flag, untested tool mapping) listed in the pull request and in the README limitations.

## Adding a source

A source is one module under `src/sources/` that implements the module contract in the header comment of [src/sources/mod.rs](src/sources/mod.rs). The steps are in the README section [Extending: adding a source](README.md#extending-adding-a-source). Tests parse saved responses under `tests/fixtures/` and never touch the network. Respect `robots.txt` and the site's terms, and do not bypass blocks.

---

## 한국어

[English](#contributing) · **한국어**

pull request를 환영한다. 정해진 모양이 있는 확장은 두 가지다. 하네스가 다른 에이전트 하네스를 부리게 하는 **러너 어댑터**, 다른 사이트에서 공고를 모으는 **수집 출처**다. 이 문서는 둘을 다룬다. 인터페이스 계약은 [docs/CONTRACT.md](docs/CONTRACT.md)이고, 러너 인터페이스는 10절에 있다.

### 기본 규칙

- pull request를 열기 전에 `scripts/check.sh`를 돌린다. 이 스크립트는 `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, `cargo audit`(`cargo-audit`이 없으면 알리고 건너뛴다)을 차례로 돌린다. pull request와 릴리스는 이것을 통과해야 한다. CI가 없으므로 출력을 pull request에 붙인다.
- 개인정보, 운영자 환경 경로, `data/`의 파일(`*.example.*` 제외)은 커밋하지 않는다.
- 에이전트 지시문, 스킬, 플레이북, 템플릿은 한국어 보고서체로 쓴다. 코드 주석은 영어로 쓴다.
- 레지스트리는 CLI만 쓴다. 에이전트와 확장은 `harness`를 부른다.

### 러너 어댑터 추가

코어에는 공모전을 처음부터 끝까지 실제로 돌려 본 `omp` 어댑터 하나만 들어 있다. 다른 하네스(Claude Code, Codex, pi, 직접 만든 하네스)의 어댑터는 [src/runners/mod.rs](src/runners/mod.rs)의 `Runner` trait을 구현하는 Rust 모듈이다.

예전 버전에는 검증하지 않은 pi·Claude Code·Codex·dsh 어댑터가 있었다. 코어에서는 뺐지만 출발점으로 쓸 만하다:

```bash
git show bf05366:src/runners/claude.rs   # pi.rs, codex.rs, dsh.rs도 있다
git show bf05366:tests/backends.rs       # 그 어댑터들의 명령줄·설치 테스트
```

#### 1. `Runner` 구현

`src/runners/<이름>.rs`에 단위 구조체를 만들고 `impl Runner for <구조체>`를 쓴다. 기준 구현은 [src/runners/omp.rs](src/runners/omp.rs)다.

| 메서드 | 기본 구현 | 해야 하는 일 |
|---|---|---|
| `name()` | 없음 | 이 어댑터를 고르는 설정 `runner` 값. 소문자, 겹치지 않게 |
| `binary()` | 없음 | `PATH`에서 찾는 실행 파일. 모든 명령줄이 이 이름으로 시작한다 |
| `available()` | `binary()`가 `PATH`에 있는지 | 실행 파일 말고도 필요한 것이 있을 때만 바꾼다 |
| `subagents()` | 없음 | task·agent 도구로 하네스 에이전트(`agents/*.md`)를 서브에이전트로 돌릴 수 있으면 `true`. `false`면 `harness launch`가 그 단계를 세션이 차례로 직접 하라고 덧붙인다 |
| `presets()` | `false` | 이름 붙은 모델 프리셋(`preset_by_level`, `harness launch --preset`)이 있으면 `true` |
| `preset_file(name, dir)` | 오류 | 프리셋 `name`의 이번 실행용 파일을 `dir`(공모전 `.harness/`)에 `<러너>-preset.<확장자>`로 쓰고 경로를 돌려준다. 명령에는 `Spec::preset_file`로 들어간다 |
| `orca_agent()` | `None` | Orca가 이 하네스를 띄울 때 쓰는 에이전트 id(`orca automations create --provider`). `schedule.scheduler = "orca"`에 필요하다 |
| `interactive(spec)` | 없음 | TTY가 있는 세션 명령. 지시문을 먼저 읽고 시작한다 |
| `oneshot(spec)` | 없음 | TTY 없이 지시문을 한 번 수행하고 끝나는 명령. `headless` 작업 환경과 `harness autorun`이 쓴다 |
| `install(root)` | 없음 | `root` 클론에서 에이전트, 스킬, 하네스에 있으면 확장 패키지를 설치한다. 여러 번 돌려도 결과가 같아야 하고, 한 일마다 한 줄을 돌려준다 |
| `installed(root)` | `None` | `root` 클론의 에이전트·스킬·확장이 설치되어 있는지. `Some(true)`나 `Some(false)`, 어댑터가 알 수 없으면 `None`을 돌려준다. `harness doctor`가 알린다. omp 어댑터는 `omp plugin list --json`에 `root`를 가리키는 켜진 `contest-harness` 패키지가 있는지 본다 |

<p>표 1. <code>Runner</code> trait</p>

`Spec`은 두 명령 생성기의 입력이다:

| 필드 | 뜻 | 넘기는 법 |
|---|---|---|
| `cwd` | 공모전 폴더나 저장소 루트 | 작업 환경이 이미 `cd <cwd> && <명령>`으로 돌린다. 작업 디렉터리 플래그가 있으면 함께 넘긴다 |
| `prompt_file` | 에이전트가 무엇보다 먼저 읽을 지시문 | 하네스가 파일 참조를 받으면 파일로, 아니면 `runners::prompt_text`로 본문을 넘긴다 |
| `first_message` | 지시문을 따르라는 첫 사용자 메시지 | 지시문 본문을 첫 메시지로 넘기면 생략해도 된다 |
| `system_append` | 페르소나와 메인 역할. 메인 세션(`harness chat`)에만 있다 | 시스템 프롬프트 파일이나 본문(`runners::system_text`)으로. 공모전 세션과 정기 실행에서는 늘 `None`이므로 그때 페르소나 관련 내용이 나타나면 안 된다 |
| `model` | 그 레벨의 `model_by_level` | 하네스의 모델 플래그로. 플래그가 없으면 pull request에 적는다 |
| `preset_file` | `preset_file()`이 쓴 파일 | 하네스가 프리셋을 읽는 방식대로 |
| `extra_args` | 설정 `runner_args.<이름>` | 바꾸지 않고 순서대로 |

<p>표 2. <code>Spec</code> 필드</p>

#### 2. 등록

`src/runners/mod.rs`에 `pub mod <이름>;`을 더하고 `ALL`에 `&<이름>::<구조체>`를 붙인다. 사람 없이 돌리는 데 고정 플래그(권한 모드, 샌드박스 설정)가 필요한 하네스면 [src/config.rs](src/config.rs)의 `defaults()`에 `runner_args` 기본값을 더한다.

#### 3. 에이전트·스킬·확장

- `agents/*.md`는 omp task agent 파일이다. YAML 앞머리(`name`, `description`, `tools`, 모델 역할, 선택 사항인 JSON Type Definition `output` 스키마)와 한국어 본문으로 되어 있다. `runners::agent_docs(root)`가 모두 읽고, `AgentDoc::instructions()`가 출력 스키마를 요구하는 마지막 절을 붙인 본문을 돌려준다. omp 도구 이름은 그 하네스의 이름으로 바꾸고, 없는 도구는 뺀다.
- 스킬 `skills/contest-harness`는 일반 Agent Skills 폴더다. 하네스가 스킬을 찾는 곳에 링크한다.
- `runners::link`와 `runners::write_file`을 쓴다. 둘 다 여러 번 돌려도 같고, 실제 파일은 덮지 않는다. 이전 설치가 쓴 에이전트 파일 가운데 `agents/`에서 사라진 것은 지운다.
- 그 하네스용 확장 패키지(`/contest` 같은 명령)는 어댑터와 함께 더한다. 모든 값은 `harness ... --json`에서 얻고, 페르소나를 넣지 않는다.

#### 4. 테스트

- [tests/backends.rs](tests/backends.rs)의 `every_adapter_keeps_the_runner_contract`가 `ALL`의 모든 어댑터에 돌아, 등록하는 순간 새 어댑터도 검사한다. 확인하는 것:
  - 두 명령이 `binary()`로 시작한다.
  - 지시문이 에이전트에 닿는다(명령에 지시문 경로나 본문이 있다).
  - `runner_args`가 바뀌지 않고 순서대로 들어간다.
  - 시스템 프롬프트 덧붙이기는 `system_append`가 있을 때만 나타난다.
  - 한 번 실행 명령이 대화형 명령과 다르다.
  - `presets()`가 `false`면 `preset_file()`이 거절한다.
- `omp_commands`처럼, 값을 다 채운 `Spec`과 빈 `Spec`의 대화형·한 번 실행 명령줄을 그대로 비교하는 테스트를 더한다.
- `install`이 파일을 쓰면 임시 홈 폴더로 테스트한다. 두 번째 실행은 아무것도 바꾸지 않고, 낡은 에이전트 파일은 지워져야 한다.
- 테스트는 실제 하네스를 띄우지 않고 네트워크에 닿지 않는다.
- [tests/e2e.rs](tests/e2e.rs)는 실제 바이너리로 수명주기 하나 전체(추가, 게이트, 라우팅, 대역 러너로 띄운 headless 세션, 승인, 제출, 정리)를 임시 홈·git 설정·`HARNESS_HOME`에서 오프라인으로 돌린다. 이 경로를 건드리는 변경이면 이 테스트가 통과하게 두고, 종단 간 테스트를 따로 만들기보다 이 테스트를 늘린다.

#### 5. pull request 점검표

- [ ] `scripts/check.sh` 통과(출력 첨부).
- [ ] `harness install --runner <이름>`을 연달아 두 번 돌린 출력.
- [ ] 라우팅한 시험 공모전의 `harness launch <id> --dry-run` 출력.
- [ ] 그 하네스로 돌린 실제 세션 하나(공모전 세션이나 `harness chat`). 에이전트가 지시문을 읽고 `harness`를 부른 것이 보여야 한다. 하네스 버전을 적는다.
- [ ] `orca_agent()`가 있으면 시험용 cron으로 `harness schedule install --scheduler orca`, `harness schedule status`, `harness schedule remove`를 돌린 출력.
- [ ] README 표 1(영어·한국어)과 CONTRACT 10절에 한 줄씩.
- [ ] 알려진 빈틈(서브에이전트 없음, 모델 플래그 없음, 확인하지 않은 도구 이름 매핑)을 pull request와 README 한계 절에 적는다.

### 수집 출처 추가

수집 출처는 `src/sources/`의 모듈 하나이고, [src/sources/mod.rs](src/sources/mod.rs) 머리 주석의 모듈 계약을 구현한다. 단계는 README의 [확장: 수집 출처 추가](README.md#확장-수집-출처-추가) 절에 있다. 테스트는 `tests/fixtures/`에 저장한 응답을 파싱하고 네트워크에 닿지 않는다. `robots.txt`와 사이트 약관을 지키고, 막힌 곳은 우회하지 않는다.
