---
name: contest-harness
description: "공모전·해커톤·경진대회를 찾고(수집), 거르고(게이트), 폴더·저장소로 배정하고, 레벨 1~5로 실행해 입상 수준 결과물과 제출 패키지를 만들 때 사용. `harness` CLI, 상태, 레벨, 사람 관문, 사용자의 작업 원칙, 유형별 플레이북 목록을 담는다."
---

# 공모전 하네스

공모전 한 건을 발견부터 제출 기록까지 끌고 가는 방법이다. 기준 문서는 하네스 저장소의 `docs/CONTRACT.md`이다. 이 스킬과 계약이 다르면 계약을 따른다. 아래에서 `<harness>`는 하네스 저장소 경로다(`harness profile path`가 내는 `<harness>/data/profile.md`에서 알 수 있다).

운영자 환경 규칙: `<harness>/data/local-rules.md`(git 제외)가 있으면 먼저 읽고 따른다. 계산 장비, 공유 자원 잠금, 개인 플레이북, 서식 도구 스크립트 같은 그 맥 전용 규칙이 거기 있다. 이 스킬의 일반 규칙과 다르면 그 파일이 우선한다(단, 페르소나 금지와 사람 관문은 바꿀 수 없다).

## 언제 쓰나

- 새 공모전을 찾거나 거를 때(ContestScout, ContestGate).
- `harness launch`로 공모전 폴더에서 시작된 세션. 첫 단계로 `CONTEST.md`와 `.harness/gate.json`을 읽는다. 시작 지시문은 `.harness/launch-prompt.md`에 있다.
- 공모전 결과물을 채점하거나 제출을 준비할 때.

## 흐름과 상태

```text
수집(collect, ContestScout) → 게이트(ContestGate, 레벨 1~5) → 라우팅(route) → 실행(launch: 레벨별 구성)
  → [레벨 5만: 계획 검수(plan_review, 사용자)] → 심사 관문(ContestJudge) → 제출 승인(사용자) → 제출 기록(submit) → 종료(close)
```

```text
discovered → gate_pass | rejected | hold
gate_pass  → routed → running → awaiting_approval → submitted → closed
running    → plan_review(레벨 5 계획 검수 대기) → running(승인) | skipped
어느 상태든 → skipped(사용자가 하지 않기로 함) | duplicate(이미 다룬 공모전)
hold       → gate_pass | rejected (원문 확인·사용자 답변 후)
running    ↔ awaiting_approval (승인 대기 중 수정 요청이 오면 running으로 돌아간다)
```

`closed`에는 `--note`로 결과(`won`, `lost`, `ended`, `withdrawn`)를 남긴다. 레지스트리(`data/registry.json`)는 CLI만 쓴다. 직접 고치지 않는다.

## CLI 요약

모든 명령은 `--json`을 받는다. 실패하면 0이 아닌 종료 코드와 이유를 낸다(오류 1, 중복 3, 페르소나 흔적 4).

| 명령 | 언제 |
|---|---|
| `harness collect [--source <출처>\|all] [--pages N]` | 모음 사이트 11곳 새 공고 수집(`linkareer`, `wevity`, `thinkcontest`, `allcon`, `contestkorea`, `detizen`, `campuspick`, `allforyoung`, `dacon`, `devpost`, `kaggle`). 사이트 간 중복은 자동 병합. config `collect_pages`(기본 3), `collect_skip`(끌 출처) |
| `harness add --title T --url U [--org O] [--deadline D] [--source scout\|manual] [--note N]` | 공고 한 건 등록(중복이면 기존 id 반환) |
| `harness dedupe-check "<제목>" [--url U]` | 라우팅·등록 전 중복 확인(반드시). 제출 기록부(`data/submissions.json`)도 대조하고, `kind: submitted` 결과가 맨 앞에 온다 |
| `harness list [--state S] [--type T]` / `harness show <id>` | 목록 / 항목 전체 |
| `harness gate <id> --file gate.json` | 게이트 결과 기록 → `gate_pass`/`rejected`/`hold` |
| `harness route <id> [--folder 이름] [--no-repo] [--no-workspace]` | 폴더·private 저장소(config `github_org`, 비면 로그인한 사용자 계정)·작업 환경(config `workspace`: orca·tmux·headless)·`CONTEST.md` 생성 → `routed`. 항목의 `session`에 `workspace`·`runner`가 남는다. 제출 기록부에 같은 공모전이 있으면 거절한다 |
| `harness launch <id> [--preset 이름] [--dry-run] [--force]` | 공모전 폴더에서 config `runner`(러너 어댑터 이름, 코어에는 `omp`뿐)로 세션 시작 → `running`. `--preset`은 프리셋이 있는 러너(omp)만, 그 밖에는 `model_by_level`. 전체·그 레벨의 동시 실행 상한이나 메모리 예산에 여유가 없으면 거절 |
| `harness peek <id> [--lines N]` | 실행 중인 세션의 화면·로그 끝 확인 |
| `harness check <id>` | 공모전 폴더의 텍스트 파일과 커밋 메시지에서 페르소나 흔적 검사(있으면 종료 코드 4와 위치). hwp·pdf 안쪽은 못 본다 |
| `harness transition <id> <상태> [--note N] [--persona-ok 사유] [--score-ok 사유]` | 상태 이동(규칙 검사). 레벨 5: `→ plan_review`는 `plan` 점수가 있어야 하고, `plan_review → running`은 `--note`에 승인 원문·시각 필수, 이 승인 기록 없이는 `awaiting_approval`로 못 간다. `awaiting_approval`은 `harness check`에 흔적이 있으면 거절되고, 작품 속 대사처럼 의도한 표현일 때만 `--persona-ok "<사유>"`(사유와 건수가 기록에 남는다). 레벨 2 이상의 `→ awaiting_approval`은 `plan` 점수가 하나 이상, 가장 최근 `final` 점수가 `ready`이고 `total ≥ target`이어야 한다. 마감 등으로 목표 미달로 내야 하면 `--score-ok "<사유>"`(사유와 점수가 기록에 남는다) |
| `harness score <id> --stage plan\|draft\|final --file <judge.json> [--note N]` / `harness score <id>` | ContestJudge 출력(`.harness/judge/<stage>-<n>.json`)을 검증해 항목 `scores`에 기록(`target` 없으면 90) / 점수 이력(시각, 단계, 총점/목표, 판정, 가장 많이 깎인 기준 셋) |
| `harness submit <id> --method M --receipt R [--files F...]` | 제출 기록 → `submitted`. 제출 기록부에도 한 줄 더한다 |
| `harness submitted [--json]` / `harness submitted add --title T [--url U] [--org O] [--date D] [--method M] [--receipt R] [--folder F] [--note N]` / `harness submitted import` | 제출 기록부(이미 제출한 공모전) 목록 / 하네스 밖에서 낸 공모전 기록 / 레지스트리의 제출 기록을 한 번 옮기기 |
| `harness clean <id> [--dry-run] [--purge] [--force]` / `harness clean --all [--dry-run]` | 제출 끝난 공모전 정리: 세션 닫기 + git이 추적하지 않는 제출본과 `.private/`를 `data/archive/<id>/`에 보관 + 로컬 폴더를 휴지통으로(`--purge`면 완전 삭제). 조건(`submitted`·`closed`, 제출 기록부에 있음, git 작업 트리 깨끗·push 완료, 제출본이 git이나 폴더에 있음)이 하나라도 어긋나면 이유(`problems`)만 내고 지우지 않는다(`--force`는 검사 생략). 레지스트리 `folder_removed_at`, 기록부 `repo`·`archive`로 나중에 되살린다. `--all`은 `submitted`·`closed` 전부 |
| `harness profile path\|eligibility\|show [--json]` | 사용자 정보(`data/profile.md`, 사용자가 직접 쓰는 마크다운, git 제외). 하네스는 읽기만 한다. 게이트는 `eligibility --json`(`## 참가 자격` 절)만, 개인 값은 승인된 제출 단계에서만 `show --json`(파일 전체) |
| `harness budget` / `harness capacity --json` | 공급자 한도 기반 권장 병렬 수 / 지금 새로 띄울 수 있는 세션 수: `slots`, `slots_by_level`, `recommended_workers`, `running`, `running_by_level`, `ram`(`available_gb`, `reserve_gb`, `budget_gb`, `pressure`), `limits`, `providers` |
| `harness tick [--no-collect] [--auto-launch]` / `harness board` | 주기 실행(config `auto_launch`가 `true`면 플래그 없이도 실행까지). 수집하면 `scout`(지금 현지 시가 config `schedule.scout_hours` 안인지, ContestScout를 이번 회차에 돌릴지)를 낸다 / `BOARD.md` 갱신 |
| `harness autorun` / `harness schedule install\|remove [--scheduler orca\|launchd]` / `harness schedule status` | 러너 한 번 실행으로 `harness/prompts/automation.md` 정기 실행(로그 `data/logs/`) / config `schedule.cron`(5필드 cron, 현지 시각) 주기로 Orca automation(기본) 또는 macOS launchd에 등록·갱신·삭제 / 설정 주기와 두 스케줄러의 실제 상태. 두 스케줄러에 다 있으면 `install`이 `warning`을 낸다 |
| `harness chat [--runner R] [--workspace terminal\|orca\|tmux]` | 저장소 루트에서 메인 인터페이스 세션 열기(공모전 세션이 아니다) |
| `harness install [--runner R ...] [--all]` | 러너에 에이전트·스킬·확장 설치(omp는 `omp plugin link`). `--all`은 코어에 있는 어댑터 중 CLI가 PATH에 있는 것 전부 |

## 레벨

게이트가 공모전마다 레벨 하나(1~5)를 매긴다. 정의·예시·매기는 규칙·설정(`max_running_by_level`, `ram_gb_by_level` 등)의 정본은 계약 5절이다. 아래는 요약이다.

| 레벨 | 크기 | 실행 |
|---|---|---|
| 1 | 하루 안 산출물 하나(숏폼·포스터·슬로건·짧은 수기) | 세션 하나가 유형 플레이북대로 → 마지막에 ContestJudge 1회(점수 루프는 강제하지 않는다) |
| 2 | 조사 근거가 필요한 문서 하나(아이디어 제안서, 짧은 기획서) | 메인 세션 + 점수 루프 + DomainResearcher 1(논문 근거는 ArticleResearcher, PoC를 계획하면 OpenSourceScout 선택) → 평가표가 있으면 ContestEditor → 최종 채점 |
| 3 | 여러 세션이 드는 결과물(정책 논문, 간단한 실험 논문, 간단한 서비스) | ContestHQ 절차로 SPEC·PLAN·TASKS + 점수 루프 + DomainResearcher(결과물에 코드·모델·서비스·실험 파이프라인·데이터 처리 도구가 들어가면 같은 웨이브에 OpenSourceScout 필수) + 러너의 서브에이전트 워커 웨이브 + 체크포인트마다 ContestJudge(글이면 ContestEditor, 코드면 `reviewer`) |
| 4 | 실험·피드백 루프가 있는 큰 프로젝트(며칠 서버 실험하는 연구형 대회, 데이콘) | 레벨 3 + OpenSourceScout 필수(사전학습 모델·공개 베이스라인·우승 해법) + LEDGER.md 실험 루프 + 원격 계산(GPU 서버나 클라우드 GPU, 끝나면 삭제) + 긴 작업은 작업 환경이 orca일 때 Orca orchestration 워커 |
| 5 | 1달 이상, 실제 프로덕션 수준 | ReferenceScout 레퍼런스 분석 → SPEC 초안 → DomainResearcher·OpenSourceScout 조사 → SPEC·PLAN → `plan_review`에서 멈춤 → 승인 뒤 HQ + 워커 웨이브 + UxReviewer + `security-reviewer` + UX를 해치지 않는 최적화 + ContestJudge |

<p>표 1. 레벨 요약(정본: `docs/CONTRACT.md` 5절)</p>

OpenSourceScout는 결과물에 코드·모델·서비스·실험 파이프라인·데이터 처리 도구가 들어가면 SPEC 확정·PLAN 전에 반드시 돌려 `research/oss.md`에 구성 요소별 `reuse`/`fork`/`reference`/`build`를 받는다. DomainResearcher와 같은 조사 웨이브에서 나란히 돌고, 그것을 대신하지 않는다. 레벨 2는 PoC를 계획할 때만 선택, 레벨 1과 글·창작만 하는 작업에는 쓰지 않는다. 기획 채점과 SPEC·PLAN은 이 결정을 따른다: 맞는 것이 없는 부분만 만들고, 아낀 품은 배점이 큰 심사기준에 돌린다.

워커는 세션을 띄운 러너의 서브에이전트(omp는 `task` 도구)로 띄운다. 서브에이전트 기능이 없는 러너는 해당 에이전트 지시문(`agents/<이름>.md`)을 읽고 같은 세션에서 차례로 한다. 러너와 작업 환경은 `harness show <id> --json`의 `session`에 있다.

레벨 1 세션 순서: `CONTEST.md`·`.harness/gate.json` 읽기 → 심사기준·이전 수상작 확인 → 플레이북 실행 → ContestJudge → `SUBMISSION.md` 작성 → `harness transition <id> awaiting_approval --note "<요약>"`.

레벨 5 세션 순서: ReferenceScout로 `research/references.md` → `RUBRIC.md`·SPEC 초안 → 조사 웨이브(DomainResearcher·OpenSourceScout 병렬, `research/oss.md`) → SPEC·PLAN(`## 레퍼런스`·`## 오픈소스 재사용` 절 필수) → 기획 채점(stage `plan`, `harness score`) → `harness transition <id> plan_review --note "<검수 요청 요약과 예측 점수>"` → 사용자에게 검수 요청하고 멈춤 → 승인되면 `harness transition <id> running --note "승인: <원문·시각>"` → 구현.

## 점수 루프(레벨 2 이상)

공모전 평가기준을 점수로 바꾸고 그 점수를 올리는 방향으로 일한다. 다 만든 뒤에만 채점하지 않고 기획 단계에서 먼저 채점한다. 레벨 2 이상은 이것이 기본 진행 방식이다.

1. 점수표: `RUBRIC.md`(템플릿 `<harness>/harness/templates/RUBRIC.md`)에 `.harness/gate.json`의 `criteria`를 기준별로 풀어 쓴다(배점, 만점 조건, 감점 요인, 필요한 근거, 입상작이라면 있었을 것, 공고 원문 위치). 배점이 공개되지 않았으면 [가정] 균등 배분. 목표 총점은 SPEC의 값(기본 90/100).
2. 기획 채점(stage `plan`): 콘셉트 후보 2~3개(또는 SPEC·PLAN 초안)를 ContestJudge가 "이대로 완성하면"으로 예측 채점한다. 가장 높은 후보를 고르고 기준별 부족분을 기획에 반영해 다시 채점한다. 예측 총점이 목표 이상이 되거나 3회를 돌 때까지. 그 뒤에 만든다.
3. 중간 채점(stage `draft`): 초안이 나오면 채점하고, 배점이 큰 기준의 감점부터 고친다.
4. 최종 채점(stage `final`): 제출 직전. 레벨 2는 평가표가 있으면 ContestEditor 다듬기 뒤에 채점한다.
5. 기록: ContestJudge 출력 JSON을 `.harness/judge/<stage>-<n>.json`에 저장하고 매번 `harness score <id> --stage <stage> --file <경로>`로 남긴다. 단계별 총점 추이는 `RUBRIC.md` 표에도 적는다.
6. 관문: `awaiting_approval`에는 `plan` 점수와 `ready`·목표 이상인 최근 `final` 점수가 필요하다(목표 미달로 내야 하면 `--score-ok "<사유>"`). 레벨 5 `plan_review`에는 `plan` 점수가 필요하다.

## 문서와 템플릿

공모전 폴더의 `SPEC.md`, `PLAN.md`, `TASKS.md`, `RUBRIC.md`(레벨 2 이상 점수표), `JUDGE.md`, `SUBMISSION.md`, `LEDGER.md`(레벨 4 실험·제출)는 `<harness>/harness/templates/`의 같은 이름 파일에서 시작한다. 레벨 1은 `JUDGE.md`와 `SUBMISSION.md`만 쓴다. 레벨 2는 SPEC·PLAN·TASKS 대신 `RUBRIC.md`·`JUDGE.md`·`SUBMISSION.md`로 충분하다.

## 사람 관문

- 창작물(영상·이미지·음악) 콘셉트는 카드 2~3장을 만들어 사용자가 고른다. 기다리는 동안 콘셉트와 무관한 준비를 한다.
- 레벨 5 계획 검수: 레퍼런스 조사와 SPEC·PLAN을 마치면 `plan_review`로 옮기고 검수를 요청한 뒤 멈춘다. 승인 전에는 구현하지 않는다. 승인 메시지 원문과 시각을 `harness transition <id> running --note`에 남긴다.
- 제출은 `awaiting_approval`에서 사용자 승인 뒤에만 한다. 승인 요청 전에 `harness check <id>`가 깨끗해야 한다. 승인 메시지 원문과 시각을 `SUBMISSION.md`의 승인란에 적은 뒤 제출하고 `harness submit`으로 기록한다. 승인 대기 중 수정 요청이 오면 `harness transition <id> running --note "<요청 원문>"`, 반영 뒤 `harness transition <id> awaiting_approval --note "<변경 요약>"`.
- 게이트의 `questions`와 SPEC의 NEEDS CLARIFICATION은 합쳐 최대 3개로, 한 번에 묶어 묻는다. 답을 기다리며 멈추지 않고 추천 기본값으로 진행하며 [가정]으로 표시한다.

## 사용자의 작업 원칙

- 언어: 사람에게 보이는 모든 글은 한국어. 문서(보고서·제안서·논문)는 한국어 보고서체로 쓰고, 지시어가 가리키는 대상을 분명히 하고, 용어는 처음 나올 때 정의한다. 수치와 주장은 [가정]/[문헌]/[계산]으로 표시하고 수치를 지어내지 않는다. 표 캡션은 표 아래 `<p>표 n. …</p>`.
- 페르소나 금지(절대 규칙): 운영자가 메인 세션에 선택 기능인 운영자 페르소나(`persona.enabled`, `persona.file`)를 켰더라도 페르소나는 메인 세션에만 있다. 결과물(코드·주석·보고서·발표자료·서식·README·커밋 메시지)에 그 페르소나의 이름·호칭·말투(반말·애교)·웃음 표기를 쓰지 않는다. 사람은 "사용자"·"참가자" 같은 중립어로 부른다. `harness check <id>`는 운영자 설정 `persona.guard_terms`와 `persona.call_user`의 낱말을 찾는다. 제출 승인 요청 전에 `harness check <id>`가 깨끗해야 하고(흔적이 있으면 `awaiting_approval` 전이가 거절된다), 작품 속 대사처럼 의도한 표현만 `--persona-ok "<사유>"`로 넘긴다. hwp·pdf·슬라이드 최종본은 ContestJudge가 텍스트를 꺼내 따로 확인한다.
- 목표는 입상: 심사기준과 이전 수상작을 먼저 읽고, 평가기준을 점수표로 바꾸고, 기획 단계부터 엄격한 심사위원처럼(ContestJudge) 예측 채점해 점수를 올린 뒤 만들고, 결과물도 목표 점수까지 고친다(점수 루프).
- 끝까지 자율 진행: 사용자는 자는 동안 끝까지 진행하라고 자주 맡긴다. 사람 관문(콘셉트 카드, 레벨 5 계획 검수, 제출 승인, 최대 3개 묶음 질문)에서만 멈춘다.
- 폴더·저장소: 공모전마다 `<contests_root>/<Folder>`(config `contests_root`)와 config `github_org`의 private 저장소(비면 로그인한 사용자 계정, GitHub Actions 없음). 웨이브·단계마다 커밋하고 푸시한다.
- 중복 확인: 라우팅·등록 전에 항상 `harness dedupe-check`. 이미 제출한 공모전을 다시 배정하는 사고를 막으려고 제출 기록부도 대조한다. `kind: submitted`가 나오면 다시 하지 않는다(`duplicate`, note "이미 제출함: <제목, 날짜>").
- 무거운 계산(학습·대형 추론·영상 생성)은 작업 맥에서 하지 않고 원격 GPU 서버나 클라우드 GPU에서 한다(클라우드 인스턴스는 작업이 끝나면 바로 삭제). 어떤 장비를 어떻게 쓰는지와 맥에서 꼭 돌릴 무거운 작업을 감싸는 방법은 `data/local-rules.md`를 따른다. 세션을 한꺼번에 많이 띄우지 않는다(메모리가 터진 적이 있다). `harness launch`·`tick`은 메모리 예산(쓸 수 있는 메모리 − `ram_reserve_gb`) 안에서만 띄운다. 여러 세션이 함께 쓰는 GUI 자원(브라우저 로그인 창, 이미지·음악 생성 서비스 등)은 한 번에 한 세션만 쓰고, 잠금 방법은 `data/local-rules.md`를 따른다.
- HWP 결과물: 한컴독스 웹에서 편집하고 정렬을 눈으로 확인한다(작업 환경이 orca면 Orca 내장 브라우저). 로그인 계정이 맞는지는 사용자가 제출을 승인한 뒤 `harness profile show --json`의 한컴독스 계정과 대조한다. PDF 변환도 한컴독스 웹으로만 한다(다른 변환기가 한글 글리프를 깨뜨렸다). 여닫기·변환 스크립트 선례가 있으면 `data/local-rules.md`에 있다.
- 브라우저는 작업 환경의 브라우저(orca면 Orca 내장 브라우저)를 쓰고, 사용자의 개인 브라우저를 자동화하지 않는다.
- 논문: 연구 실행과 집필 도구는 `playbooks/paper.md`와 `data/local-rules.md`를 따르고, 집필은 리뷰 반복으로 다듬는다.
- 숏폼: `playbooks/creative-media.md`를 따르고 제작 전에 콘셉트 카드 2~3장을 사용자에게 보인다.
- 모델·데이터·연구 대회: 운영 수준, 1등 목표. 검증 데이터로 학습한 점수를 보고하지 않고, 누수를 점검하고, 일일 제출 한도를 지키고, LEDGER에 실험과 제출을 모두 적는다.
- 사용자 정보: `data/profile.md`(git 제외)에 있고 사용자가 직접 쓴다. 하네스와 에이전트는 `harness profile`로 읽기만 하고 파일을 고치지 않는다. 없거나 비어 있으면 사용자에게 `data/profile.example.md`를 복사해 직접 채우게 한다. 자격 판정은 `harness profile eligibility --json`(`## 참가 자격` 절, 식별 값 없음)만 쓴다. 이름·연락처·생년월일·서명·도장·재학증명서 같은 개인 값은 승인된 제출 단계에서만 `harness profile show --json`으로 꺼내 제출 서식에 넣고, 저장소 파일·커밋·로그에 값을 쓰지 않는다.
- 토큰·메모리 활용: `harness capacity --json`의 `slots_by_level`과 `recommended_workers`에 여유가 있으면 병렬 웨이브를 넓히고, 공급자 한도나 메모리 `pressure`가 높으면 좁힌다.

## 제출 이후

- 승인받은 세션이 제출하고 접수 증빙(접수 번호, 확인 메일, 화면 캡처 경로)을 받아 `harness submit <id> --method <폼|메일|사이트|우편> --receipt "<증빙>" --files <파일...>`로 기록한다. 제출 기록부(`data/submissions.json`)에도 남아 같은 공모전이 다시 배정되지 않는다.
- 정리: 접수 증빙·폼 수정 링크·개인정보가 든 사본처럼 git에 올리지 않되 남길 파일은 공모전 폴더의 `.private/`(gitignore)에 둔다. 나머지 결과물을 커밋·push한 뒤 `harness clean <id> --dry-run`으로 막는 이유가 없는지 보고, 마지막 명령으로 `harness clean <id>`를 실행해 세션을 닫고 보관할 파일을 `data/archive/<id>/`에 남긴 뒤 로컬 폴더를 휴지통으로 옮긴다. 본선·발표·보완 제출·사용자가 직접 보낼 메일처럼 같은 폴더로 할 일이 남았으면 정리하지 않고 `harness transition <id> running --note "<남은 일>"`으로 돌아간다. 정기 실행은 끝에 `harness clean --all`을 돌린다.
- 결과 발표 뒤 `harness transition <id> closed --note won|lost|ended|withdrawn`.

## 플레이북

| 파일 | 유형(`type`) |
|---|---|
| `playbooks/creative-media.md` | `video`, `image`, `music`, `design` — 콘셉트 카드, 제작·검수 기준, AI 활용 고지 |
| `playbooks/idea-proposal.md` | `idea`, `report`(정책·사업 제안) — 공식 서식 순서, 조사, PoC, 채점 반복, 한컴독스 |
| `playbooks/paper.md` | `paper` — 연구, 집필, 학술지 양식 |
| `playbooks/model-competition.md` | `model` — 리더보드 반복 제출 대회, 실험 루프, LEDGER, 누수·한도 가드레일 |
| `playbooks/research.md` | `research` — 연구형 공모전: 오프라인 검증, 서버 실험, 1회 제출 전 재현성, 본선 발표 |
| `playbooks/data-analysis.md` | `data`, `report`(분석 보고서) — 데이터 분석 경진대회 |
| `playbooks/software-hackathon.md` | `software` — 제품 해커톤, 데모, 배포, 레벨 5 프로덕션 흐름 |
| `playbooks/essay-short-doc.md` | `essay`, `other`(슬로건·수기·짧은 신청서) |

에이전트: `ContestScout`, `ContestGate`, `ContestHQ`, `DomainResearcher`, `OpenSourceScout`, `ContestEditor`, `ReferenceScout`, `UxReviewer`, `ContestJudge`(이 저장소 `agents/`, `harness install`이 러너별로 설치), 논문 근거는 기존 `ArticleResearcher`, 코드·보안 리뷰는 러너의 기본 `reviewer`·`security-reviewer`(없으면 세션이 같은 관점으로 직접 검토). 역할·레벨은 계약 8절.
