[English](CHANGELOG.md) · **한국어**

# 변경 기록

주요 변경 기록이다. 버전은 [Semantic Versioning](https://semver.org/)을 따른다. 1.0 전에는 마이너 버전이 CLI나 파일 형식을 바꿀 수 있고, 그때는 항목에 무엇을 어떻게 옮기는지 적는다.

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
