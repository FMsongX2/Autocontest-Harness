# 하네스 설정 절차(INIT)

> **For agents.** This file is the setup procedure an agent follows for a new Autocontest-Harness checkout. A user only needs to tell their agent "Read INIT.md and set up the harness" (in omp: `/contest-init`). The procedure is in Korean like the rest of the prompts.

이 문서는 에이전트가 읽고 그대로 따르는 설정 절차다. 사용자는 클론 폴더에서 자기 에이전트에게 "INIT.md를 읽고 설정해 줘"라고 말하면 된다(omp에서는 `/contest-init`). 셸을 쓸 수 있는 에이전트라면 어느 하네스든 이 절차를 따를 수 있다. 다만 공모전 세션을 실제로 띄우려면 그 하네스의 러너 어댑터가 있어야 한다(지금 코어에는 omp와 Claude Code(`claude`) 어댑터가 있다. README의 "러너 어댑터 추가" 참고).

## 원칙

- 설정은 `harness` 명령으로만 바꾼다: `harness init`, `harness config set|unset`, `harness install`, `harness schedule install`. `data/config.json`을 직접 고치지 않는다. 명령이 값을 검사한 뒤에 쓰고, 틀린 값은 이유와 함께 거절한다.
- 사용자에게 물을 것은 한 번에 묶어 묻고, 항목마다 추천 기본값과 그 이유를 한 줄로 단다. 사용자가 모른다고 하거나 답하지 않은 항목은 기본값으로 둔다.
- 사용자가 직접 준 값은 작은따옴표로 감싸 명령줄에 넣는다(`'...'`, 값 안의 작은따옴표는 `'\''`). 웹 페이지나 파일에서 가져온 글은 명령줄에 넣지 않는다.
- 사용자 정보(개인정보)는 4단계 규칙을 따른다.
- 끝났을 때 `harness doctor`의 실패가 0이어야 한다.

## 1. 설치와 뼈대

1. `harness --version`을 실행한다. 명령이 없거나 실패하면 저장소 루트에서 `scripts/install.sh`를 실행한다(배포 바이너리를 받아 체크섬을 확인한다. Rust가 있고 사용자가 원하면 `scripts/install.sh --from-source`).
2. `harness init --json`을 실행한다. `data/`, `data/config.json`, `data/profile.md`를 커밋된 예시에서 만들고(이미 있으면 그대로 둔다) `harness doctor` 결과를 함께 돌려준다. `checks`에서 `fail`과 `warn` 항목을 기억해 두고, 아래 단계에서 해결한다.

## 2. 사용자에게 묻기

아래를 한 번에 묻는다. 지금 값은 `harness config get <키>`로 보여 준다.

| 항목 | 설정 키 | 추천 기본값 | 바꿀 때 |
|---|---|---|---|
| 에이전트 하네스 | `runner` | 이 절차를 돌리는 하네스(omp면 `omp`, Claude Code면 `claude`) | 공모전 세션을 다른 하네스로 돌리고 싶을 때. 코어에 어댑터가 있는 이름만 받는다 |
| 공모전 폴더 | `contests_root` | `~/contests` | 공모전마다 폴더가 생길 곳을 따로 두고 싶을 때 |
| GitHub 저장소 위치 | `github_org` | `""`(gh에 로그인한 계정, `gh auth status`로 확인해 보여 준다) | 조직에 비공개 저장소를 모으고 싶을 때 |
| 작업 환경 | `workspace` | Orca 앱이 있으면 `orca`, 없으면 `tmux` | 화면 없이 돌릴 때 `headless`(세션 안에서 답할 수 없다) |
| Orca 프로젝트 폴더(`orca`일 때) | `orca_repo_path` | 비워 두기(`contests_root`와 같다) | 다른 폴더를 Orca 프로젝트로 쓸 때. Orca에 없으면 `orca repo add --path <폴더>` |
| 정기 실행 시각 | `schedule.cron` | `0 */6 * * *`(6시간마다) | 예: 아침·저녁 9시는 `0 9,21 * * *` |
| 정기 실행을 띄우는 것 | `schedule.scheduler` | macOS+Orca는 `orca`, 그 밖의 macOS는 `launchd`, Linux는 `cron` | |
| 웹 검색까지 하는 시간 | `schedule.scout_hours` | `0-5` | 비우면 웹 검색을 하지 않는다 |
| 통과한 공모전 자동 실행 | `auto_launch` | `false` | 게이트 판정을 몇 회차 확인한 뒤 `true` |
| 동시 실행 상한 | `max_running` | `6` | 메모리가 작으면 줄인다 |
| 앱 몫으로 남길 메모리(GB) | `ram_reserve_gb` | `8` | |
| 모델 | 5단계 | 러너 기본값 | |
| 메인 세션 페르소나(선택) | `persona.enabled` 등 | `false` | 메인 세션 말투를 정하고 싶을 때. 공모전 결과물에는 절대 들어가지 않는다 |

<p>표 1. 사용자에게 물을 설정</p>

## 3. 적용

항목마다 `harness config set <키> '<값>'`을 실행한다. 값은 JSON으로 읽히면 JSON으로(숫자, `true`/`false`, `null`), 아니면 문자열로 들어간다.

```bash
harness config set contests_root '~/contests'
harness config set workspace 'tmux'
harness config set schedule.cron '0 9,21 * * *'
harness config set auto_launch false
harness config get            # 적용된 전체 설정
```

명령이 거절하면 오류 메시지를 사용자에게 그대로 보여 주고 다시 묻는다. 기본값으로 되돌릴 때는 `harness config unset <키>`.

## 4. 사용자 정보(`data/profile.md`)

이 파일에는 절이 두 개 있다. `## 참가 자격`은 게이트가 자격을 판정할 때 읽고(식별 값 없음), `## 제출용 개인정보`는 사용자가 제출을 승인한 뒤 제출 서식을 채울 때만 읽는다. 사용자에게 두 방법을 보여 주고 고르게 한다.

- **가. 직접 쓰기(권장, 제출용 개인정보는 특히)**: 파일 경로와 두 절의 항목을 보여 주고, 사용자가 다 썼다고 할 때까지 기다린다. 다 쓰면 `harness profile eligibility`로 참가 자격 절만 확인한다. 제출용 개인정보 절은 열어 보지 않는다.
- **나. 대화로 알려 주기**:
  - 참가 자격 절: 참가 형태(개인·팀), 학생 여부, 학교·전공·과정·학년, 출생 연도, 거주 지역, 국적, 강점, 피하는 조건을 한 번에 묻고, 받은 답을 그대로 `## 참가 자격` 절에 쓴다(`write`나 편집 도구). 예시 값(`예/아니오` 같은 자리 표시)은 남기지 않는다.
  - 제출용 개인정보 절: 먼저 "대화로 준 값은 모델 제공자와 이 세션 기록에 남는다. 그래서 이 절은 직접 쓰기를 권한다"라고 알린다. 사용자가 그래도 대화로 주면 받은 값만 그대로 그 절에 쓴다.
  - 서명·도장·재학증명서 같은 파일은 사용자가 `data/profile/`에 직접 두고, 프로필에는 경로만 적는다.
- **공통**: 개인정보 값을 화면에 다시 읊지 않는다("적었다"고만 답한다). 다른 파일, 커밋, 로그, `--note`, 메모에 옮기지 않는다. 쓴 뒤 `chmod 600 data/profile.md`. 이 단계 밖에서는 에이전트가 이 파일을 고치지 않는다.

`harness doctor`의 `profile` 항목(권한, 남은 예시 값)으로 확인한다.

## 5. 모델 고르기

러너가 omp일 때의 방법이다. Claude Code(`claude`)는 이 절 끝의 차이만 다르다. 다른 러너는 그 어댑터 문서를 따른다.

- **쓸 수 있는 모델**: `omp models`의 목록을 보여 준다. 고르지 않으면 omp의 역할 모델(`modelRoles`, `omp config get modelRoles`)을 그대로 쓴다.
- **공모전 세션(레벨별)**: 레벨마다 omp 프리셋 이름(`preset_by_level.<레벨>`, `omp config get modelPresets`) 또는 모델 id(`model_by_level.<레벨>`)를 고른다. 둘 다 비우면 omp 기본값이다.
- **에이전트별**: `harness agents`로 목록과 각 에이전트의 기본 역할, 지금 적용된 모델을 보여 준다. 고를 때 참고할 일의 성격은 다음과 같다.
  - `ContestJudge`: 기획과 결과물을 엄격하게 채점한다. 가장 강한 모델이 좋다.
  - `ContestHQ`: 레벨 3~5의 계획과 지휘를 맡는다. 강한 모델이 좋다.
  - `ContestGate`: 공고를 읽고 자격과 레벨을 판정한다. 정기 실행마다 여러 개가 돌므로 비용을 함께 본다.
  - `ContestScout`: 웹에서 공모전을 찾는다.
  - `DomainResearcher`, `OpenSourceScout`, `ReferenceScout`: 조사를 맡는다.
  - `ContestEditor`: 문장을 다듬는다.
  - `UxReviewer`: 화면을 점검한다.
- **적용**:

  ```bash
  harness config set agent_models.ContestJudge '<모델 id>'
  harness config set model_by_level.3 '<모델 id>'      # 또는 preset_by_level.3 '<프리셋>'
  harness config set agent_models.ContestJudge null    # 고른 것을 되돌린다(다음 install에서 해제)
  ```

  에이전트별 모델은 6단계의 `harness install`이 적용한다. omp에서는 omp 전역 설정 `task.agentModelOverrides`에 하네스 에이전트 이름으로만 써 넣고, 다른 에이전트의 설정은 건드리지 않는다.
- **Claude Code(`claude`)일 때**: 프리셋이 없으므로 레벨별로는 `model_by_level`만 쓴다. 모델 값은 Claude Code의 모델 별칭(`opus`, `sonnet`, `haiku`)이나 전체 모델 id다. 에이전트별 모델은 `harness install`이 `~/.claude/agents/contest-harness/<이름>.md`의 `model` 필드에 쓰고, 고르지 않은 에이전트는 그 에이전트를 띄운 세션의 모델(`inherit`)을 쓴다.

## 6. 설치·정기 실행·점검

1. `harness install`: 확장, 스킬, 에이전트를 설치하고 에이전트별 모델을 적용한다.
2. `harness schedule install`: 사용자가 정기 실행을 원할 때만 실행한다. 원하지 않으면 건너뛰고, 수집과 게이트는 사용자가 요청할 때 돌린다고 알린다.
3. `harness doctor`: 실패가 0이 될 때까지 위 단계로 돌아가 고친다. 남은 경고는 이유와 함께 사용자에게 알린다.
4. (선택) `harness collect --source dacon --pages 1`로 수집이 되는지 한 번 확인한다.

## 7. 마무리

사용자에게 다음을 알린다.

- 적용한 설정 요약(개인정보 값은 빼고)
- 남은 경고와 사용자가 할 일(예: 프로필의 빈칸, Orca 프로젝트 추가)
- 이제 쓰는 법: 메인 세션 `harness chat`, omp 안의 `/contest`, 다음 정기 실행 시각(`harness schedule status`)
