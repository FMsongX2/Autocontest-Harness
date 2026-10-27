# 데이터 분석 플레이북

대상 `type`: `data`, 분석 결과를 보고서로 내는 `report`(공공데이터 활용 분석, 빅콘테스트 분석 부문, 기관 데이터 분석 경진대회). 리더보드가 있으면 `model-competition.md`를 함께 따른다. 분석 보고서 하나면 대개 레벨 3, 서버 계산이나 반복 제출 루프가 있으면 레벨 4다.

## 0. 첫 단계

`CONTEST.md`, `.harness/gate.json`, `.harness/launch-prompt.md`를 읽는다. 운영자 환경 규칙 `data/local-rules.md`(하네스 저장소, git 제외)가 있으면 먼저 읽는다. 공고, 데이터 명세서, 제출 서식을 `docs/form/`에 둔다.

## 1. 규칙과 기준

- SPEC.md `FR-###`: 제공 데이터만 써야 하는지, 외부 데이터 허용 범위와 출처 표기, 분석 도구 제한, 보고서 분량·형식, 코드 제출 여부, 발표 자료·영상 요구, 데이터 보안 서약(반출 금지·분석 환경 제한).
- SC-###: 심사기준(분석 타당성, 활용 가능성, 창의성, 시각화·전달력 등)과 배점. 레벨 2 이상은 이 기준을 점수표 `RUBRIC.md`(템플릿 `<harness>/harness/templates/RUBRIC.md`, `<harness>`는 하네스 저장소)에 풀어 쓴다.
- 분석 환경이 주최 측 원격 환경으로 제한되면 그 안에서만 작업하고 데이터를 저장소에 올리지 않는다.

## 2. 문제 고르기

- 주최 기관이 풀고 싶어 하는 문제(업무계획, 보도자료, 이전 수상작)를 DomainResearcher로 확인한다.
- 분석 파이프라인·예측 모델·시각화 도구를 만들기 전에 OpenSourceScout로 쓸 수 있는 라이브러리·공개 분석 코드·사전학습 모델을 찾아 `research/oss.md`에 구성 요소별 `reuse`/`fork`/`reference`/`build`를 받는다(코드·모델을 쓰는 분석이면 필수, DomainResearcher와 같은 웨이브). 분석 도구 제한·외부 데이터 규칙과 대조하고, 맞는 것이 없는 부분만 만든다.
- 분석 질문 후보 2~3개를 만들어 데이터로 답할 수 있는지(변수·기간·단위가 맞는지) 확인하고, ContestJudge(stage `plan`)가 `RUBRIC.md`로 후보마다 "이대로 완성하면" 예측 채점한다. 가장 높은 후보를 고르고 기준별 부족분을 분석 계획에 반영해 다시 채점한다(예측 총점이 목표 이상이 되거나 3회까지). 매번 `.harness/judge/plan-<n>.json`에 저장하고 `ach score <id> --stage plan --file <경로>`로 기록한다. 근거를 `research/research.md`에 남긴다.

## 3. 분석

- 폴더: `data/raw/`(원자료, git 제외), `data/processed/`, `scripts/`(번호 순 단계 스크립트), `out/figs/`, `out/tables/`.
- 데이터 이해: 변수 정의, 결측·이상치, 단위, 기간, 표본 구성. 결과를 `docs/DATA.md`에 적는다.
- 방법: 분석 질문마다 방법·가정·한계를 PLAN.md 결정으로 남긴다. 예측 모델을 쓰면 검증 분할을 먼저 고정하고, 검증 데이터로 학습한 점수를 보고하지 않고, 누수를 점검한다(`model-competition.md` 4절).
- 큰 계산은 원격 GPU 서버나 클라우드 GPU에서 한다. 작업 맥 메모리를 넘길 데이터는 맥에서 열지 않는다.
- 모든 수치는 스크립트로 다시 만들 수 있어야 한다. 문서 안 수치는 [계산]으로 표시하고 만든 스크립트와 출력 파일을 적는다.

## 4. 보고서

- 한국어 보고서체, 용어 정의, 지시어 대상 명시, [가정]/[문헌]/[계산] 표시.
- 구성은 서식 순서를 따르고, 서식이 없으면: 문제 정의 → 데이터 → 방법 → 결과 → 활용 방안(정책·서비스 제안) → 한계.
- 그림·표는 질문 하나에 하나. 축 이름·단위·출처를 단다. 표 캡션은 표 아래 `<p>표 n. …</p>`.
- HWP 서식이면 `idea-proposal.md` 7절대로 한컴독스 웹에서 마무리하고 PDF도 한컴독스 웹으로만 만든다.
- HWP가 아니어도 페이지가 있는 보고서는 `contest-harness` 스킬의 "문서 서식의 전 페이지 비전 검수"를 따른다. 최종 파일의 모든 페이지를 비전으로 직접 확인하며, 줄간격 설정값·메타데이터·추출 텍스트만으로 서식을 통과시키지 않는다. 수정 뒤 전 페이지 재검수와 파일 해시·페이지별 근거 기록을 마친 후 최종 채점을 받는다.

## 5. 채점과 제출 준비

핵심 결과와 보고서 초안이 나오면 ContestJudge(stage `draft`), 제출 전 ContestJudge(stage `final`, 제출 파일 목록을 넘겨 `artifacts`를 받는다)로 채점하고, 결과를 `.harness/judge/<stage>-<n>.json`에 저장해 `ach score <id> --stage <stage> --file <경로>`로 기록한다. 배점이 큰 기준의 감점부터 고친다. 보고서 마무리 전에 평가표 기준으로 ContestEditor가 문체·문장을 다듬고, 그 뒤 최종 채점 전에 이 세션이 `contest-harness` 스킬의 "한글 윤문 단계"를 실행해 `JUDGE.md`에 기록한다. 코드 제출이 있으면 `scripts/`가 처음부터 끝까지 돌아가는지 깨끗한 환경에서 한 번 확인하고 러너의 기본 `reviewer`(없으면 세션이 직접)로 코드를 검토한다. SUBMISSION.md를 채우고 커밋·푸시한 뒤 `ach check <id>` → `ach readiness <id> --file .private/readiness.json`(스킬 "제출 준비 기록과 승인") → 요약을 `write`로 `.private/note.txt`에 쓰고 `ach transition <id> awaiting_approval --note-file .private/note.txt`(레벨 2 이상은 `plan` 점수와 지금 작업 판의 `ready`·목표 이상인 최근 `final` 점수가 필요하고, 목표 미달로 내야 하면 `--score-ok "<사유>"`).
