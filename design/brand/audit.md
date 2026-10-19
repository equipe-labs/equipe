# T-144 브랜드 감사 (audit) — equipe 지금 모습 (2026-10-16, designer)

방향(사람 결정, 2026-10-16): **이름·로고는 유지, 색·서체·목소리·가이드라인을 정리·체계화.** 이 문서는 "지금 무엇이 쓰이고 어디가 어긋나는지"만 적는다. 처방은 `brand.md`(전략)·`guidelines.md`(규칙).
한눈에 보기: **`audit-board.png`**. 스크린샷 원본: `audit-shots/`(사이트 라이트·다크 2026-10-16 라이브 캡처). 각 값의 출처는 파일 경로로 적었다. 직접 못 본 것은 **미확인**.

## 0. 한 줄 결론

equipe는 지금 **세 가지 얼굴**이다. ① 사이트 = 파란 2×2 점 + 파란 링크, 웜 페이퍼 / ② 데스크톱·앱 아이콘·스토어 = 검정 타일 `e.` + 회색 점, 강철 파랑·검정, Helvetica / ③ iOS·Android 앱 화면 = 시스템 색, 강조색 없음. 로고·검정·강조색·서체·한국어 말투·지원 언어가 모두 곳마다 다르다. 가장 큰 균열은 **로고**(사이트 vs 나머지 전부)와 **강조색**(3종)이다.

## 1. 로고·마크·이름

| # | 곳 | 지금 | 출처 |
|---|---|---|---|
| L1 | 사이트 헤더·404·JSON-LD 로고 | 파란(`#1D4ED8`) 2×2 점, 꽉 찬 점 1개 + 반투명 3개. 워드마크 `equipe`(점 없음) | `equipe-site/src/components/Mark.astro`, `public/logo.png` / 라이브 캡처 `audit-shots/site-light.png` |
| L2 | 사이트 파비콘·홈 아이콘 | 파비콘 = 검은 타일 + 흰 `e`(T-105 기록), `public/app-icon.svg`·`design/mark.svg` = 파란 타일 + 흰 4점 → **사이트 안에서도 파비콘과 마크가 다르다** | `equipe-site/public/favicon.svg`, `design/mark.svg`, T-105 `brand.md` |
| L3 | 사이트 OG | 파란 4점 타일 + 워드마크 `equipe` | `public/og.png` (`audit-board.png` ③) |
| L4 | 맥 앱 아이콘 (T-88) | void(`#0A0A0C`) 타일, bone `e`, **회색(`#8A909B`) 사각 마침표** | `outputs/T-88 …/png/AppIcon-mac-*.png`, `src/icon-mac.html:14` |
| L5 | iOS·Android 앱 아이콘 (T-72) | L4와 같은 계열 | `outputs/T-72 …/png/ios`. 안드로이드 현행 `ic_launcher.xml`은 "Placeholder" 주석(T-105 기록, 이번에 재확인 안 함) |
| L6 | 스토어 대표 이미지 | 워드마크 `equipe.`(점 포함) + 회색 **사각** 점 | `outputs/T-72 …/png/play/feature-graphic-ko-1024x500.png` |
| L7 | GitHub 조직 아바타 | 흰 바탕 + 검은 `e` + 크롬 회색 점("라벨") | `equipe/docs/BRAND.md` Assets |
| L8 | 이름 표기 | 사이트 `equipe` / BRAND.md·안드로이드 `wordmark` `equipe.` / iOS 홈 화면 이름: `project.yml:61`은 `Equipe`(대문자), 같은 파일 `:96`은 `equipe` — **한 파일 안에서 충돌** / 안드로이드 `app_name` `equipe` / 용어집은 "항상 소문자 `equipe`" | 각 파일, `equipe-site/design/i18n/glossary.md` §1 |
| L9 | 마침표 모양 | 앱·스토어: 글꼴(Helvetica)의 **사각** 마침표 / T-105 C안: **원** / 사이트 4점: 원 | `icon-mac.html`, T-105 `brand.md` |

→ 처방: 로고 마스터 1개 확정(`guidelines.md` §1, 사람 결정 필요), 이름 표기 규칙 1개(§1.2).

## 2. 색

사이트 값은 `equipe-site/design/tokens.md`·`src/styles/global.css`, 데스크톱은 `crates/desktop/src/theme.rs:113-163`, 앱은 `equipe-app/design/tokens.json`.

| 역할 | 사이트 라이트/다크 | 데스크톱 라이트/다크 | 앱 토큰(iOS·Android) 라이트/다크 |
|---|---|---|---|
| 바탕 | `#FAF8F4` / `#121211` | `#F6F5F1` / `#0A0A0C` | `#F6F5F1` / `#0A0A0C` |
| 카드 | `#F0EDE6` / `#1B1B19` | `#FFFFFF`(raised) / `#141417` | `#FFFFFF` / `#16171A` |
| 본문 글자 | `#1A1A17` / `#F2EFE8` | `#15161A` / `#EFEDE7` | `#0A0A0C` / `#EFEDE7` |
| 보조 글자 | `#57544C` / `#A9A59B` | `#5E6068` / `#8A8C93` | `#3F4148`·`#5B5E66` / `#A3A5AB`·`#7C7E85` |
| **강조(링크·버튼)** | **`#1D4ED8` / `#8FB0FF`** | **`#2E5C8A` / `#B4CDE6`** | **없음** — 강조 = 글자색(`#0A0A0C` / `#EFEDE7`) |
| 채운 버튼 | 파랑 면 + 흰 글자 | 거의 검정 `#15161A` / 강철 `#2F4256` | 글자색 면 |
| 위험 | 사이트 UI 없음 | `#AD2E27` / `#EE7B74` | `#A32A24` / `#E8837D` |
| 진행·대기 | — | amber `#825600`/`#E2B865`, orange `#A04610`/`#EC9A62` | 앱 토큰엔 없음(미정의) |
| 포커스 링 | `--accent` | — | `#2F5E8C` / `#B4CDE6` |

어긋남:
- **C1 검정이 둘**: 사이트 다크 `#121211`(웜) vs 앱·데스크톱 `#0A0A0C`(void). 라이트 바탕도 `#FAF8F4` vs `#F6F5F1`.
- **C2 강조색 셋**: 밝은 파랑 `#1D4ED8` / 강철 `#2E5C8A` / 없음. 같은 "링크"가 곳마다 다른 파랑이거나 파랑이 아니다.
- **C3 `BRAND.md`와 충돌**: BRAND.md는 "둘째 강조색 금지, 단색+크롬"이고 사이트는 파란 강조색이 UI 전체에 있다 (`equipe/docs/BRAND.md` Palette).
- **C4 의미색이 데스크톱에만 있다**: amber(진행)·orange(내 차례)·red(오류)는 `theme.rs`에만 있고 앱 토큰·사이트엔 없다. 완료(녹색) 의미색은 **어디에도 정의가 없다**(`theme.rs`의 녹색 `0x28803C` 등은 멤버 아바타 색 8개 중 일부).
- **C5 멤버 아바타 8색**(`theme.rs:135, 163`)은 브랜드 팔레트와 별개 체계. 이번에 대비 재계산 안 함(**미확인**).
- **C6 앱 토큰 일부는 코드에 쓰이지 않는다**: iOS `RootView.swift`는 `Color(.systemBackground)`를 쓴다 — `tokens.json` 연결 여부는 이번에 끝까지 못 봄(**미확인**). 안드로이드 `themes.xml`에서 `#FFF6F5F1` 하나만 확인.
- 대비: 사이트 값은 문서에 AA 검증했다고 적혀 있고(`tokens.md`), 데스크톱은 `theme.rs` 주석에 4.5:1 의도. 이번에 **재계산은 새 팔레트 3안만** 했다(`contrast.md`). 현행 값 재측정은 안 함.

## 3. 서체

| 곳 | 지금 | 출처 |
|---|---|---|
| 사이트 | Inter Variable(자체 호스팅), 모노 JetBrains Mono Variable, ko는 `Apple SD Gothic Neo`/`Noto Sans KR`/`Malgun Gothic` 폴백 | `global.css:10-11,197`, `package.json`(`@fontsource-variable/*`) |
| 데스크톱 | `SANS = "Helvetica Neue"`, `MONO = "Menlo"`; 본문 15/25, UI 14, 작은 13, 메타 12 | `theme.rs:190,236` |
| iOS | 시스템(SF Pro) 추정 | 코드 검색에서 별도 글꼴 지정 없음(T4) |
| 안드로이드 | Material 기본(Roboto) 추정 | 코드 검색에서 별도 글꼴 지정 없음(T4) |
| 워드마크·아이콘 | Helvetica Neue Bold, 자간 −6px@150px(BRAND.md) / −0.06em(아이콘 HTML) | `BRAND.md`, `icon-mac.html:14` |
| 스토어 대표 이미지 | Helvetica 계열 + 한글은 시스템 한글 | `feature-graphic-ko` |

- **T1** 사이트(Inter)와 데스크톱·로고(Helvetica Neue)가 달라서, 로고 글자가 사이트 본문과 다른 서체다. Helvetica Neue는 Apple 기기 전용이고 웹·안드로이드엔 없다.
- **T2** 자간 값이 두 가지(−0.04em급 / −0.06em).
- **T3** 사이트 CSS는 ko·ja·zh-CN 폴백을 이미 정의했다(`global.css:197-204`: Apple SD Gothic Neo / Hiragino / PingFang SC 등 + `word-break: keep-all`, `line-break: strict`). **es·pt-BR·de·fr는 Inter가 그대로 담당** — 문제 없음. 비어 있는 것은 zh-Hant(사이트에 없음)와 앱(iOS·안드로이드) 쪽 폴백 규칙 문서.
- **T4** iOS(`.custom(`·`UIFont(name`)·안드로이드(`fontFamily`) 코드에서 별도 글꼴 지정은 검색에서 나오지 않았다 → 둘 다 시스템 글꼴로 보인다(코드 전체를 읽은 것은 아님).

## 4. 아이콘·일러스트·스크린샷

- 데스크톱: **Lucide** 아이콘을 GPUI Kit로 번들(`icons.rs:1`), 두 크기(`ICON`, `ICON_SM`), 글자색을 따른다.
- iOS: SF Symbols(`systemName` 21곳 — `ios/Packages/Sources`). 안드로이드: Material3 의존(아이콘 세트는 **미확인**).
- 사이트: 홈 일러스트는 파란 점·선 기반의 인라인 SVG(`.d-frame`, `.d-fill` 클래스 — `global.css:100-102`), 데스크톱 실화면 PNG 1장(`src/assets/desktop-tickets.png`) + "준비 중" 캡션이 붙은 폰 목업(`.phone`, `global.css:121`).
- 스토어 스크린샷(T-72): 현재 **자리표시 틀**(실제 앱 화면 없음, `audit-board.png` ⑧). 캡션은 한국어 해요체, 상단 `APPROVE`·`EQUIPE` 같은 영문 대문자 카피.
- BRAND.md의 동기("fashion house for software teams", 크롬 오브제·필름 그레인·갤러리 플래카드)는 **어느 제품 화면에도 쓰이지 않는다**(데모 소셜 카드·아바타만).
- **I1** 아이콘 선 굵기·모서리 규칙이 문서에 없다. **I2** 일러스트 규칙 없음. **I3** 스크린샷 프레임 규칙 없음(기기 프레임·배경색·캡션 위치는 T-72 `README.md` 안에만).

## 5. 목소리·문구

**영어(출처: `equipe-site/design/copy.md`)**: "plain, concrete, a little dry. No hype words". 이름은 항상 소문자. 이 규칙은 라이브 사이트와도 대체로 맞다.

**어긋남**
- **V1 메인 문구가 두 벌**: `copy.md` H1 = "Your AI software squad, led by you." / 라이브 사이트 H1 = "Lead a team of AI agents. You approve every merge." / `llms.txt` = "an open-source AI software squad…". 정본이 없다.
- **V2 '스쿼드' 사용**: ko 메타 제목 "한 사람이 이끄는 AI 코딩 에이전트 스쿼드", 스토어 "AI 스쿼드 리모컨"; 라이브 hero는 '스쿼드'를 쓰지 않는다(영어 hero에도 squad 없음). 용어가 곳마다 비중이 다르다.
- **V3 한국어 말투** — 문장 끝 맺음을 단순 정규식으로 센 값(대략치, 명사형·조각은 제외):

| 곳 | 문자열 수 | `~합니다/습니다` | `~요`(해요·하세요·까요) |
|---|---|---|---|
| 사이트 `ko.ts` | 143(6자↑) | 13 | **79** |
| 스토어 문구(listing.md) | — | 거의 없음 | 해요체(문서에 "사이트에 맞춤"이라 적힘) |
| 데스크톱 `ko.json` | 1330 | **235** | 182 |
| iOS `Localizable.xcstrings` | 202 | **45** | 22 |
| 안드로이드 `values-ko` | 197 | **43** | 25 |

  사이트·스토어는 해요체, 앱 화면은 합쇼체 쪽, 안드로이드 한 화면 안에서도 섞임(예: "스캔하세요" + "않습니다", `values-ko/strings.xml`). 집계 방법은 `audit.md` 부록 참고.
- **V4 2인칭**: 사이트 ko "당신" 사용(예: "당신 승인 없이는") — 앱 문자열은 대개 주어 생략. 영어는 "you/your".
- **V5 영문 대문자 카피**: 스토어 스크린샷 `APPROVE`, `EQUIPE`, 데스크톱 `⚑ 허락 요청` 같은 기호·대문자 장식이 몇 곳(BRAND.md의 "tracked caps"는 시각 언어일 뿐 문구 규칙이 아님).

## 6. 지원 언어

- **사이트 8개**: en, ko, ja, zh-CN, es, pt-BR, de, fr (`src/i18n/locales.ts`).
- **데스크톱 8개**: de, en, es, fr, ja, ko, zh-Hans, zh-Hant (`crates/desktop/locales/`).
- → **pt-BR은 사이트에만, zh-Hant는 데스크톱에만.** iOS·안드로이드 지원 언어: iOS `Localizable.xcstrings`·안드로이드 `values-ko`만 확인(en·ko). 나머지 **미확인**.
- 이 티켓의 "8개 언어 샘플"은 사이트 8개 + zh-Hant 1줄(데스크톱용)로 렌더했다(`type-8lang.png`).

## 7. 우선순위 (제안)

| 순위 | 항목 | 이유 |
|---|---|---|
| 1 | L1~L9 로고 마스터와 점(색·모양) 결정 | 사람이 매번 보는 첫인상. 앱 출시 전이라 바꾸는 비용이 가장 낮다(T-105) |
| 2 | C2 강조색 하나로, C1 검정 정리 | 토큰화(다음 티켓)의 전제 |
| 3 | T1 서체 규칙(웹 Inter, 앱 시스템, 로고는 윤곽 SVG) | 로고가 글꼴에 안 묶이게 |
| 4 | V1·V3 문구 정본 + 한국어 말투 하나 | 번역·스토어 문구가 같은 목소리 |
| 5 | 언어 목록 일치(pt-BR·zh-Hant) | 제품 결정 — 디자인 범위 밖, PM에게 넘김 |

## 부록 — 집계 방법과 한계

- 한국어 말투: `locales/ko.json`·`Localizable.xcstrings`·`values-ko/strings.xml`·`i18n/ko.ts`에서 문자열을 꺼내 끝이 `니다`인지 `요`인지 정규식으로 셌다. 번호·단어·변수만 있는 문자열은 둘 다 아니라서 합계가 문자열 수보다 작다. 번역 검수가 아니라 경향 파악용.
- 라이브 사이트 캡처: 1280×800, 영어 페이지. 한국어·모바일 폭 캡처는 안 했다.
- 데스크톱·앱 UI는 코드·저장소의 PNG로 확인했고, 실제 앱을 띄워 캡처하지는 않았다.
