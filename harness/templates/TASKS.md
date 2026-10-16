# 작업: [공모전 공식 명칭]

- 입력: `SPEC.md`, `PLAN.md` · 하네스 id: `[id]`

## 형식

`- [ ] T### [P] [D#] 설명 (경로)` — 레벨 4·5는 줄 끝에 `after: T###, T###`(진짜 선행 작업만)를 붙인다.

- `[P]`: 다른 파일을 다루고 끝나지 않은 작업에 기대지 않아 같은 웨이브에서 병렬로 돌릴 수 있다.
- `[D#]`: SPEC.md 결과물 번호. 준비·기반·마무리 작업은 붙이지 않는다.
- 끝난 작업은 `[X]`. 실패한 `[P]` 작업은 그대로 두고 아래에 `↳ 실패: 이유`를 단다. 같은 파일을 다루는 작업은 차례로 돌린다.

## 1단계: 준비

- [ ] T001 공고·서식 원본을 `docs/form/`에 저장 (docs/form/)
- [ ] T002 [P] 이전 수상작·심사평 조사 (research/research.md)
- [ ] T003 [P] 도구·계정 확인(한컴독스, GPU 서버 등) (PLAN.md)
- [ ] T008 (레벨 2 이상) 점수표 작성 (RUBRIC.md)
- [ ] T009 (레벨 2 이상) 콘셉트·기획 후보 2~3개 기획 채점(ContestJudge stage `plan`, `.harness/judge/plan-<n>.json`, `ach score <id> --stage plan --file <경로>`), 최고안 선택·보완을 예측 총점이 목표 이상이 되거나 3회까지 반복. T010을 돌렸으면 후보의 범위는 `research/oss.md`의 재사용 결정을 따른다 (RUBRIC.md, research/research.md) after: T008, T010(돌릴 때)
- [ ] T006 [P] (레벨 5 필수) ReferenceScout 레퍼런스 조사 (research/references.md)
- [ ] T010 [P] (결과물에 코드·모델·서비스·실험 파이프라인·데이터 처리 도구가 들어가면 필수, 레벨 2 PoC는 선택) OpenSourceScout 오픈소스 재사용 조사: 구성 요소별 reuse/fork/reference/build, 라이선스·공고 규칙, 제출물 고지 문안. DomainResearcher 조사와 같은 웨이브 (research/oss.md)
- [ ] T007 (레벨 5 필수) SPEC·PLAN·TASKS 커밋·푸시 후 검수 요청 요약과 예측 총점을 `.private/note.txt`에 쓰고 `ach transition <id> plan_review --note-file .private/note.txt`, 사용자 검수 요청하고 멈춤 (PLAN.md) after: T006, T009

**체크포인트**: SPEC.md·RUBRIC.md 확정, 기획 채점으로 고른 주제·콘셉트와 근거 기록. 레벨 5는 계획 승인(사용자가 메인 세션이나 `ach approve <id>`로 기록, `ach show <id> --json`에서 확인) 전에는 2단계로 가지 않는다.

## 2단계: 기반 (모든 결과물이 기댄다)

- [ ] T004 용어집·문체 규칙·파일 구조 고정 (PLAN.md)
- [ ] T005 [P] 공통 데이터·그림·표 준비 (data/, out/)

**체크포인트**: 기반 완료 — 결과물 작업을 병렬로 시작할 수 있다. 모든 FR·SC가 작업에 연결되었는지 확인.

## 3단계: D1 — [결과물 이름] (P1)

- [ ] T020 [P] [D1] [설명] ([경로])
- [ ] T021 [D1] [설명] ([경로]) after: T020

**체크포인트**: D1만으로 제출 가능. ContestJudge 중간 채점(stage `draft`) → `ach score`.

## 4단계: D2 — [결과물 이름] (P2)

- [ ] T030 [P] [D2] [설명] ([경로])

**체크포인트**: ContestJudge 중간 채점(stage `draft`) → `ach score`.

## 마지막 단계: 마무리

- [ ] T090 서식·형식 최종 반영 뒤 최종 파일 전 페이지를 렌더링해 비전으로 직접 검수(줄간격·배치·잘림·겹침 포함, 메타데이터·텍스트 검사만으로 완료 금지). 파일 해시·렌더 도구·전체/확인 쪽수·페이지별 근거 기록, 수정 뒤 전 페이지 재검수(HWP/HWPX 렌더·PDF 변환은 한컴독스 웹) ([경로])
- [ ] T091 AI 활용 고지·사용한 오픈소스 고지(`research/oss.md`)·서약·동의서 확인 (SUBMISSION.md)
- [ ] T092 ContestJudge 최종 채점(stage `final`) `ready`·목표 이상, `ach score <id> --stage final --file <경로>` (JUDGE.md)
- [ ] T093 SUBMISSION.md 작성, 커밋·푸시, 요약을 `.private/note.txt`에 쓰고 `ach transition <id> awaiting_approval --note-file .private/note.txt`
- [ ] T094 (`ach approve`로 제출 승인이 기록된 뒤) 개인정보·서명을 채우거나 파일을 다시 저장·변환했다면 전 페이지 재렌더·비전 재검수 및 제출 파일 해시 대조 → 제출 → `ach submit` → 커밋·푸시 → `ach clean <id>`(남은 본선·발표가 있으면 정리하지 않는다)

## 대응 확인

| 항목 | 작업 |
|---|---|
| FR-001 | T0xx |
| SC-001 | T0xx |

<p>표 1. 규칙·심사기준과 작업의 대응</p>
