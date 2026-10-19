# 플랫폼별 적용 계획 (frontend·mobile 코드 티켓용)

원칙: **한 플랫폼 = 한 티켓**, 순서는 ① 토큰 원천의 자리 확정(PM·사람 결정, §0) → ② 데스크톱(값이 가장 적게 바뀜) → ③ 사이트 → ④ Android → ⑤ iOS(출시 심사 중이라 마지막). 모든 화면 변경은 **디자이너 QA 필수**(squad 결정 2026-10-16) — 각 티켓의 인수 기준에 "라이트·다크 스크린샷 전/후 + designer 승인"을 넣는다. 파일·줄 번호는 2026-10-16 시점(`main`) — 착수 때 다시 확인.

## 0. 먼저 정할 것 (코드 티켓 전)

| # | 결정 | 제안 | 이유 |
|---|---|---|---|
| A | **토큰 원천(`tokens/*.json` + `scripts/`)을 어느 저장소에 두나** | `equipe` 저장소 `design/system/`에 두고, 사이트·앱은 **생성 파일을 커밋**(각자 `design/generated/`). CI에서 `npm run check && git diff --exit-code` | `equipe`에 이미 `docs/BRAND.md`가 있고 데스크톱이 그 안에 있다. 생성 파일을 커밋하면 사이트·앱 빌드에 Node가 필요 없다. 사람이 "자산은 프로젝트마다" 결정했으므로 원천은 한 곳, 사본은 각자 |
| B | **제품 UI 주 버튼 = 글자색 면**(검정/bone) 확정 | T-144 §2.2 제안 그대로 | 데스크톱 다크 주 버튼이 강철 `#2F4256` → bone으로 크게 바뀐다(§2 위험) |
| C | 코드 티켓에서 Style Dictionary 버전 고정(`^4`) | 지금 `package.json` 그대로 | 포맷터는 커스텀이라 메이저 업그레이드 때 깨질 수 있음 |

## 1. 공통: 생성 파일 쓰는 법
`npm install && npm run check` → `build/`가 같은 원천에서 다시 만들어진다. 엔지니어는 **생성 파일을 손으로 고치지 않는다**. 값이 필요하면 `governance`(README §거버넌스)의 변경 절차. 검증한 것: Rust `rustc --crate-type lib`로 컴파일 ✅ · Swift `swiftc -typecheck`(iOS 시뮬레이터 SDK) ✅ · Kotlin은 `equipe-app/android`를 `/tmp`에 복사해 `:features:compileDebugKotlin` 통과 ✅(explicit API 모드 포함) · CSS는 Chrome으로 시트 렌더 ✅. **미검증**: GPUI에 실제로 물려 렌더, SwiftUI·Compose 화면에 실제 적용.

## 2. 데스크톱 — `equipe/crates/desktop`

**바꿀 파일**
1. `design/system/…` → 복사 `crates/desktop/src/tokens.rs`(= `build/rust/tokens.rs`), `main.rs`에 `mod tokens;`.
2. `src/theme.rs` `Palette`(`dark()` L111, `light()` L139): 아래 매핑으로 필드를 `tokens::palette(dark)`에서 채운다(`rgba(u32)`로 바로 사용).

| `Palette` | 토큰 | 값 변화 |
|---|---|---|
| `bg` `text` `raised`(→`color_raised`) `code_bg`(→`color_sunken`) | `bg` `text` `raised` `sunken` | 라이트 `code_bg` `#ECEBE6`→`#EAE7E0`, 다크 `raised` `#141417`→`#1B1C20`(앱과 통일) |
| `muted` | `muted` | 라이트 `#5E6068`→`#4F5159`, 다크 `#8A8C93`→`#A3A5AB` |
| `accent` `accent_bg` `on_accent` | `accent` `accent-bg` `on-accent` | **`#2E5C8A`→`#1D4ED8`, 다크 `#B4CDE6`→`#8FB0FF`** (링크·선택 색이 눈에 띄게 바뀜) |
| `primary` `on_primary` + `primary_hover()` `primary_press()`(L300~) | `action` `on-action` `action-hover` `action-pressed` | 라이트 거의 동일(`#15161A`). **다크 `#2F4256`(강철) → `#EFEDE7`(bone)** — 큰 변화, 결정 B |
| `amber` `orange` `red` | `work` `wait` `err` | 동일 값 |
| `danger()` `danger_bg()` (L289~) | `err` `err-bg` | `#B3261E`/`#F0918A` → `#AD2E27`/`#EE7B74`(위험 값 하나로) |
| `hair` | `line` | 알파 `0x24` 동일 계열 |
| 로컬 유지 | `side` `side_hover` `side_sel` `wash` `prose` `text2` `members` `popover` `tip_*` | 토큰 없음 — 그대로 |

3. `theme.rs`의 상수 `S3,S6,R_SM..R_XL,CONTROL,FIELD,ROW,ICON,ICON_SM,GAP,BODY/UI/SMALL/META(+_LINE)` — **값이 같다**(`SPACE_*`, `RADIUS_*`, `SIZE_BUTTON`, `SIZE_FIELD`, `SIZE_ROW`, `TYPE_*`). 상수를 `tokens`에서 re-export하는 한 줄씩 교체. `ICON_SM` 14 → 16(`SIZE_ICON_SM`)은 화면 영향 있음.
4. `src/motion.rs` `FAST/BASE/SLOW/SLIDE` = `MOTION_DURATION_*_MS`(값 동일), `POP_TRAVEL/RISE_TRAVEL/DIALOG_TRAVEL` = `MOTION_TRAVEL_*`, `float_shadow()` = `ELEVATION_FLOAT`(값 동일), `reduce_motion` 분기는 `MOTION_DURATION_REDUCED_MS`(100ms 페이드)로 — 지금은 "끝 상태를 즉시"라서 *선택* 변경.
5. 컴포넌트: 결정 카드(`decide.rs`)·토스트·시트(`sheet.rs`)를 `decision.*`·`toast.*`·`sheet.*` 토큰으로 — 별도 후속.

**위험**: ① 다크 주 버튼 색 변화(결정 B) ② 링크·선택 파랑 변화 ③ `ICON_SM` 14→16으로 사이드바 간격 ④ 하이라이트 `accent_bg`가 선택 행 색을 겸해서 `side_sel`과 겹칠 수 있음 ⑤ 멤버 색·프로젝트 색은 안 건드림. **크기**: 중(색 매핑 + 스크린샷 비교 라이트·다크 × 주요 8화면). **QA**: 같은 화면 전/후 PNG.

## 3. 사이트 — `equipe-site`

**바꿀 파일**
1. 복사 `src/styles/tokens.css`(= `build/css/tokens.css`) + `compat-site.css`; `src/layouts/*`(또는 `global.css` 맨 위)에서 둘을 import.
2. `src/styles/global.css` `:root`(L2-L22): 색·간격·반지름·`--tap` 선언을 **삭제**하고 `compat-site.css`의 별칭이 대신하게 한다(규칙은 그대로 `var(--bg)` 등 사용 → 한 번에 안 깨짐). 이후 규칙을 `--eq-*`로 점진 교체, 다 끝나면 `compat-site.css` 삭제.
3. 하드코딩: `font-size` 14/15/16/17/18/19/22 리터럴(L35-L67, L114-L178) → `font: var(--eq-type-*)`(`:lang()` 규칙은 유지), `border-radius: 34px/12px/6px/7px`(L118-L124)와 `.phone` 색은 §감사 3 — 이미지로 옮기거나 그대로 두고 TODO.
4. 컴포넌트 CSS는 `reference/components.css`를 시작점으로(`.btn`→`.eq-btn--brand`, `.card`, `.chip`, `details`→row, `.lang-banner`→toast 대신 배너).
5. `design/tokens.md`는 `design/system` 링크 한 줄로 대체, `Mark.astro`는 이 티켓 범위 밖(T-144 로고 티켓).

**값 변화**: `--bg` `#FAF8F4`→`#F6F5F1`, 다크 `#121211`→`#0A0A0C`; `--surface`(카드) `#F0EDE6`→`#FFFFFF`(**라이트에서 카드가 바탕보다 밝아진다** → 카드에 헤어라인이 생김); `--r-2` 14→12; `--text`/`--muted` 미세 변화; `--accent`·`--on-accent` 동일. 전부 `compat-site.css` 주석에 기록.
**위험**: ① 카드 면·모서리 변화로 홈 레이아웃의 시각 무게가 달라짐 ② 다크 `#0A0A0C`에서 사진·스크린샷 경계 ③ 번역 페이지 8개 모두 확인 필요 ④ Astro 빌드가 CSS 변수명 충돌 없는지(`--eq-` 접두라 낮음) ⑤ 캐시: 정적 CSS 해시 파일명 확인. **크기**: 중-대(별칭으로 시작하면 소). **QA**: 라이브 8개 언어 × 라이트·다크 × 1280/390 폭.

## 4. Android — `equipe-app/android`

**바꿀 파일**
1. 복사 `features/src/main/kotlin/im/equipe/design/EquipeTokens.kt`(= `build/kotlin/EquipeTokens.kt`, **explicit API 모드 대응·컴파일 확인됨**). 별도 `:design` 모듈로 분리하면 `features`·`app`이 공유(권장 — 안 해도 됨).
2. `features/…/Theme.kt` `EquipeTheme`: `MaterialTheme(colorScheme = equipeMaterialColors(dark), typography = EquipeTypography)` + `CompositionLocalProvider(LocalEquipeColors provides …)`. 기존 `Dark`/`Light` 스킴(14색)을 지운다 — 현행 매핑(surface=bg, surfaceVariant=카드 …)을 이어받았다.
3. 화면 코드의 리터럴: `16.dp`(31회) `12.dp`(21) `56.dp`(19) `8.dp`(15) `48.dp`(7) `24.dp`(6) → `EquipeSpace.s4` 등, `RoundedCornerShape(12/16/8.dp)` → `RoundedCornerShape(EquipeRadius.control/card…)`. 호출 위치는 `grep -rn "\.dp" android --include=*.kt`.
4. `app/src/main/res/values{,-night}/themes.xml`의 윈도 배경(`#FFF6F5F1` 등) → `bg` 값. 안드로이드 아이콘·스플래시는 로고 티켓.
5. 컴포넌트: 결정 카드·시트·토스트는 신규 `@Composable`(`EquipeButton`, `EquipeBadge` …) 모아 `features/…/ui/`에 — 시트 PNG가 기준.

**값 변화**: 앱은 지금 **강조색이 없다**(`primary`=글자색). v1에서도 주 버튼은 글자색 면(결정 B)이라 버튼은 그대로이고, 링크·선택·포커스에 파랑(`accent`)이 *새로* 생긴다. `outline`/`secondaryContainer`가 `line`/`sunken`으로 바뀜(다크 `#26272C`→`#111114`). Typography: M3 기본 → 역할 매핑(`bodySmall` 12→13 sp, `labelMedium` 12 유지).
**위험**: ① Material 컴포넌트(`OutlinedTextField`, `NavigationBar`)가 스킴 색을 쓰는 곳 전수 확인(**미확인** — 코드 전체를 읽지 못함) ② `sp` 배율(글자 크기 설정 최대)에서 `row` 56dp 안에서 2줄 넘침 ③ 동적 색상은 꺼 둔 상태 유지 ④ Macrobenchmark 콜드 스타트: `EquipeColors`가 `object`/top-level `val`이라 초기화 비용은 미미하나 CLAUDE.md 요건대로 측정 필요. **크기**: 중. **QA**: 에뮬레이터(android-35) 라이트·다크 × 글자 크기 기본/최대.

## 5. iOS — `equipe-app/ios`

**바꿀 파일**
1. 복사 `Packages/Sources/Features/EquipeTokens.swift`(= `build/swift/EquipeTokens.swift`, `swiftc -typecheck` 확인됨 — `Color(0x…)` 헬퍼는 `private extension`이라 충돌 없음) 또는 신규 `Packages/Sources/Design` 패키지로 분리(`public` API라 가능).
2. 루트에 `@Environment(\.colorScheme)` → `EquipeColors.resolved(scheme)`를 주입하는 `EquipeThemeModifier`(수기 파일 1개, ~15줄) 추가 — 생성기가 SwiftUI Environment 키는 만들지 않는다.
3. `Packages/Sources/Features/*.swift`: `.foregroundStyle(.secondary)` → `colors.muted`, `Color(.systemBackground)`(`RootView.swift`) → `colors.bg`, `.padding(16)`·`spacing: 8` 등 리터럴(상위: `spacing: 0`/14, `spacing: 8`/11, `.padding(.horizontal, 16)`/7, `.padding(32)`/6) → `EquipeSpace.*`, `cornerRadius: 12/16/18`(5곳) → `EquipeRadius.control/card`(18 → 16으로 정리), `.font(.footnote)` 등은 **그대로**(이미 시스템 Dynamic Type이고 역할이 같다: footnote=`small`, subheadline=`ui`, caption=`meta`, callout.monospaced=`code`).
4. `ApprovalSheetView`(`DecisionViews.swift`)를 결정 카드 구성(배지·누가·어디서·내용·안내·두 버튼)으로 맞춘다 — 구조는 이미 비슷함. `reduceMotion` 분기는 `EquipeMotion.animation(_, reduceMotion:)`로.
5. `ios/design/tokens.json`은 삭제 가능(코드가 안 읽음 — C6 확인 후). `Assets.xcassets`의 AccentColor를 `accent`로(라이트/다크 쌍).
6. 아이콘 SF Symbols 유지.

**값 변화**: 시스템 색 → 토큰 색(배경 `systemBackground` 흰/검정 → `#F6F5F1`/`#0A0A0C`, 보조 글자 `.secondary` → `muted`). 링크·`tint`에 파랑 `accent` 신규.
**위험**: ① **App Store 첫 심사 제출됨(T-117)** — 심사 통과·출시 *뒤*의 v1.1로 넣거나 심사 중이면 반려 위험 없는 수준인지 사람이 판단(이 변경은 시각만, 동작 무관이지만 스크린샷이 달라진다 → 스토어 스크린샷 교체 필요) ② `.secondary`는 vibrancy·접근성 "대비 높이기"에 자동 대응하는데 토큰 색은 안 한다 → `UIAccessibility.isDarkerSystemColorsEnabled`일 때 `text`로 올리는 규칙 추가(미구현 제안) ③ iPad 포인터 호버는 SwiftUI `.hoverEffect` 대신 오버레이(미결정) ④ Dynamic Type 최대(AX5)에서 `row` 44pt 고정 높이 금지(최소 높이만) ⑤ `LaunchTiming` 콜드 스타트 1초 요건 — 정적 `let`만이라 영향 미미(측정 필요). **크기**: 대. **QA**: 시뮬레이터 라이트·다크 × 글자 크기 3단계 × 주요 6화면.

## 6. 티켓으로 쪼개기 (제안)

| 순서 | 제목 | 담당 | 크기 | 선행 |
|---|---|---|---|---|
| 1 | 디자인 시스템 원천의 저장소 이전 + CI(`check`) | frontend | 소 | 결정 A |
| 2 | 데스크톱: `tokens.rs` 도입 + 팔레트·상수 교체 | frontend(데스크톱) | 중 | 1, 결정 B |
| 3 | 사이트: 토큰 CSS + 별칭 → 규칙 이전 | frontend | 중 | 1 |
| 4 | Android: `EquipeTheme` 교체 + dp·shape 리터럴 정리 | mobile | 중 | 1 |
| 5 | iOS: `EquipeTokens` + 테마 주입 + 리터럴 정리 | mobile | 대 | 1, 출시 일정 |
| 6 | 결정 카드·토스트·시트 신규 컴포넌트(데스크톱·iOS·Android) | frontend·mobile | 대 | 2·4·5 |
| 7 | 각 플랫폼 디자인 QA(전/후, 라이트·다크) | designer | 소×4 | 2~6 |
