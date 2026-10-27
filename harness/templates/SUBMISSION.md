# 제출 준비: [공모전 공식 명칭]

- 하네스 id: `[id]` · 마감: `[YYYY-MM-DDTHH:MM+09:00]`
- 제출 방법: [form/email/site] · 제출처(`destination`, 공식 원문 그대로): [접수 URL 또는 메일 주소] · 원문 위치: [`ach rules status <id>`의 규칙 판·원문 id·위치]
- 우편·현장 제출처럼 `form`·`email`·`site`가 아닌 방법은 제출 트랜잭션으로 기록할 수 없다. 사용자에게 알리고 지시를 받는다.
- 이 파일에는 개인정보 값을 쓰지 않는다. 항목 이름과 프로필(`data/profile.md`)의 항목 이름만 적는다. 값은 제출 승인이 `ach approve`로 기록된 뒤 제출 단계에서만 `ach profile show --for <id> --json`으로 꺼내 `.private/approved/`의 별도 최종 사본 서식에 바로 넣고, 저장소 파일·커밋 메시지·로그·메모·대화에 옮기지 않는다.

## 1. 형식·제한

- [ ] 파일 형식이 공고와 같다: [형식] (FR-[###])
- [ ] 분량·규격이 한도 안이다: [측정값] / [한도] (확인 명령: [pdfinfo/ffprobe/글자 수])
- [ ] 파일 용량이 한도 안이다: [크기] / [한도]
- [ ] 파일 이름이 공고 규칙을 따른다: [규칙]
- [ ] 블라인드 심사면 신원 정보가 결과물에 없다
- [ ] HWP/HWPX의 실제 렌더와 PDF 변환은 한컴독스 웹으로 확인했다(해당 시)
- [ ] 페이지가 있는 최종 제출 문서의 표지·본문·부록·동의서 **전 페이지를 비전으로 직접 열어** 줄간격·문단 간격·글꼴·여백·정렬·표/그림/캡션·수식·쪽 나뉨·잘림·겹침·글리프를 확인했다. 메타데이터·추출 텍스트·이미지 생성 성공만으로 완료 표시하지 않았다: [대상 파일·SHA-256·렌더 도구·전체/확인 페이지 수·페이지별 이미지/관찰/판정 기록 경로]
- [ ] 마지막 수정·재저장·변환 뒤 전 페이지를 다시 렌더링해 비전 검수했고, 실제 업로드할 파일의 SHA-256이 검수한 파일과 같다. 승인 후 개인정보·서명을 채운 별도 사본(`.private/approved/`)도 제출 직전 전 페이지를 재검수했다: [해시·검수 시각·기록 경로]. 이미지와 상세 기록은 `.private/visual-review/`에 두고 개인정보 값은 이 문서에 적지 않는다.
- [ ] 한글 본문은 최종 파일 기준의 한글 윤문 기록이 `JUDGE.md` "한글 윤문 기록" 표에 있다(해당 시): [run_id·채택 여부·적용 뒤 SHA-256]

## 2. 제출 파일

| 파일 | 경로 | SHA-256 | 비고 |
|---|---|---|---|
| [결과물] | [submission/…] | [ ] | 최종 채점 `artifacts`와 같은 해시 |
| [신청서·동의서] | | | 개인정보 채움은 승인 후 `.private/approved/` 별도 사본으로 |

<p>표 1. 제출 파일</p>

## 3. 생성형 AI 활용 공개

- 정책: [허용/표기 조건부 허용/금지/불명] (FR-[###])
- [ ] 공고가 요구한 고지 문구를 원문 그대로 넣었다: [위치]
- [ ] AI 활용 설명서·도구 목록·사람이 한 일을 첨부했다(요구 시)
- [ ] 직접 창작 서약과 충돌하지 않는다
- [ ] 유사도 검사가 요구되면 결과를 첨부했다: [결과 파일]

## 4. 필요한 개인정보 항목

| 항목 | 필요한 곳 | 프로필 항목(`## 제출용 개인정보`) |
|---|---|---|
| [성명] | [신청서] | 이름 |
| [영문·한자 성명] | | 영문 이름, 한자 이름 |
| [생년월일] | | 생년월일 |
| [연락처·이메일] | | 휴대전화, 이메일 |
| [소속(학교·학과·학년)] | | 학교·학과 정식 명칭(학년은 `## 참가 자격`) |
| [서명·도장 이미지] | | 서명 이미지 파일, 도장 이미지 파일(경로) |
| [재학증명서] | | 재학증명서 파일(경로) |
| [그 밖에 프로필에 없는 항목] | | 없음: 승인 단계에서 사용자에게 묻는다(파일에 남기지 않는다) |

<p>표 2. 개인정보 항목(값은 제출 승인이 기록된 뒤 제출 단계에서만 `ach profile show --for <id> --json`으로 채운다. 비어 있으면 사용자에게 `data/profile.md`를 직접 채워 달라고 요청한다)</p>

## 5. 제출 입력값

- 메일 제목·본문 / 폼 응답값 / 업로드 제목·설명·해시태그·공개 설정:

```text
[그대로 붙여 넣을 내용]
```

## 6. 최종 판정

- ContestJudge(stage `final`): [ready] · 총점: [ ] / 목표 [ ] · 기록: `.harness/judge/final-[n].json`(`artifacts` 포함), `ach score <id> --stage final --file <경로>` [시각], `JUDGE.md`
- 독립 최종 평가(위임 승인에 필요, 레벨 2 이상·영상 점수 면제 제외): `evaluation_id` [packet_id] · `assessment_id` [ ] · 익명 사본 확인서 [작성자·지운 항목] · 평가 횟수 n [ ](`ach evaluation report <id>`, n < 2면 분산 없음)
- 공식 규칙: `ach rules status <id>`의 `pending`·`invalidated` 없음 [확인 시각 · 규칙 판]
- 기획 채점(stage `plan`) 기록: [있음, 예측 총점 [ ] / 목표 [ ]] · 목표 미달로 내면 `--score-ok` 사유: [ ]

## 7. 제출 준비 기록과 승인

- [ ] `ach check <id>` 깨끗함(결과물·커밋 메시지에 페르소나 흔적 없음): [시각]
- [ ] `.private/readiness.json`에 다섯 필드(`submission_ready`, `needs_user_input`, `blockers`, `artifacts[{path, sha256}]`, `followup_required`)와 `destination`(위 제출처와 같은 값)을 쓰고 `ach readiness <id> --file .private/readiness.json` 실행. 레벨 2 이상은 `artifacts`가 최근 `final` 점수의 `artifacts` 안에 있다: [시각]
- [ ] 요약을 `.private/note.txt`에 쓰고 `ach transition <id> awaiting_approval --note-file .private/note.txt` 실행(레벨 2 이상은 `plan` 점수와 지금 작업 판의 `ready`·목표 이상인 최근 `final` 점수 필요): [시각]
- [ ] `ach approval-status <id>`의 `eligible`·`reasons` 확인: [시각 · request_token]
- 승인(`ach approval-status <id>`의 `submit_approval_valid`와 `ach show <id> --json`의 기록에서 옮긴다): [사용자 승인 원문 또는 위임 승인 기록] — [YYYY-MM-DD HH:MM] · 승인 방식: [사용자 / 위임]
- [ ] 승인 뒤 개인 항목·기존 서명 채움(해당 시): 원본은 덮어쓰지 않고 `.private/approved/`에 별도 사본을 만들어 전 페이지 재검수 뒤, `personalization_of`(승인된 request_token)와 항목마다 승인 원본 경로 `source`를 더한(바꾸지 않는 원본은 원본 그대로, `followup_required`는 승인 때와 같게) `.private/readiness.json`으로 `ach readiness <id> --file .private/readiness.json --personalized` 실행. 내용 수정은 하지 않았다: [시각]

## 8. 제출 트랜잭션과 접수 증빙

- [ ] 의도: `.private/submission-intent.json`(`method`, `destination`, `expected_request`)으로 `ach submission intent <id> --file .private/submission-intent.json` → attempt_id [a1] · [시각]
- [ ] 시작: 보내기 직전 `ach submission start <id> --attempt [a1]` 성공 확인 뒤에만 발송 · [시각]
- 실제 제출 시각: [ ] · 낸 파일: 승인된 마지막 제출 준비 기록의 `artifacts`(개인 항목 사본을 기록했으면 그 사본)와 정확히 같다
- 접수 증빙 파일(`.private/receipts/`): [경로·SHA-256] · 접수 번호·참조: [개인정보 없는 한 줄]
- [ ] 확인: `.private/submission-proof.json`(`attempt_id`, `receipt`, `source`, `observed_at`, `destination`, `submitted_sha256`, `artifacts`)으로 `ach submit <id> --attempt [a1] --file .private/submission-proof.json` 실행(제출 기록부에도 남는다) · [시각]
- 결과 불명(해당 시): 다시 내지 않았다. `ach submission unknown <id> --attempt [a1] --file .private/submission-unknown.json` [사유 코드·시각] → 공식 경로 확인 결과: [접수 확인 → `ach submit` / 미접수 긍정 증거 → `ach submission resolve` / 아직 불명]

## 9. 정리

- [ ] 제출본을 포함한 모든 파일을 커밋·push했다(제출본이 `.gitignore`에 걸려 있지 않다. 개인정보가 든 `.private/approved/` 사본은 커밋하지 않는다)
- [ ] 제출 준비 기록의 `followup_required`: [false / true]
- [ ] `false`: `ach clean <id> --dry-run`에 막는 이유가 없고 `ach clean <id>` 실행(세션 닫기, 로컬 폴더를 휴지통으로): [시각 · 휴지통 경로]
- [ ] `true`: 남은 일정을 `.private/note.txt`에 쓰고 `ach transition <id> followup_pending --note-file .private/note.txt`(폴더 유지, 슬롯 반환). 실제 후속 제작을 재개할 때만 `ach resume <id>`: [시각]
