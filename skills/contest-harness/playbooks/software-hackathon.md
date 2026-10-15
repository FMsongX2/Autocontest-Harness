# 소프트웨어 해커톤 플레이북

대상 `type`: `software`(제품·서비스를 만들어 데모·저장소·영상으로 내는 해커톤, Devpost 포함). 간단한 정책 서비스·탐지 서비스 정도의 해커톤은 대개 레벨 3, 1달 이상이거나 실사용 프로덕션 품질이 필요하면 레벨 5다(레벨 정의는 `docs/CONTRACT.md` 5절). 계획 흐름은 Spec Kit(명세 → 명확화 → 계획 → 작업 → 일관성 점검 → 구현)과 BMAD(레벨 판단, 아키텍처 뼈대, 작업 DAG)를 공모전용으로 줄인 것이다.

## 레벨 3과 레벨 5의 차이

| 단계 | 레벨 3 | 레벨 5 |
|---|---|---|
| 레퍼런스 | 서비스형이면 ReferenceScout 선택 | SPEC 전에 ReferenceScout 필수. 같은 일을 이미 하는 프로덕션 서비스·오픈소스가 어떻게 하는지(흐름·UI 패턴·아키텍처)를 `research/references.md`에 비교받는다. 이미 있는 프로덕션이 교본이다(베끼지 않고 배운다) |
| 오픈소스 재사용 | OpenSourceScout 필수(조사 웨이브에서 DomainResearcher와 나란히). 구성 요소마다 `reuse`/`fork`/`reference`/`build`를 `research/oss.md`에 받고, 맞는 것이 없는 부분만 만든다 | 레벨 3과 같음. SPEC 초안 뒤, SPEC 확정·PLAN 전에 돌린다 |
| 계획 | SPEC → PLAN → 기획 채점(stage `plan`) → TASKS 후 바로 구현 | 레퍼런스를 본 뒤 SPEC·PLAN(`## 레퍼런스` 절 필수: 따를 패턴과 차별점) → 기획 채점(stage `plan`) → TASKS → 검수 요청 요약과 예측 점수를 `.private/note.txt`에 쓰고 `ach transition <id> plan_review --note-file .private/note.txt`(`plan` 점수가 없으면 거절) → 사용자에게 메인 세션이나 `ach approve <id>`로 승인해 달라고 요청하고 멈춘다. 승인되면 CLI가 `running`으로 옮기고(이 세션은 `approve`를 실행하지 않는다), `ach show <id> --json`에서 확인한 뒤 구현한다. 고치라고 하면 계획을 고쳐 다시 채점하고 검수를 요청한다 |
| 구현 | 러너의 서브에이전트 워커 웨이브 | 서브에이전트 워커 웨이브 + 긴 작업은 작업 환경이 orca일 때 Orca orchestration 워커 |
| 리뷰 | 체크포인트마다 ContestJudge, 코드는 기본 `reviewer` | ContestJudge + `reviewer` + UxReviewer(실행 중인 화면을 Orca 내장 브라우저로 점검) + 기본 `security-reviewer` |
| 최적화 | 하지 않는다 | UxReviewer·측정이 짚은 곳만 UX에 영향이 없을 만큼 가볍게 한다. 과하게 하지 않는다 |
| 사람 관문 | 제출 승인 | 계획 검수(`plan_review`), 제출 승인. 계획 승인 기록 없이는 CLI가 `awaiting_approval` 전이를 거절한다 |

<p>표 1. 레벨 3과 레벨 5의 소프트웨어 해커톤 흐름</p>

## 0. 첫 단계

`CONTEST.md`, `.harness/gate.json`, `.harness/launch-prompt.md`를 읽는다. 운영자 환경 규칙 `data/local-rules.md`(하네스 저장소, git 제외)가 있으면 먼저 읽는다. 해커톤 페이지의 규칙, 심사기준, 제출 항목(Devpost라면 제출 양식 필드), 후원사 트랙·필수 기술을 `docs/RULES.md`에 원문 그대로 옮긴다.

## 1. SPEC.md

- D1 = 동작하는 최소 제품(심사위원이 직접 써 볼 수 있는 핵심 흐름 하나). D2 이후 = 데모 영상, 저장소 README, 발표 자료, 후원사 트랙 요건.
- FR-###: 제출물 규칙(저장소 공개 요구, 라이선스, 새로 만든 코드만 인정하는 기간, 필수 API·플랫폼, 데모 영상 길이, 팀 규정).
- SC-###: 심사기준(기술 완성도, 아이디어, 설계·UX, 영향, 후원 기술 활용)과 배점. 같은 기준을 점수표 `RUBRIC.md`(템플릿 `<harness>/harness/templates/RUBRIC.md`, `<harness>`는 하네스 저장소)에 풀어 쓴다.
- 명확화 질문은 최대 3개.
- 레벨 5는 SPEC을 `research/references.md`를 읽은 뒤 쓰고, `## 레퍼런스` 절에 레퍼런스 요약과 그것이 D1 범위·핵심 흐름에 준 결정을 적는다.
- 공고의 외부 코드·라이브러리·기존 프로젝트 규칙(새로 만든 코드만 인정하는 기간, 오픈소스 사용 고지)을 FR로 옮기고, `research/oss.md`(OpenSourceScout)의 결정을 `## 오픈소스 재사용` 절에 적는다. 통째로 같은 일을 하는 오픈소스가 있으면 처음부터 만들지 않고 그 위에서 차별점(배점이 큰 SC)에 품을 쓴다.

## 2. PLAN.md

- 기술 맥락: 언어·프레임워크·배포처·외부 API·비용. 무료 티어와 심사 기간 내내 살아 있는 배포를 고른다.
- 아키텍처 뼈대(레벨 4·5 필수, 병렬 워커가 같은 코드를 나눠 쓰는 레벨 3도 작성): AD-n마다 Binds / Prevents / Rule. 폴더 구조, API 계약, 데이터 모델, 공통 컴포넌트를 먼저 고정해 병렬 워커가 어긋나지 않게 한다.
- 레벨 5는 `## 레퍼런스` 절에 `research/references.md`의 요약과 어느 레퍼런스의 무엇을 따르고 무엇을 다르게 하는지 적는다.
- 웨이브: 기반(뼈대·공통 모듈) → 기능별 병렬 → 통합·데모 → 다듬기.
- 기획 채점: 제품 콘셉트 후보 2~3개(또는 SPEC·PLAN 초안)를 ContestJudge(stage `plan`)가 `RUBRIC.md`로 "이대로 완성하면" 예측 채점하고, 가장 높은 안의 기준별 부족분을 SPEC·PLAN에 반영해 다시 채점한다(예측 총점이 목표 이상이 되거나 3회까지). 매번 `.harness/judge/plan-<n>.json`에 저장하고 `ach score <id> --stage plan --file <경로>`로 기록한다.

## 3. TASKS.md와 실행

- `- [ ] T### [P] [D#] 설명 (경로)` 형식, 레벨 4·5는 `after: T###`로 진짜 선행만 잇는다.
- 웨이브마다 `ach capacity --json`의 `recommended_workers`와 `ram.budget_gb`로 띄울 수를 정한다. 짧은 작업은 러너의 서브에이전트 워커(서브에이전트가 없는 러너면 이 세션에서 차례로), 레벨 5의 긴 작업은 작업 환경이 orca일 때(`ach show <id> --json`의 `session.workspace`) Orca orchestration 워커로 돌린다(`orchestration` 스킬과 `orca skills get orchestration` 안내를 먼저 읽는다). 같은 파일을 두 워커에게 주지 않는다.
- 작업 시작 전 일관성 점검: 모든 FR·SC가 작업 하나 이상에 연결되는지 표로 확인하고 빈 곳을 작업으로 채운다.
- 레벨 5는 이 시점(SPEC·PLAN·TASKS 커밋·푸시 뒤)에 `plan_review`로 멈추고, 승인 뒤에 구현 웨이브를 시작한다.
- 체크포인트(기반 완료, 핵심 흐름 동작, 데모 준비, 제출 전)마다 ContestJudge와 `reviewer`. 핵심 흐름 동작·데모 준비는 stage `draft`, 제출 전은 stage `final`로 채점해 `.harness/judge/<stage>-<n>.json`에 저장하고 `ach score <id> --stage <stage> --file <경로>`로 기록한다. 레벨 5는 핵심 흐름 동작부터 UxReviewer, 데모 준비와 제출 전에 `security-reviewer`를 더한다.

## 4. 저장소와 배포

- 저장소는 config `github_org`의 private 저장소가 기본이다(비면 로그인한 사용자 계정, GitHub Actions 없음). 해커톤이 공개 저장소를 요구하면 제출 직전 승인 단계에서 공개 여부를 사용자에게 확인한다. 비밀 키는 저장소에 넣지 않는다(`.env`는 `.gitignore`).
- 배포 URL과 시험 계정을 SUBMISSION.md에 적고, 배포 뒤 깨끗한 브라우저 탭(Orca 내장 브라우저)에서 핵심 흐름을 끝까지 눌러 확인한다.
- 무거운 모델 추론이 필요하면 원격 GPU 서버나 관리형 API를 쓰고 비용 상한을 PLAN.md에 적는다.

## 5. 데모 영상과 문서

- 데모 영상은 `creative-media.md`의 제작·검수 규칙(렌더 동시 실행 제한, `ffprobe` 확인)을 따른다. 길이 제한 안에서 첫 10초에 문제와 해결을 보인다.
- README: 한 줄 설명, 데모 링크·영상, 실행 방법, 아키텍처 그림, 사용한 후원 기술, 팀·라이선스. 해외 해커톤은 제출 문서를 영어로 쓰되 사용자에게 보이는 요약은 한국어로 쓴다.
- 생성형 AI 사용 고지가 규칙에 있으면 README와 제출 양식에 넣는다.

## 6. 제출 준비

SUBMISSION.md(제출 양식 필드별 답, 저장소·배포·영상 URL, 팀원 정보 항목)를 채우고 커밋·푸시 후 요약을 `write`로 `.private/note.txt`에 쓰고 `ach transition <id> awaiting_approval --note-file .private/note.txt`(`plan` 점수와 `ready`·목표 이상인 최근 `final` 점수가 필요하고, 목표 미달로 내야 하면 `--score-ok "<사유>"`). 그다음 사용자에게 메인 세션이나 `ach approve <id>`로 제출 승인을 요청하고 멈춘다. 개인 값은 제출 승인이 기록된 뒤 `ach profile show --for <id> --json`에서 꺼내 그 양식에만 넣고, 저장소 파일·커밋 메시지·로그·메모에 옮기지 않는다.
