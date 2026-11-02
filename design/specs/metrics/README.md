# 지표 화면 시안 (T-311)
`connect`(연결하기), `dash`(대시보드), `state`(빈 상태·오류), `ph`(폰 요약 카드) × 라이트·다크, 1280 폭. 값은 `build/css/tokens.css`만 쓴다. 숫자는 예시 값이다.
- 숫자마다 출처 서비스와 갱신 시각(`.src`), 지난 기간 대비는 ▲▼ + 부호(색은 보조).
- 상승 `color.ok`, 하락 `color.wait`(오류색 아님). 별도 `color.metric.*` 토큰은 아직 없음 — 승인되면 추가 제안.
- 방문은 '페이지 로드' 수(PostHog가 페이지 로드마다 새 ID를 줌). GitHub는 최근 14일만 제공.
- 다시 만들기: `node gen.mjs` 후 `equipe design render <file>.html --widths 1280 --dark`.
