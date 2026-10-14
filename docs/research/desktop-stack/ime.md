# GPUI 한국어 IME·글리프 선명도 검증 (T-53)

상태: **예제와 체크리스트까지 완료, 실기 검증은 미완.** 이 세션에는 화면 캡처 권한과 UI 자동화(키 입력·IME 전환) 권한이 없어 (`screencapture`: "could not create image from display", `osascript` System Events 응답 없음) 시나리오를 직접 돌리지 못했다. 아래 표의 결과 칸은 모두 **미확인**이며, 추정치를 사실처럼 적지 않았다.

## 실행

```
cargo run --release -p equipe-desktop --example ime_probe
PROBE_FONT="Apple SD Gothic Neo" # 기본값. 다른 폰트로 바꿔 볼 때만 지정
```

창에는 한 줄 입력(`Input`), 여러 줄 입력(`Textarea`, 3~8행 자동 확장), 같은 문장 13/14/16pt, 그리고 하단 이벤트 로그(change / ENTER / focus / blur)가 있다. 로그의 `ENTER(send?)` 줄이 찍히는 시점으로 "조합 중 Enter가 전송으로 새는지"를 눈으로 확인할 수 있다.

## 시나리오 (macOS 두벌식)

| # | 시나리오 | 확인 방법 | 결과 |
|---|---|---|---|
| 1 | 조합 밑줄 표시 | "한" 입력 중 마지막 글자에 밑줄(조합 중 표시) | 미확인 |
| 2 | 후보창 위치(한자 키 / ⌥Enter) | "한" 입력 후 ⌥Enter → 후보창이 캐럿 근처에 뜨는가 | 미확인 |
| 3 | 조합 중 Backspace | "한" → ⌫ → "하" → ⌫ → "ㅎ" → ⌫ → 빈칸 | 미확인 |
| 4 | 조합 중 마우스 클릭 / 포커스 이동 | "한" 조합 중 다른 입력창·빈 곳 클릭 → 글자 확정·유실·중복 여부 | 미확인 |
| 5 | 조합 중 ⌘C / ⌘A | 패닉(프로세스 종료) 없이 동작하는가 | 미확인 |
| 6 | Enter: 조합 확정 vs 전송 | "안녕" 조합 중 Enter → 로그에 확정만, 두 번째 Enter에서 `ENTER` | 미확인 |
| 7 | 영↔한 전환 직후 첫 글자 | 영문 입력 → Caps/⌃Space로 전환 → 첫 자음이 유실·중복 없이 | 미확인 |
| 8 | 긴 문장 빠른 입력 | 40자 이상 빠르게 → 글자 누락·순서 뒤바뀜 | 미확인 |
| 9 | 한영 혼합 커서 이동 | "abc한글def"에서 ←/→/⌥←/⌥→ | 미확인 |

## 렌더링 비교 (미실시)

- 13/14/16pt × 1x/2x를 TextEdit의 같은 문장(Apple SD Gothic Neo)과 나란히 캡처해 이 폴더에 `render-*.png`로 둔다.
- 1x는 외장 모니터 또는 시스템 설정의 해상도 "Low resolution" 모드가 필요하다.

## 소스 읽기로 얻은 단서 (gpui-pre-macos 0.3.8, 실행 검증 아님)

- 렌더링: `text_system.rs`의 `recommended_rendering_mode`가 항상 `Grayscale`이다. 서브픽셀(LCD) AA는 쓰지 않는다. macOS 10.14 이후의 시스템 텍스트도 같은 방식이므로 그 자체가 흐림의 원인은 아닌 것으로 보인다. 다만 `glyph_dilation_for_color`로 글자 두께(Core Graphics 스머징)를 따라 하는데, 이는 `AppleFontSmoothing=0`이면 꺼진다. TextEdit와 굵기가 달라 보이면 이 값을 먼저 확인할 것. 글리프는 가로 서브픽셀 위치별(`SUBPIXEL_VARIANTS_X`)로 아틀라스에 따로 래스터화되므로, 흐림이 보이면 이 변형 수를 1로 줄이는(위치 스냅) 패치가 우회책 후보다. 아직 시도하지 않았다.
- IME: `handle_key_event`는 조합 중(`marked_text_range` 존재)이면 키를 먼저 `inputContext`로 보낸다. 한국어 입력원이 켜져 있으면 인쇄 가능 키도 IME 우선(`is_ime_printable_key`)이라 #7 유형의 문제는 구조상 줄어 있다.
- 후보창 위치(#2)는 `first_rect_for_character_range`가 입력 핸들러의 `bounds_for_range`에서 받은 사각형을 화면 좌표로 바꿔 주는 것에 달려 있다. 이 변환은 `NSFullSizeContentViewWindowMask` 창과 아닌 창에서 다르게 계산하므로, Root가 만드는 창(gpui-kit `open_window`)에서 실제 위치를 봐야 한다. zed #64850과 같은 증상일 가능성이 있다.
- 알려진 이슈(zed #65453 밑줄, #64389 조합 중 복사 패닉, #46319 한자 키, #57777 CJK 흐림)가 gpui-pre 0.3.8에 남아 있는지는 위 표로 확인해야 한다.

## 우회한 것 / 못 한 것

아직 없음 (검증 전).

## 결론

**① GPUI 그대로 진행 (사람의 결정, 2026-10-14)** — 실기 검증은 하지 못한 채로 정했다. 한글 문제가 나오면 GPUI Kit/gpui-pre를 우리가 패치해 해결한다. 위 표는 `ime_probe`로 문제를 만났을 때 채우는 체크리스트로 남긴다. Tauri 대조는 하지 않았다.
