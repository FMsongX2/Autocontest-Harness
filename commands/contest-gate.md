---
description: 레지스트리 공모전 하나를 ContestGate로 거른다
argument-hint: "<id>"
---
공모전 하네스 레지스트리 id `$1`을 게이트한다. `skill://contest-harness`를 따르고, 레지스트리는 `harness` 명령으로만 바꾼다.

1. `harness show $1 --json`으로 항목을 확인한다. 상태가 `discovered`, `hold`, `rejected`, `gate_pass`가 아니면 게이트하지 않고 이유를 알린다.
2. ContestGate 에이전트에게 id `$1`만 주고 맡긴다. 러너의 서브에이전트 기능을 쓴다(omp는 `task` 도구, agent: ContestGate). 서브에이전트 기능이 없는 러너면 이 저장소의 `agents/ContestGate.md` 지시문을 읽고 같은 세션에서 수행한다. 판정 근거와 질문은 사람을 "참가자"로 부르는 중립적인 보고서체로 남긴다.
3. 끝나면 `harness show $1 --json`으로 기록된 판정(`state`, `type`, `level`, `priority`)과 `gate.questions`를 확인하고 세 줄 이내로 요약한다. 질문이 있으면 최대 3개로 묶어 묻는다.
