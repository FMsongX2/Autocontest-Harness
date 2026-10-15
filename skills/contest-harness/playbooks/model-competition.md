# 모델 대회 플레이북

대상 `type`: `model`(리더보드에 반복 제출하는 예측·생성 모델 대회: 데이콘, Kaggle, 기관 AI 경진대회). 대개 레벨 4다. 리더보드 반복 제출이 없거나 적은 연구형 대회(예선 1회 제출 + 본선 발표)는 `research.md`를 따른다. 목표는 1등이고, 결과물은 운영 수준으로 재현 가능해야 한다. 패턴 출처: RD-Agent(실험 루프·판정 관문), kaggle-competition-harness(대회 메타·정직 규칙). 참고 원본은 하네스 저장소의 `references/`에 내려받아 둔다(`references/REFERENCES.md`).

## 0. 첫 단계와 등록

1. `CONTEST.md`, `.harness/gate.json`, `.harness/launch-prompt.md`를 읽는다. 운영자 환경 규칙 `data/local-rules.md`(하네스 저장소, git 제외)가 있으면 먼저 읽는다(계산 장비가 거기 있다).
2. 대회 메타를 SPEC.md 맨 위 표에 적는다: 플랫폼, 대회 URL, 규칙 URL, 과제(분류·회귀·생성 등), id 열, 목표 열, 평가 지표와 방향(높을수록/낮을수록), 일일 제출 한도와 초기화 시각(시간대 포함), 최종 선택 수, 마감(KST), 제출 방식(csv / 코드 / 업로드), 채점 때 인터넷 허용, 외부 데이터·사전학습 모델 허용 범위, 코드 공유·팀 규정.
3. 데이터를 내려받고 분할 명세(폴드 인덱스, 홀드아웃 id 목록, 시드)를 `data/splits/`에 고정한다. 한 번 고정하면 바꾸지 않는다. 바꿔야 하면 PLAN.md에 이유를 적고 이전 점수와 비교하지 않는다.
4. 학습·추론은 원격 GPU 서버나 클라우드 GPU(끝나면 바로 삭제)에서 한다. 작업 맥에서는 EDA와 작은 검증만 한다.
5. 점수표와 기획 채점(점수 루프): 최종 평가(리더보드 지표·순위, 코드 검증, 보고서·발표가 있으면 그 배점)를 점수표 `RUBRIC.md`(템플릿 `<harness>/harness/templates/RUBRIC.md`, `<harness>`는 하네스 저장소)에 풀어 쓴다. SPEC·PLAN 초안(검증 설계, 기준선, 갈래 후보)을 ContestJudge(stage `plan`)가 "이대로 완성하면"으로 예측 채점하고, 부족분을 PLAN에 반영해 다시 채점한다(예측 총점이 목표 이상이 되거나 3회까지). 매번 `.harness/judge/plan-<n>.json`에 저장하고 `ach score <id> --stage plan --file <경로>`로 기록한다. 실험 중 체크포인트는 stage `draft`, 최종 선택·코드·보고서 제출 직전은 stage `final`로 같은 방식으로 기록한다.

## 1. 반복 루프(한 바퀴)

레벨 4에서는 갈래(branch)마다 워커 하나가 이 루프를 돈다(작업 환경이 orca면 Orca orchestration 워커, 그 밖에는 러너의 서브에이전트, 서브에이전트가 없으면 이 세션이 갈래를 차례로 돈다). 갈래는 `exp/<갈래 이름>` git 브랜치다. 동시에 여는 갈래 수는 `ach capacity --json`의 `recommended_workers` 안에서 정한다.

1. 조사: DomainResearcher·ArticleResearcher로 과제 유형의 최근 우승 해법, 도메인 지식을 모은다. 첫 바퀴 전(SPEC 초안 뒤, PLAN 전)에는 같은 웨이브에서 OpenSourceScout로 공개 베이스라인·이전 우승 해법 저장소(Kaggle·DACON 토론, GitHub), 사전학습 모델(Hugging Face), 학습·검증 라이브러리를 찾아 `research/oss.md`에 구성 요소별 `reuse`/`fork`/`reference`/`build`와 외부 데이터·사전학습 모델 규칙을 받는다. 기준선은 맞는 공개 해법·모델에서 시작하고 직접 만들지 않는다. 도전 과제는 5개 이하, 각각 "데이터 기반" 또는 "도메인 기반"으로 표시한다.
2. 가설: 가설 → 비판(실현 가능성, 지표와의 정렬, 개선 방향) → 다시 쓰기. 정렬·영향·참신성을 1~10으로 매겨 고른다. 다른 갈래의 가설을 함께 보여 겹치지 않게 한다.
3. 단계: 시간 예산 사용률로 정한다. 10% 미만은 초안(기준선 완성), 중간은 개선, 마지막 구간(PLAN.md에 비율 명시, 기본 70% 이후)은 앙상블.
4. 구현 순서: 데이터 적재 → 특징 → 모델 → 앙상블 → 전체 파이프라인. 앞 단계가 긍정 판정을 받기 전에는 뒤 단계로 가지 않는다.
5. 실행: 시간 제한 안에서 돌리고 사용 시간을 예산 대비 비율로 남긴다. 산출물: `runs/<run id>/scores.csv`(모델별 행 + `ensemble` 행, 지표 이름 열 하나), `submission.csv`, `CHANGES.md`.
6. 관문 검사(사람 손 없이 먼저): 제출 파일 스키마(행 수, id 집합, 열 이름·형식) 통과, `scores.csv` 형식, 누수 점검(아래). 하나라도 실패하면 거절.
7. 판정: ContestJudge를 부른다. 순서는 제출 형식 → 평가 정렬(누수·과적합·분포 차이는 심각 위험 = 거절) → 최고 기록과 비교 → 코드 검토(미미한 개선에 큰 변경이면 거절) → 수용.
8. 기록: 실행을 갈래별 이력에 남기고(부모 실행 id 포함), 갈래 최고 기록을 갱신하고, 받아들인 아이디어를 `research/ideas.md`에 적는다.
9. 재계획: 최고 기록이 없는 상태에서 3회 연속 코딩·실행 실패, 또는 3회 연속 개선 없음이면 멈추고 공고·데이터 설명을 다시 읽고 SPEC·PLAN을 고친다. 5회 연속이면 그 갈래를 닫고 새 갈래를 연다.

## 2. 제출 후보 고르기와 제출

- 판정 수용 + 교차 검증 점수로 정렬해 상위 k개를 고르고, 고정된 홀드아웃에서 다시 확인한다.
- 새 후보의 `cv_score_mean`이 지금까지 제출한 최고 교차 검증 점수보다 `cv_score_std` 이상 높을 때만 슬롯을 쓴다.
- 리더보드 제출도 제출이다. SPEC.md에 제출 정책(하루 몇 슬롯, 예비 슬롯 1개는 마지막 날까지 남김, 어떤 조건에서 제출)을 적고 사용자 승인을 받은 범위 안에서만 중간 제출한다. 최종 선택과 코드·보고서 제출은 `awaiting_approval`에서 승인받는다.
- 제출 뒤 점수는 플랫폼 화면이나 API에서 실제로 읽어 LEDGER.md에 적는다. 점수를 추정해서 적지 않는다.

## 3. LEDGER.md(실험·제출 장부, 덧붙이기만)

템플릿: `<harness>/harness/templates/LEDGER.md`. 실행 한 번에 "실험 기록" 한 행, 제출 한 번에 "제출 장부" 한 행이고 지우거나 고치지 않는다(정정은 새 행 + `notes`). 제출 장부 필드:

`submission_id, contest_key, platform, created_kst, branch_id, run_id, parent_run_id, git_commit, submission_file_sha256, hypothesis, component, cv_scheme, cv_score_mean, cv_score_std, holdout_score, metric, metric_direction, judge_decision, eval_aligned, judge_reasoning_tag, leakage_checks_passed, leakage_checks_failed, runtime_sec, approved_by, approved_kst, platform_submission_ref, public_score, private_score, score_pulled_kst, slot_index_today, daily_limit, is_final_selection, notes`

`public_score`, `private_score`는 플랫폼에서 읽은 값만 쓴다.

## 4. 가드레일

- 일일 한도: `slot_index_today < daily_limit`를 플랫폼 초기화 시간대로 센다. 예비 슬롯 1개는 마지막 날까지 남긴다.
- 검증 규율: 폴드·홀드아웃은 등록 때 고정. 검증 데이터로 학습한 점수를 보고하지 않는다. 공개 리더보드에 맞춰 튜닝하지 않는다. 공개 점수와 교차 검증이 어긋나면 교차 검증을 믿는다. 최종 선택은 교차 검증·홀드아웃 기준이고 서로 다른 갈래에서 고른다.
- 누수 점검: 스케일러·인코더·PCA·목표 인코딩은 폴드 안에서만 적합한다. 학습·평가 id가 겹치지 않는다. 시간 순서 데이터는 시간 순서로 나눈다. 목표와 상관이 지나치게 높은 특징을 표시해 원인을 확인한다. 코드 대회는 인터넷 없이 돈다. 심각 위험이면 `eval_aligned=false`로 거절한다.
- 실행 시간: 코드 대회는 숨은 재실행 시간 예산 안에 넉넉히 끝나게 만든다. 제한에 가까운 실행은 받지 않는다.
- 변경 크기: 미미한 개선에 큰 변경이면 받지 않는다.
- 규칙: 외부 데이터·사전학습 모델 사용은 규칙 허용 범위 안에서만, 출처와 라이선스를 `docs/EXTERNAL.md`에 적는다. 다중 계정·비공개 코드 공유는 하지 않는다.

## 5. 마무리

수상 후 코드 검증·발표 자료가 필요한 대회는 재현 스크립트(`scripts/reproduce.sh`), 환경 명세, 학습 로그, 모델 설명 보고서를 준비한다. 보고서는 `data-analysis.md`의 문서 규칙을 따른다. ContestJudge(stage `final`)로 채점해 `ach score <id> --stage final --file <경로>`로 기록한 뒤, 커밋·푸시 후 최종 후보·CV·공개 점수 요약을 `write`로 `.private/note.txt`에 쓰고 `ach transition <id> awaiting_approval --note-file .private/note.txt`(`plan` 점수와 `ready`·목표 이상인 최근 `final` 점수가 필요하고, 목표 미달로 내야 하면 `--score-ok "<사유>"`).
