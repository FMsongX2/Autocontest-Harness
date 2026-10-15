# 하네스 계약

이 문서는 하네스의 CLI, 레지스트리, 파일 위치, 러너·작업 환경, 에이전트가 서로 맞물리는 약속이다. 코드와 에이전트 지시문은 이 문서를 기준으로 맞춘다. 바꿀 때는 이 문서를 먼저 고친다. 코어는 Rust 바이너리 `harness`(crate `autocontest-harness`, 저장소 `src/`)이고, 설정 기본값의 정본은 `src/config.rs`의 `defaults()`다.

## 1. 전체 흐름

```text
수집(collect, ContestScout) → 게이트(ContestGate, 레벨 1~5, 제출 기록부·자격 대조) → 라우팅(route) → 실행(launch: 레벨별 구성)
  → [레벨 2 이상: 점수표 → 기획 채점(score plan) 반복 → 만들기 → 중간·최종 채점(score draft·final)]
  → [레벨 5만: 계획 검수(plan_review, 사용자)] → 심사 관문(ContestJudge) → 페르소나 검사(check)
  → 제출 승인(awaiting_approval, 사용자) → 제출 기록(submit, 제출 기록부에도 기록) → 정리(clean: 세션 닫기·로컬 폴더 휴지통) → 종료(close)
```

- 수집: `harness collect`가 모음 사이트 11곳(6절 수집 출처)의 목록을 읽어 `discovered` 항목을 만들고, 사이트 간 중복은 자동으로 합친다. ContestScout 에이전트는 이 11곳을 빼고, 모음 사이트에 없는 공고(정부·지자체·공공기관 누리집, 학회, 기업 자체 공모, 해외 기관)를 웹 검색으로 찾아 `harness add`로 넣는다.
- 게이트: ContestGate 에이전트가 공식 원문을 읽고 참가 자격·제약·제출물·심사기준을 구조화하고, 유형과 레벨(5절)을 매겨 `harness gate`로 기록한다. 자격은 `harness profile eligibility --json`(14절)과 대조하고, 제출 기록부(13절)에 있는 공모전은 `duplicate`로 돌린다.
- 라우팅: `harness route`가 공모전 폴더, 설정 `github_org`의 private 저장소(GitHub Actions 없음), 설정 `workspace`의 작업 공간(10절), `CONTEST.md`를 만든다.
- 실행: `harness launch`가 설정 `runner`의 러너를 설정 `workspace`의 작업 환경에서 공모전 폴더를 작업 디렉터리로 띄우고, 레벨에 맞는 실행 지시문(`harness/prompts/launch.md` + `level_<n>.md`)을 보낸다. 동시 실행 수와 메모리 예산(5절) 안에서만 띄운다.
- 계획 검수: 레벨 5는 레퍼런스 조사와 SPEC·PLAN 뒤 `plan_review`에서 멈추고 사용자 승인을 받은 뒤에 구현한다(4절).
- 점수 루프: 레벨 2 이상은 공모전 평가기준을 점수표로 바꾸고, 기획 단계부터 예측 채점해 점수를 올리는 방향으로 일한다(15절).
- 페르소나 검사: `awaiting_approval`로 옮길 때 CLI가 공모전 폴더와 커밋 메시지에서 페르소나 흔적을 찾고, 있으면 거절한다(12절).
- 제출: 개인정보가 들어가고 되돌릴 수 없으므로, 제출 직전에는 반드시 `awaiting_approval`에서 멈추고 사용자 승인을 받는다. 개인 식별 값은 이 단계에서만 `harness profile show --reveal --json`으로 읽는다(14절). 제출한 공모전은 제출 기록부(13절)에 남아 다시 배정되지 않는다.
- 메인 인터페이스: 사용자는 `harness chat`으로 연 메인 세션 하나에서 위 흐름을 보고 지휘한다(12절). 메인 세션은 공모전 결과물을 직접 쓰지 않는다.

## 2. 경로

하네스 루트는 환경 변수 `HARNESS_HOME`이 있으면 그 경로, 없으면 바이너리를 빌드한 저장소 클론이다(`cargo install --path .`). 아래의 "저장소"는 하네스 루트를 가리킨다.

| 무엇 | 경로 |
|---|---|
| 하네스 저장소 | clone 위치(공개 저장소 https://github.com/FMsongX2/Autocontest-Harness, GitHub Actions 없음) |
| CLI | `harness` (`cargo install --path . --root ~/.local`이면 `~/.local/bin/harness`) |
| 설정(git 제외) | 저장소 `data/config.json` (6절). 예시 `data/config.example.json`을 `data/config.json`으로 복사해 시작한다 |
| 운영자 환경 규칙(git 제외) | 저장소 `data/local-rules.md` (예시 `data/local-rules.example.md`). 운영자 전용 장비·도구·연구 환경 규칙을 둔다. 있으면 에이전트와 스킬이 먼저 읽고 따른다 |
| 레지스트리(git 제외) | 저장소 `data/registry.json`, 잠금 파일 `data/registry.lock` |
| 상태 보드(git 제외) | 저장소 `BOARD.md` |
| 사용자 정보(자격 판정·제출용, git 제외) | 저장소 `data/profile.json`(스키마 v2, 14절, 예시 `data/profile.example.json`), 서명·도장·재학 증빙 같은 파일은 `data/profile/`. 둘 다 0600 권한으로 쓴다 |
| 제출 기록부(git 제외) | 저장소 `data/submissions.json`, 잠금 파일 `data/submissions.lock` (13절) |
| 메인 세션 시스템 프롬프트(git 제외) | 저장소 `data/main-system.md` (`harness chat`이 실행할 때마다 다시 쓴다, 12절) |
| 메인 세션 첫 지시(git 제외) | 저장소 `data/main-start.md` (`harness chat`이 실행할 때마다 다시 쓴다) |
| 정기 실행 로그(git 제외) | 저장소 `data/logs/autorun-<YYYYMMDD-HHMMSS>.log`(`harness autorun`), `data/logs/launchd.log`(launchd 표준 출력·오류) |
| 정리한 폴더 | macOS 휴지통 `~/.Trash/<폴더 이름>-<시각>`, 휴지통이 없으면 저장소 `data/trash/` (`harness clean`, 16절) |
| launchd 작업 | `~/Library/LaunchAgents/io.github.fmsongx2.autocontest-harness.plist` (`harness schedule install`) |
| 코어 소스 | 저장소 `src/` (`main.rs` CLI, `config.rs` 설정, `registry.rs` 상태, `guard.rs` 페르소나 검사, `runners/` 러너, `workspaces/` 작업 환경, `sources/` 수집 출처) |
| 수집 모듈 | 저장소 `src/sources/<출처>.rs` (출처 하나가 모듈 하나, 모듈 계약은 `src/sources/mod.rs` 머리 주석) |
| 실행 지시문 | 저장소 `harness/prompts/launch.md`, `level_1.md` ~ `level_5.md`, 정기 실행 `automation.md`, 메인 인터페이스 역할 `main.md` |
| 운영자 페르소나(선택, git 제외) | 설정 `persona.file`(기본 저장소 `data/persona.md`, 예시 `data/persona.example.md`). 메인 세션 전용, 12절 |
| 확장 패키지 | 저장소 `package.json`, `extensions/contest-harness.ts`, 프롬프트 템플릿 `commands/contest-gate.md` (11절) |
| 문서 템플릿 | 저장소 `harness/templates/` (`SPEC.md`, `PLAN.md`, `TASKS.md`, `JUDGE.md`, `RUBRIC.md`(점수표, 15절), `SUBMISSION.md`, `LEDGER.md`) |
| 에이전트·스킬 원본 | 저장소 `agents/*.md`, `skills/contest-harness/` (러너별 설치 위치는 10절) |
| 공모전 루트 | 설정 `contests_root` (기본 `~/contests`) |
| 공모전 폴더 | `<contests_root>/<Folder>` (영문 PascalCase-하이픈, 예: `City-Shorts-2026`) |
| 공모전 메타 | `<공모전 폴더>/.harness/contest.json` (레지스트리 항목 사본), `gate.json`, `launch-prompt.md`, `omp-preset.yml`(omp 프리셋 덮개), `session.log`(headless 작업 환경), `judge/<stage>-<n>.json`(ContestJudge 채점 출력, `harness score`가 읽는다, 15절) |
| 공모전 문서 | `<공모전 폴더>/CONTEST.md`(질문 절 제목 "사용자에게 확인할 것"), `README.md`, `SPEC.md`, `PLAN.md`, `TASKS.md`, `RUBRIC.md`(레벨 2 이상 점수표), `JUDGE.md`, `SUBMISSION.md`, `LEDGER.md`(레벨 4 실험·제출, `model`·`research`), `research/references.md`(레벨 5 레퍼런스) |
| 기존 공모전 목록 | `<contests_root>/.shared/existing_contests.md` (`- <폴더>: <제목>` 한 줄씩, `route`가 덧붙이고 `dedupe-check`가 읽는다. `clean`은 이 줄을 지우지 않는다) |

## 3. 레지스트리

`data/registry.json`은 `{"version": 1, "contests": {<id>: <항목>}}`이다. 쓰기는 CLI만 한다(잠금 파일로 직렬화하고 임시 파일을 바꿔치기해 저장한다). 에이전트는 파일을 직접 고치지 않는다.

항목 필드:

| 필드 | 형식 | 설명 |
|---|---|---|
| `id` | 문자열 | 출처 기반 고정 id `<출처>-<출처 고유 id>`. 예: `linkareer-354786`, `wevity-111674`, `dacon-236500`, `manual-<slug>` |
| `title`, `organizer`, `url` | 문자열 | 공고 제목, 주최, 대표 URL |
| `alt_urls` | 문자열 배열 | 같은 공모전의 다른 출처 URL(사이트 간 중복을 합친 URL 포함) |
| `source` | 문자열 | 수집 출처 `linkareer`, `wevity`, `thinkcontest`, `allcon`, `contestkorea`, `detizen`, `campuspick`, `allforyoung`, `dacon`, `devpost`, `kaggle` 또는 `scout`, `manual`, `legacy`(예전 목록에서 옮겨 온 항목. 하네스 밖에서 진행하므로 실행 상한에 세지 않는다) |
| `deadline` | ISO 8601(`+09:00`) 또는 `null` | 접수 마감 |
| `categories` | 문자열 배열 | 출처가 준 분야 |
| `discovered_at` | ISO 8601 | 처음 등록한 시각 |
| `state` | 4절 상태 | |
| `type` | `video`, `image`, `music`, `idea`, `paper`, `report`, `model`, `data`, `software`, `essay`, `design`, `research`, `other` 중 하나 또는 `null` | 게이트가 정한다. `model`은 리더보드에 반복 제출하는 대회(데이콘 등), `research`는 실험·모델 개발로 결과를 내고 보고하는 연구형 공모전(반복 제출이 없거나 적다. 예: 며칠 서버 실험을 하는 연구 대회) |
| `level` | 정수 `1`~`5` 또는 `null` | 게이트가 정한다(5절) |
| `priority` | 정수 | 클수록 먼저 실행. 게이트가 정한다 |
| `gate` | 객체 또는 `null` | 7절의 게이트 결과 |
| `folder`, `repo` | 문자열 또는 `null` | 라우팅 결과(`folder`는 절대 경로, `repo`는 `<github_org>/<Folder>`) |
| `session` | `{workspace, workspace_id, runner, handle}` 또는 `null` | 세션 위치(10절). `route`가 `workspace`·`workspace_id`·`runner`를 쓰고(`handle`은 `null`), `launch`가 `handle`까지 채운다. `handle`은 작업 환경마다 다르다: Orca 터미널 handle, tmux `contest-harness:@<창 id>`, headless `pid:<pid>:<로그 경로>` |
| `submissions` | 배열 | `{at, method, receipt, files}`. `submit`이 같은 내용을 제출 기록부(13절)에도 남긴다 |
| `scores` | 배열(기본 빈 배열) | 점수 이력 `{at, stage, total, target, verdict, criteria, note, file}`(15절) |
| `folder_removed_at` | ISO 8601 또는 없음 | `harness clean`이 로컬 폴더를 정리한 시각(16절). `folder`는 그대로 둔다 |
| `history` | 배열 | `{at, from, to, by, note}` 상태 변경 기록 |

이 버전이 모르는 필드는 읽은 그대로 보존한다.

## 4. 상태

```text
discovered → gate_pass | rejected | hold
gate_pass  → routed → running → awaiting_approval → submitted → closed
running    → plan_review(레벨 5 계획 검수 대기)
plan_review → running(승인) | skipped
어느 상태든 → skipped(사용자가 하지 않기로 함) | duplicate(이미 다룬 공모전)
hold       → gate_pass | rejected (원문 확인·사용자 답변 후)
running    ↔ awaiting_approval (승인 대기 중 수정 요청이 오면 running으로 돌아간다)
skipped | duplicate → discovered (판정을 바로잡아 다시 게이트할 때)
```

허용 전이의 정본은 `src/registry.rs`의 `allowed()`다. `running`과 `plan_review`가 실행 상한(5절)에 세는 진행 중 상태다. `closed`에는 `note`로 결과(`won`, `lost`, `ended`, `withdrawn`)를 남긴다. 보드(`BOARD.md`)의 절 제목은 "제출 승인 대기"(`awaiting_approval`), "레벨 5 계획 검수 대기"(`plan_review`), "진행 중"(`running`) 순으로 이어진다. 진행 중·승인 대기 행의 유형·레벨 칸에는 최근 점수 `총점/목표`가 붙는다(예: `idea / L2 · 88/90`, 15절).

레벨 5 규칙:

- `plan_review → running`은 `--note`에 승인 근거(사용자가 승인한 메시지 원문과 시각)가 있어야만 된다.
- 레벨 5 항목은 `plan_review → running` 승인 기록이 `history`에 없으면 `awaiting_approval`로 갈 수 없다(CLI가 거절한다).
- 레벨 5 `→ plan_review`는 `plan` 점수(15절)가 하나 이상 있어야 된다. 검수 요청에 예측 점수가 함께 간다.
- 레벨 5 세션 순서: 레퍼런스 조사(ReferenceScout) → 점수표·SPEC·PLAN과 기획 채점(`harness score <id> --stage plan`) → `harness transition <id> plan_review --note "<검수 요청 요약>"` → 사용자에게 검수를 요청하고 멈춤 → 승인되면 `harness transition <id> running --note "승인: <원문·시각>"` → 구현.

점수 관문(레벨 2 이상, 15절):

- `→ awaiting_approval`은 `plan` 점수가 하나 이상 있고, 가장 최근 `final` 점수가 있으며, 그 점수가 `verdict == ready`이고 `total >= target`이어야 된다. 아니면 CLI가 거절한다(종료 코드 1).
- 마감 등으로 목표에 못 미친 채 내야 하면 `--score-ok "<사유>"`로 넘긴다. 사유와 그때의 점수가 `history`의 `note`에 남는다.
- 레벨 1은 점수 관문이 없다.

제출 기록부 규칙: `harness route`는 제출 기록부(13절)에 같은 공모전이 있으면 `이미 제출한 공모전이다: <title> (<submitted_at>)`로 거절한다.

`awaiting_approval` 규칙: 공모전 폴더와 커밋 메시지에 페르소나 흔적(12절)이 있으면 CLI가 거절한다. 작품 속 대사처럼 의도한 표현이면 `--persona-ok "<사유>"`로 넘기고, 사유와 건수가 `history`의 `note`에 남는다.

같은 공모전이 여러 사이트에 따로 올라와 둘 다 `discovered`이면, `dedupe-check --id`는 먼저 수집된 쪽(`discovered_at`, 같으면 id 순)만 나중 쪽의 중복 후보로 낸다. 동시에 게이트해도 나중 쪽만 `duplicate`가 된다. `duplicate` 항목은 대표가 아니므로 `--id` 확인의 후보에서 빠진다.

## 5. 레벨

게이트가 공모전마다 레벨 하나(정수 1~5)를 매긴다.

| 레벨 | 크기 | 공모전 예 | 실행 구성 |
|---|---|---|---|
| 1 | 하루 안 산출물 하나 | 숏폼, 포스터, 이미지, 슬로건, 짧은 수기·간단한 신청서 | 단일 에이전트: 세션 하나가 유형 플레이북대로 만든다. 제출 관문으로 마지막에 ContestJudge 1회 |
| 2 | 조사 근거가 필요한 문서 하나 | 아이디어 제안서, 짧은 기획서 | 메인 에이전트(세션) 1 + 조사 에이전트 1(DomainResearcher, 논문 근거는 ArticleResearcher). 평가표(`gate.criteria`)가 있으면 마지막에 ContestEditor가 평가표 기준으로 문체·어투·문장을 점검하고 다듬는다. 그 뒤 ContestJudge |
| 3 | 여러 세션이 드는 결과물 | 정책 아이디어 논문, 간단한 실험으로 증명되는 논문, 간단한 정책 관련 서비스, 불법광고 탐지 서비스 정도의 프로젝트 | HQ(이 세션이 ContestHQ 절차로 SPEC·PLAN·TASKS) + DomainResearcher(필요 시 ArticleResearcher) + 서브에이전트 워커 웨이브 + 리뷰어(체크포인트마다 ContestJudge, 글이면 ContestEditor, 코드면 omp 기본 `reviewer`) |
| 4 | 실험·피드백 루프가 있는 큰 프로젝트 | 며칠 서버를 써서 실험하는 연구 대회, 데이콘처럼 제출→결과→분석→개선을 반복하는 대회 | 레벨 3 구성 + 실험 루프(LEDGER.md에 모든 실험·제출), 서버 계산(GPU 서버 또는 클라우드 GPU, 끝나면 바로 삭제. 구체 장비는 `data/local-rules.md`), 긴 작업은 Orca orchestration 워커 |
| 5 | 1달 이상, 실제 프로덕션 수준 | Meta AI 해커톤, 하나비 프로젝트처럼 실사용 품질이 필요한 제품 | 1) ReferenceScout가 웹에서 이미 있는 프로덕션 레퍼런스를 찾아 분석(이미 있는 프로덕션이 교본) → 2) 레퍼런스를 본 뒤 SPEC·PLAN → 3) `plan_review`로 사람 검수를 요청하고 멈춤 → 4) 승인 뒤 구현: HQ + DomainResearcher + 워커 웨이브 + UxReviewer(UI/UX 점검) + omp 기본 `security-reviewer` + 최적화(UX에 영향이 없을 만큼만, 과하게 하지 않음) + ContestJudge |

<p>표 1. 레벨 정의와 실행 구성</p>

서브에이전트 기능이 없는 러너(10절)에서는 `harness launch`가 실행 지시문 끝에 "같은 세션에서 차례로 수행한다"는 절을 덧붙인다. 위 표에서 서브에이전트·워커 웨이브로 하는 단계는 그 세션이 하나씩 직접 수행한다(pi는 `contest_agent` 도구로 에이전트 하나를 별도 프로세스로 돌릴 수 있다).

레벨 매기는 규칙(ContestGate):

- 위 정의에 맞는 레벨을 고르고, 두 레벨 사이면 높은 쪽을 고른다. 근거는 `gate.json`의 `reasons`에 적는다.
- 신호: 며칠 이상 서버·GPU 실험이나 반복 제출 피드백 루프가 있으면 최소 4. 실사용 프로덕션 품질이나 1달 이상이면 5. 실험·서비스 구현이 필요하면 최소 3. 조사 근거가 필요한 문서면 최소 2.
- 정의 밖에서 레벨을 한 단계 올리는 규칙은 없다.
- 우선순위의 일정 여유 기준(마감까지 남은 날의 최소치): 레벨 1 2일, 2 5일, 3 10일, 4 14일, 5 30일. 남은 날이 최소치의 2배 이상이면 20점, 최소치 이상이면 10점, 미만이면 0점.
- 레벨 2 이상은 점수 루프(15절)가 기본 진행 방식이다. 레벨 1은 제출 전 ContestJudge 1회만 한다.

동시 실행 상한: 전체 `max_running`과 레벨별 `max_running_by_level`을 함께 지킨다. 세는 대상은 하네스가 띄운 진행 중(`running`, `plan_review`) 항목이고, `source`가 `legacy`인 항목은 하네스 밖에서 돌아가므로 세지 않는다. `max_running_by_level`의 값이 0이면 그 레벨은 게이트를 통과해도 실행하지 않는다(`gate_pass`에 머문다).

메모리 예산: 쓸 수 있는 메모리(`vm_stat`의 free + inactive + speculative)에서 `ram_reserve_gb`를 뺀 값이 예산이다. 새 세션은 그 레벨의 `ram_gb_by_level`이 남은 예산 안에 들어갈 때만 띄운다. macOS memory pressure가 `warning` 이상이면 새 세션은 0개다. 세션 안 권장 워커 수(`recommended_workers`)도 `ram_gb_per_worker`로 같은 예산에 묶인다.

토큰 속도: `omp usage --json`이 알려 주는 사용 한도 창마다 pace = 남은 한도 비율 ÷ 남은 시간 비율을 구하고, 가장 빠듯한 창으로 공급자의 `mode`(`maxing` | `steady` | `conserve` | `throttle`)와 권장 워커 수를 정한다. omp가 없거나 실패하면 토큰 조건 없이 메모리·개수 상한만 적용한다.

## 6. 설정과 CLI

### 설정

`data/config.json`(git 제외, 개인 설정)을 `src/config.rs`의 기본값 위에 덮어 읽는다. 처음에는 `data/config.example.json`을 `data/config.json`으로 복사해 고친다. 적지 않은 키는 기본값을 쓴다. 사전 값 키(`preset_by_level`, `model_by_level`, `max_running_by_level`, `ram_gb_by_level`, `runner_args`, `persona`)는 적은 키만 덮는다. 예: `{"max_running_by_level": {"5": 0}}`은 레벨 5만 바꾼다. 레벨 키는 문자열 `"1"`~`"5"`다.

| 키 | 기본값 | 뜻 |
|---|---|---|
| `github_org` | `""` | `route`가 `gh`로 공모전마다 private 저장소를 만드는 GitHub 조직 또는 사용자. 빈 값이면 `gh`에 로그인한 사용자 계정에 만든다 |
| `contests_root` | `"~/contests"` | 공모전 폴더를 두는 곳. `dedupe-check`가 `<contests_root>/*/README.md`와 `.shared/existing_contests.md`를 읽는다 |
| `orca_repo_path` | `null`(= `contests_root`) | `orca` 작업 환경이 공모전 worktree를 만드는 git 저장소. Orca에 추가돼 있어야 한다 |
| `runner` | `"omp"` | 공모전 세션·정기 실행·메인 세션의 기본 러너. `omp`, `pi`, `claude`, `codex`, `dsh` 중 하나(10절) |
| `workspace` | `"orca"` | 공모전 세션의 작업 환경. `orca`, `tmux`, `headless` 중 하나(`terminal`은 `launch`가 거절한다) |
| `preset_by_level` | `{"1": null, "2": null, "3": null, "4": null, "5": null}` | omp 전용. 레벨별 `~/.omp/agent/config.yml` `modelPresets` 이름(예: `{"2": "ResearchPreset"}`). 실행마다 `.harness/omp-preset.yml` 덮개를 만들어 `--config`로 넘기므로 전역 omp 설정은 바뀌지 않는다. `null`이면 프리셋 없이 띄운다. `launch --preset`을 주면 그 값을 쓴다 |
| `model_by_level` | `{}` | omp 밖 러너의 레벨별 모델 id(`--model`, codex는 `-m`). omp도 프리셋이 없을 때 이 값을 쓴다. dsh는 모델 인자를 받지 않는다 |
| `runner_args` | `{"omp": [], "pi": [], "claude": ["--permission-mode", "acceptEdits"], "codex": ["--sandbox", "workspace-write"], "dsh": []}` | 러너 명령에 그대로 덧붙이는 인자 |
| `max_running` | `6` | 진행 중 세션의 전체 상한 |
| `max_running_by_level` | `{"1": 3, "2": 2, "3": 2, "4": 1, "5": 1}` | 레벨별 진행 중 상한. 0이면 그 레벨은 실행하지 않는다 |
| `ram_reserve_gb` | `8` | 사용자가 쓰는 앱 몫으로 남겨 둘 메모리(GB) |
| `ram_gb_by_level` | `{"1": 2, "2": 3, "3": 4, "4": 6, "5": 8}` | 그 레벨 세션 하나가 쓴다고 보는 메모리(GB). [가정] 추정치 |
| `ram_gb_per_worker` | `1.5` | 세션 안 워커 하나가 쓴다고 보는 메모리(GB). 권장 워커 수를 메모리 예산에 묶는다 |
| `max_workers` | `6` | 세션 하나에 권장하는 병렬 워커 수의 상한 |
| `primary_provider` | `"anthropic"` | 토큰 속도 판단의 기준 공급자(`capacity`의 `limits.basis`) |
| `collect_pages` | `3` | `collect`·`tick`이 출처마다 읽는 목록 쪽수 |
| `collect_skip` | `[]` | `collect --source all`과 `tick`에서 끌 출처 이름 목록 |
| `auto_launch` | `false` | `true`면 `tick`이 `--auto-launch` 없이도 라우팅·실행한다 |
| `gate_batch` | `16` | 정기 실행 한 번에 병렬로 게이트하는 `discovered` 항목 수의 상한(`maxing` 전부, `steady` 절반, `conserve` 4, `throttle` 0) |
| `persona.enabled` | `false` | 메인 세션(`harness chat`)에 운영자 페르소나 파일(`persona.file`)을 넣을지. 공모전 세션에는 이 값과 관계없이 넣지 않는다 |
| `persona.file` | `"data/persona.md"` | 운영자 페르소나 파일(git 제외, 예시 `data/persona.example.md`) |
| `persona.call_user` | `""` | 페르소나가 사용자를 부르는 말(`{{call_user}}` 자리). 페르소나 검사 대상에도 들어간다 |
| `persona.guard_terms` | `[]` | 공모전 결과물·커밋 메시지에 있으면 안 되는 페르소나 흔적(12절). 운영자가 페르소나 이름·호칭·말투 표지(웃음 표기 등)를 넣는다 |

<p>표 2. 설정 키</p>

### CLI

모든 명령은 전역 플래그 `--json`을 받는다(사람용 텍스트 대신 JSON).

| 명령 | 하는 일 |
|---|---|
| `harness collect [--source <출처>\|all] [--pages N]` | 수집 출처(아래)에서 새 공고를 모아 `discovered`로 넣고 새 항목을 출력한다. 사이트 간 중복은 `alt_urls`로 합친다. `--pages` 기본은 설정 `collect_pages`, `all`은 `collect_skip`에 든 출처를 뺀다 |
| `harness add --title T --url U [--org O] [--deadline D] [--source scout\|manual] [--note N] [--force]` | 공고 한 건을 넣는다. 중복이면 새로 만들지 않고 후보를 알려 준다(`--force`로 강제) |
| `harness dedupe-check "<제목>" [--url U] [--id 자기id] [--org O] [--deadline D]` | 제출 기록부(13절), 레지스트리, `<contests_root>/*/README.md`, `<contests_root>/.shared/existing_contests.md`에서 같은 공모전을 찾는다. `--id`는 자기 항목을 빼고 그 URL·주최·마감을 쓴다. 기록부 결과(`kind: submitted`)는 다른 결과보다 앞에 온다 |
| `harness list [--state S] [--type T]` | 목록(마감 순) |
| `harness show <id>` | 항목 전체 |
| `harness gate <id> --file gate.json` | 게이트 결과를 기록한다(`type`, `level`, `priority` 포함). `decision`에 따라 `gate_pass`/`rejected`/`hold`로 옮긴다 |
| `harness route <id> [--folder 이름] [--no-repo] [--no-workspace]` | 폴더(git 초기화·첫 커밋)·저장소·작업 공간·`CONTEST.md`를 만들고 `session`을 기록한 뒤 `routed`로 옮긴다. 제출 기록부에 같은 공모전이 있으면 거절한다 |
| `harness launch <id> [--preset 이름] [--dry-run] [--force]` | 설정 러너를 설정 작업 환경에서 띄우고 `running`으로 옮긴다. 전체 상한, 그 레벨의 상한, 메모리 예산(memory pressure 포함) 중 하나라도 여유가 없으면 거절한다(`--force`로만 넘김). `--preset`은 omp 전용이다. `--dry-run`은 명령과 지시문만 만들고 띄우지 않는다 |
| `harness peek <id> [--lines N]` | 띄운 세션의 화면(orca, tmux)이나 로그(headless) 끝 N줄(기본 40) |
| `harness check <id>` | 공모전 폴더와 커밋 메시지에서 페르소나 흔적을 찾는다(12절) |
| `harness transition <id> <상태> [--note N] [--by 누가] [--persona-ok 사유] [--score-ok 사유]` | 상태를 옮긴다(4절 규칙, 레벨 5 승인 규칙, `awaiting_approval` 페르소나 검사·점수 관문). `--score-ok`는 점수 관문을 사유와 함께 넘긴다. `--by` 기본 `agent` |
| `harness submit <id> --method M --receipt R [--files F...] [--note 승인근거] [--by 누가]` | 제출 기록을 남기고 `submitted`로 옮긴다. 제출 기록부에도 한 줄 더한다(같은 key가 있으면 갱신). 사용자 승인 근거는 `--note`에 남긴다 |
| `harness submitted` | 제출 기록부 목록(날짜, 제목, 방법, 접수 증빙). 13절 |
| `harness submitted add --title T [--url U] [--org O] [--date D] [--method M] [--receipt R] [--folder F] [--note N]` | 하네스 밖에서 낸 공모전을 기록부에 남긴다(`source: manual`). `--date`는 `YYYY-MM-DD` 또는 `YYYY-MM-DDTHH:MM+09:00`, 기본은 지금 |
| `harness submitted import` | 레지스트리에서 제출한 항목을 기록부로 한 번 옮긴다(`source: import`, 이미 있는 key는 건너뜀) |
| `harness profile fields` | 사용자 정보 입력 항목 정의 목록(14절) |
| `harness profile show [--reveal]` | 사용자 정보. 기본은 민감 값을 가린다. `--reveal`은 원래 값(승인된 제출 단계에서만) |
| `harness profile set <key> <value>` | 점 경로 키 하나를 검증해 저장한다. 파일 항목은 `data/profile/`로 복사한다 |
| `harness profile unset <key>` | 키 하나를 비운다 |
| `harness profile init` | 터미널에서 항목마다 묻는다(현재 값이 기본, Enter면 유지) |
| `harness profile eligibility` | 게이트용 자격 정보(식별 값 없음) |
| `harness profile path` | 사용자 정보 파일 경로 |
| `harness score <id> --stage plan\|draft\|final --file <judge.json> [--note N]` | ContestJudge 출력을 검증해 레지스트리 `scores`에 더한다(15절) |
| `harness score <id>` | 점수 이력(시각, 단계, 총점/목표, 판정, 가장 많이 깎인 기준 셋) |
| `harness clean <id> [--dry-run] [--purge] [--force]` | 제출 끝난 공모전의 세션을 닫고 로컬 폴더를 휴지통으로 옮긴다(16절). 조건이 하나라도 어긋나면 이유를 나열하고 아무것도 지우지 않는다. `--purge`는 완전히 지우고, `--force`는 조건 검사를 건너뛴다. `--dry-run`은 검사 결과와 할 일만 보여 준다 |
| `harness clean --all [--dry-run]` | `submitted`·`closed` 항목 전부를 같은 방식으로 훑어 정리한다 |
| `harness budget` | `omp usage --json`으로 공급자별 남은 한도와 pace를 계산해 권장 병렬 수를 낸다 |
| `harness capacity` | 한도·메모리·진행 중 세션 수를 합쳐 지금 새로 띄울 수 있는 세션 수를 낸다(필드는 아래) |
| `harness board` | 상태별 보드 `BOARD.md`를 쓴다 |
| `harness tick [--no-collect] [--auto-launch]` | 수집하고(`--no-collect`면 건너뜀), 게이트가 필요한 항목을 출력하고, `--auto-launch`이거나 설정 `auto_launch`가 `true`면 `gate_pass`·`routed` 항목을 우선순위 순으로 전체·레벨별 개수와 메모리 예산 안에서만 라우팅·실행한다(나머지는 `waiting`). 끝에 `BOARD.md`를 갱신한다 |
| `harness install [--runner R ...] [--all]` | 러너에 에이전트·스킬·확장을 설치한다(10절). 기본은 설정 `runner`, `--runner`는 여러 번 줄 수 있고, `--all`은 실행 파일이 PATH에 있는 러너 전부 |
| `harness chat [--runner R] [--workspace terminal\|orca\|tmux]` | 저장소 루트에서 메인 인터페이스 세션을 연다(12절). 기본 러너는 설정 `runner`, 기본 작업 환경은 `terminal` |
| `harness autorun` | 설정 러너의 한 번 실행 명령으로 `harness/prompts/automation.md`를 수행하고 `data/logs/autorun-*.log`에 남긴다 |
| `harness schedule install [--every-hours N]` | `harness autorun`을 N시간(기본 6)마다 돌리는 launchd 작업을 쓰고 불러온다. 작업에는 하네스 루트(`HARNESS_HOME`)와 설치 시점의 `PATH`가 고정된다 |
| `harness schedule remove` | launchd 작업을 내리고 plist를 지운다 |
| `harness schedule status` | 설치·적재 여부, 주기, 마지막 종료 코드, 마지막 `autorun` 로그 |

<p>표 3. CLI 명령</p>

종료 코드:

| 코드 | 뜻 |
|---|---|
| 0 | 성공 |
| 1 | 오류(이유는 표준 오류에 `오류: ...`로 낸다). `collect`는 모든 출처가 실패하고 추가·병합이 0건일 때, `autorun`은 러너가 0이 아닌 코드로 끝났을 때도 1이다. `transition ... awaiting_approval`의 페르소나·점수 관문 거절, 제출 기록부에 있는 공모전의 `route` 거절도 1이다 |
| 3 | 중복 후보가 있다(`dedupe-check`, `--force` 없는 `add`) |
| 4 | 페르소나 흔적이 있다(`check`) |

<p>표 4. 종료 코드</p>

`harness capacity --json` 필드:

| 필드 | 뜻 |
|---|---|
| `slots` | 지금 새로 띄울 수 있는 세션 수(전체) |
| `slots_by_level` | 레벨별 새로 띄울 수 있는 세션 수 `{"1": n, "2": n, "3": n, "4": n, "5": n}` |
| `recommended_workers` | 세션 하나 안에서 권장하는 병렬 워커 수(공급자 한도와 `ram_gb_per_worker` 메모리 예산을 함께 반영) |
| `running`, `running_by_level` | 진행 중 세션 수(전체, 레벨별) |
| `ram` | `available_gb`, `reserve_gb`, `budget_gb`, `pressure`(`normal` \| `warning` \| `critical` \| `unknown`) |
| `limits` | 적용한 상한(`max_running`, `max_running_by_level` 등 설정값)과 기준 공급자 `basis` |
| `providers` | 공급자별 남은 한도와 `mode`(`harness budget`과 같은 계산) |

<p>표 5. capacity 필드</p>

### 수집 출처

`src/sources/`에 출처 하나가 모듈 하나다. 모듈은 `NAME`, `collect(pages, known_ids)`, `url_key(url)`을 내놓고 `src/sources/mod.rs`의 `SOURCES`에 한 줄로 등록한다(계약은 그 파일 머리 주석). 새 출처는 모듈 하나 + `SOURCES` 한 줄 + `tests/source_<출처>.rs`와 `tests/fixtures/<출처>_*` 저장 응답으로 더한다.

| `--source` 이름 | 사이트 |
|---|---|
| `linkareer` | 링커리어 |
| `wevity` | 위비티 |
| `thinkcontest` | 씽굿 |
| `allcon` | 올콘 |
| `contestkorea` | 콘테스트코리아 |
| `detizen` | 대티즌 |
| `campuspick` | 캠퍼스픽 |
| `allforyoung` | 요즘것들 |
| `dacon` | 데이콘 |
| `devpost` | Devpost(온라인 해커톤만) |
| `kaggle` | Kaggle |

<p>표 6. 수집 출처</p>

제외한 곳: 캠퍼스몬(robots.txt `Disallow: /`), AIConnect(2026-07-31 서비스 종료). 2026-10-15 첫 수집에서 11곳 375건을 보고 사이트 간 중복 75건을 자동 병합해 새 공고 246건을 넣었다.

## 7. 게이트 결과(`gate.json`)

ContestGate가 쓰고 `harness gate`가 읽는다.

```json
{
  "decision": "pass | fail | hold",
  "checked_at": "2026-10-15T12:00+09:00",
  "official_url": "공식 공고 원문 URL",
  "title": "공식 명칭",
  "folder": "영문 폴더 이름 제안(영문·숫자·하이픈, 예: City-Shorts-2026). 없으면 하네스 id를 쓴다",
  "organizer": "주최/주관",
  "deadline": "2026-11-02T24:00+09:00",
  "type": "idea",
  "level": 2,
  "priority": 70,
  "eligibility": {"individual_ok": true, "team_ok": true, "student_only": false, "region": null, "age": null, "fee": 0, "offline_required": false, "ai_policy": "허용 | 표기 조건부 허용 | 금지 | 불명", "original_work_oath": "직접 창작 서약 문구 요약 또는 null"},
  "deliverables": [{"name": "아이디어 제안서", "format": "hwp", "limits": "분량 제한 없음"}],
  "criteria": [{"stage": "예비", "name": "참신성", "weight": 30}],
  "submission": {"method": "폼 | 메일 | 사이트 | 우편", "where": "URL 또는 주소", "files": 1, "notes": "첨부 1개, 1MB 추정"},
  "prize": "총 1,000만 원",
  "reasons": ["판정 근거 문장(레벨 근거 포함)"],
  "questions": ["사용자에게 물어야 할 것(최대 3개)"]
}
```

`level`은 정수 1~5다(5절). `pass`에는 `deliverables`나 `criteria`가 있어야 한다. `reasons`와 `questions`는 "참가자"를 주어로 하는 중립적인 보고서체로 쓴다. `fail` 사유 예: 지역·소속 한정, 팀 필수, 참가비, 현장 촬영·오프라인 필수, 생성형 AI 금지, 본인 경험 서술이 필수인데 대신 쓸 수 없음, 마감 경과. 이미 제출한 공모전(`dedupe-check`의 `kind: submitted`)은 게이트하지 않고 `duplicate`로 옮긴다(note `이미 제출함: <제목, 날짜>`).

## 8. 에이전트

| 에이전트 | 역할 | 쓰는 레벨 | 모델 역할 | 쓰기 권한 |
|---|---|---|---|---|
| `ContestScout` | 수집 출처 11곳 밖(정부·지자체·공공기관 누리집, 학회, 기업 자체 공모, 해외 기관)에서 새 공모전을 찾아 `harness dedupe-check` 후 `harness add` | — | `@task` | CLI 호출만 |
| `ContestGate` | 공식 원문·첨부를 읽고 7절 형식의 `gate.json`(유형·레벨 포함)을 써서 `harness gate` 호출. 참가 자격은 `harness profile eligibility --json`만 읽어 대조한다(개인 식별 값은 읽지 않는다). `dedupe-check`에 `kind: submitted`가 나오면 게이트하지 않고 `duplicate`(note `이미 제출함: <제목, 날짜>`) | — | `@task` | `gate.json` 한 파일과 CLI |
| `ContestHQ` | SPEC·PLAN·TASKS를 쓰고 워커 웨이브를 지휘, 체크포인트마다 Judge. 레벨 5는 `plan_review` 관문을 지킨다 | 3~5 | `@plan` | 공모전 폴더 |
| `DomainResearcher` | 정책·통계·보도·이전 수상작 등 논문 밖 근거를 인용과 함께 모은다 | 2~5 | `@DomainResearcher`(없으면 `@task`) | 없음(결과 반환) |
| `ContestEditor` | 평가표(`gate.criteria`) 기준으로 글 산출물의 문체·어투·문장을 점검하고 다듬는다. 내용·수치·주장은 바꾸지 않는다 | 2 이상 글 산출물 | `@task` | 호출자가 준 결과물 파일만 |
| `ReferenceScout` | 웹에서 실제 프로덕션 레퍼런스를 찾아 분석하고 `research/references.md`로 남긴다 | 5 필수, 3~4 서비스형 선택 | `@task` | `research/references.md`만 |
| `UxReviewer` | 화면이 있는 결과물의 UI/UX를 Orca 내장 브라우저로 점검한다 | 5 필수, 화면이 있는 3~4 선택 | `@task` | 없음(결과 반환, 스크린샷은 `/tmp`에만) |
| `ContestJudge` | `gate.json`의 심사기준(점수표 `RUBRIC.md`)으로 결과물을 채점하고 고칠 점을 낸다. stage `plan`이면 콘셉트·SPEC·PLAN을 "이대로 완성했을 때"로 예측 채점하고 점수를 올릴 기획 변경을 낸다. 출력 JSON은 `.harness/judge/<stage>-<n>.json`(15절). 최종본의 페르소나 흔적도 확인한다(`harness check`가 못 보는 hwp·pdf 등 포함) | 1~5 | `@slow` | 없음(결과 반환) |
| `reviewer`(omp 기본) | 코드 리뷰 | 3~5 코드 | omp 기본 정의 | omp 기본 정의 |
| `security-reviewer`(omp 기본) | 보안 리뷰(읽기 전용) | 5 필수 | omp 기본 정의 | 없음(결과 반환) |

<p>표 7. 에이전트</p>

에이전트 원본은 omp 형식(`agents/*.md`, 앞머리에 이름·설명·도구·모델 역할)이다. 다른 러너에는 `harness install`이 변환하거나 연결한다(10절). 모델 역할(`@task` 등)은 omp에서만 뜻이 있고, 변환한 claude 에이전트는 `model: inherit`(호출한 세션의 모델)를 쓴다. 에이전트·스킬·템플릿·실행 지시문은 사용자를 "사용자"로 부른다. 이 문서들이 공모전 세션의 문서로 그대로 옮겨지기 때문이다.

논문 근거는 기존 `ArticleResearcher` 에이전트(이 저장소 밖)를 쓴다. 논문 대회의 연구 도구는 운영자 환경 규칙(`data/local-rules.md`)을 따른다.

운영자 환경 규칙: `data/local-rules.md`(git 제외)가 있으면 에이전트·스킬·실행 지시문은 그 규칙을 먼저 읽고 따른다. 운영자 전용 장비(GPU 서버, 클라우드 계정), 공유 도구, 개인 플레이북, 연구 도구는 여기에 둔다. 형식은 `data/local-rules.example.md`를 본다.

실행 지시문: `harness/prompts/level_1.md` ~ `level_5.md`가 레벨별 실행 구성을 담는다. `launch.md`의 자리표시자는 `{title} {folder} {id} {type} {level} {deadline} {url} {repo} {playbook} {level_section}`만 쓴다(문자 그대로의 중괄호는 `{{ }}`).

## 9. 사람 관문

- 창작물(영상·이미지·음악) 콘셉트는 카드 2~3장을 만들어 사용자가 고른다.
- 레벨 5 계획 검수: 레퍼런스 조사와 SPEC·PLAN을 마치면 `plan_review`로 옮기고 사용자에게 검수를 요청한 뒤 멈춘다. 구현은 승인 뒤에만 하고, 승인 메시지 원문과 시각을 `harness transition <id> running --note "승인: <원문·시각>"`에 남긴다(4절).
- 제출 승인 전 페르소나 검사: `awaiting_approval`로 옮기기 전에 `harness check <id>`로 흔적이 없음을 확인한다. 흔적이 있으면 CLI가 전이를 거절한다(12절).
- 제출은 `awaiting_approval`에서 사용자 승인 후에만 한다. 승인 근거(메시지 원문과 시각)는 `SUBMISSION.md` 승인란과 `harness submit ... --note`에 남긴다. 제출 양식에 넣을 개인 식별 값은 이 단계에서만 `harness profile show --reveal --json`으로 읽는다.
- 레벨 2 이상에서 최종 점수가 목표에 못 미친 채 내야 하면 사용자에게 알리고 `--score-ok "<사유>"`로 넘긴다(4절).
- 게이트의 `questions`는 사용자에게 한 번에 묶어 묻는다.
- 승인과 질문은 메인 인터페이스 세션(12절)에서 사용자에게 받고, 공모전 세션에는 승인 원문을 인용으로만 전한다.

## 10. 러너와 작업 환경

러너(에이전트 하네스, `src/runners/<이름>.rs`)는 세션을 띄우는 명령과 설치 방식을 정한다. 작업 환경(`src/workspaces/<이름>.rs`)은 그 명령을 어디서 돌릴지 정한다. 대화형 작업 환경은 러너의 대화형 명령을, 대화형이 아닌 작업 환경(`headless`)은 러너의 한 번 실행 명령을 쓴다. 설정 `runner`와 `workspace`로 고른다.

| 러너 | 대화형 명령 | 한 번 실행 명령 | 서브에이전트 | 시스템 프롬프트 덧붙이기(메인 세션) | 모델 지정 |
|---|---|---|---|---|---|
| `omp` | `omp --cwd <폴더> @<지시문> "<첫 메시지>"` | 같은 명령 + `-p` | 있음(`task` 도구) | `--append-system-prompt <파일>` | `preset_by_level`(`--config` 덮개) 또는 `--model` |
| `pi` | `pi @<지시문> "<첫 메시지>"` | `pi -p @<지시문> ...` | 없음. 확장의 `contest_agent` 도구로 에이전트 하나를 별도 pi 프로세스로 돌린다 | `--append-system-prompt <파일>` | `--model` |
| `claude` | `claude "<지시문 본문>"` | `claude -p ...` | 있음(변환한 서브에이전트) | `--append-system-prompt-file <파일>` | `--model` |
| `codex` | `codex -C <폴더> "<지시문 본문>"` | `codex exec -C <폴더> ...`(`runner_args`에 샌드박스가 없으면 `--sandbox workspace-write`) | 있음(변환한 custom agent) | `-c developer_instructions=<본문>` | `-m` |
| `dsh` | `dsh-tui -- "<본문>"`(없으면 `dsh --profile dsh-tui -- -- "<본문>"`) | `dsh --profile headless "<본문>"` | 없음. 같은 세션에서 차례로 수행 | 플래그가 없어 첫 메시지 앞에 붙인다 | 없음 |

<p>표 8. 러너별 명령과 지원 기능</p>

- 공모전 세션의 시스템 프롬프트에는 아무것도 덧붙이지 않는다(페르소나 없음). 시스템 프롬프트 덧붙이기는 메인 세션(`harness chat`)만 쓴다.
- `runner_args.<러너>`는 지시문 앞에 그대로 덧붙는다.
- 서브에이전트가 없는 러너(`pi`, `dsh`)에는 `launch`가 실행 지시문 끝에 "같은 세션에서 차례로" 절을 덧붙인다(5절).
- pi, claude, dsh는 작업 디렉터리 플래그가 없어 작업 환경이 그 폴더에서 띄운다.

| 러너 | `harness install`이 하는 일 |
|---|---|
| `omp` | `omp plugin link <저장소>`(확장·스킬·에이전트를 패키지로 읽는다). 예전 방식으로 `~/.omp/agent/agents/`·`~/.omp/agent/skills/contest-harness`에 걸었던, 저장소를 가리키는 심볼릭 링크는 지운다 |
| `pi` | `pi install <저장소>`(로컬 경로를 `~/.pi/agent/settings.json`에 등록, 확장·스킬·프롬프트 템플릿) |
| `claude` | `agents/*.md`를 `~/.claude/agents/contest-harness/<이름>.md` 서브에이전트로 변환해 쓰고(omp 도구 이름을 Claude Code 도구 이름으로 바꾸고, 없는 도구는 뺀다), 스킬을 `~/.claude/skills/contest-harness`에 심볼릭 링크한다. 원본에서 사라진 변환본은 지운다 |
| `codex` | `agents/*.md`를 `~/.codex/agents/contest-<이름>.toml` custom agent로 변환하고, 스킬을 `~/.agents/skills/contest-harness`에 심볼릭 링크한다. 원본에서 사라진 `contest-*.toml`은 지운다 |
| `dsh` | 스킬을 `~/.agents/skills/contest-harness`에 심볼릭 링크한다 |

<p>표 9. 러너별 설치</p>

| 작업 환경 | 대화형 | `route`가 만드는 것(`workspace_id`) | `launch`가 띄우는 방식(`handle`) | `peek` |
|---|---|---|---|---|
| `orca` | 예 | `orca_repo_path` 저장소의 worktree(이름은 공모전 제목, 메모 `harness <id>`) | 그 worktree에 `orca terminal create --command "cd <폴더> && <명령>"`. handle은 Orca 터미널 handle | `orca terminal read` |
| `tmux` | 예 | 없음 | tmux 세션 `contest-harness`에 공모전마다 창 하나(창 이름은 id), `cd <폴더> && <명령>`. handle은 `contest-harness:@<창 id>` | `tmux capture-pane` |
| `headless` | 아니오 | 없음 | 표준 입력을 닫은 새 세션의 백그라운드 프로세스로 러너의 한 번 실행 명령. 출력은 `<폴더>/.harness/session.log`. handle은 `pid:<pid>:<로그 경로>` | 로그 끝 |
| `terminal` | 예 | 없음 | 지금 터미널 프로세스를 그 명령으로 바꾼다. `harness chat`의 기본값이고 `launch`에는 쓰지 않는다 | 불가 |

<p>표 10. 작업 환경</p>

## 11. 패키지와 확장

저장소는 omp와 pi가 함께 읽는 확장 패키지다. `package.json`의 `omp.extensions`와 `pi.extensions`(+ `pi.skills`, `pi.prompts`)가 같은 확장 파일 `extensions/contest-harness.ts`를 가리킨다. `harness install --runner omp`는 `omp plugin link`, `--runner pi`는 `pi install`로 등록한다(10절).

| 이름 | 종류 | 하는 일 |
|---|---|---|
| `/contest` | 명령 | 보드 요약: 상태별 건수, 제출 승인·계획 검수 대기, 보류 질문, 7일 안 마감, 용량. 보드도 갱신한다 |
| `/contest <id>` | 명령 | 공모전 하나의 요약 |
| `/contest-chat` | 명령 | 메인 세션 여는 법(`harness chat`)과 바이너리 버전 |
| `/contest-harness-init` | 명령 | 사용자 정보 설정(14절): `harness profile fields --json`의 항목을 입력 대화상자로 묻고 `harness profile set`으로 저장한 뒤 `harness profile show`(가린 값)로 요약한다 |
| `/contest-gate <id>` | 프롬프트 템플릿(`commands/contest-gate.md`) | 레지스트리 항목 하나를 ContestGate로 거르고 결과를 요약한다 |
| `contest_agent {agent, task}` | 도구(pi 전용) | `agents/<agent>.md` 본문을 시스템 프롬프트로 `pi -p --no-session` 하위 프로세스를 돌려 마지막 출력을 돌려준다. 하위 프로세스에서는 이 도구를 등록하지 않는다(재귀 금지) |

<p>표 11. 확장이 등록하는 명령과 도구</p>

- 확장은 모든 정보를 `harness ... --json` 출력에서 얻는다. 바이너리는 `HARNESS_BIN` → 저장소 `target/release/harness` → `target/debug/harness` → `PATH`의 `harness` 순으로 찾는다.
- 확장은 페르소나를 넣지 않는다. 페르소나와 메인 역할은 `harness chat`만 넣는다.
- 등록 확인(토큰을 쓰지 않는다): `omp --mode rpc --no-session --no-extensions -e <저장소>`, `pi --mode rpc --no-session -ne -e <저장소> --offline`의 명령 목록.

## 12. 메인 인터페이스와 페르소나

### 메인 인터페이스

`harness chat [--runner R] [--workspace terminal|orca|tmux]`가 저장소 루트를 작업 디렉터리로 메인 세션을 연다.

1. `data/main-system.md`를 쓴다: `persona.enabled`가 `true`면 `persona.file`(`{{call_user}}`를 `persona.call_user`로 바꾼 것), 이어서 `harness/prompts/main.md`(메인 인터페이스 역할).
2. `data/main-start.md`에 첫 지시(보드·용량 요약, 사용자에게 물을 것 묶기)를 쓴다.
3. 러너의 대화형 명령에 `data/main-system.md`를 시스템 프롬프트 덧붙이기로(10절 표 8), `data/main-start.md`를 첫 지시로 넘긴다. `headless`처럼 대화형이 아닌 작업 환경은 거절한다.

메인 세션은 사용자의 말을 하네스 명령과 에이전트 작업으로 옮기고 결과를 요약한다. 공모전 결과물(코드·주석·보고서·발표자료·서식·커밋 메시지)은 직접 쓰거나 고치지 않는다. 그 일은 페르소나 없는 공모전 세션(`harness launch`)이 한다.

### 선택 기능: 운영자 페르소나

- 운영자는 메인 세션에 말투·호칭을 입힌 페르소나를 붙일 수 있다. 공개 저장소에는 페르소나가 들어 있지 않다. 쓰려면 `data/persona.example.md`를 참고해 `persona.file`(기본 `data/persona.md`, git 제외)을 쓰고 `data/config.json`에 `"persona": {"enabled": true, "call_user": "<호칭>", "guard_terms": [...]}`를 둔다. 끄면(기본) 메인 세션은 `main.md`만 받는다.
- 페르소나는 메인 인터페이스 세션에만 들어간다. 공모전 세션, 서브에이전트, 정기 실행(`autorun`), 확장에는 넣지 않으며 설정으로도 켤 수 없다.
- 메인 세션이 공모전 세션·서브에이전트에 보내는 과제·승인 전달·수정 요청은 페르소나의 이름·호칭·반말·웃음 표기 없이 중립적인 보고서체로 쓰고, 사용자 원문은 인용으로만 옮긴다.

### 페르소나 차단 규칙

규칙: 공모전 결과물(코드·주석·보고서·발표자료·서식)과 커밋 메시지에 페르소나(이름, 사용자 호칭, 반말·애교 말투, 웃음 표기)가 들어가면 안 된다.

- 검사 대상 낱말: `persona.guard_terms`와 `persona.call_user`. `persona.enabled`와 관계없이 검사한다. 페르소나를 쓰면 그 이름·호칭·말투 표지를 `guard_terms`에 넣는다.
- `harness check <id>`: 공모전 폴더의 텍스트 파일(UTF-8)과 `git log`의 커밋 메시지에서 낱말을 찾아 `{file, line, term, text}`로 낸다. 커밋 메시지는 `file`이 `commit <짧은 해시>`다. 흔적이 있으면 종료 코드 4.
- 한글 음절로 시작하는 낱말은 낱말 첫머리에서만 센다. 바로 앞 글자가 한글 음절이면 세지 않는다(낱말이 `누리`이면 "누리가", "HQ(누리)"는 세고 "한누리" 안의 "누리"는 세지 않는다). 자모 낱말(`ㅎㅎ` 등)은 어디서나 센다.
- 건너뛰는 것: 폴더 `.git`, `.harness`, `node_modules`, `target`, `.venv`, `venv`, `__pycache__`, `.pytest_cache`, `.mypy_cache`, `dist`, `build`, `data`, `dataset`, `datasets`, `raw`, `cache`, `.cache`, 4MB 넘는 파일, 바이너리 파일(앞 8KB에 NUL), UTF-8이 아닌 파일. 따라서 hwp·pdf·pptx 같은 이진 서식의 안쪽은 보지 못한다. ContestJudge가 최종본의 텍스트를 따로 확인한다.
- `harness transition <id> awaiting_approval`은 흔적이 있으면 거절한다(종료 코드 1, 흔적 목록 출력). 작품 속 대사처럼 의도한 표현이면 `--persona-ok "<사유>"`로 넘기고, `페르소나 검사 예외(<건수>건): <사유>`가 `history`의 `note`에 남는다.
- 에이전트·스킬·템플릿·실행 지시문은 사용자를 "사용자"로, 게이트 판정 근거는 "참가자"로 쓴다.

## 13. 제출 기록부

이미 한 번 제출한 공모전을 한곳에 모아 다시 배정·제출하지 않게 한다. 레지스트리와 따로 두는 이유는 하네스 밖에서 낸 공모전과 레지스트리에서 지워지거나 바뀐 항목도 남기기 위해서다.

- 파일: 저장소 `data/submissions.json`(git 제외, 개인 상태). 쓰기는 CLI만 하고, 레지스트리와 같은 방식으로 잠금 파일 `data/submissions.lock`으로 직렬화하고 임시 파일을 바꿔치기해 저장한다.
- 형식: `{"version": 1, "submissions": [<항목>...]}`.

| 필드 | 형식 | 설명 |
|---|---|---|
| `key` | 문자열 | 레지스트리 id. 없으면 `manual-<slug>` |
| `id` | 문자열 또는 `null` | 레지스트리 id |
| `title`, `organizer`, `url` | 문자열 | 공고 제목, 주최, 대표 URL |
| `alt_urls` | 문자열 배열 | 같은 공모전의 다른 URL |
| `folder` | 문자열 또는 `null` | 공모전 폴더 경로 |
| `submitted_at` | `YYYY-MM-DDTHH:MM+09:00` 또는 `YYYY-MM-DD`(KST) | 제출 시각 |
| `method`, `receipt` | 문자열 | 제출 방법, 접수 증빙 |
| `files` | 문자열 배열 | 제출 파일 |
| `note` | 문자열 | 메모 |
| `source` | `harness` \| `manual` \| `import` | `submit`이 남긴 것, `submitted add`로 직접 적은 것, `submitted import`로 옮긴 것 |
| `repo` | 문자열(선택) | 원격 저장소 URL. `harness clean`이 기록해 나중에 `git clone`으로 되살릴 수 있게 한다(16절) |
| `folder_removed_at` | ISO 8601(선택) | `harness clean`이 로컬 폴더를 정리한 시각 |

<p>표 12. 제출 기록부 항목</p>

- `harness submit`은 레지스트리 기록 뒤 기록부에도 한 줄 더한다(같은 key가 있으면 갱신, `source: harness`).
- `harness submitted add ...`는 하네스 밖에서 낸 공모전을 남긴다(`source: manual`).
- `harness submitted import`는 레지스트리에서 한 번 옮긴다. 대상은 `submissions`가 있거나 state가 `submitted`인 항목, 그리고 state가 `closed`이면서 제목이나 `history` note에 "제출"이 들어간 항목이다(`source: import`). 이미 있는 key는 건너뛴다.
- 대조: `dedupe-check`(와 `add`의 중복 확인)는 기록부도 본다. URL 키·`alt_urls`가 같거나 제목이 비슷하면 결과에 `{"kind": "submitted", "id": <key>, "title", "submitted_at", "by": "url" | "title"}`을 넣고, 다른 결과보다 앞에 둔다. `--id`와 같은 key는 뺀다.
- `harness route`는 기록부에 같은 공모전이 있으면 `이미 제출한 공모전이다: <title> (<submitted_at>)`로 거절한다.
- ContestGate·ContestScout·메인 세션: `dedupe-check`에 `kind: submitted`가 나오면 게이트하지 않고 `duplicate`로 옮긴다(note `이미 제출함: <제목, 날짜>`).

## 14. 사용자 정보

게이트의 자격 판정과 제출 양식 작성에 쓰는 참가자 정보다. 값은 이 컴퓨터에만 둔다.

- 파일: 저장소 `data/profile.json`(git 제외). 서명·도장·재학 증빙 같은 이미지는 `data/profile/`(git 제외)로 복사하고 상대 경로로 둔다. 둘 다 0600 권한으로 쓴다. 공개 저장소에는 빈 값 예시 `data/profile.example.json`만 있다.
- 스키마 v2:

```json
{
  "version": 2,
  "identity": {"name": "", "name_en": "", "name_hanja": "", "birth_date": ""},
  "contact": {"phone": "", "email": ""},
  "affiliation": {"is_student": null, "school": "", "department": "", "level": "", "year": null, "proof": ""},
  "residence_region": "",
  "nationality": "",
  "participation": {"individual": true, "team_available": null, "team_note": ""},
  "accounts": {"hancom": ""},
  "signature": "",
  "seal": "",
  "strengths": [],
  "constraints": []
}
```

- 예전 v1 파일(`name`, `student{...}` 등)은 읽을 때 v2로 옮겨 다루고, 쓸 때 v2로 저장한다.

| 명령 | 하는 일 |
|---|---|
| `harness profile fields --json` | 입력 항목 정의 목록 `[{key, label, kind, example, sensitive}]`. `kind`는 `text`, `date`, `bool`, `int`, `file`, `list`. 정의는 Rust 한 곳에 있고 `/contest-harness-init`와 `profile init`이 이걸로 질문을 만든다 |
| `harness profile show [--json] [--reveal]` | 기본은 민감 값을 가린다: 이름 `홍*동`, 전화 `010-****-1234`, 이메일 `a***@example.com`, 생년월일 `YYYY-**-**`, 한자·영문 이름은 첫 글자만, 파일은 "있음/없음". `--reveal`은 원래 값이고 승인된 제출 단계에서만 쓴다 |
| `harness profile set <key> <value>` | 점 경로 키(`identity.name`, `contact.phone`, `affiliation.school` …). 검증: 생년월일 `YYYY-MM-DD`, 이메일에 `@`, 전화는 숫자·하이픈·`+`, bool은 예/아니오/true/false, list는 쉼표 구분. `signature`·`seal`·`affiliation.proof`는 파일을 `data/profile/`로 복사하고 상대 경로를 저장한다 |
| `harness profile unset <key>` | 키 하나를 비운다 |
| `harness profile init` | TTY에서 항목마다 묻는다. 현재 값이 기본이고 Enter면 유지한다 |
| `harness profile eligibility --json` | 게이트용. 이름·연락처·이메일·생년월일·서명 같은 식별 값 없이 `is_student`, `level`, `year`, `school`, `department`, `residence_region`, `nationality`, `age`(생년월일로 계산), `participation`, `strengths`, `constraints`만 낸다 |
| `harness profile path` | 사용자 정보 파일 경로 |

<p>표 13. 사용자 정보 명령</p>

- 읽기 규칙: ContestGate는 `harness profile eligibility --json`만 읽는다. 개인 식별 값은 사용자가 승인한 제출 단계에서 `harness profile show --reveal --json`으로만 읽는다. 에이전트는 `data/profile.json`을 직접 읽거나 고치지 않는다.
- 처음 설정: omp·pi에서는 `/contest-harness-init`(11절)를 쓴다. `harness profile fields --json`으로 항목을 받아 입력 대화상자로 묻고 `harness profile set`으로 저장한 뒤 `harness profile show`(가린 값)로 요약한다. 대화상자를 못 쓰는 모드면 터미널에서 `harness profile init`을 쓰라고 안내한다. 다른 러너는 `harness profile init`을 쓴다.

## 15. 점수 루프

레벨 2 이상은 공모전 평가기준을 점수로 바꾸고(ContestJudge), 그 점수를 올리는 방향으로 일한다. 다 만든 뒤에만 채점하지 않고, 기획 단계에서 "이대로 완성하면 몇 점"을 먼저 예측 채점해 기획을 고친 뒤에 만든다. 레벨 1은 점수 루프를 강제하지 않는다(제출 전 ContestJudge 1회는 그대로).

흐름(레벨 2~5):

1. 점수표(`RUBRIC.md`, 템플릿 `harness/templates/RUBRIC.md`): `.harness/gate.json`의 `criteria`(단계·이름·배점)를 기준별로 풀어 쓴다. 만점 조건, 감점 요인, 필요한 근거, 입상작이라면 있었을 것, 공고 원문 위치. 배점이 공개되지 않았으면 [가정] 균등 배분. 목표 총점은 SPEC의 값(기본 90/100).
2. 기획 채점(stage `plan`): 콘셉트 후보 2~3개(또는 SPEC·PLAN 초안)를 ContestJudge가 점수표로 예측 채점한다. 가장 높은 후보를 고르고 기준별 부족분을 기획에 반영해 다시 채점한다. 예측 총점이 목표 이상이 되거나 3회를 돌 때까지 하고, 그 뒤에 만든다.
3. 중간 채점(stage `draft`): 결과물 초안이 나오면 채점하고, 배점이 큰 기준의 감점부터 고친다.
4. 최종 채점(stage `final`): 제출 직전. 레벨 2는 평가표가 있으면 ContestEditor 다듬기 뒤에 채점한다.
5. 점수는 매번 `harness score`로 레지스트리에 남긴다. 기준별 점수와 총점 추이가 남아야 무엇이 점수를 올렸는지 안다.

데이터:

- ContestJudge의 출력 JSON은 공모전 폴더 `.harness/judge/<stage>-<n>.json`에 저장한다. 최상위 필드 `stage`(`plan`, `draft`, `final` 중 하나), `total`, `target`, `verdict`, `scores[{name, round, weight, score, rationale}]`(`round`는 심사 단계 이름). stage `plan`에는 선택 필드 `candidate`(채점한 콘셉트 후보)와 `plan_changes[{criterion, gain, change}]`(점수를 올릴 기획 변경)가 붙는다.
- `harness score <id> --stage plan|draft|final --file <judge.json> [--note N]`이 이 파일을 검증해 레지스트리 항목의 `scores`에 더한다. `target`이 없으면 90.
- `harness score <id>`(stage 없이)는 점수 이력을 보여 준다(시각, 단계, 총점/목표, 판정, 가장 많이 깎인 기준 셋). `harness show --json`에도 `scores`가 그대로 나온다.

| `scores` 항목 필드 | 형식 | 설명 |
|---|---|---|
| `at` | ISO 8601 | 기록 시각 |
| `stage` | `plan` \| `draft` \| `final` | 채점 단계 |
| `total` | 수 | 100점 환산 총점 |
| `target` | 수 | 목표 총점(기본 90) |
| `verdict` | `ready` \| `revise` \| `reject` | 판정 |
| `criteria` | `[{name, weight, score}]` 배열 | 기준별 배점과 점수 |
| `note` | 문자열 또는 `null` | 메모 |
| `file` | 문자열 | 읽은 Judge 출력 파일 경로 |

<p>표 14. 점수 기록</p>

관문(레벨 2 이상, 4절):

- `→ awaiting_approval`: `plan` 점수가 하나 이상 있고, 가장 최근 `final` 점수가 있으며, 그 점수가 `verdict == ready`이고 `total >= target`이어야 한다. 아니면 거절한다. 마감 등으로 목표에 못 미친 채 내야 하면 `--score-ok "<사유>"`로 넘기고, 사유와 점수가 `history` note에 남는다.
- 레벨 5 `→ plan_review`: `plan` 점수가 있어야 한다.

보드: 진행 중·승인 대기 행의 유형·레벨 칸에 최근 점수 `총점/목표`를 붙인다(예: `idea / L2 · 88/90`).

## 16. 제출 끝난 공모전 정리

제출까지 끝난 공모전은 문제가 없으면 세션을 닫고 로컬 폴더를 지운다. 결과물은 원격 저장소와 제출 기록부에 남는다.

명령: `harness clean <id> [--dry-run] [--purge] [--force]`, `harness clean --all [--dry-run]`(`submitted`·`closed` 항목 전부).

조건(모두 만족해야 지운다. 하나라도 어긋나면 이유를 `problems`에 나열하고 아무것도 지우지 않는다. `--force`는 이 검사를 건너뛴다):

1. 상태가 `submitted` 또는 `closed`다.
2. 제출 기록부(13절)에 이 공모전 기록이 있다.
3. 폴더가 git 저장소이고 원격 upstream이 있으며, 작업 트리가 깨끗하고(`git status --porcelain`이 비어 있음, 무시되지 않은 untracked 파일 없음), push 안 된 커밋이 없다(먼저 `git fetch`, `git rev-list @{u}..HEAD`가 비어 있음).
4. 레지스트리 `submissions[].files`에 적힌 파일 가운데 폴더 안에 있는 것은 모두 git이 추적한다(`git ls-files --error-unmatch`). `.gitignore`에 걸려 git에 없는 제출본이 있으면 지우지 않는다.

동작:

1. 세션 닫기: `session`의 작업 환경으로 handle을 닫는다(Orca 터미널 닫기, tmux 창 종료, headless 프로세스 종료). 작업 공간이 있으면 제거한다(Orca worktree). 작업 공간 제거가 실패해도 폴더 정리는 계속하고 경고를 낸다.
2. 폴더 삭제: 기본은 휴지통으로 옮긴다(macOS `~/.Trash/<이름>-<시각>`, 휴지통이 없으면 저장소 `data/trash/`). `--purge`면 완전히 지운다.
3. 레지스트리: `session`을 `null`로, 새 필드 `folder_removed_at`에 시각을 쓴다. `folder`는 그대로 둔다. 상태는 바꾸지 않고 `history`에 같은 상태로 note `로컬 폴더 정리: <휴지통 경로 또는 삭제>`를 남긴다.
4. 제출 기록부: 해당 항목에 `repo`(원격 URL)와 `folder_removed_at`을 기록한다.
5. `<contests_root>/.shared/existing_contests.md`의 줄은 지우지 않는다(중복 대조용).

출력 JSON: `{id, ok, problems: [...], actions: [...], trash: <경로 또는 null>}`. `--all`이면 이 객체의 배열이다.

흐름:

- 공모전 세션은 `harness submit` 뒤 `harness clean <id>`를 실행한다. 본선·발표처럼 같은 폴더로 할 일이 남았으면 `harness transition <id> running`으로 돌아가고 정리하지 않는다.
- 정기 실행은 끝에 `harness clean --all`을 돌린다.
- 메인 세션은 정리 결과(지운 폴더, 막힌 이유)를 사용자에게 알린다.
