# 컴포넌트 규칙 v1

9개: 버튼 · 입력창 · 카드 · 칩 · 배지 · 목록 행 · 시트/모달 · 결정 카드 · 토스트(요청한 8개에서 칩·배지를 나눔). 각 컴포넌트는 **용도 → 변형 → 상태 → 크기 → 토큰 → 플랫폼 차이 → 하지 말 것** 순서. 렌더: `components/<이름>-light.png` · `-dark.png` (HTML 원본 `build/sheets/`, `npm run sheets`가 만든다, 웹 참조 구현 `reference/components.css`). 값은 전부 `tokens/component.json`(→ 의미 토큰 → 원시값)에서 온다.

## 공통 규칙

**상태** — 모든 눌러지는 것은 같은 문법을 쓴다.
| 상태 | 표현 | 토큰 |
|---|---|---|
| 기본 | 변형별 기본 면·글자 | 컴포넌트 토큰 |
| 호버 (포인터 있는 곳만: 웹·데스크톱·iPad 포인터) | 중립 오버레이 한 겹. 주 버튼만 전용 색 | `color.hover`, `button.primary-bg-hover` |
| 눌림 | 호버보다 한 단계 진한 오버레이. iOS·Android는 시스템 눌림 효과 대신 이것을 쓴다 | `color.pressed`, `button.primary-bg-pressed` |
| 포커스 | 링 2px, 오프셋 3px(데스크톱 2px). **키보드·보조 기술 포커스에만** 보인다(`:focus-visible`) | `color.focus`, `focus.ring-*` |
| 비활성 | 불투명도 0.4 + 동작 없음 + `aria-disabled`/`disabled`. 이유가 필요하면 옆에 글로 | `opacity.disabled` |
| 로딩 | 글자를 "…중" 문구로 바꾸고 비활성처럼 막는다(스피너만 두지 않는다). 폭은 유지 | — |
| 오류 | 색 + 아이콘/기호 + 문구 셋 | `color.err` |

**접근성** — 글자 4.5:1, 그래픽 3:1(`contrast.md` 116조합 전부 통과). 색만으로 의미를 전달하지 않는다. 눌림 영역 웹·iOS 44 / Android 48 / 데스크톱 28(포인터) 이상 — 보이는 크기가 작으면 투명 영역으로 키운다.
**움직임** — 시간 3개(`fast` 120 · `base` 160 · `slow` 200ms), 곡선 1개(`out-cubic`). 이동은 팝오버 4 · 카드 6 · 대화상자 8. **동작 줄이기**(웹 `prefers-reduced-motion`, iOS `accessibilityReduceMotion`, Android 시스템 애니메이션 배율 0, 데스크톱 macOS 손쉬운 사용): 이동 0, 남기는 건 100ms 이하 투명도 페이드뿐.
**글자** — 이 표의 역할 이름(`label`, `ui`…)은 `tokens/semantic.json`의 `type.*`. 앱은 플랫폼 글꼴을 쓰고 iOS는 Dynamic Type, Android는 sp 배율을 따른다 — 고정 높이로 글자를 자르지 않는다(최소 높이만).
**언어** — 문구는 짧게, 한국어 해요체, 이름은 `equipe`(brand.md). 버튼 글자는 동사: "허용", "거절"(예/아니오 금지).

## 1. 버튼 Button — `components/button-*.png`
- **용도**: 한 가지 행동을 시작한다. 한 영역에 *주 버튼은 하나*.
- **변형**: `primary`(`color.action` 면 — 글자색 면, 제품 UI의 주 행동) · `secondary`(테두리 `border-strong`) · `ghost`(링크색 글자, 면 없음 — 3순위 행동) · `danger`(`err` 면 — 되돌릴 수 없는 행동) · `brand`(`accent` 파랑 면 — **웹 마케팅 전용**, 제품 UI에 쓰지 않는다. 이유: 승인 같은 한 행동이 묻히지 않게, T-144 가이드 §2.2).
- **크기**: md = `size.button`, sm = `button.height-sm`. 가로 패딩 `button.padding-x`.
- **토큰**: `button.*`(`tokens/component.json`).

| | 웹 | 데스크톱 | iOS | Android |
|---|---|---|---|---|
| 높이 md / sm | 44 / 36 | 28 / 24 | 44 / 36 | 48 / 36(터치 영역 48) |
| 가로 패딩 | 20 | 12 | 16 | 24 |
| 모서리 | 8 | 8 | 12 (연속 곡선 `.continuous`) | 12 |
| 눌림 | 오버레이 | 오버레이 | 오버레이(시스템 하이라이트 대신) | 오버레이(리플 대신 — 리플 색을 `pressed`로) |
| 글자 | `label` 16/600 | 14/600 | 15/600(subheadline) | 14/600(labelLarge) |

- **하지 말 것**: 한 영역에 주 버튼 둘 · 모바일에서 둘을 세로로 놓을 때 위험을 *위에*(주 행동이 위, 취소는 아래) 두되 위험한 일이면 위험 버튼이 주 자리 · 글자 2줄 이상 · 아이콘만 있는 버튼에 라벨(접근성 이름) 빠뜨리기.

## 2. 입력창 Input — `components/input-*.png`
- **용도**: 한 줄 글 입력(이름·코드). 여러 줄은 같은 토큰, 높이만 `auto`.
- **구성**: 라벨(항상 보임, `label` 글자) → 필드 → 도움말(`small`). 플레이스홀더는 예시이지 라벨이 아니다.
- **상태**: 기본 · 호버 · 포커스(테두리 2px `focus` — 링이 아니라 테두리가 두꺼워짐) · 오류(테두리 `err` + `✕` + 문구) · 비활성.
- **크기**: `size.field` — 웹 44 · 데스크톱 36 · iOS 44 · Android 48. 글자 `ui`; **웹·iOS에서 16px 미만이면 포커스 시 화면이 확대되므로** 웹 입력은 16px(`ui` 웹 값 16 ✓).
- **플랫폼**: iOS는 `TextField` + `.textFieldStyle(.plain)`에 직접 테두리, Android는 `OutlinedTextField` 색만 토큰으로(Material 라벨 애니메이션은 쓰지 않고 라벨을 위에 고정 — 웹과 같은 동작).
- **하지 말 것**: 헤어라인(`line`)을 필드 테두리로 쓰기(3:1 미달) · 플레이스홀더로 형식 설명을 대신하기 · 오류를 색만으로 표시.

## 3. 카드 Card — `components/card-*.png`
- **용도**: 한 묶음의 정보. 변형: 기본(`surface` + 헤어라인) · 떠 있는(`raised` + `elevation.float` — 팝오버·메뉴·토스트처럼 위로 뜨는 것에만) · 누르는 카드(기본 + 오버레이 + 포커스).
- **패딩**: `card.padding` 웹·데스크톱 24 · 모바일 16. 모서리 `radius.card` 웹·데스크톱 12 · 모바일 16.
- **상태**: 눌러서 여는 카드만 호버·눌림·포커스가 있다. 정보 카드는 상태 없음.
- **플랫폼**: iOS 카드는 `.background(colors.surface, in: RoundedRectangle(cornerRadius:, style: .continuous))`, 그림자 대신 헤어라인(다크에서 그림자는 안 보임).
- **하지 말 것**: 카드 안에 카드 · 카드마다 다른 모서리·그림자 · 그림자로 계층 만들기(헤어라인과 면 색으로).

## 4. 칩 Chip — `components/chip-*.png`
- **용도**: 필터·선택(여러 개 중 고름). 알약형(`radius.chip`). 변형: 기본 · 선택됨(`accent-bg` + `accent` 글자·테두리 + 앞에 `✓`).
- **크기**: 높이 32(데스크톱 24), 가로 12, 글자 `small`. 모바일은 눌림 영역 44/48까지 투명 영역으로.
- **하지 말 것**: 상태 표시에 쓰기(→ 배지) · 선택을 색만으로 · 칩 안에 긴 문장.

## 5. 배지 Badge — `components/badge-*.png`
- **용도**: 한눈에 알아볼 상태. 5종: `ok` 완료 · `work` 일하는 중 · `wait` 내 차례 · `err` 오류 · `neutral`.
- **구성**: **기호 + 글자 필수**(✓ 완료, ⋯ 일하는 중, ! 내 차례, ✕ 오류). 기호는 플랫폼 아이콘 세트(Lucide/SF Symbols/Material Symbols)로 — 시트의 유니코드 글리프는 자리표시. 면은 의미색 12% 투명 + 글자는 의미색 — 어느 바탕 위에서도 4.5:1(`contrast.md` 배지 행).
- **크기**: 높이 22(데스크톱 20), 글자 `meta` 600, 가로 8.
- **의미 고정**: amber(work)는 "에이전트가 일하는 중", orange(wait)는 "사람을 기다림". **프로젝트·멤버 색으로 amber/orange를 쓰지 않는다**(desktop `PROJECT_HUES` 규칙 그대로).
- **상태**: 눌러서 동작하는 배지는 만들지 않는다(눌러야 하면 칩이나 버튼).

## 6. 목록 행 List row — `components/row-*.png`
- **용도**: 같은 종류 항목의 나열(멤버, 티켓, 기기). 앞(아바타/점) · 본문 2줄(제목 `ui`, 보조 `small`) · 뒤(배지/값/›).
- **높이** `size.row`: 웹 48 · 데스크톱 32 · iOS 44 · Android 56. 가로 패딩 16(데스크톱 12). 구분선 `line` 1px(마지막 행은 없음). 데스크톱 행은 모서리 8(호버 면이 둥글게), 나머지는 0.
- **상태**: 호버·눌림(오버레이) · 선택(`accent-bg`, **글자색은 그대로** — 선택을 면 하나로만 알리지 않고 체크·굵기·`aria-selected`를 함께) · 포커스 · 비활성.
- **플랫폼**: iOS는 `List` 대신 `LazyVStack`로 구분선을 직접(시스템 인셋 구분선이 토큰과 어긋남), 스와이프 동작은 시스템 것을 그대로. Android는 `ListItem` 대신 `Row` + `Modifier.clickable`(리플 색 `pressed`).
- **하지 말 것**: 행 안에 버튼 둘 이상(눌림 영역 충돌) · 뒤 칸에 긴 글 · 구분선과 카드 테두리 동시 사용.

## 7. 시트·모달 Sheet / Modal — `components/sheet-*.png`
- **용도**: 지금 하던 일을 막고 하나의 결정·입력을 받는다. 웹·데스크톱: 가운데 대화상자(최대 `sheet.max-width` 480). 모바일: 바텀 시트(손잡이 36×4, 위 모서리 24, 시스템 시트 `presentationDetents`/`ModalBottomSheet`를 쓰되 색·모서리는 토큰).
- **구성**: 제목(`title`) → 설명(`body`) → 행동(오른쪽 정렬, 모바일은 세로 전체 폭). 행동은 최대 둘 + 닫기.
- **상태**: 열림(대화상자 `slow` 200ms + 8 이동, 시트는 시스템 곡선) · 닫는 길 항상 있음(취소 버튼 · 바깥 탭 · Esc/뒤로) · 처리 중(행동 비활성 + 문구).
- **뒤**: `color.scrim`. 포커스는 시트 안에 가두고, 닫히면 연 곳으로 돌려준다.
- **하지 말 것**: 시트 위 시트 · 닫는 길 없는 모달 · 단순 알림에 모달(→ 토스트/배너) · 위험 행동을 기본 포커스로.

## 8. 결정 카드 Decision card — `components/decision-*.png` (equipe 고유)
- **용도**: 사람의 승인이 필요한 순간(허락 요청·내 차례). **제품에서 가장 크게 보여야 하는 컴포넌트**(playbook A2 원칙: 승인이 가장 크게).
- **구성**(위→아래): 배지 `! 내 차례` + 시각 → 누가 무엇을(`title`) → 어디서(`small`, muted) → 실제 내용(코드 블록 `sunken`) → 한 줄 안내("한 번만 허용돼요…") → 행동 둘(같은 폭, **거절=보조 · 허용=주**, 거절이 왼쪽).
- **면**: 테두리 2px `wait`(내 차례 색). 해결되면 테두리가 `line`으로 돌아가고 배지가 `✓ 허용했어요`로 바뀐다 — 사라지지 않고 기록으로 남는다.
- **상태**: 기본 · 포커스(허용 버튼) · 처리 중(두 버튼 비활성, 허용 글자 "보내는 중…") · 완료 · 오류(카드 안 배너, 버튼 재활성).
- **플랫폼**: iOS는 시트(`ApprovalSheetView`)로 올라오며 그 안에서 같은 구성, 완료 햅틱 1회. Android는 바텀 시트. 데스크톱은 대화 흐름 안 인라인 + 알림(알림 본문에 내용을 넣지 않는다 — 누가 무엇이 필요한지만, `desktop-cmdk-palette` 결정).
- **하지 말 것**: 허용을 기본 포커스로 *자동* 이동 · 두 버튼 크기·색을 같게 · 코드 내용 숨기기("자세히" 뒤에 두면 안 됨 — 판단 근거는 앞에) · 같은 승인을 토스트로.

## 9. 토스트 Toast — `components/toast-*.png`
- **용도**: 방금 한 일의 가벼운 확인("복사했어요"). 사라져도 큰일이 없는 정보만.
- **면**: 페이지 반대색(`inverse` / `on-inverse`, 액션 `inverse-accent`) — 어떤 바탕 위에서도 읽힌다. 모서리 `radius.control`, 그림자 `elevation.float`, 최대 폭 520.
- **시간**: `motion.duration.toast` 5초, 호버·포커스·스크린리더 읽는 동안 멈춤. 액션이 있으면 닫기 전까지 두거나 다른 길(내역·설정)을 남긴다. `role="status"`(iOS `UIAccessibility.post(.announcement)`, Android `LiveRegion.Polite`).
- **위치**: 웹·데스크톱 하단 가운데(화면 가장자리에서 `space.4`), iOS·Android 하단 탭바/입력창 위.
- **하지 말 것**: 오류를 토스트로(→ 배너·필드 오류, 놓치면 안 되므로) · 토스트에 두 개 이상 액션 · 쌓아 올리기(최대 1개, 새 것이 이전 것을 대체).

## 부록 — 플랫폼 이름 대응 (토큰 → 코드)

| 토큰 | CSS | Rust (`tokens.rs`) | Swift | Kotlin |
|---|---|---|---|---|
| `color.surface` | `var(--eq-color-surface)` | `palette(dark).color_surface` | `colors.colorSurface` | `colors.colorSurface` |
| `space.4` | `var(--eq-space-4)` | `SPACE_4` | `EquipeSpace.s4` | `EquipeSpace.s4` |
| `radius.card` | `var(--eq-radius-card)` | `RADIUS_CARD` | `EquipeRadius.card` | `EquipeRadius.card` |
| `type.body` | `font: var(--eq-type-body)` | `TYPE_BODY` | `EquipeType.body.font` | `EquipeType.body.toTextStyle()` |
| `motion.duration.fast` | `var(--eq-motion-duration-fast)` | `MOTION_DURATION_FAST_MS` | `EquipeMotion.durationFast` (초) | `EquipeMotion.durationFast` (ms) |
| `button.primary-bg` | `var(--eq-button-primary-bg)` | `palette(dark).button_primary_bg` | `colors.buttonPrimaryBg` | `colors.buttonPrimaryBg` |

규칙: 이름은 토큰 경로 그대로(점 → CSS `-`, Rust `_`, Swift·Kotlin camelCase). 숫자로 시작하는 단계는 `space` → `s4`, `border` → `w1`로 글자를 붙인다. 단위: CSS px · Rust 논리 px(f32)·시간 ms · Swift pt(CGFloat)·시간 초 · Kotlin dp/sp·시간 ms. 색: CSS `#RRGGBB(AA)` · Rust `0xRRGGBBAA` · Swift `Color(0x…)` · Kotlin `Color(0xAARRGGBB)`.
