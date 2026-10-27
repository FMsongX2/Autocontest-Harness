[English](CHANGELOG.md) · **한국어**

# 변경 기록

주요 변경 기록이다. 버전은 [Semantic Versioning](https://semver.org/)을 따른다. 1.0 전에는 마이너 버전이 CLI나 파일 형식을 바꿀 수 있고, 그때는 항목에 무엇을 어떻게 옮기는지 적는다.

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
