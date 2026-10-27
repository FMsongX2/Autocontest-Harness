[English](CONTRIBUTING.md) · **한국어**

# 기여하기

pull request를 환영한다. 정해진 모양이 있는 확장은 두 가지다. 하네스가 다른 에이전트 하네스를 부리게 하는 **러너 어댑터**, 다른 사이트에서 공고를 모으는 **수집 출처**다. 이 문서는 둘을 다룬다. 인터페이스 계약은 [docs/CONTRACT.md](docs/CONTRACT.md)이고, 러너 인터페이스는 10절에 있다.

## 기본 규칙

- pull request를 열기 전에 `scripts/check.sh`를 돌린다. 이 스크립트는 `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, Python 컨트롤러 테스트(`python3 -B -m unittest discover -s tests -p 'test_*.py'`, `controllers/` 대상), `cargo audit`(`cargo-audit`이 없으면 알리고 건너뛴다)을 차례로 돌린다. pull request와 릴리스는 이것을 통과해야 한다. CI가 없으므로 출력을 pull request에 붙인다.
- 개인정보, 운영자 환경 경로, `data/`의 파일(`*.example.*` 제외)은 커밋하지 않는다.
- 에이전트 지시문, 스킬, 플레이북, 템플릿은 한국어 보고서체로 쓴다. 코드 주석은 영어로 쓴다.
- 레지스트리는 CLI만 쓴다. 에이전트와 확장은 `ach`를 부른다.

## 러너 어댑터 추가

코어에는 공모전을 처음부터 끝까지 실제로 돌려 본 `omp` 어댑터와 `claude`(Claude Code) 어댑터가 들어 있다. `claude`는 Claude Code가 이 문서를 따라 직접 썼고, 정기 실행(수집과 `ContestGate` 서브에이전트의 게이트 판정)까지 돌려 봤지만 대화형 공모전 세션은 돌려 보지 않았다. 다른 하네스(Codex, pi, 직접 만든 하네스)의 어댑터는 [src/runners/mod.rs](src/runners/mod.rs)의 `Runner` trait을 구현하는 Rust 모듈이다.

예전 버전에는 검증하지 않은 pi·Claude Code·Codex·dsh 어댑터가 있었다. Claude Code는 코어에 다시 썼고 나머지는 뺐지만 출발점으로 쓸 만하다:

```bash
git show bf05366:src/runners/codex.rs    # pi.rs, dsh.rs도 있다
git show bf05366:tests/backends.rs       # 그 어댑터들의 명령줄·설치 테스트
```

### 1. `Runner` 구현

`src/runners/<이름>.rs`에 단위 구조체를 만들고 `impl Runner for <구조체>`를 쓴다. 기준 구현은 [src/runners/omp.rs](src/runners/omp.rs)다.

| 메서드 | 기본 구현 | 해야 하는 일 |
|---|---|---|
| `name()` | 없음 | 이 어댑터를 고르는 설정 `runner` 값. 소문자, 겹치지 않게 |
| `binary()` | 없음 | `PATH`에서 찾는 실행 파일. 모든 명령줄이 이 이름으로 시작한다 |
| `available()` | `binary()`가 `PATH`에 있는지 | 실행 파일 말고도 필요한 것이 있을 때만 바꾼다 |
| `subagents()` | 없음 | task·agent 도구로 하네스 에이전트(`agents/*.md`)를 서브에이전트로 돌릴 수 있으면 `true`. `false`면 `ach launch`가 그 단계를 세션이 차례로 직접 하라고 덧붙인다 |
| `presets()` | `false` | 이름 붙은 모델 프리셋(`preset_by_level`, `ach launch --preset`)이 있으면 `true` |
| `preset_file(name, dir)` | 오류 | 프리셋 `name`의 이번 실행용 파일을 `dir`(공모전 `.harness/`)에 `<러너>-preset.<확장자>`로 쓰고 경로를 돌려준다. 명령에는 `Spec::preset_file`로 들어간다 |
| `orca_agent()` | `None` | Orca가 이 하네스를 띄울 때 쓰는 에이전트 id(`orca automations create --provider`). `schedule.scheduler = "orca"`에 필요하다 |
| `interactive(spec)` | 없음 | TTY가 있는 세션 명령. 지시문을 먼저 읽고 시작한다 |
| `oneshot(spec)` | 없음 | TTY 없이 지시문을 한 번 수행하고 끝나는 명령. `headless` 작업 환경과 `ach autorun`이 쓴다 |
| `session_records(cwd)` | `None` | 작업 디렉터리가 `cwd`인 이 하네스의 저장된 세션 id 목록(`restore`가 받는 값, 최신 순). 어댑터가 목록을 낼 수 없으면 `None`이고, 그때 `ach restart`·`ach resume`은 `--restore <UUID>`를 직접 받아야 한다 |
| `restore(spec, id, message, interactive)` | 거절 | 저장된 세션 `id`(UUID, `runners::valid_session_id`로 검사)를 `spec.cwd`에서 `spec`의 프리셋·모델·시스템 추가·`runner_args`로 다시 열고 `message`를 다음 턴으로 보내는 명령. TTY 없는 작업 환경이면 `interactive`가 `false`다. 하네스가 복원을 못 하면 거절한다. `ach restart`·`resume`은 새 세션으로 몰래 바꾸지 않는다. omp는 `--restore <id>`, Claude Code는 `--resume <id>` |
| `install(root)` | 없음 | `root` 클론에서 에이전트, 스킬, 하네스에 있으면 확장 패키지를 설치한다. 여러 번 돌려도 결과가 같아야 하고, 한 일마다 한 줄을 돌려준다 |
| `installed(root)` | `None` | `root` 클론의 에이전트·스킬·확장이 설치되어 있는지. `Some(true)`나 `Some(false)`, 어댑터가 알 수 없으면 `None`을 돌려준다. `ach doctor`가 알린다. omp 어댑터는 `omp plugin list --json`에 `root`를 가리키는 켜진 `contest-harness` 패키지가 있는지 본다 |
| `apply_agent_models(models)` | `Some`가 하나라도 있으면 거절, 모두 `None`이면 할 일 없음 | 설정 `agent_models`(에이전트 이름 → 이 하네스 모델 문법의 `Some(모델 지정)`, 또는 하네스가 전에 적용한 선택을 지우라는 `None`)를 하네스가 에이전트별 모델을 읽는 곳에 적용한다. `ach install`이 설정 러너에만 부른다. `models`에 적힌 에이전트만 건드리고 다른 설정은 그대로 두며, 한 일마다 한 줄을 돌려준다. 여러 번 돌려도 결과가 같아야 한다. omp 어댑터는 omp 전역 설정 `task.agentModelOverrides`에 이름을 합친다(`omp config get` 뒤 `omp config set`) |
| `agent_models()` | `None` | 하네스가 지금 적용하는 에이전트별 모델(에이전트 이름 → 모델 지정). 어댑터가 알 수 없으면 `None`. `ach agents`가 보여 주고 `ach doctor`가 `agent_models`와 비교한다 |

<p>표 1. <code>Runner</code> trait</p>

`Spec`은 두 명령 생성기의 입력이다:

| 필드 | 뜻 | 넘기는 법 |
|---|---|---|
| `cwd` | 공모전 폴더나 저장소 루트 | 작업 환경이 이미 `cd <cwd> && <명령>`으로 돌린다. 작업 디렉터리 플래그가 있으면 함께 넘긴다 |
| `prompt_file` | 에이전트가 무엇보다 먼저 읽을 지시문 | 하네스가 파일 참조를 받으면 파일로(`claude`는 `@<경로>` 언급으로 넘긴다), 아니면 `runners::prompt_text`로 본문을 넘긴다. `ach launch --dry-run`에서는 이 파일이 없을 수 있다 |
| `prompt` | 호출한 쪽이 지시문을 메모리에 들고 있을 때(`ach launch`) 그 본문 | 직접 읽지 않는다. `runners::prompt_text`가 이 값을 돌려주고, `None`일 때만 `prompt_file`을 읽는다 |
| `first_message` | 지시문을 따르라는 첫 사용자 메시지 | 지시문 본문을 첫 메시지로 넘기면 생략해도 된다 |
| `system_append` | 페르소나와 메인 역할. 메인 세션(`ach chat`)에만 있다 | 시스템 프롬프트 파일이나 본문(`runners::system_text`)으로. 공모전 세션과 정기 실행에서는 늘 `None`이므로 그때 페르소나 관련 내용이 나타나면 안 된다 |
| `model` | 그 레벨의 `model_by_level` | 하네스의 모델 플래그로. 플래그가 없으면 pull request에 적는다 |
| `preset_file` | `preset_file()`이 쓴 파일 | 하네스가 프리셋을 읽는 방식대로 |
| `extra_args` | 설정 `runner_args.<이름>` | 바꾸지 않고 순서대로 |

<p>표 2. <code>Spec</code> 필드</p>

### 2. 등록

`src/runners/mod.rs`에 `pub mod <이름>;`을 더하고 `ALL`에 `&<이름>::<구조체>`를 붙인다. 사람 없이 돌리는 데 고정 플래그(권한 모드, 샌드박스 설정)가 필요한 하네스면 [src/config.rs](src/config.rs)의 `defaults()`에 `runner_args` 기본값을 더한다.

### 3. 에이전트·스킬·확장

- `agents/*.md`는 omp task agent 파일이다. YAML 앞머리(`name`, `description`, `tools`, `model`의 모델 역할, 선택 사항인 JSON Type Definition `output` 스키마)와 한국어 본문으로 되어 있다. `runners::agent_docs(root)`가 모두 읽고(`AgentDoc::model`이 기본 모델 지정 목록), `AgentDoc::instructions()`가 출력 스키마를 요구하는 마지막 절을 붙인 본문을 돌려준다. omp 도구 이름은 그 하네스의 이름으로 바꾸고, 없는 도구는 뺀다.
- 스킬 `skills/contest-harness`는 일반 Agent Skills 폴더다. 하네스가 스킬을 찾는 곳에 링크한다.
- `runners::link`와 `runners::write_file`을 쓴다. 둘 다 여러 번 돌려도 같고, 실제 파일은 덮지 않는다. 이전 설치가 쓴 에이전트 파일 가운데 `agents/`에서 사라진 것은 지운다.
- 그 하네스용 확장 패키지(`/contest` 같은 명령)는 어댑터와 함께 더한다. 모든 값은 `ach ... --json`에서 얻고, 페르소나를 넣지 않는다.

### 4. 테스트

- [tests/backends.rs](tests/backends.rs)의 `every_adapter_keeps_the_runner_contract`가 `ALL`의 모든 어댑터에 돌아, 등록하는 순간 새 어댑터도 검사한다. 확인하는 것:
  - 두 명령이 `binary()`로 시작한다.
  - 지시문이 에이전트에 닿는다(명령에 지시문 경로나 본문이 있다).
  - `runner_args`가 바뀌지 않고 순서대로 들어간다.
  - 시스템 프롬프트 덧붙이기는 `system_append`가 있을 때만 나타난다.
  - 한 번 실행 명령이 대화형 명령과 다르다.
  - `presets()`가 `false`면 `preset_file()`이 거절한다.
- `omp_commands`처럼, 값을 다 채운 `Spec`과 빈 `Spec`의 대화형·한 번 실행 명령줄을 그대로 비교하는 테스트를 더한다.
- `install`이 파일을 쓰면 임시 홈 폴더로 테스트한다. 두 번째 실행은 아무것도 바꾸지 않고, 낡은 에이전트 파일은 지워져야 한다.
- `apply_agent_models`를 구현하면 합치는 부분을 순수 함수로 두고, [tests/setup.rs](tests/setup.rs)의 `merge_overrides` 테스트처럼 하네스 CLI 없이 시험한다. 이름을 적은 에이전트는 정해지거나 지워지고, 나머지 항목은 그대로여야 한다.
- 테스트는 실제 하네스를 띄우지 않고 네트워크에 닿지 않는다.
- [tests/e2e.rs](tests/e2e.rs)는 실제 바이너리로 수명주기 하나 전체(추가, 게이트, 라우팅, 대역 러너로 띄운 headless 세션, 승인, 의도·시작·접수 증빙으로 이어지는 제출 트랜잭션, 정리)를 임시 홈·git 설정·`ACH_HOME`에서 오프라인으로 돌린다. 이 경로를 건드리는 변경이면 이 테스트가 통과하게 두고, 종단 간 테스트를 따로 만들기보다 이 테스트를 늘린다. 테스트는 실제 공모전 사이트에 접속하거나 제출하거나 실제 세션을 닫지 않고, 규칙 원문 받기는 로컬 HTTP 픽스처로 한다.

### 5. pull request 점검표

- [ ] `scripts/check.sh` 통과(출력 첨부).
- [ ] `ach install --runner <이름>`을 연달아 두 번 돌린 출력.
- [ ] 라우팅한 시험 공모전의 `ach launch <id> --dry-run` 출력.
- [ ] 그 하네스로 돌린 실제 세션 하나(공모전 세션, `ach chat`, `ach autorun` 가운데 하나). 에이전트가 지시문을 읽고 `ach`를 부른 것이 보여야 한다. 하네스 버전을 적는다.
- [ ] `orca_agent()`가 있으면 시험용 cron으로 `ach schedule install --scheduler orca`, `ach schedule status`, `ach schedule remove`를 돌린 출력.
- [ ] README.md와 README_ko.md의 표 1, CONTRACT 10절에 한 줄씩.
- [ ] 알려진 빈틈(서브에이전트 없음, 모델 플래그 없음, 확인하지 않은 도구 이름 매핑)을 pull request와 README.md·README_ko.md의 한계 절에 적는다.

## 수집 출처 추가

수집 출처는 `src/sources/`의 모듈 하나이고, [src/sources/mod.rs](src/sources/mod.rs) 머리 주석의 모듈 계약을 구현한다. 단계는 레퍼런스의 [확장: 수집 출처 추가](docs/reference_ko.md#확장-수집-출처-추가) 절에 있다. 테스트는 `tests/fixtures/`에 저장한 응답을 파싱하고 네트워크에 닿지 않는다. `robots.txt`와 사이트 약관을 지키고, 막힌 곳은 우회하지 않는다.
