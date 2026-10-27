# 창작물 플레이북: 영상·이미지·음악·디자인

대상 `type`: `video`, `image`, `music`, `design`. 숏폼·AI 영상 한 편은 대개 레벨 1이다. 여러 편이나 장편이면 레벨 3, 며칠 GPU 생성이 필요하면 레벨 4다(레벨 정의는 `docs/CONTRACT.md` 5절).

## 0. 첫 단계

1. `CONTEST.md`, `.harness/gate.json`, `.harness/launch-prompt.md`를 읽는다.
2. 운영자 환경 규칙 `data/local-rules.md`(하네스 저장소, git 제외)가 있으면 먼저 읽는다. 운영자의 숏폼 플레이북·선례 폴더·도구 잠금·무거운 작업 감싸기 규칙이 거기 있고, 이 플레이북과 다르면 그 규칙을 따른다.
3. 선례가 있으면 본다: 이전 숏폼·AI 영상 공모전 폴더의 README(전체 흐름, AI 사용 고지 문구, 규정 요약, 소재·라이선스 표).

폴더 관례: `README.md`, `research/`, `docs/form/`(공고·서식 원본), `assets/ai/`(그림 + `PROMPTS.md`), `assets/music/`, `video/`(Remotion 등 합성 프로젝트), `submission/`, `logs/`, `tools/`.

## 1. 규정 추출

`docs/RULES.md`에 공고 원문 그대로 옮긴다: 길이, 화면비, 해상도, fps, 파일 형식·용량, 생성형 AI 정책(`gate.json` `eligibility.ai_policy`), 직접 창작 서약, 필수 문구·로고·해시태그, 업로드 방식(유튜브 공개·비공개 등), 실존 인물·상표·음원 제한, 제출 서류(신청서, 저작권 동의서, AI 활용 설명서). 공고에 없는 값은 비워 두고 [가정]을 붙인다.

## 2. 이기는 방향 찾기

- 심사기준 배점표를 `docs/DECISIONS.md` 맨 위에 둔다.
- 이전 회차 수상작(주최 누리집·유튜브)을 찾아 길이, 훅, 메시지, 화풍, 음악을 표로 정리한다. 찾지 못하면 그렇다고 적는다.
- 주최가 알리고 싶은 핵심 키워드·사실을 공고와 보도자료에서 뽑는다. 사실 오류는 감점이 아니라 탈락 사유가 될 수 있다.

## 3. 콘셉트 카드(사람 관문)

카드 2~3장을 `docs/CONCEPT_CARDS.md`에 쓰고 카드마다 대표 키프레임 1장을 만든다. 카드 항목:

- 제목, 한 문장 메시지
- 첫 3초 훅
- 장면 흐름(5~8줄)
- 심사기준별 대응(기준 이름 → 이 카드가 점수를 얻는 이유)
- 화풍·색·음악 분위기
- 위험(규정 위반 소지, 제작 난도, GPU 필요 여부)

레벨 2 이상이면 카드를 보이기 전에 점수 루프의 기획 채점을 한다: 점수표 `RUBRIC.md`(템플릿 `<harness>/harness/templates/RUBRIC.md`, `<harness>`는 하네스 저장소)를 쓰고, ContestJudge(stage `plan`)가 카드마다 "이대로 완성하면" 예측 점수를 매긴다. 결과를 `.harness/judge/plan-<n>.json`에 저장하고 `ach score <id> --stage plan --file <경로>`로 기록한다. 가장 높은 카드의 부족분을 반영해 다시 채점하고(예측 총점이 목표 이상이 되거나 3회까지), 카드에 예측 총점을 적어 보인다. 레벨 1은 이 단계를 건너뛰어도 된다.

사용자에게 카드를 보이고 고르게 한다. 고를 때까지 콘셉트와 무관한 일(서식 채우기, 도구 준비, 음악 후보 생성 설정, 합성 프로젝트 뼈대)을 한다.

## 4. 제작

- 그림: 이미지 생성 도구(예: Codex `image_gen`). 여러 세션이 같은 생성 계정·슬롯을 쓰면 한 번에 한 세션만 쓰도록 잡고 놓는다(잠금 방법은 `data/local-rules.md`). 프롬프트와 SHA-256 manifest를 `assets/ai/PROMPTS.md`에 남긴다.
- 구도: 컷마다 composition plan(카메라 구도·움직임)을 `docs/COMPOSITION.md`에 쓰고 그대로 구현한다.
- AI 영상 클립: 원격 GPU 서버나 클라우드 GPU. 작업 맥에서 영상 모델을 돌리지 않는다. 클립은 받는 즉시 로컬로 옮기고 서버 중간 산출물을 지운다. 서버를 쓸 수 없으면 키프레임 + 합성 도구의 카메라 움직임으로 만든다.
- 음악: 음악 생성 모델(예: ACE-Step)이나 생성 서비스. 서비스 결과물은 이용약관의 상업·공모전 사용 조건과 다운로드 형식을 확인한다. 맥에서 돌리는 무거운 생성은 `data/local-rules.md`의 감싸기 규칙을 따른다.
- 합성: Remotion 같은 코드 합성 도구. 렌더(미리보기·still 포함)는 동시 실행 수를 낮게(예: `--concurrency 2`) 두고, studio·dev server는 띄우지 않는다.
- 이미지·디자인 공모전: 출력 해상도·색 공간·여백·글꼴 라이선스를 공고와 맞추고, 글자는 그림 생성이 아니라 레이아웃 도구로 얹는다.
- 음악 공모전: 길이·포맷·스템 제출 여부, 가사 저작권, AI 작곡 허용 범위를 확인한다.

## 5. 품질 검수

기준: 컷 밀도(한 그림 5초 이하, 그림 수 ≥ 길이(초) ÷ 3), 정지 슬라이드쇼 금지, 세이프 영역, −14 LUFS 안팎·피크 −1 dBTP, 렌더 후 contact sheet를 직접 보고 고친 뒤 재렌더, `ffprobe`로 길이·해상도·fps·코덱 확인.

## 6. AI 활용 고지와 서류

- 공고가 요구하는 AI 사용 고지는 원문 문구 그대로 영상·설명란·서류에 넣는다. 요구가 없어도 `ai_policy`가 "표기 조건부 허용"이면 고지한다.
- AI 활용 설명서: 단계별 사용 도구, 사람이 한 일, 프롬프트 예시, 소재 라이선스 표(폰트·효과음·음악 포함).
- 서식(hwp/hwpx): 스크립트로 채우고(`idea-proposal.md` 7절), 한컴독스 웹(작업 환경이 orca면 Orca 내장 브라우저)에서 열어 정렬을 눈으로 확인한다. PDF는 한컴독스 웹으로만 만들고, 변환 뒤 모든 쪽을 렌더해 확인한다.
- 개인정보·서명은 제출 승인이 기록된 뒤 제출 단계에서만 `ach profile show --for <id> --json`에서 꺼내, 원본 서류는 그대로 두고 `.private/approved/`의 별도 최종 사본에만 넣는다. 사본은 전 페이지를 다시 검수하고 `ach readiness <id> --file .private/readiness.json --personalized`로 기록한다(`contest-harness` 스킬 "제출 준비 기록과 승인" 4번). 기존 서명만 쓰고 새 서명을 만들지 않는다. 저장소 파일·커밋 메시지·로그·메모에 값을 남기지 않고, 승인한 제출처 밖으로 보내지 않는다.

## 7. 심사와 제출 준비

1. ContestJudge를 부른다(레벨 1은 1회, 레벨 2 이상은 초안에서 stage `draft`, 제출 직전 stage `final`). `final`에는 제출할 파일 목록을 넘기고 Judge 출력의 `artifacts`를 확인한다. 레벨 2 이상은 결과를 `.harness/judge/<stage>-<n>.json`에 저장하고 `ach score <id> --stage <stage> --file <경로>`로 기록한다. `reject`·`revise`면 배점이 큰 기준의 감점부터 고치고 다시 부른다.
2. `SUBMISSION.md`를 채운다: 제출처, 업로드 제목·설명·해시태그·공개 설정, 메일 제목·본문, 첨부 목록, 폼 응답값(개인 값은 승인 뒤에 채운다).
3. 커밋·푸시하고 `ach check <id>`를 확인한 뒤, 제출 파일의 경로·SHA-256과 막는 것을 `.private/readiness.json`에 써서 `ach readiness <id> --file .private/readiness.json`로 기록한다(`contest-harness` 스킬 "제출 준비 기록과 승인"). 결과물·점수 요약을 `write`로 `.private/note.txt`에 쓰고 `ach transition <id> awaiting_approval --note-file .private/note.txt`. 레벨 2 이상은 `plan` 점수와 지금 작업 판의 `ready`·목표 이상인 최근 `final` 점수가 있어야 하고 제출 준비 기록의 파일은 그 `final` 점수 `artifacts` 안에 있어야 한다(마감 때문에 목표 미달로 내야 하면 `--score-ok "<사유>"`).
4. `ach approval-status <id>`로 `eligible`·`reasons`를 확인하고, 사용자에게 메인 세션이나 `ach approve <id>`로 제출 승인을 요청하고 멈춘다(이 세션은 `approve`를 실행하지 않는다. 사용자가 상시 위임했으면 메인 세션이 위임 범위 안에서 기록한다). 유튜브 업로드, 메일 발송, 폼 제출은 승인이 기록된 뒤에만 한다.
