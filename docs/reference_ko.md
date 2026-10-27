[English](reference.md) · **한국어**

# 레퍼런스

Autocontest-Harness를 운영하거나 고치는 사람을 위한 레퍼런스다. 하네스를 쓰는 데는 필요 없다. 사용자는 메인 세션과 대화하고, 하네스 코어 `ach`는 에이전트가 부른다. 아래 명령은 `ach <명령>`으로 적었고, 클론의 터미널에서는 `bin/ach <명령>`으로 실행한다. 인터페이스 계약은 [CONTRACT.md](CONTRACT.md), 개요는 [README](../README_ko.md)에 있다.

## `ach` 바이너리

`ach`는 하네스 코어다. 바이너리 하나가 레지스트리, 제출 기록부, 정기 실행 작업을 맡고, 공고를 수집하고, 공모전을 라우팅·실행하고, 승인·점수·페르소나 관문을 지킨다. 부르는 것은 에이전트이고, 사용자는 보통 `bin/ach chat`만 실행한다.

- **설치.** `scripts/install.sh`는 태그가 `v`와 `Cargo.toml`의 버전을 이은 GitHub 릴리스에서 이 플랫폼용 바이너리를 내려받아 릴리스의 `SHA256SUMS`로 확인하고, 클론의 `bin/ach`(git 제외)에 설치한다. 미리 빌드한 대상은 `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`이다. 맞는 파일이 없으면 `cargo build --release --locked`로 소스에서 빌드하고, `scripts/install.sh --from-source`는 소스 빌드를 강제한다. `PATH`는 건드리지 않고, 0.5까지의 버전이 설치한 `$BIN_DIR/harness` 링크(기본 `~/.local/bin`)와 `bin/harness`는 지운다.
- **PATH에 없다.** 하네스가 띄우는 모든 세션(공모전 세션, 메인 세션, 정기 실행)은 `env [ACH_SESSION=<id>] PATH=<클론>/bin:<PATH> <명령>`으로 돌아서, 그 에이전트가 `ach`를 찾는다. omp 확장은 자기를 불러온 omp 세션의 `PATH` 맨 앞에 `<클론>/bin`을 넣는다. Orca automation의 프롬프트에는 바이너리 경로가 적혀 있고, launchd·cron 작업은 경로로 부른다. 그 밖의 에이전트 세션(예: 클론에서 연 Claude Code)은 스킬의 규칙을 따른다. `ach`를 찾지 못하면 `<클론>/bin/ach`를 부른다.
- **어느 클론인가.** `ACH_HOME`이 있으면 그곳, 없으면 실제 바이너리 경로(심볼릭 링크를 풀어서)의 위쪽 폴더 가운데 `harness/prompts/launch.md`가 있는 가장 가까운 곳, 그것도 없으면 빌드한 클론이다. `data/`, 지시문, 에이전트, 스킬을 그곳에서 읽는다. 그 폴더에 `harness/prompts/launch.md`가 없으면 `doctor`, `--help`, `--version`을 뺀 모든 명령이 실행을 거절하고, 그곳에 아무것도 만들지 않는다.

## 손으로 단계 돌리기

메인 세션에 시키면 아래를 모두 대신 한다. 클론의 터미널에서 직접 돌리려면:

```bash
bin/ach init                                 # data/(0700), 예시에서 data/config.json과 data/profile.md(0600), data/logs/를 만든다. 있는 파일은 그대로 두고, 끝에 점검을 돌린다
bin/ach config set workspace tmux            # 점으로 이은 키. 쓰기 전에 설정 전체를 검사한다
bin/ach config set schedule.cron '0 9,21 * * *'
bin/ach config get                           # 적용된 설정 전체(기본값 + 파일). 설정 절 참고
$EDITOR data/profile.md                      # 사용자 정보: 참가 자격과 제출 양식에 들어갈 정보
bin/ach doctor                               # [실패] 줄은 모두 고친다. [경고] 줄은 참고한다
bin/ach install                              # 설정한 러너에 에이전트·스킬·확장 설치(omp: omp plugin link)와 agent_models 적용
bin/ach collect                              # 모든 출처. 또는: bin/ach collect --source dacon --pages 1
bin/ach list --state discovered
# 게이트: 메인 세션(또는 아무 omp 세션)에 id를 주며 ContestGate 실행을 요청하거나,
# omp에서 /contest-gate <id> 프롬프트 템플릿을 쓴다. 에이전트가 gate.json을 쓰고
# ach gate <id> --file gate.json 으로 기록한다
bin/ach show wevity-111674
bin/ach route wevity-111674                  # 폴더 <contests_root>/<Folder>, private 저장소, 작업 공간, CONTEST.md
bin/ach capacity                             # 레벨별 빈 자리, 메모리 예산, 권장 워커 수
bin/ach launch wevity-111674 --dry-run       # 실행하지도 쓰지도 않고 명령·지시문·경고만 출력
bin/ach launch wevity-111674
bin/ach peek wevity-111674                   # 세션 화면이나 로그의 끝
```

`route`에 `--no-repo`나 `--no-workspace`를 붙이면 GitHub 저장소나 작업 공간을 만들지 않는다. 상한이 허용하는 만큼 한꺼번에 실행하려면 `ach tick --auto-launch`를 쓴다. `gate_pass`·`routed` 공모전을 우선순위 순으로 라우팅·실행하고, 자리가 없는 공모전은 `waiting`으로 알린다. 자기 환경 규칙은 `data/local-rules.md`에 적는다([로컬 규칙](#로컬-규칙) 참고).

## 설치 점검(`ach doctor`)

`ach doctor [--json]`은 이 클론, 설정, 부르는 도구, 정기 실행이 준비되었는지 확인한다. 읽기만 하고, 클론을 찾지 못해 다른 명령이 거절하는 곳에서도 돈다. 문제마다 고치는 법을 함께 낸다. `ach init`이 끝에 돌리고, 메인 세션도 시작할 때 돌린다.

- **checkout**: 클론 루트와 그 안의 `harness/prompts/launch.md`.
- **version**: 바이너리를 빌드한 커밋과 클론의 `HEAD`. 문서만 바꾼 커밋은 치지 않는다.
- **os**: macOS 또는 Linux. 그 밖의 시스템은 실패다.
- **config**: `data/config.json`이 읽히고 검증을 통과하는지. 파일이 없으면 경고이고 기본값을 쓴다.
- **registry·ledger**: `data/registry.json`과 `data/submissions.json`이 읽히는지.
- **data 권한**: group이나 other가 `data/`를 읽을 수 있으면 경고.
- **git 작성자 정보**: `git var GIT_AUTHOR_IDENT`가 성공하는지(아니면 실패).
- **gh**: `PATH`에 있고 `gh auth status`가 성공하는지(경고).
- **runner**: 실행 파일이 `PATH`에 있는지(실패), 이 클론으로 설치되었는지(omp는 `omp plugin list --json`, 경고).
- **workspace**: 실행 파일이 `PATH`에 있는지(실패). `orca`면 `orca_repo_path`가 `orca repo list`의 Orca 프로젝트인지(실패, 고치는 법은 `orca repo add --path <경로>`).
- **agent_models**: `agent_models`의 이름이 모두 `agents/`의 에이전트인지(아니면 실패, 고치는 법은 `ach config unset agent_models.<이름>`). 러너가 지금 적용한 모델을 알려 주면, 설정과 다를 때 경고이고 고치는 법은 `ach install`이다. 비어 있으면 ok이고 에이전트마다 기본 역할 모델을 쓴다.
- **contests_root**: 있고 쓸 수 있는지.
- **profile**: 있고, 권한이 0600이며, 예시 자리표시가 남아 있지 않은지(경고).
- **memory**: 쓸 수 있는 메모리를 읽을 수 있는지(경고).
- **schedule**: 설정한 스케줄러에 정기 실행이 설치되어 설정과 맞고, 다른 스케줄러에는 걸려 있지 않은지(경고).

텍스트 출력은 점검마다 `[ok]`, `[경고]`, `[실패]` 가운데 한 줄, 문제 아래 고치는 법 한 줄, 마지막 집계 `실패 n건, 경고 m건`이다. JSON은 `{"checks": [{name, level, detail, fix?}]}`이고 `level`은 `ok`, `warn`, `fail` 가운데 하나다. 실패가 하나라도 있으면 종료 코드 1이고, 경고만 있으면 0이다.

## omp 확장

이 저장소는 omp 확장 패키지다. `omp` 러너로 `ach install`을 실행하면 `omp plugin link <저장소>`가 돌고, omp가 확장 파일과 함께 관례 위치의 스킬, 프롬프트 템플릿(`commands/`), 에이전트를 읽는다.

| 이름 | 종류 | 하는 일 |
|---|---|---|
| `/contest` | 명령 | 보드 요약: 상태별 건수, 제출 승인·계획 검수 대기, 보류 질문, 7일 안에 마감하는 진행 중 공모전, 용량 |
| `/contest <id>` | 명령 | 공모전 하나의 요약 |
| `/contest-chat` | 명령 | 메인 인터페이스 세션 여는 법과 바이너리 버전 |
| `/contest-gate <id>` | 프롬프트 템플릿(`commands/contest-gate.md`) | 레지스트리 항목 하나를 `ContestGate`로 거르고 결과를 요약한다 |
| `/contest-init` | 프롬프트 템플릿(`commands/contest-init.md`) | [INIT.md](../INIT.md)대로 사용자와 하네스 설정을 마치고, `ach doctor` 결과와 남은 할 일을 알린다 |

<p>표 1. 확장이 등록하는 명령</p>

확장은 모든 정보를 `ach ... --json`에서 얻는다. 바이너리는 `ACH_BIN`, 클론의 `bin/ach`, `target/release/ach`, `target/debug/ach`, `PATH`의 `ach` 순으로 찾는다. omp가 확장을 불러올 때 omp 프로세스의 `PATH` 맨 앞에 클론의 `bin/`을 넣어, 그 세션의 에이전트가 `ach`를 부를 수 있게 한다. 확장은 페르소나를 넣지 않는다.

## 수집 출처

출처 하나가 `src/sources/`의 모듈 하나다.

| 이름 | 사이트 | 방식 | 비고 |
|---|---|---|---|
| `linkareer` | 링커리어 | HTML | 목록 페이지의 Next.js `__NEXT_DATA__`를 읽는다. 마감 시각까지 들어 있다. 첫 쪽만 읽는다 |
| `wevity` | 위비티 | HTML | 목록에 마감이 없어 새 항목만 상세를 연다 |
| `thinkcontest` | 씽굿 | API | 내부 JSON 목록 API. 접수중과 접수예정을 따로 읽는다 |
| `allcon` | 올콘 | API + HTML | 내부 JSON 목록과 새 항목의 HTML 상세 |
| `contestkorea` | 콘테스트코리아 | HTML | 목록과 상세 페이지. 접수가 끝난 행은 거른다 |
| `detizen` | 대티즌 | API | 사이트의 공개 publishable key로 Supabase REST를 읽는다 |
| `campuspick` | 캠퍼스픽 | API + HTML | 내부 목록 API와 새 항목 상세의 `__INITIAL_STATE__` |
| `allforyoung` | 요즘것들 | API | 공개 JSON API. 마감은 상세 API에서 가져온다 |
| `dacon` | 데이콘 | HTML + API | 목록 페이지의 Nuxt 데이터와 상세 API(참가 신청·제출 마감) |
| `devpost` | Devpost | API + HTML | 공개 JSON API, 온라인 해커톤만. 상세 페이지는 Cloudflare에 자주 막히며 우회하지 않는다 |
| `kaggle` | Kaggle | API | 상금과 UTC 마감이 들어 있는 내부 목록 API. Getting Started와 Community 대회는 뺀다 |

<p>표 2. 기본 수집 출처</p>

두 곳은 일부러 넣지 않았다. 캠퍼스몬은 robots.txt가 `Disallow: /`이고, AIConnect는 2026-07-31에 서비스를 종료했다. 마감은 모두 KST(`+09:00`)로 저장한다. 출처가 날짜만 주면 그날 23:59 KST로 두고, 정확한 시각은 게이트가 공식 공고에서 다시 확인한다.

수집 규칙:

- 상세 페이지를 읽지 못해도 그 공고를 버리지 않는다. 목록 행을 그대로 두고(마감은 목록 값, 없으면 비움) 메모 `상세 실패: <오류>`를 붙여 게이트가 다시 확인하게 한다.
- 같은 사이트의 공고는 제목으로 병합하지 않는다. 한 사이트에서 id가 다르면 다른 공모전이다. URL이 같으면 그대로 병합한다.
- 제목에 남은 숫자 토큰이 다르면(ARC-AGI-2와 ARC-AGI-3, Track 1과 Track 2) 비슷한 제목으로 보지 않는다. 한쪽에만 있는 숫자(상금 등), 연도, 회차는 일치 판정을 가르지 않는다.
- 수집한 id는 `[a-z0-9-]+`여야 한다. 아니면 건너뛰고 `errors`에 알린다. 제목과 주최는 한 줄로 만든다. 제어 문자와 보이지 않는 문자를 지우고 공백을 하나로 줄이며, 레지스트리에도 같게 적용한다.
- HTTP 요청의 제한 시간은 30초이고, 같은 사이트에 보내는 요청은 1초 이상 간격을 둔다. 시간 초과, I/O·연결·DNS 오류, 429, 5xx는 2초, 6초 뒤에 다시 시도한다(모두 세 번). 숫자로 된 `Retry-After`가 30초 이하이면 따르고, 그보다 길면 다시 시도하지 않는다. 그 밖의 4xx는 바로 실패한다. 오류 메시지는 URL로 시작한다.
- 수집 중 패닉이 난 출처는 `<이름>: 수집 중 패닉: <메시지>`로 알리고, 다른 출처는 계속 수집한다.

## 설정

설정은 [`src/config.rs`](../src/config.rs)의 기본값 위에 `data/config.json`을 덮어 읽는다. 적지 않은 키는 기본값을 그대로 쓴다. 사전 값 키(`preset_by_level`, `model_by_level`, `max_running_by_level`, `ram_gb_by_level`, `runner_args`, `persona`, `schedule`, `agent_models`)는 적은 항목만 덮는다. `{"max_running_by_level": {"5": 0}}`은 레벨 5만 바꾼다. 레벨 키는 문자열 `"1"`~`"5"`다.

`ach config`는 파일을 열지 않고 설정을 읽고 바꾼다. 키는 점으로 이은 경로다(`schedule.cron`, `agent_models.ContestJudge`, `model_by_level.3`). `set`은 값이 JSON으로 읽히면 JSON으로(숫자, `true`/`false`, `null`, `"..."`, `[...]`, `{...}`), 아니면 문자열로 받고, 설정 전체를 검사한 뒤에야 파일을 쓴다(원자적으로, 권한 0600). 틀린 값은 거절하고 파일은 그대로 둔다. `unset`은 파일에서 키를 지워 기본값으로 되돌린다. `get`은 적용되는 값(기본값 + 파일)을 보여 주고, 키가 없으면 설정 전체를 보여 준다.

```bash
ach config path                          # 설정 파일 경로
ach config get schedule                  # 적용되는 값
ach config set max_running_by_level.5 0  # 숫자 0으로 읽는다
ach config set github_org my-org         # JSON이 아니므로 문자열로 읽는다
ach config unset max_running_by_level.5  # 기본값으로 되돌린다
```

설정 파일은 읽을 때 검증하고, 설정 오류는 모두 파일 경로로 시작한다. 모르는 키는 거절한다(`unknown field ...`). `persona`와 `schedule` 안도 같다. `runner`는 코어에 든 어댑터 이름, `workspace`는 `orca`·`tmux`·`headless`·`terminal`, `schedule.scheduler`는 `orca`·`launchd`·`cron` 가운데 하나여야 한다. `schedule.cron`과 `schedule.scout_hours`(cron의 시 칸, 0~23, 비워도 된다)는 해석되어야 한다. 레벨별 키는 `"1"`~`"5"`만 쓴다. 메모리 값은 유한한 0 이상의 수, `ram_gb_per_worker`는 0보다 큰 수, `collect_pages`는 1 이상이어야 한다.

| 키 | 기본값 | 설명 |
|---|---|---|
| `github_org` | `""` | `ach route`가 `gh`로 공모전마다 private 저장소를 만드는 GitHub 조직 또는 사용자. 비워 두면 `gh`에 로그인한 계정에 만든다 |
| `contests_root` | `"~/contests"` | 공모전마다 폴더 하나를 두는 곳. `dedupe-check`는 `<contests_root>/*/README.md`와 `<contests_root>/.shared/existing_contests.md`도 읽는다 |
| `orca_repo_path` | `null`(= `contests_root`) | Orca에 등록된 git 저장소. 공모전 작업 공간은 이 저장소의 worktree로 만든다 |
| `runner` | `"omp"` | 공모전 세션·정기 실행·메인 세션에 쓸 러너 어댑터(에이전트 하네스). 코어에는 `omp`와 `claude`(Claude Code)가 있다. [러너 어댑터 추가](../README_ko.md#러너-어댑터-추가) 참고 |
| `workspace` | `"orca"` | 공모전 세션을 돌릴 곳: `orca`, `tmux`, `headless` |
| `preset_by_level` | `{"1": null, "2": null, "3": null, "4": null, "5": null}` | 프리셋을 지원하는 어댑터의 레벨별 모델 프리셋. omp는 예컨대 `{"3": "ResearchPreset"}`을 `~/.omp/agent/config.yml`의 `modelPresets`에서 가져와 실행마다 따로 만드는 `--config` 덮개로 넘기므로, 전역 omp 설정은 바뀌지 않는다. `null`이면 omp 기본 역할을 쓴다. 이름을 적은 프리셋이 omp 설정에 없으면 `launch`가 실패한다 |
| `model_by_level` | `{}` | 프리셋이 없을 때 쓰는 레벨별 모델 id |
| `runner_args` | `{"omp": [], "claude": ["--permission-mode", "auto"]}` | 러너 이름별로, 그 어댑터의 모든 명령에 붙이는 인자 |
| `max_running` | `6` | 모든 레벨을 합쳐 동시에 진행하는 공모전 수의 상한. `0`이면 실행을 모두 멈춘다 |
| `max_running_by_level` | `{"1": 3, "2": 2, "3": 2, "4": 1, "5": 1}` | 레벨별 동시 진행 상한. `0`이면 그 레벨의 공모전은 `gate_pass`에 머물고 실행되지 않는다 |
| `ram_reserve_gb` | `8` | 사용자가 쓰는 앱 몫으로 남겨 두는 메모리. 메모리 예산은 쓸 수 있는 메모리(macOS는 `vm_stat`의 free + inactive + speculative, Linux는 `/proc/meminfo`의 `MemAvailable`)에서 이 값을 뺀 것이다 |
| `ram_gb_by_level` | `{"1": 2, "2": 3, "3": 4, "4": 6, "5": 8}` | 레벨별로 세션 하나가 쓴다고 보는 메모리. 측정값이 아닌 추정치다 |
| `ram_gb_per_worker` | `1.5` | 세션 안 워커 하나가 쓰는 메모리. 권장 워커 수의 상한을 정한다 |
| `max_workers` | `6` | 세션 하나에 권장하는 병렬 워커 수의 상한 |
| `primary_provider` | `"anthropic"` | 토큰 사용 속도 계산의 기준이 되는 모델 공급자 |
| `collect_pages` | `3` | `collect`와 `tick`이 출처마다 읽는 목록 쪽수의 기본값(1 이상) |
| `collect_skip` | `[]` | 끌 출처 이름 목록. 예: `["devpost", "kaggle"]` |
| `auto_launch` | `false` | `true`면 `ach tick`이 통과한 공모전을 용량만큼 라우팅·실행한다. 게이트 판정을 믿을 수 있을 때까지는 꺼 둔다 |
| `gate_batch` | `16` | 정기 실행 한 번에 병렬로 게이트하는 `discovered` 공모전 수의 상한. 실제 수는 `primary_provider`의 토큰 사용 속도로 정한다: mode가 maxing이면 이 값 전부, steady면 절반, conserve면 4개, throttle이면 0개 |
| `persona.enabled` | `false` | 메인 세션에 운영자 페르소나를 넣을지. 공모전 세션에는 넣지 않는다 |
| `persona.file` | `"data/persona.md"` | 페르소나 글 파일(git 제외). 예시는 `data/persona.example.md` |
| `persona.call_user` | `""` | 페르소나가 사용자를 부르는 말. 페르소나 검사 대상에도 들어간다 |
| `persona.guard_terms` | `[]` | 결과물과 커밋 메시지에 있으면 안 되는 낱말: 페르소나 이름, 호칭, 말투 표지 |
| `schedule.cron` | `"0 */6 * * *"` | 정기 실행 시각. 이 컴퓨터의 현지 시각 기준 다섯 칸 cron 식(분 시 일 월 요일)이다. `ach schedule install`이 적용한다 |
| `schedule.scheduler` | `"orca"` | 정기 실행을 띄우는 것: `orca`(러너의 Orca 에이전트를 띄우는 Orca automation), `launchd`(`ach autorun`을 띄우는 launchd 작업, macOS 전용), `cron`(`ach autorun`을 띄우는 사용자 crontab 항목) |
| `schedule.scout_hours` | `"0-5"` | 정기 실행이 `ContestScout` 웹 검색까지 하는 현지 시각. cron의 시 칸처럼 쓴다(`0-5`, `0,12`, `*`). 비우면 하지 않는다. 판정은 `ach tick`이 `scout`로 알린다 |
| `agent_models` | `{}` | 하네스 에이전트(`agents/<이름>.md`)마다 쓸 모델. 러너 문법으로 쓴다(omp: `provider/model:effort` 또는 `@역할`). `ach install`이 러너에 적용하고, `null`은 하네스가 전에 적용한 선택을 지운다. 없는 에이전트 이름은 `install`이 거절하고 `doctor`가 실패로 알린다. [에이전트별 모델](../README_ko.md#에이전트별-모델) 참고 |
| `delegation` | `{"enabled": false, "granted_at": "", "note": "", "contest_ids": [], "allow_personalization": false, "video_score_exempt": false, "concept_selection": false}` | 제출 승인 상시 위임. 기본은 꺼져 있다. `enabled: true`는 `granted_at`과 `note`가 비어 있지 않아야 받아들인다. `contest_ids`가 비면 모든 공모전, 아니면 그 id만이다. `allow_personalization`은 위임 승인 뒤 기존 개인 항목과 기존 서명을 별도 최종 사본에 넣는 것을, `video_score_exempt`는 영상 공모전의 위임 승인에서 점수 조건 면제를, `concept_selection`은 창작물 콘셉트 카드 선택 위임을 켠다. 참가비, 새 계정, 새 동의, 새 서명, 레벨 5 계획 승인은 위임되지 않는다. 위임 승인에는 공식 규칙 추적과, 레벨 2 이상(영상 면제 제외)에서 검증된 독립 평가에 묶인 최종 점수가 더 필요하다([0.8.0 지속 작업 흐름](#080-지속-작업-흐름) 참고). 공모전 세션은 바꿀 수 없다. 철회는 `ach config set delegation.enabled false`이고, 철회하면 아직 쓰지 않은 위임 승인으로는 제출 의도·`submission start`·`ach profile show`를 할 수 없다. 이미 시작했거나 확인된 제출 기록은 그대로 남는다. [CLI가 강제하는 승인](../README_ko.md#cli가-강제하는-승인) 참고 |
| `new_work_frozen` | `false` | 새 작업 동결. `true`면 `collect`, `add`, `gate`, `route`, `launch`(`--force`도), `tick`의 수집·자동 실행, 대기열 컨트롤러의 새 시작을 거절한다. 체크포인트, 재조정, 진행 중 공모전 복구, 승인, 제출 트랜잭션, 정리는 그대로 된다. 사용자(또는 사용자의 직접 지시를 받은 메인 세션)만 `ach config set new_work_frozen true\|false`로 바꾸고, 풀어도 대기 중인 공모전을 저절로 시작하지 않는다 |

<p>표 3. 설정 키</p>

`ach capacity`의 계산 방식:

- **토큰 사용 속도.** `omp usage --json`이 알려 주는 사용 한도 창마다 pace = 남은 한도 비율 ÷ 남은 시간 비율을 구한다. 가장 빠듯한 창을 기준으로 pace가 1.5 이상이면 `max_workers`(maxing), 1~1.5면 그 절반(steady), 1 미만이면 그보다 줄인다(conserve). 남은 한도가 10% 이하이면 워커 1명만 권장하고 새 세션은 띄우지 않는다(throttle). omp가 없으면 메모리와 개수 상한만 적용한다.
- **메모리.** 새 세션의 `ram_gb_by_level`이 메모리 예산 안에 들어갈 때만 띄운다. 쓸 수 있는 메모리는 macOS에서 `vm_stat`, Linux에서 `/proc/meminfo`의 `MemAvailable`로 읽는다. memory pressure가 `warning` 이상이면 새 세션을 띄우지 않는다. macOS는 memory pressure 수준을, Linux는 `/proc/pressure/memory`의 `some avg10`을 본다(10 이상 `warning`, 40 이상 `critical`, 파일이 없으면 알 수 없음). 메모리를 읽을 수 없으면 메모리 상한은 건너뛰고(개수·토큰 상한은 그대로 적용한다) `ach capacity --json`이 `ram.basis: "unavailable"`, null인 `ram.available_gb`·`ram.budget_gb`, `ram.warning`을 낸다. 평소에는 `ram.basis`가 `"measured"`다.
- **개수 상한.** 토큰·메모리 조건 위에 `max_running`과 `max_running_by_level`을 함께 적용한다. `running`과 `plan_review` 공모전을 진행 중으로 센다.

## CLI 명령

모든 명령은 `--json`을 받는다. CLI 메시지와 `BOARD.md`는 한국어다. 종료 코드: `0` 성공, `1` 오류(이유는 표준 오류로 낸다. 페르소나·점수 관문 거절과 제출 준비 기록·위임 승인·제출 검사의 거절도 포함), `2` 사용법 오류(인자 해석기가 알리는 잘못된 인자), `3` 중복 후보 있음(`dedupe-check`, `--force` 없는 `add`), `4` 페르소나 흔적 있음(`check`).

| 명령 | 하는 일 |
|---|---|
| `ach --version` | 버전과 바이너리를 빌드한 커밋을 보여 준다: `<버전> (<12자리 sha>)` |
| `ach doctor` | 클론, 설정, 도구, 정기 실행을 점검한다. 실패가 하나라도 있으면 종료 코드 1. [설치 점검](#설치-점검ach-doctor) 참고 |
| `ach init` | 새 클론에 필요한 것을 만든다: `data/`(0700. 이미 있는 `data/`를 다른 사용자가 열 수 있으면 0700으로 좁힌다), 커밋된 예시에서 `data/config.json`과 `data/profile.md`(0600), `data/logs/`. 있는 파일은 그대로 둔다. 끝에 점검을 돌린다. JSON은 `{created: [...], kept: [...], tightened: [...], checks: [...], next: "<루트>/INIT.md"}`. 종료 코드 0 |
| `ach config path` | 설정 파일 경로(`data/config.json`)를 보여 준다 |
| `ach config get [KEY]` | 점으로 이은 키의 적용되는 값(기본값 + 파일), 또는 설정 전체를 보여 준다 |
| `ach config set <KEY> <VALUE>` | 점으로 이은 키를 정한다. 값은 JSON으로 읽히면 JSON, 아니면 문자열이다. 파일을 쓰기 전에 설정 전체를 검사하고(원자적 쓰기, 0600), 틀린 값은 거절하며 파일은 바뀌지 않는다 |
| `ach config unset <KEY>` | 파일에서 키를 지워 기본값으로 되돌린다. 파일에 없는 키면 오류다 |
| `ach agents` | `agents/*.md`의 에이전트를 보여 준다: 이름, 설명, 기본 모델(frontmatter `model`), `agent_models`에서 고른 모델, 러너가 지금 적용한 모델. JSON은 `[{name, description, default_model: [...], configured, applied}]` |
| `ach collect [--source <이름>\|all] [--pages N]` | 새 공고를 모아 사이트 간 중복을 병합하고, 새 공모전을 `discovered`로 넣는다. `--pages`(기본 `collect_pages`)는 1 이상이어야 한다 |
| `ach add --title T --url U [--org O] [--deadline D] [--source scout\|manual] [--note N] [--force]` | 공모전 한 건을 넣는다. URL은 http(s)여야 한다. `--deadline`은 `YYYY-MM-DD`(`YYYY-MM-DDT23:59+09:00`으로 저장) 또는 `YYYY-MM-DDTHH:MM[:SS]+09:00`을 받는다(`24:00`은 23:59로 읽고, 다른 오프셋은 거절한다). 이미 있는 id면 `-2`, `-3` 같은 꼬리를 붙인다. 중복으로 보이면 새로 만들지 않고 후보를 알려 준다 |
| `ach add --from-file <json> [--source scout\|manual] [--force]` | JSON 객체 하나나 `{title, url, org?, deadline?, source?, note?, categories?}`의 배열을 넣는다. 무엇이든 쓰기 전에 모든 항목을 검증하고, 잘못된 항목이 있으면 `<경로>: [i] ...`로 파일 전체를 거절한다. JSON 출력은 `{"added": [...], "duplicates": [{title, url, matches}]}`다. 중복 후보로 보류한 항목이 하나라도 있으면 종료 코드 3 |
| `ach dedupe-check ["<제목>"] [--id ID] [--url U] [--org O] [--deadline D]` | 제출 기록부, 레지스트리, 공모전 폴더에서 같은 공모전을 찾는다. 기록부 일치는 `kind: submitted`로 맨 앞에 둔다. `--id`를 주면 그 항목은 대조에서 빼고 그 항목의 제목·URL·주최·마감을 쓰므로(플래그가 우선) 제목을 생략할 수 있다 |
| `ach list [--state S] [--type T]` | 마감 순으로 목록을 보여 준다 |
| `ach show <id>` | 레지스트리 항목 하나를 보여 준다 |
| `ach gate <id> --file gate.json` | 게이트 결과를 기록하고 `gate_pass`·`rejected`·`hold` 가운데 하나로 옮긴다 |
| `ach route <id> [--folder 이름] [--no-repo] [--no-workspace]` | 사전 점검을 거친 뒤 폴더·저장소·작업 공간·`CONTEST.md`를 만들고 `routed`로 옮긴다. 실패한 뒤 다시 실행하면 이어서 한다. 제출 기록부에 있는 공모전은 거절한다. [라우팅과 실행](#라우팅과-실행) 참고 |
| `ach launch <id> [--preset 이름] [--dry-run] [--force]` | 설정한 러너를 설정한 작업 환경에서 레벨 지시문으로 띄우고 `running`으로 옮긴다. 그 레벨에 빈 자리나 메모리가 없으면 거절한다(`--force`로만 넘긴다). `--preset`은 omp 전용이다. `--dry-run`은 아무것도 쓰지 않고 명령과, 러너·작업 환경 실행 파일이 없을 때의 경고를 보여 준다. JSON에는 `prompt`(실제 실행이 쓸 경로), `prompt_text`, `warnings`가 있다 |
| `ach peek <id> [--lines N]` | 세션 화면이나 로그의 마지막 N줄(기본 40)을 보여 준다 |
| `ach check <id>` | 공모전 폴더와 커밋 메시지에서 페르소나 흔적을 찾는다 |
| `ach transition <id> <상태> [--note N \| --note-file F] [--by 누가] [--persona-ok 사유] [--score-ok 사유]` | 전이 규칙, 승인 규칙, 점수 관문, 페르소나 검사를 지키며 상태를 옮긴다. `plan_review → running`(`approve`만), `→ submitted`(`submit`만), `submitted`·`followup_pending → running`(`resume`만)은 거절한다. `--score-ok`는 이력 항목에 타입이 있는 점수 미달 표시를 남긴다. 인정되는 것은 이 표시뿐이고, 그것도 사용자 승인에서만 통한다. 위임 승인에서는 통하지 않고, 노트 문구로는 점수 관문을 넘을 수 없다 |
| `ach readiness <id> [--file F] [--personalized]` | `--file`이 없으면 지금 제출 준비 기록을 보여 준다. `--file`이 있으면 다섯 필드 `submission_ready`, `needs_user_input`, `blockers`, `artifacts`(`[{path, sha256}]`, 공모전 폴더 안의 일반 파일이고 해시가 지금 파일과 같아야 하며, 레벨 2 이상은 최근 `final` 점수의 `artifacts` 안), `followup_required`와 `destination`(공식 제출 URL 또는 메일 주소 그대로. 0.8.0부터 새 제출 의도에 필요)을 검사해 지금 작업 판에 기록한다. `running`·`awaiting_approval`에서만 되고, 새로 기록하면 이전 기록에 대한 승인은 효력을 잃는다. `--personalized`는 유효한 제출 승인 뒤 승인된 별도 최종 사본을 기록한다. 파일에 `personalization_of`(승인된 `request_token`)를 더하고, `artifacts`의 개인 항목 사본마다 `source`(채운 승인된 원본의 상대 경로)를 적는다. 바뀌지 않은 원본은 `source` 없이 자기 경로와 해시로 적어도 된다. 승인된 원본 하나하나가 정확히 한 번씩 대응해야 하고 사본이 원본을 덮어쓰면 안 된다. `followup_required`와 `destination`은 승인된 기록과 같아야 한다(바꾸려면 일반 제출 준비 기록을 다시 남기고 새로 승인받는다). 위임 승인이면 `delegation.allow_personalization`도 켜져 있어야 한다 |
| `ach approval-status <id>` | `{id, revision, request_token, submit_approval_valid, eligible, reasons, readiness, latest_final}`와 페르소나·위임 정책 검사를 보여 준다. 0.8.0부터 `reasons`에 규칙 코드(`rules_pending_review`, `rules_gate_invalidated`, `rules_plan_invalidated`, `rules_final_invalidated`, 위임 승인에만 `rules_untracked`), 위임 승인의 독립 평가 코드(`independent_final_missing`, `independent_final_stale`), 제출 시도 코드(`submission_intent_pending`, `submission_dispatching`, `submission_unknown`)가 들어갈 수 있다 |
| `ach approve <id> (--note N \| --note-file F) [--by 누가]` | 사용자의 승인을 기록한다. `plan_review`에서는 `plan` 승인을 남기고 `running`으로 옮기며, `awaiting_approval`에서는 지금 작업 판·제출 준비 기록·점수에 묶인 `submit` 승인을 남긴다(상태는 그대로). `ACH_SESSION`이 있으면, 곧 공모전 세션 안에서는 거절한다 |
| `ach approve <id> --delegated --expected-request 토큰 (--note N \| --note-file F)` | 설정 `delegation` 범위 안의 위임 `submit` 승인. 메인 세션이 `approval-status`의 `request_token`으로 실행한다. 레지스트리 잠금 안에서 자격, 페르소나 검사, 위임 범위, 토큰을 다시 확인하고 하나라도 어긋나면 기록하지 않는다. 계획 승인은 할 수 없고, `--score-ok` 점수 미달은 받아들이지 않는다(영상 공모전은 `video_score_exempt`일 때만 면제). 공모전 세션 안에서는 거절한다 |
| `ach submission intent <id> --file F` | 제출 의도를 기록한다: `{"method": "form\|email\|site", "destination": "<승인된 제출 준비 기록의 destination과 같은 값>", "expected_request": "<request_token>"}`. 승인·규칙·파일 해시를 다시 확인하고 승인된 `artifacts`·작업 판·규칙 기준을 새 `attempt_id`(`a1`, `a2`, …)에 묶는다. 같은 요청은 같은 시도를 돌려주고, 다른 요청이나 `dispatching`·`unknown` 시도가 있으면 거절한다 |
| `ach submission start <id> --attempt A` | 외부 발송 전에 의도를 `dispatching`으로 옮겨 저장한다. 승인·토큰·제출처·파일·규칙·작업 판을 다시 확인한다. 두 번째 시작은 늘 실패하고(종료 코드 1) 다시 보내도 된다는 뜻이 아니다 |
| `ach submission unknown <id> --attempt A --file F` | 발송 결과를 모름을 기록한다(`{"attempt_id", "reason": "timeout\|response_lost\|crash\|ambiguous_response\|other", "observed_at", "evidence"?}`, `dispatching`에서만). 새 의도와 재발송을 막는다 |
| `ach submission resolve <id> --attempt A --file F` | 접수되지 않았다는 긍정 증거를 기록한다(`{"attempt_id", "verified_not_submitted": true, "source": "portal_listing\|organizer_reply\|mailbox_search", "observed_at", "artifacts"}`, `.private/` 아래 증거 하나 이상, `dispatching`·`unknown`에서). 이것만 새 의도를 다시 연다. `--force`는 없다 |
| `ach submission status <id>` | `{tracked, unresolved, latest, attempts, blockers, review_needed}`. 접수 문구·제출처·증빙 내용은 내지 않는다 |
| `ach submit <id> --attempt A --file <proof.json>` | 시작한 시도를 접수 증빙으로 확인하고 `submitted`로 옮긴다(이리로 가는 유일한 길). 같은 쓰기에서 제출 기록부에도 더한다. 증빙: `{"attempt_id", "receipt": "<접수 번호·참조 한 줄>", "source": "confirmation_page\|receipt_email\|portal_listing\|organizer_reply", "observed_at", "destination", "submitted_sha256": [...], "artifacts": [{path, sha256}]}`. `dispatching`·`unknown` 시도만 받고 `intent`는 거절한다. 증빙 파일은 `.private/submissions/<A>/`에 읽기 전용 사본으로 남고 메타데이터에는 접수 참조와 해시만 남는다. 같은 증빙으로 다시 실행하면 빠진 기록부 기록만 되살리고, 다른 증빙은 거절한다. 그사이 작업 흐름이 바뀌었으면 증빙과 기록부는 남기되 상태는 바꾸지 않고 `review_needed`를 낸다 |
| `ach score <id> --stage plan\|draft\|final --file <judge.json> [--note N \| --note-file F]` | `ContestJudge` 점수를 지금 작업 판에 기록한다. 기준은 `gate.json`과 정확히 같아야 하고(심사 단계, 이름, 공개 배점), 배점은 0보다 크며, 심사 단계와 이름이 겹치면 안 되고, `total`은 환산한 합과 0.5 넘게 다르면 안 된다. `plan` 점수는 공모전을 시작할 때 폴더 `RUBRIC.md`의 `## 0. 채점 근거` 절에 문서화한 근거(주최·주관·개최 목적·심사기준 네 행, 행마다 내용과 공고 원문 위치, 템플릿 자리표시 없음), Judge가 낸 `basis: {organizer, purpose, criteria_source}`, 채점한 Judge 실행 `evaluator: {id, model, run_id}`가 있어야 하고, 이 공모전에 이미 기록된 `run_id`면 거절한다(매번 새 실행). 기록에는 그 절의 `{path, sha256}`이 `basis.document`로 남는다. `final` 점수는 `artifacts: [{path, sha256}]`가 있고 해시가 지금 파일과 같아야 하며, `evaluation_id`·`assessment_id`가 있으면 그 독립 평가와 정확히 같아야 하고 묶음이 남는다. 모든 점수에 지금 `rules_binding`이 붙고, 새 `plan`·`final` 점수는 그 단계의 규칙 무효를 푼다. 승인 뒤 새 `final`을 기록하면 그 승인은 효력을 잃는다. `--stage`가 없으면 점수 이력을 보여 준다 |
| `ach submitted` | 제출 기록부 목록: 날짜, 제목, 방법, 접수 증빙 |
| `ach submitted add --title T [--url U] [--org O] [--date D] [--method M] [--receipt R] [--folder F] [--note N]` | 하네스 밖에서 낸 공모전을 기록한다(보관용 기록. 하네스 제출 시도를 시작·확인·해소하지 않는다) |
| `ach submitted import` | 레지스트리의 제출 항목을 기록부로 한 번 옮긴다. 이미 있는 key는 건너뛴다 |
| `ach clean <id> [--dry-run] [--purge] [--force]` | 제출이 끝난 공모전의 세션·작업 공간을 닫고 로컬 폴더를 휴지통으로 옮긴다(`--purge`면 완전히 지운다). 정리 조건을 모두 만족하지 않으면 거절한다(`--force`는 검사를 건너뛰지만 풀리지 않은 제출 시도는 건너뛰지 못한다. 이때 폴더가 없으면 중단하지 않고 문제로 알린다) |
| `ach clean --all [--dry-run]` | 폴더가 아직 남아 있는 `submitted`·`closed` 공모전 전부에 `clean`을 돌린다. 실패한 공모전은 실패 결과로 남기고, 나머지는 그대로 정리해 저장한다 |
| `ach restart <id> [--preset P] [--restore UUID] [--dry-run]` | `running`·`awaiting_approval`·`plan_review` 세션을 같은 대화 기록으로 다시 띄운다(프리셋 바꾸기 등). `--restore`가 없으면 저장된 세션 기록이나 하나뿐인 러너 기록을 쓰고, 찾지 못하면 기록을 버리지 않고 거절한다. 상태와 작업 판은 바뀌지 않는다 |
| `ach resume <id> [--preset P] [--restore UUID] [--dry-run]` | `submitted`·`followup_pending` 공모전의 실제 후속 작업을 재개한다. `data/launch.lock` 아래에서 용량을 확인하고 세션을 연 뒤에만 `running`으로 옮긴다(새 작업 판) |
| `ach session-close <id> --expected-state S --expected-history N [--dry-run]` | 끝난 공모전(`submitted`·`followup_pending`·`closed`·`skipped`·`duplicate`)의 세션만, 상태와 이력 길이가 기대값과 같을 때 닫는다. 폴더는 지우지 않는다. 공모전 세션 안에서는 거절한다 |
| `ach profile path` | 사용자 정보 파일 경로(`data/profile.md`)와 파일이 있는지 |
| `ach profile eligibility [--json]` | `## 참가 자격` 절만 보여 준다. 게이트에 필요한 자격 정보이고 식별 값은 없다. JSON은 `{path, eligibility}`. 이 명령과 `profile show`는 group이나 other가 파일을 읽을 수 있거나 참가 자격 절에 예시 자리표시가 남아 있으면 표준 오류로 경고한다 |
| `ach profile show --for <id> [--json]` | 개인정보를 포함한 파일 전체를 보여 준다. 유효한 제출 승인이 있는 `awaiting_approval` 공모전이나 이미 `submitted`인 공모전에만 쓰고, 공모전 세션 안에서는 그 세션의 id에만 쓴다. JSON은 `{path, text}` |
| `ach budget` | 공급자별 남은 한도, pace, 권장 워커 수를 보여 준다 |
| `ach capacity` | 지금 띄울 수 있는 세션 수를 보여 준다. JSON 필드: `slots`, `slots_by_level`, `recommended_workers`, `running`, `running_by_level`, `ram`(`basis`, `available_gb`, `reserve_gb`, `budget_gb`, `pressure`, `warning`), `limits`, `providers` |
| `ach board` | 저장된 기록만으로(네트워크·백엔드 호출 없음) 할 일 중심 `BOARD.md`를 쓴다([보드](#보드) 참고) |
| `ach tick [--no-collect] [--auto-launch]` | 제어 루프를 한 번 돈다: 수집하고, 게이트 대기 목록을 내고, 선택하면 용량 안에서 라우팅·실행하고, `BOARD.md`를 갱신한다. `new_work_frozen`이면 수집도 실행도 하지 않는다. 정기 실행이 JSON을 읽을 수 있게 종료 코드는 0이다. 텍스트는 오류 건수로 시작하고, JSON에는 `warnings`와 `scout`(지금 현지 시각이 `schedule.scout_hours` 안인지)가 더해진다 |
| `ach contract [--json]` | 이 바이너리의 인터페이스 계약 `{protocol: 2, version: "0.8.2", capabilities: [...]}`. 컨트롤러는 무엇이든 쓰기 전에 `protocol == 2`를 확인한다 |
| `ach checkpoint <id> [--file F]` | 단계 체크포인트를 기록하거나, `--file` 없이 마지막 체크포인트를 보여 준다 |
| `ach reconcile [<id>] [--apply]` | 기록된 세션과 작업 공간을 관측한다. 기본은 읽기 전용, `--apply`는 관측만 저장한다 |
| `ach recover <id> --expected-observation TOKEN [--dry-run]` | 복구 조건을 모두 만족할 때만 사라진 세션을 저장된 대화로 다시 연다 |
| `ach rules capture\|review\|acknowledge\|status <id> ...` | 공식 규칙 원문 보관, 구조화 검토, 단계 재검토, 상태 |
| `ach evaluation prepare\|record\|report\|outcome <id> ...`, `ach evaluation calibration` | 블라인드 평가 패킷, 독립 평가, 분산 보고, 공식 결과, 관측 표본 집계 |
| `ach controller <approve\|queue\|briefs\|preset> [인자...]` | 체크아웃에서 셸 없이 `python3 -m controllers ...`를 띄운다 |
| `ach install [--runner R ...] [--all]` | 설정한 러너, 지정한 러너들, 또는 CLI가 `PATH`에 있는 코어 어댑터 전부에 에이전트·스킬·확장을 설치하고, 설정한 러너에 `agent_models`를 적용한다. `agent_models`에 없는 에이전트 이름이 있으면 거절한다 |
| `ach chat [--runner R] [--workspace W]` | 저장소 루트에서 메인 인터페이스 세션을 연다 |
| `ach autorun` | 설정한 러너의 한 번 실행 명령으로 정기 실행을 한 번 돌린다(launchd·cron 작업이 띄우는 명령) |
| `ach schedule install [--scheduler orca\|launchd\|cron]` | `schedule.cron`으로 정기 실행을 그 스케줄러(기본 `schedule.scheduler`)에 만들거나 고친다. 다른 스케줄러에도 작업이 있으면 경고한다 |
| `ach schedule remove [--scheduler orca\|launchd\|cron]` | 그 스케줄러에서 정기 실행을 지운다 |
| `ach schedule status` | 설정한 일정과 Orca·launchd·cron에 실제로 걸린 것: cron, 켜짐·적재 여부, 다음·마지막 실행, 마지막 종료 코드와 로그, 설정과 맞는지(`in_sync`, 설정한 스케줄러가 아니면 null). `--json` 없이는 짧은 요약을 보여 준다 |

<p>표 4. 명령</p>

## 정기 실행

정기 실행은 매번 [harness/prompts/automation.md](../harness/prompts/automation.md)의 절차를 따른다:

1. `ach tick`으로 공고를 수집하고 보드를 갱신한다(`new_work_frozen`이면 수집·실행을 하지 않고 2~5단계를 건너뛴다). 이어서 `ach reconcile --json`으로 기록된 세션을 읽기 전용으로 관측한다. 런타임이 `missing`·`unknown`·`ambiguous`인 공모전은 보고만 하고 정기 실행이 다시 띄우지 않는다.
2. `discovered` 글을 1차로 걸러, 분명히 해당하지 않는 글을 `skipped`로 옮긴다. 애매하면 게이트에 남긴다. 거르는 글은 세 가지다:
   - 공모전이 아닌 글. 서포터즈, 기자단, 교육 과정, 체험단, 채용 모집, 행사 참가 안내, 댓글 이벤트가 여기에 든다.
   - 제목·note만 보고도 사용자 정보의 자격 값(`ach profile eligibility --json`)과 대조해 자격이 분명히 안 되는 글. 초·중·고생 전용, 특정 지역 거주자·재직자·소속 전용, 현장 참석·촬영 필수, 참가비 필수, 한국 참가를 제외한 해외 대회 같은 경우다.
   - 마감이 지난 글.
3. 이번 회차의 게이트 수를 정한다. `ach capacity --json`에서 `limits.basis` 공급자의 `mode`를 읽어 maxing이면 `gate_batch`개, steady면 그 절반, conserve면 4개, throttle이면 0개를 게이트한다. 남은 `discovered` 공모전을 마감이 가까운 순으로 그만큼 골라, 공모전마다 `ContestGate` 하나씩 병렬로 돌린다.
4. 1단계 `tick`이 `scout: true`를 냈을 때만(현지 시각이 `schedule.scout_hours` 안, 기본 00:00~05:59) `ContestScout`로 웹을 탐색한다.
5. `ach tick --no-collect`를 한 번 더 돌린다. `auto_launch`가 `true`면 통과한 공모전을 전체·레벨별 실행 상한과 메모리 예산 안에서 라우팅·실행하고, 나머지는 `waiting`으로 알린다.
6. `ach clean --all`로 작업이 안전하게 push된 제출 완료 공모전을 정리한다([제출 뒤 정리](#제출-뒤-정리) 참고). 풀리지 않은 제출 시도가 있는 공모전은 정리하지 않는다.
7. 세션이 다섯 줄 이내로 보고한다. 보고 내용은 새로 통과한 공모전(마감·레벨·우선순위), 보류와 그 질문, 계획 검수 대기(`plan_review`), 실행한 것, 정리했거나 막힌 것, 오류다.

### 실행 시각

`data/config.json`에 시각을 적고 적용한다:

```json
"schedule": {"cron": "0 9,21 * * *", "scheduler": "orca", "scout_hours": "21"}
```

```bash
ach schedule install     # schedule.cron으로 정기 실행을 만들거나 고친다
ach schedule status      # Orca·launchd·cron에 걸린 것과 설정이 맞는지
ach schedule remove      # 지운다(다른 스케줄러는 --scheduler orca|launchd|cron)
```

`cron`은 이 컴퓨터의 현지 시각 기준 다섯 칸(분, 시, 일, 월, 요일)이다. 칸마다 `*`, 숫자, 범위 `a-b`, 간격 `*/n`·`a/n`·`a-b/n`, 또는 이것들의 쉼표 목록을 쓴다. 요일은 0~7이고 0과 7이 모두 일요일이다. 이름(`MON`, `JAN`)과 매크로(`@daily`)는 받지 않아 모든 스케줄러가 같은 식을 읽는다. cron과 같이 일과 요일을 둘 다 제한하면 둘 중 하나만 맞아도 돈다. `cron`을 바꾼 뒤에는(Orca면 `runner`를 바꾼 뒤에도) `schedule install`을 다시 실행한다. 그 전까지 `status`는 `in_sync: false`를 보인다.

**Orca automation**(`scheduler: "orca"`, 기본값). 하네스는 프롬프트가 이 클론의 `harness/prompts/automation.md`를 가리키는 Orca automation을 찾아 trigger, 에이전트(러너의 Orca 에이전트, `omp`), 켜짐 여부를 고친다. 없으면 `orca_repo_path`에 `Autocontest-Harness`라는 Orca 작업 공간을 만들고, 같은 이름의 automation을 현지 시간대로 만든다. `remove`는 automation만 지우고 작업 공간은 남긴다. 지금 한 번 돌리려면 `orca automations run <id>`를 쓴다.

**launchd**(`scheduler: "launchd"`, macOS 전용이며 다른 곳에서는 거절한다). `ach autorun`은 설정한 러너의 한 번 실행 명령으로 저장소 루트에서 이 절차를 한 번 돌리고, 출력을 `data/logs/autorun-<시각>.log`에 남긴다. `schedule install`은 cron을 `StartCalendarInterval` 항목으로 바꾼 `~/Library/LaunchAgents/<라벨>.plist`를 쓰고 불러온다. 라벨은 `io.github.fmsongx2.autocontest-harness.<16진수 8자리>`이고, 끝자리는 클론 경로의 해시라 클론 두 개가 부딪히지 않는다. 예전 고정 라벨의 plist는 `status`가 `launchd.legacy`로 알리고, 이 클론의 것이면 `install`이 정리한다. 작업에는 `ACH_HOME`(클론 위치)과 설치 시점의 `PATH`가 고정되므로, 도구 위치가 바뀌면 `schedule install`을 다시 실행한다. 일 칸의 `*` 간격과 요일을 함께 건 식(`0 8 */2 * 1`)은 launchd로 나타낼 수 없어 거절한다.

**cron**(`scheduler: "cron"`, macOS와 Linux). `schedule install --scheduler cron`은 사용자 crontab에서 `# BEGIN autocontest-ach <루트>`와 `# END autocontest-ach <루트>` 사이의 이 클론 블록을 새로 쓰고, 다른 줄은 건드리지 않는다. 작업은 `<cron> cd <루트> && ACH_HOME=<루트> PATH=<설치 시점 PATH> <harness 실행 파일> autorun >> <루트>/data/logs/cron.log 2>&1`이다. `remove`는 그 블록만 지운다. `status`는 `crontab: {available, installed, cron, in_sync, next_run, log}`로 알린다.

`--json` 없이 `schedule status`를 실행하면 짧은 요약을 보여 준다. 설정 한 줄 뒤에 Orca·launchd·cron마다 설치 여부, 설정과 맞는지, 다음 실행을 적는다.

스케줄러는 하나만 둔다. 둘이면 정기 실행이 두 번 돌아 같은 공모전을 게이트·실행한다. `schedule install`은 다른 스케줄러에도 작업이 있으면 경고하고, `ach doctor`도 알린다.

## 라우팅과 실행

`ach route <id>`는 `gate_pass` 공모전에 폴더 `<contests_root>/<Folder>`와 `CONTEST.md`, private GitHub 저장소, 작업 공간을 만들고 `routed`로 옮긴다.

- **사전 점검.** 무엇이든 쓰기 전에 작업 환경 백엔드(와 `orca_repo_path`가 있는지)를 `--no-workspace`가 없으면, `PATH`의 `gh`와 `gh auth status`를 `--no-repo`가 없으면, 그리고 git 작성자 정보를 확인한다. 오류마다 고치는 법을 적는다.
- **이어서 하기.** `.harness/`를 만든 바로 뒤에 `.harness/contest.json`을 쓰므로, 어디서 실패하든 `route`를 다시 실행하면 이어서 한다. 실패한 라우팅은 항목을 그대로 둔다(`gate_pass`, 폴더 없음).
- **`.gitignore`.** 공모전 폴더의 `.gitignore`에 `.DS_Store`, `.venv/`, `__pycache__/`, `.private/`, `.harness/*-preset.*`, `.harness/launch-prompt.md`, `.harness/session.log`, `.harness/contest.json`을 넣는다. `.gitignore`가 이미 있으면 빠진 줄만 덧붙인다. 첫 커밋 전에 `contest.json`을 쓰므로 `route` 뒤 작업 트리는 깨끗하다.
- **이미 있는 저장소.** 폴더 이름과 같은 GitHub 저장소가 이미 있으면 private이어야 하고, 비어 있거나 이 공모전의 것(그 저장소의 `.harness/contest.json`, 또는 `CONTEST.md`의 `| 하네스 id | <id> |` 행)이어야 한다. 이력이 다른 원격 `main`은 거절한다.
- **GitHub Actions 끄기.** 저장소를 만든 뒤 GitHub Actions를 끈다(`gh api -X PUT repos/<소유자>/<저장소>/actions/permissions -F enabled=false`). 여기서 실패하면 경고이고, route JSON의 `warnings`에 남는다.
- `existing_contests.md`와 커밋 메시지에 쓰는 제목은 한 줄로 만든다.

`ach launch <id>`는 설정한 러너를 설정한 작업 환경에서 레벨 지시문으로 띄운다. 모든 공모전 세션은 `env ACH_SESSION=<id> PATH=<클론>/bin:… <명령>`으로 돈다([CLI가 강제하는 승인](../README_ko.md#cli가-강제하는-승인) 참고). `data/launch.lock`이 용량 확인과 실행을 차례로 묶으므로 두 실행이 함께 상한을 통과하지 못한다. `--dry-run`은 아무것도 쓰지 않는다.

`route`, `launch`, `tick`은 네트워크·세션 작업을 배타적 레지스트리 잠금 밖에서 항목 사본으로 하고, 그동안 항목의 상태와 이력이 바뀌지 않았을 때만 결과를 저장한다. `collect`의 마감 갱신이나 새 점수 같은 동시 수정은 그대로 남는다. `route`가 성공하고 이어진 실행이 실패하면 항목은 `routed`에 머문다.

`headless` 세션은 `pid:<pid>/<시작 시각>:<로그>`로 기록한다. 닫을 때는 그 프로세스 그룹에만, 그 pid의 시작 시각이 기록과 같을 때만 신호를 보내므로, 재사용된 pid는 건드리지 않는다.

## 0.8.0 지속 작업 흐름

필드와 코드의 정본은 [CONTRACT.md](CONTRACT.md) 17~22절이다. 이 절은 실행 예시와 함께 요약한다. 입력은 모두 `write`로 쓴 JSON 파일(공모전 폴더 `.private/` 또는 `mktemp -d` 폴더)로 넘기고, 명령줄에는 id, `request_token`, `attempt_id`, 관측 토큰만 쓴다. `ach`는 파일 내용을 셸 명령으로 해석하지 않는다.

### 체크포인트·재조정·복구

레지스트리 `state`는 작업 흐름 상태다. 세션이 실제로 살아 있는지는 따로 관측하고, 관측하지 못한 세션을 정상으로 보이지 않는다.

```json
{"stage": "draft", "completed_artifacts": [{"path": "drafts/proposal.md", "sha256": "<64자리 16진수>"}], "next_action": "기준 3의 근거 표를 보강한 뒤 draft 채점을 받는다", "blocked_reason": null, "needs_user_input": false, "next_action_at": "2026-10-26T09:00+09:00"}
```

- `ach checkpoint <id> --file .private/checkpoint.json`(`running`·`awaiting_approval`·`plan_review`): 파일 해시를 다시 확인하고 지금 작업 판에 기록한다. `stage`는 `research`, `plan`, `draft`, `review`, `submission`. 모르는 키는 거절한다. `next_action`(1000자 이하)은 실행하지 않고 보드에도 옮기지 않는다. `ach checkpoint <id>`는 마지막 체크포인트를 보여 준다.
- `ach reconcile [<id>] [--apply]` → `{entries: [{id, workflow_state, runtime_state, observed_at, observation_token, checkpoint_valid, checkpoint_seq, artifact_integrity, approval_integrity, blocked_reasons, next_action, next_action_at, needs_user_input, recoverable}], changed, conflicts, applied}`. `runtime_state`는 `running`, `missing`, `ambiguous`, `unknown`, `not_started`, `stopped`이고 `unknown`은 `missing`이 아니다. 띄우거나 닫지 않고, 기록되지 않은 터미널을 받아들이지 않는다. `--apply`는 상태·이력·세션 CAS 아래에서 관측만 저장하고, 작업을 완료로 표시하거나 승인을 고치지 않는다.
- `ach recover <id> --expected-observation <토큰> [--dry-run]`: 상태가 `running`·`awaiting_approval`(`plan_review`는 아님)이고, 토큰이 지금과 같고, 런타임이 `missing`으로 확인됐고, 지금 작업 판의 체크포인트가 유효하고, 풀리지 않은 제출 시도가 없고, 막힘·사용자 입력이 없고, `next_action_at`이 지났고, 저장된 대화가 정확히 하나이고, 이 공모전을 뺀 용량이 있을 때만 저장된 대화를 다시 연다. 재개 지시에 체크포인트를 넣는다. 거절은 `observation_changed`, `runtime_unknown`, `checkpoint_stale_revision`, `submission_unresolved`, `saved_conversation_ambiguous`, `next_action_not_due`, `human_plan_review`, `no_capacity` 같은 코드로 낸다. 먼저 `--dry-run`을 돌린다. 작업 판은 바뀌지 않는다.

### 공식 규칙 증거

```json
{"sources": [{"id": "notice", "url": "https://contest.example.org/notice/123"}, {"id": "guide", "url": "https://contest.example.org/files/guide.pdf", "path": ".private/rules/inbox/guide.pdf"}]}
```

- `ach rules capture <id> --file F`: 실제 바이트를 내용 주소 불변 파일로 저장한다(`.private/rules/sha256/<해시>`, 라우팅 전에는 `data/rules/<id>/sha256/<해시>`). `path`가 있으면 그 파일을 복사하고, 없으면 쿠키·로그인 없는 제한된 GET으로 받는다. 해시가 바뀌면 검토 대기(`changed_pending`), 같으면 `unchanged`이고 아무것도 무효로 만들지 않는다.
- `ach rules review <id> --file F`: 검토 대기 `version`을 `base_version`에 대해 받아들인다. 규칙은 `{"category": "eligibility|deadline|ai_policy|forms|criteria|submission", "key"?, "value", "source": {"id", "sha256", "locator"}}`. 위치·해시가 없으면 거절하고, `"ambiguous": true`나 `"decision": "unresolved"`면 대기로 남는다. CLI가 값을 비교해 영향을 정한다: 참가 자격·AI 정책 → 게이트·기획·최종 무효, 심사기준 → 기획·최종, 서식 → 최종, 마감·접수처·방법 → 제출 준비·승인만. 값이 모두 같으면(배너만 바뀜) 무효 없이 받아들인다.
- `ach rules acknowledge <id> --stage gate|plan|final --file F`(`{"rules_version", "epoch", "decision": "pass", "reviewer", "rationale", "evidence": [{path, sha256}]}`)는 명시적 재검토 뒤 한 단계만 푼다. 새 `plan`·`final` 점수도 그 단계를 푼다. `ach rules status <id>`는 `tracked`, `pending`, `invalidated`, `binding`을 보여 준다.
- 승인 코드: `rules_pending_review`, `rules_*_invalidated`는 모든 승인을 막고, `rules_untracked`는 위임 승인만 막는다. 올려도 기존 공모전의 규칙 증거를 지어내지 않는다.

### 독립 최종 평가와 결과

기획·초안·개선 채점은 작업 맥락을 봐도 된다. 위임 승인이 기대는 최종 점수는 블라인드 패킷에서 나온다.

1. 결과물(과 점수표)에서 신원 정보를 지운 사본을 만들고 `ach evaluation prepare <id> --file F`: `{"purpose": "independent_final", "stage": "final", "candidates": [{"artifacts": [{"source": {path, sha256}, "sanitized": {path, sha256}}]}], "rubric"?, "references"?, "attestation": {"by", "identity_removed": true, "statement"}}`. `.private/`, `.harness/`, `JUDGE.md`, 프로필 파일은 거절하고, 익명 사본만 중립 이름표로 `.harness/evaluation/<packet_id>/packet/`에 복사하며, 작업 판·규칙 묶음·원본 해시는 비공개로 묶는다. CLI는 메타데이터만 가리고, 문서 안의 신원 정보는 호출자가 지우고 확인서로 남긴다.
2. 새 `ContestJudge` 실행이 그 패킷만 읽는다. `ach evaluation record <id> --file F`(`{"packet_id", "evaluator": {"id", "model", "run_id"}, "mode": "independent", "candidates": [{"label", "total", "target", "verdict", "scores", "artifacts": [{"file", "sha256"}]}]}`)로 기록한다.
3. 같은 총점·목표·판정·기준에 `evaluation_id`·`assessment_id`를 더해 `.harness/judge/final-<n>.json`에 쓰고 `ach score <id> --stage final`. 하나라도 다르면 거절된다. `"independent": true` 표시만으로는 인정되지 않는다.

`ach evaluation report <id>`는 같은 패킷의 평가를 묶어 `n`, 평균, 표본 표준편차, 최솟값, 최댓값, 판정 불일치를 낸다. `n < 2`면 분산은 0이 아니라 없음이다. 결과 발표 뒤 `ach evaluation outcome <id> --file F`로 `{"result": "won|finalist|lost|disqualified|withdrawn", "source_url", "announced_at", "evidence": {path, sha256}, "official_score"?, ...}`를 남기고, `ach evaluation calibration`은 관측 표본 수와 점수·결과 비교를 표본이 모자라면 `inadequate_data`로 표시해 보여 준다. 수상 확률은 어디서도 만들지 않는다.

### 제출 트랜잭션

하네스는 임의의 웹사이트 제출을 자동화하지 않는다. 승인받은 세션이 폼·메일 제출을 하고, CLI는 그 앞뒤의 지속되는 의도와 증거를 기록한다. 결과를 모르는 제출은 실패가 아니며 자동으로 다시 내지 않는다.

1. 제출 준비 기록에 `"destination": "<공식 URL 또는 메일>"`을 넣고 그 기록으로 승인을 받는다.
2. `ach submission intent <id> --file .private/submission-intent.json`(`{"method": "form", "destination": "<같은 값>", "expected_request": "<request_token>"}`) → `attempt_id` `a1`.
3. 보내기 전에 `ach submission start <id> --attempt a1`로 `dispatching`을 저장한다.
4. 승인된 파일을 그 제출처에만 낸다.
5. `.private/`에 남긴 접수 증빙으로 `ach submit <id> --attempt a1 --file .private/submission-proof.json`. 시간 초과·응답 끊김·중단이면 다시 내지 않고 `ach submission unknown <id> --attempt a1 --file ...`. 나중에 접수 목록·메일함에서 접수를 찾으면 `ach submit`으로 확인하고, 접수되지 않았다는 긍정 증거가 있을 때만 `ach submission resolve`로 남긴다. 그 뒤에만 새 의도를 기록할 수 있다.

시도 상태는 `intent`, `dispatching`, `unknown`, `verified_not_submitted`, `confirmed`, `expired`(시작하지 않은 의도의 승인이 지난 경우. 보낸 것 없음)다. `ach submission status <id>`는 접수 문구와 제출처를 내지 않는다. 풀리지 않은 시도가 있으면 새 의도, 복구, 정리가 거절된다.

### 관리 컨트롤러

상시 자동화는 `controllers/` 패키지에 있고 `data/`에는 상태와 기록만 둔다.

```bash
ach controller approve --once --dry-run     # 위임 승인 루프 한 주기, 쓰기 없음
ach controller queue                        # 대기열 루프, QUEUE_READY 출력
ach controller briefs refresh --dry-run --id <id>
ach controller preset --pick
```

- `approve [--once] [--dry-run] [--interval 120]`: `ach reconcile --apply`(관측만), `approval-status`, `approve --delegated --expected-request`, 끝난 공모전의 `session-close`·`clean`. 준비 줄 `AUTOAPPROVE_READY`.
- `queue [--once] [--dry-run] [--interval 60]`: 주기마다 `data/queue-master/queue.json`, `preset.json`, `new_work_frozen`을 다시 읽고, 동결이 아니면 `queue.json`의 `routed` 공모전만 띄운다. 세션을 복구하지 않는다. 준비 줄 `QUEUE_READY`.
- `briefs create <id>...` / `briefs refresh [--dry-run] [--adopt-legacy] (--id ... | --file ...)`: `.private/controller-brief.md`를 관리한다(종료 코드 3은 검토 필요). 운영자 전용 문구는 공개 패키지가 아니라 `data/queue-master/brief-overrides.json`에 둔다.
- `preset <프리셋> (<id>... | --all | --video) [--resume] [--dry-run]` 또는 `preset --pick`(`preset.json`의 `auto_providers`·`auto_default`가 있어야 `auto`를 쓴다).
- 모든 하위 명령은 쓰기 전에 `ach contract --json`의 `protocol: 2`를 확인한다(아니면 종료 코드 2). `approve`·`queue`는 수명 동안 비차단 잠금(`data/controller-approve.lock`, `data/controller-queue.lock`)을 잡고, 두 번째 실행은 75로 끝난다. `--once` 주기 오류는 1이다. `--dry-run`은 잠금도 쓰기도 하지 않는다. `ach controller`는 자기 경로를 `ACH_BIN`으로 넘긴다.
- 알림은 지문(코드, 정렬한 이유 코드, 마감 구간 `none|invalid|expired|lt24h|lt72h|lt7d|gte7d`)이 새로 생기거나 바뀌거나 풀릴 때만 `data/auto-approve/events.jsonl`, `data/queue-master/events.jsonl`에 쓴다. 상태는 `alerts.json`(0600)에 남아 재시작해도 같은 경고를 되풀이하지 않는다.

### 보드

`BOARD.md`는 `## 사용자 확인 필요 (N)`(`| id | 상태 | 이유 | 다음 조치 | 기한 |`)으로 시작하고, 사람이 할 일이 있는 공모전을 코드로만 보인다. 진행 절(`awaiting_approval`, `plan_review`, `running`)은 `| 마감 | 공모전 | 유형·레벨 | 폴더 | id | 런타임 | 다음 조치 | 막힘 | 근거 시각 |`이고, 런타임은 마지막 관측이거나 관측이 없거나 오래되면 `unobserved`다. 보드는 저장된 기록만 읽고 사적 막힘 문장·체크포인트 문장·접수 문구를 옮기지 않는다.

## 제출 기록부

`data/submissions.json`(git 제외)은 이미 제출한 공모전을 모두 기록해, 같은 공모전을 다시 배정하거나 제출하지 않게 한다. 항목마다 레지스트리 id(없으면 `manual-<slug>` key), 제목, 주최, URL과 다른 URL, 폴더, KST 제출 시각, 방법, 접수 증빙, 파일, 메모, 출처(`harness`·`manual`·`import`)를 둔다.

- `ach submit <id> --attempt <A> --file <증빙>`은 레지스트리를 쓰는 같은 과정 안에서 기록부를 쓰므로, 기록부 쓰기가 실패하면 레지스트리도 바뀌지 않는다. key는 레지스트리 id이고, 같은 key의 예전 기록(이전 회차·직접 적은 기록)은 그대로 두고 `extra.rounds`에 이번 회차를 덧붙인다. 기록부에는 접수 참조와 해시만 남기고 증빙 원문은 넣지 않는다.
- `ach submitted add --title T ...`는 하네스 밖에서 낸 공모전을 기록한다. 이미 있는 수동 기록은 제목과 날짜가 같을 때만 바꾸고, 아니면 새 key에 `-2`, `-3` 같은 꼬리를 붙인다. 글자나 숫자가 없는 제목은 해시 key를 받는다.
- `ach submitted import`는 레지스트리에서 지난 제출을 한 번 옮긴다: 제출 기록이 있거나 상태가 `submitted`인 항목, 그리고 `closed`이면서 제목이나 history note에 "제출"이 들어간 항목.
- `ach dedupe-check`는 URL 키·다른 URL 일치나 비슷한 제목으로 기록부를 대조하고, 그 결과를 `{"kind": "submitted", ...}`로 맨 앞에 둔다. 두 제목에 모두 연도나 회차(제N회, N기, Nth)가 있으면 같아야 제목 일치로 본다. URL 일치는 이 조건과 관계없다. `ContestGate`·`ContestScout`·메인 세션은 이런 공모전을 게이트하지 않고 `duplicate`(note "이미 제출함: <제목, 날짜>")로 둔다.
- `ach route`는 기록부에 있는 공모전을 거절한다.

## 제출 뒤 정리

제출 뒤 정리 방식은 세션이 제출 준비 기록의 `followup_required`로 정한다. `false`면 `ach clean <id>`를 실행하고 정기 실행은 `ach clean --all`을 쓴다. 본선·발표 등 미래 일정만 남아 `true`면 할 일을 기록하고 `followup_pending`으로 옮긴다. 기존 제출 기록이 있어야 하며, 폴더와 접수 증빙을 보존하고 실행 슬롯을 반환하며 정리 대상에서도 제외된다. 실제 후속 작업을 재개할 때만 용량을 확인하는 `ach resume <id>`로 `running`에 돌아간다. 메인 세션의 자동 정리는 `followup_required: false`로 기록된 공모전만 정리하고, `true`이거나 구조화된 기록이 없는(0.7.0 이전) 공모전은 사용자가 요청할 때만 정리한다. 끝난 공모전에 남은 세션은 `ach session-close`로 닫는다.

`clean`은 아래 다섯 조건을 모두 만족할 때만 지운다. 하나라도 어긋나면 이유를 나열하고 아무것도 지우지 않는다. `--force`는 1~4 검사를 건너뛰지만 5는 건너뛰지 않는다. 이때 폴더가 없으면 실행을 멈추지 않고 그 공모전의 문제로 알린다.

1. 상태가 `submitted` 또는 `closed`다.
2. 제출 기록부에 이 공모전 기록이 있다.
3. 폴더가 upstream이 있는 git 저장소이고, 작업 트리가 깨끗하며(변경 없음, 무시되지 않은 untracked 파일 없음), push 안 된 커밋이 없다(`git fetch` 뒤 확인).
4. 레지스트리 `submissions[].files`에 적힌 파일이 git이나 폴더에 있다. 서명본처럼 일부러 git에서 뺀 파일은 문제가 아니고, 다음 단계에서 보관된다.
5. 풀리지 않은 제출 시도(`intent`·`dispatching`·`unknown`)가 없다.

그다음:

1. 세션(Orca 터미널, tmux 창, headless 프로세스)을 닫고, Orca 작업 공간이 있으면 지운다. 여기서 실패하면 경고하고 정리는 계속한다.
2. git에 없고 다시 만들 수 없는 파일을 `data/archive/<id>/`(git 제외, 권한 0700)에 보관한다. git이 추적하지 않는 제출본과 공모전 폴더의 `.private/` 안 파일(접수 증빙, 폼 수정 링크, 개인정보가 든 사본)이다. `..`가 든 경로, 절대 경로, 폴더 밖으로 이어지는 심볼릭 링크는 거절한다. 그 밖의 git 제외 파일(렌더, 캐시, 가상 환경)은 개수만 세고 폴더와 함께 보낸다.
3. 폴더를 휴지통으로 옮긴다. macOS는 `~/.Trash/<이름>-<시각>`, Linux는 XDG 휴지통 `$XDG_DATA_HOME/Trash/files`(기본 `~/.local/share/Trash/files`, `.trashinfo` 항목과 함께)가 있으면 그곳이고, 없으면 `data/trash/`다. `--purge`면 완전히 지운다.
4. 레지스트리에서 `session`을 비우고 `folder_removed_at`을 기록하며, 같은 상태로 history note를 남긴다. 상태는 바꾸지 않는다.
5. 기록부 항목에 원격 `repo` URL, `folder_removed_at`, 보관 위치 `archive`를 기록해, 나중에 `git clone`으로 되살릴 수 있게 한다.
6. `<contests_root>/.shared/existing_contests.md`의 줄은 중복 대조용이라 지우지 않는다.

`--dry-run`은 검사 결과와 할 일만 보여 준다. JSON 출력은 `{id, ok, problems, actions, trash, archive}`다(`--all`이면 배열).

`git fetch`와 검사는 읽기 잠금 아래에서 돌고, 그다음 공모전마다 쓰기 잠금 아래에서 다시 확인하고 정리한다. 기록부는 폴더를 옮기기 전에 쓰고, 옮기기가 실패하면 되돌린다. `--all`에서 실패한 공모전은 실패 결과로 남고, 나머지는 그대로 정리해 저장한다.

## 사용자 정보

`data/profile.md`(git 제외)는 사용자 정보를 담은 마크다운 파일이다. `ach init`이 빈 칸만 있는 커밋된 `data/profile.example.md`를 권한 0600으로 복사해 두며, 권한은 계속 0600으로 둔다. 하네스 명령은 이 파일을 읽기만 하고 고치지 않는다. 사용자가 직접 채우거나, 설정 절차([INIT.md](../INIT.md) 4단계)에서 에이전트에게 대화로 알려 준다:

- 직접 쓰기를 권한다. 제출용 개인정보 절은 특히 그렇다.
- 대화로 알려 주면 에이전트는 받은 값만 그대로 해당 절에 쓴다. 제출용 개인정보 절은 먼저 "대화로 준 값은 모델 제공자와 세션 기록을 거친다"고 알리고 직접 쓰기를 권한다. 쓴 뒤 `chmod 600 data/profile.md`를 실행하고, 값을 다시 읊지 않으며, 다른 곳(다른 파일, 커밋, 로그, 메모)에 옮기지 않는다. 이 단계 밖에서는 에이전트가 이 파일을 고치지 않는다.
- 서명·도장·증명서 같은 파일은 사용자가 `data/profile/`에 직접 두고, 프로필에는 경로만 적는다.

절은 두 개다:

- `## 참가 자격`(또는 `## Eligibility`): 게이트가 대조하는 정보. 참가 형태(개인·팀), 학생 여부, 학교, 학과, 과정, 학년, 출생 연도(나이 제한 판정용, 생년월일 대신), 거주 지역, 국적, 재학증명서 보유 여부, 강점, 피하는 조건. 식별 값은 쓰지 않는다.
- `## 제출용 개인정보`(또는 `## Submission details`): 제출 양식에 들어갈 정보. 이름(한글·영문·한자), 생년월일, 전화, 이메일, 학교·학과 정식 명칭, 서명·도장·재학증명서 파일 경로(예: git 제외인 `data/profile/` 아래), 한컴독스 계정. 사용자가 제출을 승인한 뒤에만 읽는다.

- `ContestGate`는 `ach profile eligibility --json`(`## 참가 자격` 절)만 읽는다.
- 식별 값은 승인된 제출 단계에서만 `ach profile show --for <id>`로 읽는다([CLI가 강제하는 승인](../README_ko.md#cli가-강제하는-승인) 참고).
- 파일이나 `## 참가 자격` 절이 없으면 명령은 `data/profile.example.md`를 가리키는 오류를 낸다.

## 로컬 규칙

자기 컴퓨터에서만 뜻이 있는 규칙(개인 도우미 스크립트, 계산 서버, 문서 도구, 폴더 관례 등)은 `data/local-rules.md`에 적는다. git에서 제외되므로 공개 저장소에 올라가지 않는다. 커밋된 예시 `data/local-rules.example.md`에서 시작한다. 이 파일이 있으면 에이전트와 스킬이 먼저 읽고 따른다.

## 확장: 수집 출처 추가

1. `src/sources/<이름>.rs`를 만들고 [`src/sources/mod.rs`](../src/sources/mod.rs) 머리 주석의 모듈 계약을 구현한다:
   - `NAME`: 출처 이름이자 레지스트리 id 접두어. 하이픈 없는 소문자 영문.
   - `collect(pages, known_ids) -> Result<Vec<Item>>`: 목록 앞쪽 `pages`쪽에 보이는 공모전 전부. `known_ids`에 이미 있는 id는 상세를 열지 않고 목록 값만 돌려준다. 0건을 돌려주면 사이트 구조가 바뀌었거나 접수 중인 공고가 없는 것으로 본다.
   - `url_key(url) -> Option<String>`: 이 출처의 상세 URL이면 `Some("<NAME>:<고유 id>")`, 다른 URL이면 `None`. 중복 판정에 쓴다.
   - `Item` 필드는 `entry_id`(`"<NAME>-<고유 id>"`), `title`, `url`, `organizer`, `deadline`(KST `YYYY-MM-DDTHH:MM+09:00` 또는 `None`), `categories`이고, 선택 필드로 `note`와 `alt_url`(공식 누리집)이 있다.
2. `src/sources/mod.rs`에 모듈을 선언하고 `SOURCES`에 한 줄 더한다. `--source` 선택지에는 자동으로 들어간다.
3. 해석과 요청을 나눈다. 공통 HTTP·텍스트·날짜 도우미는 `src/sources/common.rs`와 `src/http.rs`에 있다. 같은 사이트에 보내는 요청은 1초 이상 간격을 둔다.
4. `tests/source_<이름>.rs`를 더한다. 테스트는 `tests/fixtures/<이름>_*.html|json`에 저장한 실제 응답을 해석하고, 네트워크는 쓰지 않는다. 목록 해석, KST 마감 변환, `url_key`를 확인한다.
5. `robots.txt`와 사이트 약관을 지킨다. 차단은 우회하지 않는다. 예를 들어 Devpost 모듈은 Cloudflare가 상세 페이지를 막으면 목록 값을 그대로 쓴다.

pull request를 열기 전에 `scripts/check.sh`를 돌린다([CONTRIBUTING_ko.md](../CONTRIBUTING_ko.md) 참고).

## 폴더 구성

```text
Contest-Harness/
├── Cargo.toml                  Rust crate (바이너리 `ach`)
├── src/
│   ├── main.rs                 CLI 진입점
│   ├── config.rs, paths.rs     설정과 파일 위치
│   ├── registry.rs, board.rs   레지스트리, 상태 기계, BOARD.md
│   ├── store.rs                버전이 붙은 원자적 JSON 저장
│   ├── collect.rs, dedupe.rs   수집과 중복 판정
│   ├── route.rs, launch.rs     폴더·저장소·세션, 메인 인터페이스
│   ├── budget.rs               토큰 속도, 메모리, 용량
│   ├── guard.rs                페르소나 차단 검사
│   ├── doctor.rs               ach doctor 점검
│   ├── process.rs              제한 시간·프로세스 그룹을 둔 외부 명령 실행
│   ├── install.rs, schedule.rs 어댑터 설치, agent_models 적용, cron·Orca automation·launchd·crontab·autorun
│   ├── init.rs                 ach init
│   ├── runners/                러너 어댑터 인터페이스(mod.rs)와 omp 어댑터
│   ├── workspaces/             orca, tmux, headless, terminal
│   ├── runtime.rs, rules.rs    체크포인트·재조정·복구, 공식 규칙 원문 보관·검토
│   ├── rubric.rs               RUBRIC.md 채점 근거 절(기획 채점 전 필수)
│   ├── evaluation.rs           블라인드 패킷, 독립 평가, 결과
│   ├── submission.rs           제출 트랜잭션(의도, 시작, 불명, 해소, 확인)
│   └── sources/                모음 사이트마다 모듈 하나
├── controllers/                관리 컨트롤러(python3 -m controllers approve|queue|briefs|preset)
├── scripts/                    install.sh, check.sh(필수 로컬 검사), release.sh
├── bin/ach                     하네스 코어. scripts/install.sh가 설치한다 (git 제외, PATH에 넣지 않는다)
├── package.json                omp 확장 패키지 매니페스트
├── extensions/contest-harness.ts  omp용 /contest 명령
├── commands/                   /contest-gate, /contest-init 프롬프트 템플릿
├── agents/                     에이전트 (ContestScout, ContestGate, ContestHQ, DomainResearcher,
│                               OpenSourceScout, ContestJudge, ContestEditor, ReferenceScout,
│                               UxReviewer)
├── skills/contest-harness/     수명주기 스킬과 유형별 플레이북
├── harness/
│   ├── prompts/                launch.md, level_1.md … level_5.md, automation.md, main.md
│   └── templates/              SPEC, PLAN, TASKS, JUDGE, RUBRIC, SUBMISSION, LEDGER
├── data/
│   ├── config.example.json     설정 예시 (ach init이 config.json으로 복사)
│   ├── config.json             사용자의 설정 (git 제외)
│   ├── profile.example.md      사용자 정보 예시 (ach init이 profile.md로 복사)
│   ├── profile.md              사용자 정보 (git 제외)
│   ├── profile/                서명·도장·증빙 파일 (git 제외)
│   ├── logs/                   정기 실행 로그 (git 제외)
│   ├── local-rules.example.md  로컬 규칙 예시
│   ├── local-rules.md          사용자의 로컬 규칙 (git 제외)
│   ├── persona.example.md      선택 운영자 페르소나 예시
│   ├── persona.md              사용자의 페르소나 (git 제외)
│   ├── submissions.json        제출 기록부 (git 제외)
│   └── registry.json           공모전 레지스트리 (git 제외)
├── INIT.md                     에이전트가 따르는 설정 절차
├── docs/CONTRACT.md            명령, 상태, 파일 형식, 백엔드
├── docs/reference*.md          이 레퍼런스(영어·한국어)
├── docs/diagrams/              README 그림: HTML 원본(diagram-design)과 내보낸 SVG
├── CONTRIBUTING.md             러너 어댑터·수집 출처 추가 방법
├── SECURITY.md                 위협 모델, 취약점 신고
├── CHANGELOG.md                릴리스 기록
├── *_ko.md                     README·CONTRIBUTING·SECURITY·CHANGELOG의 한국어판
├── references/REFERENCES.md    참고 프로젝트 (사본은 git 제외)
├── tests/                      통합 테스트(e2e.rs는 전체 수명주기를 오프라인으로 돌린다)와 저장된 출처 응답
└── BOARD.md                    자동 생성 보드 (git 제외)
```
