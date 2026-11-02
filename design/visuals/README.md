# 대화 속 그림 (T-336) — 도표 테마와 HTML 카드

대화에 나오는 그림이 equipe 화면과 같은 색·서체·간격으로 보이게 하는 값과 템플릿이다. 모든 값은 `tokens/`에서 나온다(`npm run build` 뒤 `build/css/tokens.css`·`scripts/lib.mjs`가 읽는다). 계약(언제 어떤 그림을 쓰나, 크기·안전)은 `docs/VISUALS.md`(T-335). 여기는 **모양**만 다룬다.

```
visuals/
  mermaid/   theme.{light,dark}.json · classdefs.{light,dark}.mmd · theme.rs (생성) · build.mjs · examples/
  cards/     card.mjs (데이터 → HTML) · card.css · examples/{board,compare,figure}.json
  preview/   모든 템플릿·도표의 PNG (라이트·다크 × 360·720)
  proposals/ 사람이 고르기 전 세 안(A 카드·B 레인·C 알약) — 고른 것은 A (taste.md)
  contrast.mjs → contrast.md   글자 4.5:1 · 그래픽 3:1 계산, 미달이면 종료 코드 1
```
다시 만들기: `node mermaid/build.mjs` · `node contrast.mjs`. 생성 파일(`theme.*`, `classdefs.*`, `theme.rs`, `contrast.md`)은 손으로 고치지 않는다.

## 1. Mermaid — 고른 모양: A 카드형

**원칙**: 노드는 둥근 카드(`color.surface` 면 + `color.border-strong` 1.5px 선), 선과 화살표는 `color.muted` 1.5px. 상태가 있는 노드는 상태 색 틴트 면 + 같은 색 테두리 + **라벨 앞 기호**(색만으로 말하지 않는다). 글자는 항상 `color.text`. 역할(backend 등)은 라벨 둘째 줄(`<br/>`). 한 줄 14px, 라벨 최대 30자에서 줄바꿈, 노드 간격 28 · 단계 간격 56 · 안쪽 여백 22×14.

| 상태 | 클래스 | 기호 | 면 · 테두리 토큰 |
|---|---|---|---|
| 다음(기본) | `next` (지정 안 해도 같은 모양) | ○ | `surface` · `border-strong` |
| 진행 중 | `work` | ● | `work-bg` · `work` |
| 사람 차례 | `wait` | ◆ | `wait-bg` · `wait` |
| 완료 | `ok` | ✓ | `ok-bg` · `ok` |
| 막힘 | `err` | ✕ | `err-bg` · `err` |

멤버가 쓰는 법: `T336("● T-336 도표 테마<br/>designer"):::work` 처럼 클래스 이름만 쓴다. **색 코드는 쓰지 않는다**(라이트·다크가 달라 멤버가 알 수 없다). 원그래프 조각은 `accent → ok → work → wait → err → border-strong` 순으로 칠하고 조각 글자는 라이트 흰색·다크 `bg`색이다(조각 색이 상태 색과 겹치니, 상태가 아닌 값에는 범례 글자로 뜻을 적는다).

### 데스크톱에 적용하는 법 (T-338 — `crates/desktop/src/render/raster.rs`의 `mermaid_svg`)
1. `design/visuals/mermaid/theme.rs`를 `crates/desktop/src/render/`에 **복사해 쓴다**(생성 파일 — 고치지 말고, 값이 바뀌면 디자인이 다시 만든다. 사본 어긋남 검사는 `design/copies.json`에 한 줄 추가).
2. `Theme`은 지금의 손으로 쓴 색 블록(97~111줄)을 `equipe_theme_dark()` / `equipe_theme_light()`로 바꾼다. 글꼴 한 줄(`"Helvetica Neue", "Apple SD Gothic Neo", sans-serif`)은 그대로 들어 있다.
3. `RenderOptions { theme, layout: equipe_layout(LayoutConfig::default()) }`.
4. 원문에 `classDef`가 **없으면** `CLASSDEFS_DARK` / `CLASSDEFS_LIGHT`를 원문 끝에 줄바꿈으로 이어 붙여 그린다(있으면 멤버가 직접 꾸민 것이니 그대로). 화면에 보이는 원문·복사에는 이어 붙이기 전의 원문을 쓴다.
5. 배경은 투명(`background: transparent`) 유지 — 블록 면은 UI가 칠한다.

### 도표 블록 모양 (코드 블록 머리 없애기)
- 지금의 `mermaid` 이름·`Source`·`Copy` 줄을 **없앤다**. 블록은 카드 한 장: 면 `color.surface`, 테두리 1px `color.line`, 모서리 `radius.card`(12), 안쪽 여백 위 `space.7`(48 — 도구줄 자리) · 옆·아래 `space.4`(16).
- 도표는 가운데 정렬, 블록 폭에 맞춰 줄이되 **키우지 않는다**(원래 크기 이상으로 늘리지 않음).
- **원문·복사**: 오른쪽 위 `space.2`(8) 자리에 작은 글자 버튼 둘(`type.meta`, 면 `color.sunken`, 글자 `color.muted`, 모서리 `radius.sm`, 높이 24 이상). 마우스를 올리거나 키보드 포커스가 들어오면 `motion.duration.fast`(120ms)에 나타나고 평소엔 숨는다. **터치·키보드만 쓰는 화면에서는 항상 보인다**(숨기면 못 연다). 누르면 원문 보기는 같은 블록 안에서 코드(`type.code`, 면 `color.sunken`)로 바뀌고 다시 누르면 도표로 돌아온다. 복사는 이어 붙이기 전의 원문을 복사한다.
- 그림 아래 대체 글(접근성): 도표 제목 한 줄을 `aria-label`로 단다. 제목이 없으면 "도표".
- 그리기에 실패하면 지금처럼 원문 코드 블록으로 보인다(머리 없이, 오류 한 줄 `color.err`).
- 모양 확인용 그림: `preview/mermaid-{flow,sequence,pie}-{360,720}.png` (라이트가 위, 다크가 아래).

### 렌더러의 한계 (`mermaid-rs-renderer` 0.3, 확인함)
- 한 노드 안에서 글자 두 가지 스타일(굵기·색)은 안 된다 → 위계는 면·테두리 색과 기호로 낸다.
- 시퀀스 도표의 화살표 색은 다크에서 테마 값보다 옅게 나온다(렌더러 고정). 읽을 수 있지만 3:1 확인은 못 했다(미확인).
- 위 그림은 Chrome으로 SVG를 그린 것이다. GPUI의 SVG 렌더러가 같은 글꼴 폭을 쓰는지는 데스크톱에서 직접 봐야 한다(미확인) → T-338 디자인 QA에서 확인.

## 2. HTML 카드 템플릿 3종

`node cards/card.mjs <board|compare|figure> <data.json> [--mode light|dark] > card.html` — 데이터만 채우면 **스크립트·외부 글꼴·네트워크 없는 HTML 한 파일**이 나온다. 글은 전부 이스케이프하고, 페이지에 `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; img-src data:`가 들어 있다. `--mode`를 안 주면 화면의 라이트·다크를 따른다. 폭은 부모 폭에 따라 바뀐다(560px 미만 = 폰 모양, 이상 = 대화 폭). 글꼴은 토큰의 스택(`Inter` → `Pretendard` → 시스템)이고 설치된 것만 쓴다 — 렌더하는 데몬 맥에 Pretendard가 없으면 한글은 시스템 한글 글꼴로 나온다(미확인).

- **board 현황 보드** `{title, subtitle?, asOf, source?, projects:[{name, work:[item], next:[item], wait:[item]}]}` · item `{id?, title, who?, note?}`. 프로젝트마다 진행 중 · 다음 · 사람 차례 세 줄. 비면 "없음". 사람 차례의 `note`에는 사람이 할 일을 한 문장으로.
- **compare 비교 표** `{title, subtitle?, options:[{name, pick?}], rows:[{label, values:[…]}], why?, asOf?}`. `pick`인 열은 틴트 면 + ★ 추천, `why`는 추천 이유 한 문장. 좁은 폭에서는 표가 항목별 묶음으로 바뀐다(값 앞에 옵션 이름).
- **figure 숫자 요약** `{title, asOf?, source?, stats:[{label, value, unit?, delta?, deltaTone?:'good'|'bad'|'flat', points?:[숫자], from?, to?}]}` 1~3개. 증감은 ▲ ▼ – 기호 + 글자(색만 쓰지 않음). `points`가 2개 이상이면 스파크라인(`color.accent` 2px, 값 지어내기 금지 — 실제 값만).
- 모든 카드는 `asOf`("11월 3일 기준")를 맨 아래에 단다. 숫자는 `tabular-nums`.
- 예시 데이터는 이 스쿼드의 2026-11-03 보드(진행 중·다음·사람 차례)와 `git log` main 병합 기록(10/28~11/3, 17건)이다. `compare` 예시는 T-335 계약 후보 비교 문장이다.
- PNG 확인: `equipe design render <html…> --widths 360,720` → `preview/{board,compare,figure}-{light,dark}-{360,720}.png`.

## 3. 대비 (계산, `contrast.md`)
카드와 Mermaid가 쓰는 조합 전부(라이트·다크 각 ~50쌍)를 계산했고 AA 미달 0건이다 — 글자 4.5:1, 선·테두리·조각 3:1.

## 4. 이 폴더가 하지 않는 것
- 새 토큰을 만들지 않았다(원그래프 색도 기존 상태·강조 토큰을 순서만 정해 씀).
- 데몬이 HTML을 PNG로 굽는 일은 T-337, 대화 안 표시는 T-338·T-339.
