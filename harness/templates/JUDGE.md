# 심사 기록: [공모전 공식 명칭]

- 기준: `.harness/gate.json` `criteria`, `RUBRIC.md`, `SPEC.md` FR·SC · 목표 점수: [90]
- ContestJudge 결과를 체크포인트마다 아래에 덧붙인다. 이전 기록은 지우지 않는다. 원본 JSON은 `.harness/judge/<stage>-<n>.json`에 두고 `ach score <id> --stage <stage> --file <경로>`로 기록한다.
- 이 파일은 개선 채점 기록이다. 독립 최종 평가의 평가자에게는 이 파일·이전 점수·대화 이력을 주지 않는다(`ach evaluation prepare`가 이 파일을 패킷에 넣지 않는다). 독립 평가 결과는 평가가 끝난 뒤 아래 "독립 최종 평가" 표에 옮긴다.

## 판정 기록

| 시각 | 단계(stage) | 체크포인트 | 총점(100 환산) / 목표 | 판정 | CRITICAL | HIGH | 원본 JSON | 비고 |
|---|---|---|---|---|---|---|---|---|
| [YYYY-MM-DD HH:MM] | [plan/draft/final] | [D1 초안] | [ ] / [ ] | [ready/revise/reject] | [n] | [n] | [.harness/judge/draft-1.json] | [ ] |

<p>표 1. 판정 기록</p>

## 독립 최종 평가

개선이 끝난 최종본을 블라인드 패킷으로 평가한 기록이다(`contest-harness` 스킬 "독립 최종 평가"). 익명화는 CLI가 하지 않으므로 지운 항목과 확인자를 적는다. 같은 패킷을 여러 번 평가했으면 모두 적고, 좋은 값만 고르지 않는다.

| 시각 | packet_id | assessment_id | 평가자(id·모델·run_id) | 원본 파일·SHA-256 | 익명 사본 확인(지운 항목·확인자) | 총점 / 목표 | 판정 | 같은 패킷 평가 n·분산(`ach evaluation report`) | `ach score` 기록 |
|---|---|---|---|---|---|---|---|---|---|
| [YYYY-MM-DD HH:MM] | [ ] | [ ] | [ContestJudge · <모델> · <run_id>] | [submission/proposal.pdf · <해시>] | [이름·학교·머리말 · 확인 세션] | [ ] / [ ] | [ready/revise/reject] | [n=1, 분산 없음] | [.harness/judge/final-<n>.json] |

<p>표 2. 독립 최종 평가</p>

## 한글 윤문 기록

한글 본문(보고서·제안서·기획서·신청서·칼럼·에세이)마다 호출 세션이 `humanize-korean`을 실행한 뒤 한 행을 더한다(`contest-harness` 스킬 "한글 윤문 단계"). ContestEditor·리뷰어·ContestJudge는 이 표를 읽기만 한다. 최종 채점 대상 파일의 SHA-256은 그 파일의 마지막 행과 같아야 한다.

| 시각 | 대상 파일 | run_id | 경로 | 작업 폴더 | `guarded_check.py` 결과 | 직접 대조(수치·인용·수식·코드·제목·표·서식) | 채택 | 적용 뒤 SHA-256 |
|---|---|---|---|---|---|---|---|---|
| [YYYY-MM-DD HH:MM] | [docs/manuscript/본문.md] | [YYYY-MM-DD-NNN-TAG] | [light/standard/heavy] | [.private/humanize/_workspace/<run_id>/] | [종료 코드 0, 통과] | [어긋남 없음] | [윤문본 채택 / 기준본 유지: 사유] | [ ] |

<p>표 3. 한글 윤문 기록</p>

---

## [YYYY-MM-DD HH:MM] [체크포인트]

### 관문 검사

| 검사 | 통과 | 증거 |
|---|---|---|
| 형식 | [예/아니오] | [ ] |
| 분량·규격 | | |
| 파일 구성 | | |
| 자격·규칙 | | |
| 생성형 AI 정책·고지 | | |
| 직접 창작 서약 | | |
| 문서 위생(`§`, 자리표시자, 캡션, 수식) | | |
| 문서 전 페이지 비전 검수(메타데이터·추출 텍스트만으로 통과 금지) | | [최종 파일·SHA-256·렌더 도구·원본/파생 PDF 대응·전체/확인 페이지 수·페이지별 이미지/관찰/판정 기록 경로. 렌더·상세 기록은 .private/visual-review/에 두고 개인정보 값은 적지 않는다] |
| 수치 정직성 | | |

<p>표 4. 관문 검사</p>

### 기준별 점수

| 심사 단계 | 기준 | 배점 | 점수 | 근거(위치) |
|---|---|---|---|---|
| [예비] | [참신성] | [30] | [ ] | [파일:위치 — 이유] |
| | **합계** | | | |

<p>표 5. 기준별 점수</p>

### 대응 표

| 항목 | 상태 | 충족 위치 |
|---|---|---|
| FR-001 | [covered/partial/missing] | [파일:위치] |
| SC-001 | | |

<p>표 6. 규칙·심사기준 충족 위치</p>

### 발견 사항

| ID | 심각도 | 분류 | 위치 | 내용 | 고칠 점 | 처리 |
|---|---|---|---|---|---|---|
| J-001 | [CRITICAL/HIGH/MEDIUM/LOW] | [compliance/coverage/criterion/evidence/consistency/clarity/polish] | [파일:위치] | [ ] | [ ] | [미처리/고침(커밋)] |

<p>표 7. 발견 사항</p>

### 판정

[ready / revise / reject] — [한두 문장 이유]

### 검토 파일(`final`만)

ContestJudge 출력의 `artifacts`를 그대로 옮긴다. 제출 준비 기록(`ach readiness`)의 `artifacts`는 이 목록 안에 있어야 한다.

| 파일(공모전 폴더 기준) | SHA-256 |
|---|---|
| [submission/proposal.pdf] | [ ] |

<p>표 8. 최종 채점에서 검토한 파일</p>
