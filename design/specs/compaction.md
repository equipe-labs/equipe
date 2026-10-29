# 대화 요약 중 표시 — 인계 스펙 (T-164)

상태: **확정** — 사람이 안 B(흐르는 막대 + 경과 시간)를 골랐다(2026-10-30). 그림: `specs/compaction/options.png`(안 비교) · `states.png`(상태) · `platforms.png`(플랫폼별) · 개별 화면 `desktop|ios|android-{light,dark}.png`, `tui.png`. 렌더 원본은 `specs/compaction/build.mjs`(토큰은 `build/css/tokens.css`만 읽는다).

## 1. 무엇을 알리나
엔진(Claude Code·Codex)이 대화를 스스로 압축하거나, 사람이 `/compact`("요약해서 새로 시작")를 누르면 멤버는 한동안 답이 없다. 그동안 멈춘 게 아니라 **요약 중**임을 보여 준다. 엔진은 진행률을 주지 않는다 → **%를 지어내지 않는다**(안 B: 흐르는 막대 + 실제 경과 시간).

## 2. 상태
| # | 상태 | 시작 조건 | 보이는 것 | 끝 조건 |
|---|---|---|---|---|
| 1 | 요약 중 (자동) | 엔진이 압축 시작 이벤트 | 띠 `대화를 요약하고 있어요` + 흐르는 막대 + 경과 | 끝/실패 이벤트 |
| 2 | 요약 중 (`/compact`) | 사람이 실행 | 제목만 `요약해서 새로 시작하고 있어요` | 〃 |
| 3 | 오래 걸림 | 요약 60초 경과 | 제목 `요약이 오래 걸리고 있어요`, 막대·경과 그대로 | 〃. **시간 제한으로 실패 처리하지 않는다** |
| 4 | 요약 중 입력 | 사람이 메시지 전송 | 입력은 받아 두고, 띠에 `메시지 N개 대기 중 · 요약이 끝나면 보낼게요` 칩 | 끝나면 순서대로 전달(백엔드). 실패해도 전달 |
| 5 | 끝 | 끝 이벤트 | 띠가 `요약했어요 / 이어서 대화할 수 있어요`(ok, ✓)로 바뀌고 4초 뒤 사라짐. 대화에 구분선 `요약했어요 · 이어서 대화할 수 있어요` 남음 | — |
| 6 | 실패 | 실패 이벤트 | 띠 `요약하지 못했어요 / 대화는 그대로예요. 다시 시도할까요?` + `다시 시도`(보조 버튼). 사람이 닫을 때까지 유지 | 다시 시도 → 1 |

가정(미확인 — 백엔드 확인 필요): 엔진이 시작·끝·실패 이벤트를 준다 / 실패해도 대화 원문은 그대로다 / 대기 메시지를 순서대로 전달한다.

## 3. 위치
- **데스크톱·iOS·Android**: 대화 목록 맨 아래, 입력창 바로 위. 입력창과 간격 `space.3`(12). 대화 스크롤 영역을 줄이고 띠가 보이는 동안 바닥 고정.
- **사이드바(멤버 목록)**: 요약 중인 멤버 줄은 기존 "일하는 중" 스피너 + 보조 줄 `대화 요약 중 · 12초`. 끝나면 일반 상태로.
- **터미널 TUI**: 입력 프롬프트 바로 위 한 줄 `⠹ 대화 요약 중… 12초`(+ 대기 메시지가 있으면 보조 줄). 끝 `✓ 요약했어요. 이어서 대화할 수 있어요`, 실패 `✕ 요약하지 못했어요. 대화는 그대로예요 · /compact로 다시 시도`.

## 4. 토큰 (디자인 시스템 v1)
| 부분 | 토큰 |
|---|---|
| 띠 면·테두리·모서리 | `color.surface`, `color.line`, `radius.card` |
| 띠 안 여백 | 세로 `space.3` · 가로 `space.4`, 요소 간격 `space.3` |
| 아이콘(압축) | 20px(`size.icon.md`), 요약 중 `color.work` · 끝 `color.ok` · 실패 `color.err` |
| 제목 / 보조 | `type.small` semibold + `color.text` / `type.meta` + `color.muted` |
| 막대 | 높이 4(`space.1`), 모서리 `radius.full`, 트랙 `color.sunken`, 채움 `color.work`, 조각 폭 32% |
| 경과 시간 | `type.small` + `color.muted`, 숫자 tabular, 최소 폭 48 |
| 대기 칩 | `type.meta`, 면 `color.sunken`, 모서리 `radius.full` |
| 다시 시도 버튼 | 컴포넌트 `button` secondary (데스크톱 sm) |
| 구분선(대화 속) | 선 `color.line`, 글자 `type.meta` + `color.muted`, ✓는 `color.ok` |
| 사이드바 | 기존 멤버 행(`row.*`) + 스피너 `color.work` |
| 모바일 | 띠 모서리·여백은 모바일 카드 규칙(`radius.card` 16, 패딩 16) |

**새 토큰 제안(원천 수정 → T-145 `tokens/`)**: `motion.duration.loop` = 1600ms (막대 한 번 흐르는 시간). 스피너 회전(1000ms)은 기존 사이드바 스피너 값을 따른다. 이 외 새 값 없음.

## 5. 모션
- 나타남: 투명도 0→1, `motion.duration.fast`(120ms) `out-cubic`, 위로 `motion.travel.rise`(6px) 이동.
- 막대: 32% 폭 조각이 트랙을 왼쪽에서 오른쪽으로 `motion.duration.loop` 선형 반복(끝에서 다시 처음).
- 끝: 아이콘·제목 교차 페이드 `fast`, 4초 뒤 사라짐은 페이드 `slow`(200ms).
- **동작 줄이기**(웹 `prefers-reduced-motion`·iOS `accessibilityReduceMotion`·Android 애니메이션 배율 0·macOS 손쉬운 사용): 이동 0, 막대 조각은 고정(왼쪽에서 20%), 나타남·사라짐은 `motion.duration.reduced`(100ms) 페이드만. **경과 시간 숫자는 계속 올라간다**(움직임이 아니라 글자). TUI 스피너는 고정 기호 `…`.

## 6. 문구 키 (ko 해요체 / en)
| 키 | ko | en |
|---|---|---|
| `compact.title.auto` | 대화를 요약하고 있어요 | Summarizing the conversation |
| `compact.body.auto` | 길어진 대화를 정리하는 중이에요. 끝나면 이어서 할 수 있어요. | Tidying up a long conversation. You can continue when it's done. |
| `compact.title.manual` | 요약해서 새로 시작하고 있어요 | Summarizing to start fresh |
| `compact.body.manual` | 지금까지의 대화를 요약해 다음 대화로 넘겨요. | Carrying a summary into the next conversation. |
| `compact.title.long` | 요약이 오래 걸리고 있어요 | Summarizing is taking a while |
| `compact.body.long` | 대화가 길어서 시간이 더 필요해요. 그대로 기다려 주세요. | The conversation is long and needs more time. Hang tight. |
| `compact.elapsed` | {n}초 / {m}분 {s}초 | {n}s / {m}m {s}s |
| `compact.queued` | 메시지 {n}개 대기 중 · 요약이 끝나면 보낼게요 | {n} message(s) waiting · I'll send when it's done |
| `compact.done.title` / `.body` | 요약했어요 / 이어서 대화할 수 있어요. | Summarized / You can keep chatting. |
| `compact.divider` | 요약했어요 · 이어서 대화할 수 있어요 | Summarized · you can keep chatting |
| `compact.fail.title` / `.body` | 요약하지 못했어요 / 대화는 그대로예요. 다시 시도할까요? | Couldn't summarize / The conversation is unchanged. Try again? |
| `compact.retry` | 다시 시도 | Try again |
| `compact.sidebar` | 대화 요약 중 · {elapsed} | Summarizing · {elapsed} |
| `compact.tui.run` / `.queued` / `.fail` | 대화 요약 중… {elapsed} / 메시지 {n}개는 요약이 끝나면 보낼게요 / 요약하지 못했어요. 대화는 그대로예요 · /compact로 다시 시도 | Summarizing… {elapsed} / {n} message(s) will be sent when it's done / Couldn't summarize. The conversation is unchanged · retry with /compact |

글은 `word-break: keep-all` + `overflow-wrap: anywhere`(웹·데스크톱), 시간은 tabular 숫자.

## 7. 접근성
- **띠는 `role="status"`(polite live region)**. 시작 때 한 번 `대화를 요약하고 있어요`를 읽고, **경과 시간은 읽지 않는다**(1초마다 읽히지 않게 `aria-live` 밖의 별도 요소 또는 `aria-hidden`). 60초에 한 번 `요약이 오래 걸리고 있어요`, 끝에 `요약했어요. 이어서 대화할 수 있어요`, 실패는 `assertive` 한 번.
- **VoiceOver(iOS)**: 띠를 하나의 접근성 요소로 묶고 label = 제목, value = `{경과}`. 시작·끝·실패는 `UIAccessibility.post(.announcement, …)`. 막대 장식은 숨김. `다시 시도`는 버튼 trait, 눌림 영역 44.
- **TalkBack(Android)**: 띠는 `liveRegion = polite`, 막대는 `contentDescription` 없음(장식). 끝·실패는 `announceForAccessibility`. 눌림 영역 48.
- 색만으로 전달 안 함: 요약 중 압축 아이콘, 끝 ✓, 실패 ✕ + 문구. 대비는 `docs/contrast.md`의 `work/ok/err`·`muted` 조합(전부 AA 통과).
- 막대는 진행률(`progressbar` 값)이 아니다 → 값을 가진 `progressbar` 역할을 쓰지 않는다(없는 값을 읽게 되므로). 필요하면 불확정 `progressbar`이되 값 없음.
- 키보드(데스크톱·TUI): `다시 시도`에 포커스 링(`focus.ring-*`). 띠가 입력 포커스를 빼앗지 않는다.

## 8. 하지 말 것
- %를 보여 주는 것(근거 없는 값) · 일정 시간 뒤 자동 실패 처리 · 요약 중 입력창 막기(입력은 받는다) · 띠가 대화 위를 덮어 메시지를 가리기 · 이모지·귀여운 문구 · 새 색 도입(`work`·`ok`·`err`만).

## 9. 완성 후 디자인 QA에서 볼 것
요약 중/오래 걸림/끝/실패/대기 칩 5개 상태 × 라이트·다크 × 데스크톱·iOS·Android·TUI, 동작 줄이기에서 막대 고정, 320px 폭에서 막대·시간 한 줄 유지, 스크린리더 안내 횟수.
