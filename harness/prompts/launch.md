# 하네스 실행 지시: {title}

이 세션은 공모전 폴더 `{folder}`의 담당이다. 하네스 id는 `{id}`, 유형은 `{type}`, 레벨은 `{level}`, 접수 마감은 `{deadline}`이다. 목표는 수상이다.

## 먼저 읽을 것

1. 스킬 `skill://contest-harness`와 플레이북 `skill://contest-harness/playbooks/{playbook}.md`
2. 이 폴더의 `CONTEST.md`와 `.harness/gate.json`
3. 공식 공고 원문 {url} 를 다시 열어 게이트 판정 뒤 바뀐 것(마감 연장, 제출 양식, Q&A)이 없는지 확인한다. 바뀌었으면 `CONTEST.md`를 고치고 README 진행 메모에 남긴다.
4. 운영자 환경 규칙 `<harness>/data/local-rules.md`가 있으면 읽고 따른다(계산 장비, 공유 자원 잠금, 서식 도구 같은 이 컴퓨터 전용 규칙). 이 문서에서 `<harness>`는 하네스 저장소 경로다. `harness profile path`가 내는 `<harness>/data/profile.md`에서 알 수 있다.

## 레벨 {level} 진행 방식

{level_section}

## 상태와 기록

- 상태는 이미 `running`이다. 진행 메모는 `README.md`에 날짜와 함께 남기고, 의미 있는 단위마다 커밋하고 push한다(저장소: {repo}).
- 페르소나 금지: 결과물(코드·주석·보고서·발표자료·서식·README·커밋 메시지)에 하네스 페르소나의 이름·호칭·말투(반말·애교)·웃음 표기를 쓰지 않는다. 사람을 가리킬 때는 "사용자"나 "참가자"처럼 중립어로, 문서는 보고서체로 쓴다. 워커에게 맡길 때도 이 규칙을 지시에 적는다.
- 심사 관문: 제출물이 ContestJudge(`agent: ContestJudge`)에서 `ready`를 받기 전에는 제출 단계로 가지 않는다. 레벨 2 이상은 점수 루프의 `plan`·`final` 점수가 `harness score`로 기록돼 있어야 하고, 최근 `final` 점수가 목표 미만이면 `awaiting_approval` 전이가 거절된다(마감 때문에 목표 미달로 내야 할 때만 `--score-ok "<사유>"`).
- 제출 승인 요청 전에 `harness check {id}`가 깨끗해야 한다(페르소나 흔적이 있으면 종료 코드 4와 위치가 나온다). 나온 곳을 고치고 커밋 메시지까지 다시 확인한다. `awaiting_approval` 전이는 흔적이 있으면 거절된다. 작품 속 대사처럼 의도한 표현일 때만 `--persona-ok "<사유>"`를 붙인다.
- 사용자 정보: 제출 서식에 들어갈 이름·연락처·생년월일·소속·서명 같은 개인 값은 `SUBMISSION.md`와 저장소 파일에 쓰지 않는다. `SUBMISSION.md`에는 필요한 항목 이름(프로필 `## 제출용 개인정보` 절의 항목)만 적고, 사용자 승인 뒤 제출하는 단계에서만 `harness profile show --json`으로 값을 꺼내 제출 서식에 넣는다. 값이 비어 있으면 사용자에게 `data/profile.md`를 직접 채워 달라고 요청한다(하네스와 에이전트는 이 파일을 고치지 않는다).
- 제출 준비가 끝나면 `SUBMISSION.md`를 채우고 `harness transition {id} awaiting_approval --note "<제출물·점수 요약>"`을 실행한 뒤, 사용자에게 승인을 요청하고 멈춘다. 제출은 사용자 승인 뒤에만 한다.
- 제출을 마치면 `harness submit {id} --method <폼|메일|사이트|우편> --receipt "<접수번호나 증빙>" --files <파일...>`을 실행한다(제출 기록부에도 남는다).
- 정리: 제출 기록 뒤 모든 결과물·제출본을 커밋하고 push한 다음, 마지막 명령으로 `harness clean {id}`를 실행한다(이 세션을 닫고 로컬 폴더를 휴지통으로 옮긴다. 저장소는 원격에 남는다). 먼저 `harness clean {id} --dry-run`으로 막는 이유(커밋 안 된 변경, push 안 된 커밋, git에 없는 제출본 등)가 없는지 보고, 있으면 고친 뒤 실행한다. 본선·발표·보완 제출처럼 같은 폴더로 할 일이 남았으면 정리하지 않고 `harness transition {id} running --note "<남은 일>"`으로 돌아간다.
- 사용자에게 물을 것은 한 번에 묶어 묻는다. 위 진행 방식에 적힌 사람 관문(레벨 5의 계획 검수 `plan_review` 포함) 밖에서는 멈추지 말고 끝까지 진행한다.
