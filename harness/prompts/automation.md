공모전 하네스 정기 실행이다. `skill://contest-harness`를 따르고, 레지스트리는 `harness` 명령으로만 바꾼다.

1. `harness tick --json`으로 새 공고를 모으고 보드를 갱신한다.
2. `harness list --state discovered --json`에서 1차 거름을 한다. 판정은 게이트의 일이니 애매하면 남긴다.
   - 공모전이 아닌 글(서포터즈·기자단·교육생·강좌·체험단·채용 모집, 행사 참가 안내, 댓글 이벤트): `harness transition <id> skipped --note "공모전 아님: <이유>"`.
   - 제목·note만으로 자격이 분명히 안 되는 글(초·중·고생 전용, 특정 지역 거주자·재직자·소속 전용, 오프라인 현장 참석·촬영 필수, 참가비 필수, 해외 대회인데 한국 참가 제외): `harness transition <id> skipped --note "1차 거름: <이유>"`. 참가자 조건은 `harness profile eligibility --json`(식별 값 없음)으로만 본다.
   - 마감이 지난 글: `harness transition <id> skipped --note "마감 경과"`.
3. 이번 회차 게이트 수를 정한다. `harness capacity --json`의 `providers`에서 `limits.basis` 공급자의 `mode`가 `maxing`이면 `data/config.json`의 `gate_batch`개, `steady`면 그 절반, `conserve`면 4개, `throttle`이면 0개(게이트를 건너뛴다). 남은 `discovered`를 마감이 가까운 순으로 그만큼 골라 항목 하나에 `ContestGate` 하나씩 서브에이전트(task·agent 도구, pi는 `contest_agent` 도구)로 병렬 실행한다(agent: ContestGate, 과제에는 레지스트리 id만 준다). 서브에이전트 기능이 없는 러너면 이 저장소의 `agents/ContestGate.md` 지시문을 읽고 같은 세션에서 차례로 수행한다.
4. 지금 시각이 00시부터 05시 사이일 때만 `ContestScout`를 한 번 실행한다.
5. 게이트가 끝나면 `harness tick --no-collect --json`을 한 번 더 실행한다. `data/config.json`의 `auto_launch`가 `true`면 이때 통과한 공모전이 전체·레벨별 실행 상한과 메모리 예산 안에서만 라우팅·실행되고(나머지는 `waiting`), `false`면 보드만 갱신된다.
6. 정리: `harness clean --all --json`을 실행한다. 제출이 끝났고 문제가 없는 공모전(`submitted`·`closed`, 제출 기록부에 있음, 커밋·push 완료, 제출본이 git에 있음)의 세션을 닫고 로컬 폴더를 휴지통으로 옮긴다. 막힌 공모전은 지우지 않고 이유(`problems`)만 남긴다.
7. 마지막 답변은 여섯 줄 이내로: 새로 통과한 공모전(마감·레벨·우선순위), 보류와 그 질문, 계획 검수 대기(`plan_review`), 실행한 것, 정리 결과(정리한 폴더와 휴지통 경로, 막힌 공모전과 이유), 오류.

레지스트리(`data/registry.json`), 제출 기록부(`data/submissions.json`), 사용자 정보(`data/profile.json`)와 `BOARD.md`는 공개 저장소에 올리지 않는 로컬 상태이므로 커밋하지 않는다.
