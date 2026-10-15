# Changelog

All notable changes. Versions follow [Semantic Versioning](https://semver.org/); until 1.0, a minor version may change the CLI or file formats, and the entry says how. Korean: [한국어](#한국어).

## Unreleased

### Added
- `claude` runner adapter for Claude Code: converts the agents into subagent files under `~/.claude/agents/contest-harness/`, links the skill into `~/.claude/skills/`, applies `agent_models` through the `model` field of those files, and defaults `runner_args.claude` to `--permission-mode auto`. Checked against `claude --help` and install tests only; no contest session has run on it.

### Fixed
- `harness launch --dry-run` works with adapters that pass the instructions as text (it built the command from a file the dry run never wrote).

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

---

## 한국어

[English](#changelog)

### 미배포

#### 추가
- Claude Code용 러너 어댑터 `claude`. 에이전트를 `~/.claude/agents/contest-harness/`의 서브에이전트 파일로 변환하고, 스킬을 `~/.claude/skills/`에 링크하며, `agent_models`를 그 파일의 `model` 필드로 적용한다. `runner_args.claude` 기본값은 `--permission-mode auto`다. `claude --help`와 설치 테스트로만 확인했고 공모전 세션은 돌려 보지 않았다.

#### 수정
- 지시문을 본문으로 넘기는 어댑터에서 `harness launch --dry-run`이 동작한다(dry run이 쓰지 않은 파일로 명령을 만들던 문제).

### 0.4.0 - 2026-10-15

에이전트가 설정을 맡는다. 에이전트에게 `INIT.md`를 읽게 하면 사용자와 함께 하네스를 설정한다.

- **추가**:
  - `INIT.md`: 설치, 뼈대 만들기, 추천 기본값을 단 질문 한 묶음, CLI로 적용, 사용자 정보, 모델 고르기, 설치·정기 실행·점검까지의 절차(omp: `/contest-init`)
  - `harness init`, `harness config path|get|set|unset`
  - 에이전트별 모델 `agent_models`와 `harness agents`. `harness install`이 러너에 적용한다(omp: 하네스 에이전트의 `task.agentModelOverrides`만)
- **바뀜**:
  - 메인 세션은 `harness doctor`로 시작하고, 설정이 안 된 클론이면 `INIT.md` 절차부터 진행한다.
  - 사용자 정보는 설정 에이전트에게 대화로 알려 줘 받아 적게 할 수도 있다. 제출용 개인정보 절은 직접 쓰기를 계속 권한다. 대화로 준 값은 모델 제공자와 세션 기록을 거치기 때문이다.

### 0.3.0 - 2026-10-15

프로덕션 수준 보강이다. 새 컴퓨터에 설치해 바로 돌고, 중간에 실패해도 되살아나며, 사람 없이 도는 에이전트가 사용자의 승인 밖으로 나가지 못하게 했다.

- **추가**:
  - macOS(arm64, x86_64)·Linux(x86_64, arm64) 배포 바이너리와 `SHA256SUMS`, `scripts/install.sh`
  - `harness doctor`
  - `harness approve`: 제출과 개인정보 읽기, 레벨 5 계획 검수 통과에 필요하다. 공모전 세션은 스스로 승인할 수 없다
  - 가져온 글을 파일로 넘기는 `add --from-file`·`--note-file`·`--receipt-file`
  - `cron` 스케줄러와 Linux 지원
  - `--version`의 커밋 표시
  - `SECURITY.md`, 점검·배포 스크립트, 전체 생명주기 종단 시험
- **바뀜**:
  - 설정은 엄격하게 검사한다.
  - `route`는 사전 점검을 하고, 실패해도 다시 실행할 수 있으며, 공개·남의 저장소를 거절하고 Actions를 끈다.
  - 네트워크 작업은 레지스트리 잠금 밖에서 하고, 실행은 상한을 넘지 않게 직렬화한다.
  - `clean`은 일부 실패에도 성공한 기록을 지키고 경로 탈출을 막는다.
  - 수집은 상세 실패를 견디고, 같은 사이트의 다른 공모전을 합치지 않으며, 일시적인 HTTP 오류를 다시 시도한다.
  - 기록부의 덮어쓰기와 다음 회차 오판을 고쳤다.
  - 데이터 파일은 형식 버전·0600·fsync를 쓰고, 외부 명령에 시간 제한을 둔다.
- **옮길 때**:
  - 이 버전 전에 `transition`으로 계획 검수를 끝낸 레벨 5 공모전은 `plan_review`로 되돌려 `harness approve`를 받는다.
  - 설정의 모르는 키는 지운다(`harness doctor`가 알려 준다).
  - `git pull` 뒤 `scripts/install.sh`로 바이너리를 다시 설치한다.

### 0.2.0 - 2026-10-15

Rust로 다시 쓴 첫 버전이다.
