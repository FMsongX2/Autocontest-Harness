[English](CHANGELOG.md) · **한국어**

# 변경 기록

주요 변경 기록이다. 버전은 [Semantic Versioning](https://semver.org/)을 따른다. 1.0 전에는 마이너 버전이 CLI나 파일 형식을 바꿀 수 있고, 그때는 항목에 무엇을 어떻게 옮기는지 적는다.

## 0.9.4 - 2026-11-24

- **고침**:
  - 같은 제출물이 두 번 인계됐다. 요청 토큰은 제출 준비 기록이나 점수가 더해질 때마다 바뀌는데, 0.9.2와 0.9.3은 새 토큰을 모두 새 요청으로 봤다. 그래서 한 이벤트(예: 최종 점수 기록)로 인계된 공모전이, 세션이 같은 파일과 제출처로 제출 준비 기록을 한 번 더 남기자 다시 승인·인계됐다. 이제 인계 기록에 그 요청이 내는 것의 해시(작업 판, 파일 경로와 SHA-256, 제출처, 접수 개시 일시, 제출 준비 기록의 필드, 규칙 기준)를 둔다. 이미 인계된 내용과 같은 새 토큰에는 승인만 다시 기록하고(`reapproved`, `continues`에 이전 토큰) 메시지는 다시 보내지 않는다. 이전 메시지의 전달 결과가 불명이면 승인만 기록하고 `handoff_unknown`으로 둔다. 파일이나 제출처가 바뀌거나 작업 판이 새로 시작되면 새 요청이고 다시 인계된다. 0.9.4 전의 기록에는 해시가 없으므로 그 토큰이 가리키는 제출 준비 기록에서 해시를 구해 비교한다. 인계 메시지는 토큰 하나가 아니라 `submit_approval_valid`를 확인하게 하므로, 메시지 뒤에 토큰만 바뀌어도 세션은 제출로 이어 간다.
- **추가**:
  - 제출 준비 기록의 `submission_opens_at`(선택, `YYYY-MM-DD` 또는 `YYYY-MM-DDTHH:MM+09:00`): 준비는 끝났는데 접수가 아직 열리지 않은 요청의 공식 접수 개시 일시. 그 전에는 위임 인계가 승인도 인계도 하지 않고(`waiting`, `submission_not_open`) 사용자 승인 대기로도 보이지 않는다. `ach tick`이 승인 대기 전체에 위임 인계를 돌리므로(`new_work_frozen`이어도 하고 공모전 세션 안에서는 하지 않는다) 접수가 열리면 정기 실행이 이어 간다. `ach handoff`와 위임 승인 컨트롤러도 같다.
  - 모든 위임 인계 결과와 `ach approval-status`의 `handoff` 아래에 `display {state, label, opens_at?}`: `승인됨`, `자동 승인 대기`, `제출 중`, `이미 발송/접수 확인 중`, `접수 기간 전(개시 …)`, `필수 정보 부족`, `제출 준비 중`, `마감 경과`, `인계 확인 필요`, 그리고 `user_approval_request`가 `true`일 때만 `사용자 승인 대기`. 훅의 표준 오류 줄에도 이 문구가 들어간다.
  - `BOARD.md`가 `awaiting_approval` 공모전을 "제출 단계(승인·접수)" 절에 제출 상태 칸과 함께 보이고, "사용자 확인 필요" 절의 상태 칸에도 제출 상태를 붙인다. `runtime` 요약에 `handoff`가 들어가고, `awaiting_approval`의 다음 조치에 `await_delegated_approval`, `wait_submission_window`, `finish_submission_request`, `submission_in_progress`, `inspect_handoff`, `deadline_passed`가 생겼다.
- **바뀜**:
  - `awaiting_approval` 공모전은 유효한 승인이 없기만 하면 사용자 승인이 필요한 것으로 나왔다(`user_submit_approval`, "사용자 확인 필요"). 이제는 그 요청이 사용자 승인만 남겨 둔 때에만 그렇게 나온다: 위임이 꺼져 있거나 범위 밖이거나 `--score-ok`로 점수 관문을 넘겼고 다른 것은 다 갖춘 경우다. 접수 개시, 자동 승인, 진행 중인 제출을 기다리는 건은 사용자에게 묻지 않는다.
  - 보드의 "제출 승인 대기" 절 이름이 "제출 단계(승인·접수)"로 바뀌었다. omp 확장의 `/contest`도 같은 절에 상태 문구를 붙여 보인다.
  - 위임 승인 컨트롤러가 `reapproved`·`would_reapprove`를 기록하고, `approval_blocked` 알림 상태에 `display:<state>`를 함께 남긴다.
  - 인계 메시지가 세션에 사용자 승인을 다시 요청하지 말 것, 같은 파일을 다시 기록하지 말 것, 접수가 아직 열리지 않았으면 `submission_opens_at`을 기록할 것을 적는다.
- **옮기는 법**:
  - 데이터는 옮길 것이 없다. "접수 개시 전"을 `blockers`에 적어 둔 세션은 `submission_opens_at`으로 다시 기록하기 전까지 지금처럼 대기한다(`필수 정보 부족`).

## 0.9.3 - 2026-11-24

- **추가**:
  - `ach gate-criteria … [--criteria-key K]`로 공식 부문·트랙 하나의 심사기준을 채운다. 수락된 공식 규칙이 부문마다 다른 기준을 두면(부문마다 `key`가 다른 `criteria` 규칙 하나씩) 이 명령은 정정 기준 행을 모든 부문의 행을 합친 목록과 대조했다. 그래서 출품한 한 부문의 기준은 늘 거절됐고 그 항목은 독립 평가 패킷을 만들 수 없었다. `--criteria-key K`를 주면 규칙 하나와 대조한다. `key`가 `K`(영문자·숫자·`.`·`_`·`-`, 100자 이하)인 수락된 `criteria` 규칙이 정확히 하나이고, 그 규칙의 모든 원소가 구조화된 행이며, 정정 기준 행이 그 규칙의 행 전부와 같아야 한다(개수·심사 단계·이름·배점). 부문 기준의 일부, 여러 부문의 행을 합친 것, 어느 규칙에도 없는 행, 규칙이 없거나 둘 이상인 `key`, 구조화되지 않은 원소가 있는 규칙은 거절한다. 이 명령의 다른 조건은 그대로 적용된다(공모전 세션 밖, 제출 기록·해소되지 않은 제출 시도 없음, 검토한 작업 판과 요청 토큰, `criteria`만 다른 게이트, 보관 때의 해시 그대로인 원문 스냅숏, 같은 기준 행을 쓴 점수, 이번 승인 대기의 제출 승인 기록 없음, 독립 평가 기록 없음, 기록과 같은 폴더 사본).
  - 결과와 `extra.gate_amendments` 기록에 `criteria_key`(옵션이 없으면 `null`)가 남고 `evidence.rules`는 고른 규칙 하나만 인용한다. `history` 표시에도 그 `key`가 적힌다. 다른 부문의 규칙은 지우거나 바꾸지 않는다.
- **바뀜**:
  - `--criteria-key`가 없으면 대조는 전과 같다(수락된 `criteria` 규칙 전체의 행을 한 목록으로). 그 대조가 거절하고 `key`가 있는 `criteria` 규칙 여럿이 대조에 쓰였으면 거절 사유에 고를 수 있는 `key`가 나온다.
- **옮기는 법**:
  - 없다. 이 명령을 실행한 항목만 바뀐다.

## 0.9.2 - 2026-11-24

- **추가**:
  - 위임 인계(`ach handoff [<id>] [--dry-run] [--resend-request 토큰]`, `src/handoff.rs`). 상시 위임(`delegation`)이 켜져 있으면 준비 요청이 `awaiting_approval`에서 자격을 갖추는 순간 사람의 추가 지시 없이 CLI가 승인을 기록하고 기록된 공모전 세션에 알린다. `ach transition … awaiting_approval`, `ach readiness --file`, `ach score --file`, `ach rules review`·`acknowledge`가 성공한 직후 그 항목에 한 번 실행되므로 상태 전이와 제출 준비 기록의 순서와 관계없이 동작한다. 메인 세션과 위임 승인 컨트롤러는 `ach handoff`로 실행한다. 승인은 기존 위임 승인 그대로다. `approval-status`가 `eligible`이어야 하고, 쓰기 직전에 레지스트리 잠금 안에서 위임 범위, 페르소나 검사, 위임 승인 근거, 핵심 조건, 요청 토큰을 다시 대조한다. 세션에는 `orca orchestration send --type handoff`로 알리고 터미널에는 입력하지 않는다.
  - 멈추는 조건은 `{status, reasons}`의 안정된 코드로 나온다. 위임이 꺼져 있거나 그 공모전이 범위 밖(남은 인계도 멈춘다), 제출 기록이 있거나 `intent`·`dispatching`·`unknown`·`confirmed` 시도가 있거나 지금 작업 판의 시도가 있음, 다른 곳에서 기록한 유효한 승인이 있음, 마감이 지남, 기록된 Orca 작업 공간의 살아 있는 omp 터미널로 확인되지 않는 세션.
  - 요청마다 기록 `data/handoff/<id>--<request_token>.json`(`claimed → approved → sending → sent | send_failed | unknown`)을 두고, 한 잠금 아래에서 다음 단계로 가기 전에 저장한다. `sent`인 요청은 다시 보내지 않는다. 보내는 도중 끊긴 실행은 `unknown`으로 읽고 자동으로 다시 보내지 않는다(컨트롤이 세션에 도착하지 않았음을 확인한 뒤 `--resend-request <토큰>`). Orca가 거절을 답한 경우는 다음 실행이 세 번까지 다시 보낸다. 끊기기 전에 기록된 승인은 다시 기록하지 않는다.
  - 결과의 `user_approval_request`가 사용자 자신의 승인이 필요한 요청(위임이 꺼짐·범위 밖, 또는 `--score-ok` 예외이고 사용자 승인 조건은 모두 맞음)과 사유가 풀려야 하는 대기를 구별한다. 위임 승인 컨트롤러는 둘을 다른 알림(`user_approval_request`, `approval_blocked`)으로 남기고, 승인·인계 자체가 어긋나면 `handoff_attention`을 남긴다.
  - `ach contract`의 `capabilities`에 `delegated_handoff`가 생겼다.
- **바뀜**:
  - `ach controller approve`는 직접 승인하고 알리지 않고 승인 대기 공모전마다 `ach handoff <id> --trigger controller`를 실행한다. 예전에는 세션 터미널에 알림 글을 입력했는데(`orca terminal send`) 그 경로를 없앴다. 기록 이름 `approved`·`would_approve`는 `handed_off`·`would_approve_and_hand_off`·`would_hand_off`가 됐다.
  - 승인 정책 함수를 실행 파일에서 `src/approval.rs`로 옮겨 `ach approval-status`, `ach approve --delegated`, 위임 인계가 같은 코드로 판단한다. 두 명령의 동작은 그대로다.
- **옮기는 법**:
  - 데이터는 옮길 것이 없다. `delegation.enabled: true`이면 이 버전부터 공모전 세션이 준비를 마치는 즉시 승인·인계된다. 손으로 승인하려면 끈다(`ach config set delegation.enabled false`). 위임 승인 컨트롤러는 이 버전의 CLI(`ach handoff`)가 필요하다.

## 0.9.1 - 2026-11-24

- **추가**:
  - `ach gate-criteria <id> --file <정정 gate.json> --expected-revision N [--expected-request 토큰] [--dry-run]`. 진행 중인 공모전(`running`·`plan_review`·`awaiting_approval`)의 게이트 기록에서 비어 있는 심사기준만 채운다. `criteria: []`로 기록된 게이트는 라우팅 뒤 다시 기록할 수 없고 `ach evaluation prepare`가 그 항목을 거절하므로, 위임 승인에 필요한 독립 최종 평가를 받을 길이 없었다. 파일은 기록된 게이트에서 `criteria`만 채운 사본이다. 다음 조건을 모두 만족할 때만 쓴다. 공모전 세션 밖에서 실행한다. 제출 기록과 해소되지 않은 제출 시도가 없다. 작업 판과 요청 토큰이 넘긴 값과 같다. 기록된 `criteria`가 비어 있고 파일이 다른 필드에서는 기록된 게이트와 같다. 기준 행이 수락된 규칙 판 `criteria` 범주의 구조화된 행 `{stage?, name, weight?}`과 같고 그 원문 스냅숏 해시가 보관 때와 같다. 기록된 모든 점수가 같은 기준 행을 썼다. 이번 승인 대기에 제출 승인 기록이 없다. 독립 평가 패킷·평가 기록이 없다. `gate.criteria`와 폴더 사본 `.harness/gate.json`을 고치고, 변경 내용을 `extra.gate_amendments`에 더하고(전·후 값, 근거, 파일 해시, 작업 판, 요청 토큰), 같은 상태의 `history` 표시를 남긴다. 상태·작업 판·요청 토큰·점수·제출 준비 기록은 그대로다. `--dry-run`은 같은 검사를 하고 아무것도 쓰지 않는다.
- **옮기는 법**:
  - 없다. 이 명령을 실행한 항목만 바뀐다. 공모전 세션은 바뀐 `.harness/gate.json`을 커밋한다.

## 0.9.0 - 2026-11-13

- **바뀜**:
  - 점수 루프에 한도를 둔다. 26개 공모전의 Judge 실행 421회를 보면 같은 묶음을 다시 채점해도 총점이 중앙값 9점(최대 11.5점) 흔들렸고, 최고점 갱신 폭은 2회차 +3.9, 3회차 +1.7, 4회차 +2.6, 5회차부터는 평균 0.7 이하였다. 제출 관문이 최근 `final`의 `total >= target`을 요구해 세션이 운 좋은 표본이 나올 때까지 다시 채점했다(한 공모전은 draft·final 41회, 끝에는 85.5~92를 오갔다). 기획 채점은 후보마다 따로 Judge를 불러 전체 실행의 40%를 차지했고, 따로 받은 후보 점수는 그 흔들림을 사이에 두고 비교됐다.
  - ContestJudge 판정은 발견 사항으로 정한다. 페르소나 흔적 말고 관문 실패나 규칙·자격 위반이면 `reject`, 페르소나 흔적이나 CRITICAL·HIGH(한글 윤문 기록 HIGH 포함)가 남으면 `revise`, 그 밖에는 `ready`다. 총점이 목표 미만인 것만으로는 `revise`가 아니다. 독립 평가 후보도 같다.
  - 새 설정 `final_rounds`(정수, 1 이상, 기본 3). `ach score --stage final`이 그때의 값을 `final` 기록에 `final_rounds`로 남긴다(Judge 출력의 같은 키는 쓰지 않는다). 점수 관문(`ach transition … awaiting_approval`, `ach approval-status`, 제출 준비·승인 검사)은 최근 `final`이 지금 작업 판의 `ready`이고 검토 파일 해시가 맞아야 하는 것은 그대로이고, `final_below_target`은 지금 작업 판의 `final` 기록 수가 최근 `final`에 남은 `final_rounds`보다 적은 동안만 낸다. 메시지에 `최근 final 86/90 목표 미달(이번 작업 판 final 2/3회)`처럼 쓴 횟수가 나온다. 그 수에 이르면 목표 미만인 `ready` `final`이 관문을 넘는다. 한도 때문에 점수 기록을 거절하지는 않는다.
  - 세션 반복 규칙(스킬 "점수 루프", CONTRACT 15절, ContestHQ, 레벨 지시문, 템플릿, 플레이북): `draft`는 체크포인트마다 한 번 채점하고 같은 체크포인트를 다시 채점하지 않는다. `final`은 작업 판마다 `final_rounds`회가 예산이고, 실행마다 CRITICAL·HIGH부터 고치며, 최근 `final`이 `ready`이고 목표 이상이거나 예산을 다 쓰면 멈춘다. 고치지 않은 파일을 더 높은 점수를 바라고 다시 채점하지 않는다. 예산을 다 쓰고도 `revise`·`reject`면 남은 CRITICAL·HIGH를 사용자에게 알리고 사용자 승인으로 `--score-ok`를 쓴다. 레벨 1은 Judge 1회, `revise`면 고친 뒤 한 번만 더.
  - 기획 채점은 회차마다 새 ContestJudge 한 번에 그 회차의 후보 전부를 넘긴다(최대 3회). Judge는 같은 근거·채점표로 후보를 하나씩 채점해 새 선택 필드 `plan_candidates[{candidate, total, scores, plan_changes}]`에 모두 내고, 최상위 값은 총점이 가장 높은 후보의 것이라 `ach score --stage plan`이 그대로 기록한다. 세션은 후보마다의 총점을 `RUBRIC.md` 추이 표에 같은 회차 번호로 적는다. 고른 후보의 예측 총점이 목표 이상이거나 남은 `plan_changes`가 흔들림보다 작으면 일찍 멈춘다.
  - 새 설정 `main_model`(문자열 또는 `null`, 기본 `null`). 메인 인터페이스 세션(`ach chat`)을 이 모델로 연다. 공모전 세션과 정기 실행에는 쓰지 않는다.
  - ContestEditor가 AI 티(같은 문형·종결 반복, 상투 연결과 번역투, 기계적인 셋 나열과 대구, 문단 끝 요약, 근거 없는 강조어)도 `humanize-korean` 분류를 기준으로 걷어 내, 내용·수치·주장은 그대로 두고 사람이 쓴 글처럼 문장 전체를 교정한다.
  - 글을 다듬는 Gemini 호출을 세션 러너가 아니라 Antigravity CLI로 한다. 새 `scripts/agy-polish.sh`가 호출자가 쓴 지시 파일로 `agy`(기본 모델 `gemini-4-hydrogen-high`)에 한 단계를 맡기고, 파일 읽기·쓰기만 허용하고 명령 실행은 막으며, 대상 파일의 SHA-256 변화를 알려 주고, 실패하면 대상 파일이 하나도 안 바뀐 경우에만 다시 시도한다. ContestEditor는 지시 파일을 쓰고 스크립트를 돌린 뒤 원본 사본과 diff를 검수해 보존 규칙을 어긴 고침을 되돌린다(`.private/editor/`에 쓰는 `write` 도구가 생겼다). humanize-korean의 역할 콜(진단·윤문·마무리)도 러너 하위 에이전트 대신 같은 스크립트로 돌린다. 실패한 호출은 보고하고 다른 모델로 바꾸지 않는다.
  - 공모전 세션 시작(`launch`, `restart`, `resume`, `recover`, `tick`, 수정 요청 뒤 `running`)을 막는 상한은 창 `max_running`(기본 6) 하나다. 메모리 예산, 메모리 압박, 토큰 throttle은 더 이상 세션을 거절하지 않고, `ach capacity`가 새 `warnings` 목록으로 알리며 `recommended_workers`와 정기 실행의 게이트 수를 정하는 데만 쓴다. `slots_by_level`은 남기되 모든 레벨에 같은 여유 수를 보인다. 설정 키 `max_running_by_level`과 `ram_gb_by_level`은 없앴다.
- **수정**:
  - `ach launch`와 `ach restart`가 레지스트리에 기록된 작업 공간 id를 작업 환경이 이미 잊은 뒤에도(손으로 지웠거나 정리로 빠짐) 그대로 다시 써서, 몇 번을 다시 실행해도 Orca `selector_not_found`로 실패했다. 이제 `orca worktree show`가 모르는 기록된 작업 공간은 버리고 새로 만든다.
- **옮기는 법**:
  - 레지스트리 이전은 없다. 이전 점수 기록은 그대로 유효하고, `final_rounds`가 없는 `final` 기록은 3으로 본다.
  - 실행 중인 세션은 스킬을 다시 읽으면 새 규칙을 따른다.
  - 한도 없는 반복으로 되돌리는 설정은 없다. 예산을 늘리려면 `ach config set final_rounds <n>`을 쓴다. 바꾼 값은 다음에 기록하는 `final`부터 적용된다.
  - `data/config.json`에서 `max_running_by_level`과 `ram_gb_by_level`을 지운다(`ach config unset max_running_by_level`, `ach config unset ram_gb_by_level`). 이제 설정을 읽을 때 모르는 키로 거절한다. 창은 `ach config set max_running <n>`으로 정한다.

## 0.8.5 - 2026-11-12

- **수정**:
  - 배점 0으로 기록한 통과·탈락 선별 단계(예: 1차 주제 적합 선별)가 있는 게이트는 `ach score`가 모든 점수를 거절했다. Judge가 그 기준을 넣을 수도(배점은 0보다 커야 함), 뺄 수도(모든 게이트 기준을 채점해야 함) 없었기 때문이다. 이제 배점 0인 게이트 기준은 점수가 없는 선별 단계로 보고 채점을 요구하지 않는다.

## 0.8.4 - 2026-11-07

- **바뀜**:
  - ContestEditor 다듬기와 한글 윤문(`humanize-korean`)은 제출 직전에 한 번만 한다. 개선 채점이 끝나 내용이 확정된 제출 후보에 ContestEditor → 윤문 → 전 페이지 검수 → 최종 채점 순서로 돌리고, 초안 직후나 개선 반복 중에는 돌리지 않는다(스킬 점수 루프 3~5단계, 레벨 지시문, ContestHQ, ContestEditor, 플레이북, CONTRACT).
  - ContestJudge는 호출자가 다듬기를 마친 제출 후보라고 밝힌 `final`에서만 윤문 기록을 본다. `draft`와 개선 반복 중의 `final`은 기록이 없어도 HIGH 발견 사항을 내지 않는다.

## 0.8.3 - 2026-11-03

- **수정**:
  - 공모전 세션이 자기 세션 안(`ACH_SESSION`이 그 id)에서 `ach clean`을 실행하면 세션부터 닫아, 폴더 이동과 레지스트리 기록 전에 명령이 멈췄다. 이제 보관·폴더 이동·기록부와 레지스트리 저장을 먼저 끝내고 자기 세션(과 작업 공간)은 맨 마지막에 닫는다. 작업 공간에 기록되지 않은 다른 터미널이 살아 있으면 여전히 정리하지 않는다.
  - 중간에 멈춘 정리를 다시 실행하면 이미 보관된 파일(읽기 전용 공고 원문 snapshot) 때문에 실패했다. 내용이 같은 보관 파일은 건너뛰고, 내용이 다르면 덮어쓰지 않고 정리를 멈춘다.

## 0.8.2 - 2026-10-27

- **바뀜**:
  - 채점 근거를 공모전을 시작할 때 한 번 문서화한다. `RUBRIC.md` 맨 앞에 `## 0. 채점 근거` 절을 두고, 주최·주관·개최 목적·심사기준을 공고에 적힌 그대로 원문 위치와 함께 쓰며 주최 의도 요약을 붙인다. 레벨 3~5는 ContestHQ가 공고 정독 직후, 레벨 2는 세션이 조사 전에 쓴다.
  - 기획 채점을 하는 Judge 실행은 모두 이 절을 같은 잣대로 쓴다. 근거 절을 공고 원문과 대조하고, 다르면 공고 원문으로 채점하며 차이를 발견 사항으로 낸다.
  - `ach score --stage plan`은 이 절이 있고 네 행이 내용과 원문 위치로 채워질 때까지(템플릿 자리표시 없음) 거절하고, 절의 `{path, sha256}`을 `basis.document`로 남긴다.
- **옮기는 법**:
  - 진행 중인 공모전에 이 절이 없으면 다음 기획 채점 전에 `RUBRIC.md`에 더한다. 이미 기록한 기획 점수는 그대로다.

## 0.8.1 - 2026-10-27

- **바뀜**:
  - 기획 채점을 지금 진행하는 공모전에 묶는다. `ContestJudge` 기획 채점은 호출할 때마다 새 실행이고, 이 공모전의 주최·주관 기관, 개최 목적, 심사기준에 근거해 채점하라는 지시를 받는다. Judge는 그 근거를 공고 위치와 함께 `basis {organizer, purpose, criteria_source}`로 내고, 기준별 `rationale`도 그 근거에 비추어 쓴다.
  - `ach score --stage plan`은 `basis` 세 칸이나 `evaluator {id, model, run_id}`가 없으면 거절하고, 이 공모전에 이미 기록된 `run_id`도 거절한다. 앞선 Judge 실행을 다시 써서 재채점할 수 없다. 두 객체는 점수 기록에 함께 남는다.
- **옮기는 법**:
  - 0.8.1 전에 기록한 기획 점수는 그대로 유효하다. 실행 중인 세션의 새 기획 점수에는 `basis`와 `evaluator`가 필요하고, 거절 메시지가 빠진 칸을 알려 준다.

## 0.8.0 - 2026-10-27

- **추가**:
  - 검증된 결과물을 포함한 런타임 checkpoint, 관찰 전용 `ach reconcile [--apply]`, 최신 checkpoint 기반의 보수적인 `ach recover`를 추가했다. 확인할 수 없는 상태를 세션 소실로 간주하지 않고, 소유가 불명확한 터미널을 채택하지 않으며, 복구로 라우팅 대기 작업을 시작하지 않는다.
  - 공식 공고 원본 snapshot과 의미 기반 변경 검토(`ach rules capture|review|acknowledge|status`)를 추가했다. 변경 범주에 따라 게이트·계획·최종·제출 근거만 무효화하고, 바이트 변경은 검토 전까지 보류한다.
  - 개선용 채점과 분리된 블라인드 독립 평가 패킷과 측정 보고(`ach evaluation prepare|record|report|outcome|calibration`)를 추가했다. 근거 없는 입상 확률은 만들지 않는다.
  - 승인된 접수처·파일에 묶인 외부 제출 트랜잭션(`ach submission intent|start|unknown|resolve|status`)과 증빙 확정(`ach submit <id> --attempt A --file proof.json`)을 추가했다. 결과가 불명확한 제출은 증명 전까지 재시도를 막는다.
  - `controllers/` 관리 실행기(`ach controller approve|queue|briefs|preset`)에 계약 확인, 단일 실행 잠금, 반복 알림 억제, `new_work_frozen` 설정을 넣었다.
  - 보드에 런타임·다음 조치·차단 사유·근거 시각과 사용자 확인 필요 구역을 추가했다. 백엔드를 직접 조회하지 않고 저장된 기록만 사용한다.
- **바뀜**:
  - 위임 승인에는 공식 공고 추적과, 점수 대상 비영상 작업의 검증된 독립 최종 평가가 추가로 필요하다. 검토 대기·무효화된 공고 변경은 `--score-ok`를 포함한 모든 승인 경로를 막는다.
  - `ach submit`의 직접 접수 증빙 옵션을 제거했다. `ach transition … submitted`도 계속 거절하며, 외부 제출 결과가 미확인인 동안 상태 전이를 막는다.
- **수정**:
  - 관리 실행기의 프리셋 이름 검증을 복원해 대기 작업 실행과 명시적 프리셋 전환이 검증기 누락으로 실패하지 않게 했다.
  - 같은 주기에서 이미 발생한 알림도 공모전이 목록에서 사라지면 해제한다.
  - Orca 세션 상태를 `terminal show`의 `connected`로 판단한다. Orca가 기록을 남겨 둔 닫힌 터미널 때문에 `restart`·`session-close`가 막히지 않고, 연결이 끊긴 터미널은 실행 중으로 세지 않는다.
  - omp 복원 시 메시지를 넘기지 않고(`omp --restore`는 프롬프트를 받지 않는다) 프리셋의 기본 모델을 명시해, 복원한 세션이 새 프리셋 모델로 바뀐다. `restart`는 메시지 미전달을 경고로 알리고, `recover`는 checkpoint 안내를 `.harness/recovery-<seq>.md`에도 남긴다.
  - `.gitignore`가 커밋된 예시 파일을 뺀 `data/` 아래 전부(실행기 상태·공고 snapshot·지시서 포함)를 로컬에만 둔다.
  - `rustls-webpki`를 0.103.16으로, `webpki-roots`를 1.0.11로 올렸다(RUSTSEC-2026-0382, RUSTSEC-2026-0381: 신뢰가 철회된 중간 인증서를 받아들일 수 있던 문제).
- **옮기는 법**:
  - `scripts/install.sh --from-source`로 빌드한다. `data/` 아래에 있던 실행기 스크립트는 `controllers/`(`ach controller …`)로 옮겼다. 로컬 실행기는 새 CLI로 다시 띄운다.
  - 기존 공모전에 공고 근거·독립 평가·제출 트랜잭션을 자동 생성하지 않는다. 해당 기록이 생기기 전까지 위임 승인은 막힌다.

## 0.7.0 - 2026-10-25

- **바뀜**:
  - `ach readiness`에 차단 조건·사용자 확인 필요·후속 일정·제출 파일 SHA-256을 구조화해 기록한다. 로컬 컨트롤러는 `ach approval-status`의 승인 유효성·적격 판정을 쓰며 노트 문구로 준비 상태를 추정하지 않는다.
  - 최종 점수와 제출 승인을 현재 작업 revision·파일 목록에 묶는다. 수정·새 채점(reject 포함)·파일 변경·새 준비 요청이 기존 승인을 무효화한다. 총점 합계, 양수 배점, 중복 기준과 공개된 게이트 평가표를 검증한다.
  - 범위와 철회가 명시된 제출 위임 설정을 추가하고 기본값은 꺼 둔다. `approve --delegated --expected-request`는 잠금 안에서 다시 검사하며 공모전 세션은 위임 설정 변경·자기 승인을 할 수 없다. 개인정보·기존 서명은 승인 원본을 보존한 별도 최종 사본에 반영하고 재검수한 해시로 연결한다.
  - `ach restart`는 실행 환경·추가 인자·대화 기록을 보존한다. `ach resume`은 용량 확인과 실제 세션 복원 뒤 running으로 옮긴다. `ach session-close`는 잠금 안에서 상태·이력을 대조하고 실패를 재시도할 수 있게 남긴다. submitted로의 직접 전이와 제출·후속 대기에서 running으로의 직접 전이는 거절한다.
  - Orca handle 만료를 빈 작업 공간으로 간주하지 않는다. 현재 터미널 목록을 대조해 소유권이 확인되지 않은 터미널이나 유휴 여부가 불명확한 shell이 있으면 재시작·작업 공간 제거를 거절한다.
  - 한글 윤문은 호출 세션이 실행하고 Editor·Judge는 보존 검사 기록과 해시를 확인한다. 윤문 근거 누락은 HIGH/revise로 일치시킨다. brief 갱신은 관리 구역만 바꾸고 수동 변경을 감지하며, 명시적으로 이전 형식을 채택할 때 원문을 백업한다.
  - 보고서·제안서·논문·신청서 등 페이지가 있는 제출 문서는 렌더된 모든 페이지를 비전으로 직접 확인해야 한다. 메타데이터·서식 설정값·추출 텍스트·OCR·이미지 생성 성공으로 전 페이지 검수를 대신하지 않는다.
  - 작성 플레이북, 실행/HQ 지시, ContestEditor, ContestJudge, 제출 템플릿에 파일 해시와 페이지별 검수 근거를 요구한다. 수정·서명·재저장·변환 뒤에는 이전 시각 검수를 무효로 하고 전 페이지를 재검수한다. 최종본의 미검수 페이지가 있으면 ContestJudge가 거절한다. `ach`의 이미지 판독 기능이 아니라 에이전트 검수 지침이다.
  - 접수 후 미래 본선·발표 일정만 남은 공모전은 `followup_pending`으로 옮겨 폴더·접수 증빙을 보존하면서 실행 슬롯을 반환한다. 미제출 건은 이 상태로 갈 수 없으며 실제 후속 작업은 `ach resume`으로 재개한다.

- **옮기는 법**:
  - `scripts/install.sh --from-source`로 현재 소스를 설치한다. 이전 점수·증빙·승인 기록은 보존하되 새 제출 권한으로 자동 승격하지 않는다. 현재 파일 재검수 → 새 final·구조화된 readiness 기록 → 새 승인이 필요하다.
  - 기존 사용자 위임의 범위를 확인한 뒤 `ach config`로 기록한다. 새 CLI로 로컬 컨트롤러를 재시작하고, 기존 brief는 dry-run 뒤 명시적으로 legacy 구역을 갱신한다. 마이그레이션 명령이 공모전을 제출하거나 세션을 종료하지 않는다.
## 0.6.0 - 2026-10-16

사용자는 메인 세션과 대화하고, 하네스 코어는 에이전트가 부른다. 코어의 이름은 이제 `ach`이고 사용자 `PATH`에 설치하지 않는다.

- **바뀜**:
  - **바이너리 이름이 `ach`다**(전에는 `harness`). 릴리스 파일은 `ach-<버전>-<대상>.tar.gz`다. `scripts/install.sh`는 클론의 `bin/ach`에 설치하고 `PATH`에는 아무것도 링크하지 않는다. 예전 버전이 설치한 `~/.local/bin/harness` 링크와 `bin/harness`는 지운다.
  - **환경 변수**: `HARNESS_HOME`은 `ACH_HOME`, `HARNESS_SESSION`은 `ACH_SESSION`, omp 확장의 `HARNESS_BIN`은 `ACH_BIN`이 됐다.
  - **에이전트가 `ach`를 찾는 법**: 하네스가 띄우는 모든 세션(공모전 세션, 메인 세션 `bin/ach chat`, 정기 실행)은 `PATH` 맨 앞에 클론의 `bin/`을 받는다. omp 확장은 자기를 불러온 omp 세션에 같은 일을 하고, Orca automation의 프롬프트에는 바이너리 경로가 적히며, 스킬은 그 밖의 세션에 `<클론>/bin/ach`를 부르게 한다.
  - **정기 실행**: 0.5 이전에 만든 작업(`HARNESS_HOME`으로 `harness`를 부른다)과 프롬프트가 예전 것인 Orca automation을 `ach schedule status`와 `ach doctor`가 설정과 다르다고 알린다. `ach schedule install`이 다시 쓰고, 이미 있는 Orca automation의 프롬프트도 이제 고친다.
  - **문서**: README를 에이전트 중심으로 고쳤다. 설치하고, 에이전트에게 `INIT.md`를 읽게 하고, `bin/ach chat`으로 메인 세션을 열어 대화한다. CLI 명령, 설정, 수집 출처, 정기 실행, 라우팅, 제출 기록부, 정리, 사용자 정보, 로컬 규칙, 폴더 구성은 `docs/reference.md`와 `docs/reference_ko.md`로 옮겼다. 그림은 메인 세션과 `ach`를 중심으로 다시 그렸다.
- **옮기는 법**: `git pull && scripts/install.sh` 뒤에 `bin/ach schedule install`을 한 번 실행해 정기 실행이 `ach`를 부르게 한다. 0.5가 띄운 공모전 세션은 이제 없는 `harness`를 부르므로, 다시 실행하거나 그 세션의 에이전트에게 `<클론>/bin/ach`를 쓰라고 알린다. 레지스트리, 제출 기록부, 설정 파일은 그대로다.

## 0.5.2 - 2026-10-15

- **수정**:
  - `harness init`이 이미 있는 `data/`의 권한을 0700으로 좁힌다. 클론에는 커밋된 예시 때문에 `data/`가 이미 있고 umask대로 권한이 붙어 다른 사용자가 목록을 볼 수 있었다. Ubuntu처럼 umask가 002면 그룹이 쓸 수도 있었다. JSON 보고에 `tightened` 목록이 생겼다.

## 0.5.1 - 2026-10-15

- **바뀜**:
  - 한국어 문서를 따로 뒀다: `README_ko.md`, `CONTRIBUTING_ko.md`, `SECURITY_ko.md`, `CHANGELOG_ko.md`. 영어 파일에는 한국어 절이 더 없다.
- **수정**:
  - 클론이 바이너리를 빌드한 커밋 뒤로 문서만 바꾼 커밋을 더했으면 `harness doctor`가 버전 경고를 내지 않는다. 그래서 최신 릴리스로 설치한 `main` 새 클론도 버전 점검을 통과한다.

## 0.5.0 - 2026-10-15

두 번째 러너 어댑터다. Claude Code 어댑터는 Claude Code가 `CONTRIBUTING.md`를 따라 직접 썼다.

- **추가**:
  - Claude Code용 러너 어댑터 `claude`. 에이전트를 `~/.claude/agents/contest-harness/`의 서브에이전트 파일로 변환하고, 스킬을 `~/.claude/skills/`에 링크하며, `agent_models`를 그 파일의 `model` 필드로 적용하고, 지시문을 `@<경로>` 언급으로 넘긴다(경로에 공백이 있으면 본문으로). `runner_args.claude` 기본값은 `--permission-mode auto`다. Claude Code 2.1.296에서 정기 실행(`harness autorun`)을 돌려 공모전 하나를 수집하고 `ContestGate` 서브에이전트로 게이트 판정을 기록했다. 대화형 공모전 세션, `harness chat`, 에이전트별 모델, Orca automation은 돌려 보지 않았다.
  - 러너 어댑터용 `Spec.prompt`. 메모리에 든 지시문 본문으로, `runners::prompt_text`가 `prompt_file`보다 먼저 돌려준다.
- **바뀜**:
  - `INIT.md`가 어느 러너를 쓸지 묻고, Claude Code에서 모델을 고르는 방법을 다룬다.
- **수정**:
  - 지시문을 본문으로 넘기는 어댑터에서 `harness launch --dry-run`이 동작한다. dry run이 쓰지 않는 파일로 명령을 만들던 문제로, 이제 본문을 `Spec.prompt`로 넘긴다.

## 0.4.0 - 2026-10-15

에이전트가 설정을 맡는다. 에이전트에게 `INIT.md`를 읽게 하면 사용자와 함께 하네스를 설정한다.

- **추가**:
  - `INIT.md`: 설치, 뼈대 만들기, 추천 기본값을 단 질문 한 묶음, CLI로 적용, 사용자 정보, 모델 고르기, 설치·정기 실행·점검까지의 절차(omp: `/contest-init`)
  - `harness init`, `harness config path|get|set|unset`
  - 에이전트별 모델 `agent_models`와 `harness agents`. `harness install`이 러너에 적용한다(omp: 하네스 에이전트의 `task.agentModelOverrides`만)
- **바뀜**:
  - 메인 세션은 `harness doctor`로 시작하고, 설정이 안 된 클론이면 `INIT.md` 절차부터 진행한다.
  - 사용자 정보는 설정 에이전트에게 대화로 알려 줘 받아 적게 할 수도 있다. 제출용 개인정보 절은 직접 쓰기를 계속 권한다. 대화로 준 값은 모델 제공자와 세션 기록을 거치기 때문이다.

## 0.3.0 - 2026-10-15

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

## 0.2.0 - 2026-10-15

Rust로 다시 쓴 첫 버전이다.
