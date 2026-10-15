# 아이디어·제안서 플레이북

대상 `type`: `idea`, 정책·사업 제안형 `report`. 아이디어 제안서·짧은 기획서는 대개 레벨 2, PoC 서비스나 여러 결과물이 붙으면 레벨 3이다(레벨 정의는 `docs/CONTRACT.md` 5절).

## 레벨별 흐름

- 레벨 2: 메인 세션 하나가 이 플레이북 순서대로 직접 쓴다. 지휘(HQ) 절차와 워커 웨이브는 쓰지 않는다. 조사는 조사 에이전트 하나에 맡긴다(DomainResearcher, 기술·논문 근거가 중심이면 ArticleResearcher). 점수 루프(점수표 → 기획 채점 반복 → 쓰기 → 중간·최종 채점 → `harness score` 기록)를 따른다(3절·6절). 원고가 서면 평가표(`gate.json` `criteria`)가 있을 때 최종 채점 전에 ContestEditor에게 결과물 경로를 넘겨 평가표 항목별로 문체·어투·문장을 점검하고 다듬게 한다. ContestEditor가 넘긴 내용 문제(`content_issues`)는 이 세션이 고친다. 평가표가 없으면 건너뛰고 최종 채점한다.
- 레벨 3: ContestHQ가 이 순서로 SPEC·PLAN·TASKS를 만들고 워커 웨이브로 나눠 쓴다. 체크포인트마다 ContestJudge, 글 마무리는 ContestEditor, PoC 코드는 러너의 기본 `reviewer`.

## 0. 첫 단계

`CONTEST.md`, `.harness/gate.json`, `.harness/launch-prompt.md`를 읽는다. 운영자 환경 규칙 `data/local-rules.md`(하네스 저장소, git 제외)가 있으면 먼저 읽는다. 공고문과 제출 서식 원본을 `docs/form/`에 둔다.

## 1. 서식이 곧 목차

- 공식 서식의 항목 순서, 항목명, 표 구조를 바꾸지 않는다. 서식이 없으면 공고가 말한 구성 요소 순서를 목차로 쓴다.
- 분량(쪽수, 글자 수), 글꼴·크기, 첨부 허용 여부, 파일 형식, 블라인드 심사(이름·학교 기재 금지)를 SPEC.md `FR-###`으로 옮긴다.
- 심사기준(`gate.json` `criteria`)을 `SC-###`로 옮기고, 서식 항목마다 어느 기준을 점수로 바꾸는지 표로 연결한다. 레벨 2 이상은 같은 기준을 점수표 `RUBRIC.md`(템플릿 `<harness>/harness/templates/RUBRIC.md`, `<harness>`는 하네스 저장소)에 풀어 쓴다: 배점, 만점 조건, 감점 요인, 필요한 근거, 입상작이라면 있었을 것, 공고 원문 위치. 배점이 공개되지 않았으면 [가정] 균등 배분, 목표 총점은 SPEC의 값(없으면 90/100).

## 2. 조사

- 이전 수상작·심사평, 주최 기관의 정책 방향(업무계획, 보도자료), 관련 통계는 DomainResearcher에게 결정 단위로 맡긴다.
- 기술 근거(유사 연구, 선행 기술)는 ArticleResearcher에게 맡긴다.
- PoC를 계획하면 OpenSourceScout로 가져다 쓸 라이브러리·사전학습 모델·공개 데모를 찾아 `research/oss.md`에 구성 요소별 `reuse`/`fork`/`reference`/`build`를 받는다(레벨 2는 선택, PoC 코드를 결과물로 내는 레벨 3은 필수). PoC는 맞는 것이 없는 부분만 만들고, 사용한 오픈소스는 제안서에 고지한다.
- 결과를 `research/research.md`(결정 / 근거 / 검토한 대안)와 `research/sources.json`에 저장한다.

## 3. 이길 주제 고르기(기획 채점)

후보 2~3개를 만들어 각각 개요(핵심 주장, 서식 항목별 내용, 근거, PoC 계획)로 쓴다. ContestJudge(stage `plan`)가 `RUBRIC.md`로 후보마다 "이대로 완성하면" 예측 채점하고 기준별로 점수를 올릴 기획 변경을 낸다. 가장 높은 후보를 고르고 부족분을 반영해 다시 채점한다. 예측 총점이 목표 이상이 되거나 3회를 돌 때까지 반복한 뒤 쓰기 시작한다. 매번 결과를 `.harness/judge/plan-<n>.json`에 저장하고 `harness score <id> --stage plan --file <경로>`로 기록한다. 비교표와 고른 이유를 `research/research.md`에 남긴다. 레벨 1처럼 점수 루프를 강제하지 않는 경우에도 후보 비교는 한다.

## 4. PoC로 주장 강화

실현 가능성 배점을 얻기 위해 주장 하나 이상을 작은 실물로 보인다.

- 예: 공공데이터로 문제 규모를 계산, 작은 모델·규칙으로 동작 시연, 화면 목업, 처리 흐름 데모.
- 코드는 `poc/`, 결과 그림·표는 `poc/out/`. 문서 안 수치는 [계산]으로 표시하고 계산 스크립트 경로를 주석·부록에 적는다.
- 학습·대형 추론은 원격 GPU 서버나 클라우드 GPU에서 한다. 작업 맥에서는 가벼운 분석만 한다.

## 5. 쓰기

- 한국어 보고서체. 지시어가 가리키는 대상을 분명히 하고, 용어는 처음 나올 때 정의한다.
- 모든 수치·주장에 [가정]/[문헌]/[계산] 중 하나와 출처를 단다. 수치를 지어내지 않는다.
- 표 캡션은 표 아래 `<p>표 n. …</p>`, 그림 캡션도 같은 규칙으로 번호를 매긴다. 표 주석과 표 내용이 맞는지 확인한다.
- 원고는 `docs/draft/*.md`로 쓰고 서식으로 옮긴다.

## 6. 채점 반복

초안 완성 뒤 ContestJudge(stage `draft`), 서식 반영 뒤 제출 전 ContestJudge(stage `final`)를 부르고, 결과를 `.harness/judge/<stage>-<n>.json`에 저장해 `harness score <id> --stage <stage> --file <경로>`로 기록하고 JUDGE.md에도 적는다. 목표 점수와 `ready`가 나올 때까지 배점이 큰 기준의 감점과 CRITICAL·HIGH부터 고친다. 레벨 2는 초안 완성 뒤 ContestEditor(평가표가 있을 때) → 최종 채점 순서로 마무리한다.

## 7. HWP 서식 마무리

- hwpx 서식은 스크립트로 채운다(XML을 직접 고치는 채우기·빌드 스크립트와 형식 점검 스크립트). 운영자의 선례 스크립트가 있으면 `data/local-rules.md`에 있다.
- 바이너리 hwp만 있거나 스크립트로 정렬이 맞지 않으면 한컴독스 웹에서 직접 편집한다. 한컴독스는 작업 환경의 브라우저(orca면 Orca 내장 브라우저)에서 연다. 로그인 계정이 맞는지는 제출 승인이 `harness approve`로 기록된 뒤 `harness profile show --for <id> --json`의 한컴독스 계정과 대조하고, 계정 값을 어디에도 옮겨 적지 않는다.
- 표 칸 넘침, 쪽 나뉨, 캡션과 표가 다른 쪽에 갈린 곳을 화면으로 확인한다.
- PDF가 필요하면 한컴독스 웹으로만 변환한다. 변환 뒤 모든 쪽을 그림으로 렌더해 깨진 글리프가 없는지 본다.

## 8. 제출 준비

- 생성형 AI 활용 공개 요구, 표절·유사도 검사 요구, 서약서·개인정보 동의서를 확인해 SUBMISSION.md에 적는다. 개인 값은 제출 승인이 `harness approve`로 기록된 뒤 `harness profile show --for <id> --json`에서 꺼내 그 서식에만 넣고, 저장소 파일·커밋 메시지·로그·메모에 옮기지 않는다.
- 커밋·푸시 후 요약을 `write`로 `.private/note.txt`에 쓰고 `harness transition <id> awaiting_approval --note-file .private/note.txt`. 레벨 2 이상은 `plan` 점수와 `ready`·목표 이상인 최근 `final` 점수가 필요하다(목표 미달로 내야 하면 `--score-ok "<사유>"`). 그다음 사용자에게 메인 세션이나 `harness approve <id>`로 승인해 달라고 요청하고 멈춘다(이 세션은 `approve`를 실행하지 않는다).
