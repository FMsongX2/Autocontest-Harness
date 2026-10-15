# 참고 오픈소스

공모전 자동화 하네스를 설계할 때 참고할 오픈소스이다. 모두 `git clone --depth 1`로 받은 최신 커밋 한 개이며, 코드를 그대로 가져다 쓰기보다 구조와 산출물 형식을 참고한다. 받은 날은 2026-10-15이다.

<p>표 1. 받은 저장소와 하네스에서 맡을 역할</p>

| 폴더 | 원본 | 커밋(날짜) | 라이선스 | 하네스에서 참고할 부분 |
|---|---|---|---|---|
| `BMAD-METHOD` | https://github.com/bmad-code-org/BMAD-METHOD | 8f0d50e (2026-10-13) | MIT | 일의 크기에 맞춰 계획 깊이를 고르는 방식(작은 일은 바로 구현, 큰 일은 브리프·명세·아키텍처). 헤드쿼터의 규모 판정. main은 v7 preview alpha이고 안정판 v6.12는 `V6.12` 브랜치에 있다 |
| `spec-kit` | https://github.com/github/spec-kit | 3238fa9 (2026-10-14) | MIT | specify → plan → tasks 순서의 명세·계획·작업 분해 산출물 형식. 헤드쿼터가 쓰는 문서 형식 |
| `gpt-researcher` | https://github.com/assafelovic/gpt-researcher | 3582a68 (2026-10-10) | Apache-2.0 | 질문을 계획하고 검색·읽기·인용해 보고서를 만드는 지식 수집 에이전트 |
| `storm` | https://github.com/stanford-oval/storm | fb951af (2025-09-30) | MIT | 여러 관점의 질문으로 자료를 모아 개요를 잡고 인용이 달린 긴 글을 쓰는 지식 정리 방식 |
| `open_deep_research` | https://github.com/langchain-ai/open_deep_research | 1b7d2e8 (2026-08-10) | MIT | LangGraph 기반 딥리서치 에이전트(계획, 병렬 하위 연구, 종합) |
| `RD-Agent` | https://github.com/microsoft/RD-Agent | 484776c (2026-09-23) | MIT | 연구(가설)와 개발(구현·실험)을 반복하는 데이터 과학·Kaggle 에이전트. 모델 대회 실행기 |
| `kaggle-competition-harness` | https://github.com/chenxi-bot21/kaggle-competition-harness | 8da91b7 (2026-07-11) | MIT | 내려받기 → 모델 → 제출 → 점수의 반복을 감사 가능한 형태로 묶은 작은 대회 하네스. 제출 기록 구조 |

논문 대회용(AI-Scientist-v2)은 받지 않았다. 논문 대회의 연구 도구는 운영자 환경에 따라 다르므로 운영자 환경 규칙(`data/local-rules.md`)에 둔다.
