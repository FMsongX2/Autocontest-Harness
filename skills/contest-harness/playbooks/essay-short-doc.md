# 짧은 글·신청서 플레이북

대상 `type`: `essay`, `other`(슬로건, 네이밍, 짧은 수기, 캠페인 문구, 간단한 참가 신청서). 대개 레벨 1이고 세션 하나로 끝낸다.

## 0. 첫 단계

`CONTEST.md`, `.harness/gate.json`, `.harness/launch-prompt.md`를 읽는다. 운영자 환경 규칙 `data/local-rules.md`(하네스 저장소, git 제외)가 있으면 먼저 읽는다. 공고와 서식을 `docs/form/`에 둔다.

## 1. 쓸 수 있는 글인지 다시 확인

- 본인 경험 서술이 필수인 수기는 대신 쓸 수 없다. 게이트가 통과시켰더라도 공고에 "본인의 실제 경험"이 요구되면 사용자에게 한 문장으로 확인한다. 사용자가 경험을 주면 그 내용으로 쓰고, 하지 않기로 하면 근거를 `write`로 `.private/note.txt`에 쓰고 `harness transition <id> skipped --note-file .private/note.txt`.
- 직접 창작 서약과 생성형 AI 정책(`gate.json` `eligibility`)을 확인한다. AI 사용이 금지면 진행하지 않고 사용자에게 알린다.

## 2. 규격

글자 수 제한이 공백 포함인지 제외인지, 줄 수·단어 수, 응모 편수 제한, 필수 키워드, 금지 표현을 `docs/RULES.md`에 원문 그대로 적는다. 글자 수는 손으로 세지 않고 명령으로 센다(UTF-8 로캘에서 공백 포함 `wc -m < <파일>`, 공백·줄바꿈 제외 `tr -d ' \n' < <파일> | wc -m`. 앞뒤 빈 줄은 먼저 지운다).

## 3. 이기는 글

- 이전 수상작(주최 발표)을 찾아 길이, 어조, 구조, 핵심 단어를 표로 정리한다.
- 주최가 알리려는 메시지와 핵심 키워드를 공고·보도자료에서 뽑는다.
- 안을 넉넉히 만든다(슬로건은 20개 이상, 수기·짧은 글은 구성이 다른 3안). ContestJudge로 채점해 상위안을 고르고 다듬는다. 응모 편수 제한 안에서 낸다. 레벨 2 이상으로 매겨졌으면 점수 루프를 따른다: `RUBRIC.md` → 구성안 기획 채점(stage `plan`) → 쓰기 → `final` 채점, 매번 `.harness/judge/<stage>-<n>.json`에 저장하고 `harness score <id> --stage <stage> --file <경로>`로 기록한다.
- 한국어 맞춤법·띄어쓰기를 확인하고, 사실 주장은 출처를 확인한다.

## 4. 서식

hwp/hwpx 신청서는 `idea-proposal.md` 7절대로 채우고 한컴독스 웹에서 정렬을 확인한다. PDF는 한컴독스 웹으로만 만든다. 개인정보는 제출 승인이 `harness approve`로 기록된 뒤 제출 단계에서만 `harness profile show --for <id> --json`에서 꺼내 그 서식에만 채우고, 저장소 파일·커밋 메시지·로그·메모에 남기지 않는다.

## 5. 제출 준비

ContestJudge `ready` 확인 → SUBMISSION.md(제출처, 입력값, 첨부) → 커밋·푸시 → 최종안 요약을 `.private/note.txt`에 쓰고 `harness transition <id> awaiting_approval --note-file .private/note.txt` → 사용자에게 메인 세션이나 `harness approve <id>`로 제출 승인을 요청하고 멈춤.
