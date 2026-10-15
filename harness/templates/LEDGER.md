# 실험·제출 장부: [대회 공식 명칭]

- 하네스 id: `[id]` · 유형: [`model`/`research`] · 플랫폼: [데이콘/Kaggle/기관] · 지표: [metric] ([높을수록/낮을수록])
- 일일 제출 한도: [n] (초기화: [HH:MM 시간대], 연구형 1회 제출이면 "1회") · 최종 선택 수: [n] · 마감: [YYYY-MM-DDTHH:MM+09:00]
- 분할 명세: `data/splits/` [고정 시각, 시드]
- 규칙: 레벨 4는 모든 실험과 제출을 여기에 남긴다. 덧붙이기만 한다. 행을 지우거나 고치지 않고, 정정은 새 행과 `notes`로 한다. `public_score`, `private_score`는 플랫폼 화면·API에서 읽은 값만 쓴다.

## 제출 정책 (사용자 승인 범위)

- 하루 사용 슬롯: [n] · 예비 슬롯: 1개(마지막 날까지 보존)
- 제출 조건: 새 후보 `cv_score_mean` > 제출된 최고 CV + `cv_score_std`
- 승인: [메시지 원문, 시각]

## 실험 기록

제출하지 않은 실행도 한 행씩 남긴다. 제출한 실행은 아래 제출 장부에 `run_id`로 잇는다.

| run_id | created_kst | branch_id | parent_run_id | git_commit | hypothesis | component | cv_scheme | cv_score_mean | cv_score_std | holdout_score | judge_decision | leakage_checks_failed | compute | runtime_sec | notes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|

<p>표 1. 실험 기록</p>

## 제출 장부

| submission_id | contest_key | platform | created_kst | branch_id | run_id | parent_run_id | git_commit | submission_file_sha256 | hypothesis | component | cv_scheme | cv_score_mean | cv_score_std | holdout_score | metric | metric_direction | judge_decision | eval_aligned | judge_reasoning_tag | leakage_checks_passed | leakage_checks_failed | runtime_sec | approved_by | approved_kst | platform_submission_ref | public_score | private_score | score_pulled_kst | slot_index_today | daily_limit | is_final_selection | notes |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|

<p>표 2. 제출 장부</p>

## 필드 설명

| 필드 | 뜻 |
|---|---|
| `submission_id` | `S###` 순번 |
| `branch_id`, `run_id`, `parent_run_id` | 실험 갈래(`exp/<이름>`)와 실행 id, 그 실행이 이어받은 부모 실행 |
| `git_commit`, `submission_file_sha256` | 재현용 커밋과 제출 파일 해시 |
| `hypothesis`, `component` | 이 실행이 시험한 가설, 바꾼 단계(DataLoad/Feature/Model/Ensemble/Workflow) |
| `cv_scheme`, `cv_score_mean`, `cv_score_std`, `holdout_score` | 검증 방식과 점수(검증 데이터로 학습하지 않은 값) |
| `judge_decision`, `eval_aligned`, `judge_reasoning_tag` | ContestJudge 판정, 평가 정렬 여부, 이유 태그(`[Submission format error]`, `[Evaluation error]`, `[Experiment Analysis]`, `[Code Analysis]`) |
| `leakage_checks_passed`, `leakage_checks_failed` | 통과·실패한 누수 점검 목록 |
| `approved_by`, `approved_kst` | 제출 승인자와 시각 |
| `platform_submission_ref`, `public_score`, `private_score`, `score_pulled_kst` | 플랫폼 제출 번호와 실제로 읽어 온 점수, 읽은 시각 |
| `slot_index_today`, `daily_limit`, `is_final_selection` | 오늘 몇 번째 슬롯인지, 한도, 최종 선택 여부 |
| `compute` | 실험 기록의 계산 자원(`mac`, `server:<이름>`, `cloud:<인스턴스 id>`). 클라우드 인스턴스는 삭제 시각을 `notes`에 적는다 |

<p>표 3. 장부 필드</p>
