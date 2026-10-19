# equipe 디자인 시스템

사이트·데스크톱·iOS·Android가 **한 원천**(`tokens/*.json`, DTCG)에서 나온 값을 쓴다. 이 폴더가 그 원천이고, equipe 데스크톱의 **디자인 시스템** 화면이 여기의 `tokens.json`·`guidelines.md`·`assets/`·`components/`를 읽는다(`docs/DESIGN-SYSTEM.md`). 토대는 T-144 브랜드 가이드라인 v1, 토큰은 T-145 디자인 시스템 v1, 폴더 구조는 T-149.

## 생성 파일은 손으로 고치지 않는다

| 손으로 고치는 원천 | 생성 파일 (`npm run build` 등이 만든다 — **손대지 않는다**) |
|---|---|
| `tokens/primitive.json` → `semantic.json`·`semantic.color.light.json`·`semantic.color.dark.json` → `component.json` | `tokens.json` (화면이 읽는 합친 파일, 라이트·다크) |
| | `build/css/tokens.css`·`compat-site.css` · `build/rust/tokens.rs` · `build/swift/EquipeTokens.swift` · `build/kotlin/EquipeTokens.kt` |
| `scripts/contrast.mjs`의 조합 목록 | `docs/contrast.md` |
| `scripts/sheets.mjs`·`reference/components.css` | `components/*-{light,dark}.png` (`npm run sheets`) |

값이나 이름을 바꾸려면 `tokens/`를 고치고 `npm run check`. 생성 파일의 형식(이름·단위)이 문제면 `scripts/build.mjs`를 고친다. 생성 파일만 고치면 다음 빌드에서 사라지고, 원천과 어긋난 값이 화면에 남는다. 다른 저장소(equipe-site·equipe-app)는 생성 파일을 복사해 쓰고 거기서도 고치지 않는다.

## 폴더

```
design/
  tokens.json        생성 — 데스크톱 디자인 시스템 화면이 읽는 DTCG 파일
  guidelines.md      규칙 문장 (T-144 브랜드 + 원칙) — 화면의 '가이드라인'
  CHANGELOG.md       토큰·컴포넌트·브랜드 변경 한 줄씩
  tokens/            원천 3층 (primitive / semantic(.light|.dark) / component)
  build/             생성 — 플랫폼별 코드 (css·rust·swift·kotlin), build/sheets/는 시트 HTML(커밋 안 함)
  components/        컴포넌트 시트 PNG (라이트·다크) + 규칙 components.md
  assets/logo/       mark.svg, wordmark-light.svg, wordmark-on-dark.svg
  brand/             브랜드 근거: brand.md(전략·목소리), audit.md, contrast.md, 비교·로고 사용 시트
  docs/              audit.md(T-145 감사), adoption.md(플랫폼 적용 계획), contrast.md(생성)
  reference/         웹 참조 구현 components.css
  scripts/           build · contrast · lint-css · sheets
```

## 재생성
```sh
cd design
npm ci               # style-dictionary ^4 (참조 해석에 사용; 출력은 scripts/build.mjs의 포맷터)
npm run build        # tokens/ → tokens.json + build/{css,rust,swift,kotlin}
npm run contrast     # 대비 계산 → docs/contrast.md (미달이 있으면 종료 코드 1)
npm run lint:css     # reference/components.css에 원시값이 없는지
npm run check        # 위 셋 한 번에
npm run sheets       # components/*.png 재렌더 (Chrome 필요, 글꼴은 Google Fonts·jsDelivr 네트워크)
```
`crates/core`의 테스트(`design::tokens`)가 `tokens.json`을 데몬과 같은 파서로 읽어 문제가 하나도 없는지, 라이트·다크가 다 있는지 확인한다.

### `tokens.json`이 만들어지는 방식
- 네 원천 파일을 라이트 기준으로 합치고, `{참조}`는 그대로 둔다(화면이 별칭을 보여 주고 직접 푼다).
- 다크 파일과 값이 다른 토큰만 `$extensions.mode.dark`에 다크 값을 넣는다. `$value`는 라이트.
- 플랫폼 차이는 `$extensions["im.equipe"].platform`에 남고, `$value`는 기본(웹) 값이다 — 화면은 기본 값을 보여 준다.
- 라이트와 다크의 키가 다르면 빌드가 실패한다.

### 플랫폼 출력 규칙
| | 색 | 길이 | 시간 | 글자 | 모드 |
|---|---|---|---|---|---|
| CSS | `#RRGGBB(AA)`, `--eq-*` | px | ms | `font` 약식 + `-tracking` | `:root`(라이트) · `prefers-color-scheme`+`[data-theme]`(다크) · 좁은 화면·동작 줄이기 `@media` |
| Rust | `0xRRGGBBAA` `u32`, `Palette` 구조체 `LIGHT`/`DARK` | `f32` 논리 px | `u64` ms | `TextStyle` | `palette(dark)` |
| Swift | `Color(0x…)`, `EquipeColors.light/.dark` | `CGFloat` pt | `TimeInterval` 초 | `EquipeTextStyle`(Dynamic Type) | `.resolved(colorScheme)` |
| Kotlin | `Color(0xAARRGGBB)`, `EquipeColors.Light/.Dark` | `Dp`·`sp` | `Int` ms | `EquipeTextStyle`·`Typography` | `equipeMaterialColors(dark)` |

생성 코드(`build/`)는 아직 어느 화면에도 붙지 않았다. 데스크톱 `crates/desktop/src/theme.rs`·`motion.rs`는 여전히 자기 값을 쓴다 — 적용 순서는 `docs/adoption.md`.

## 거버넌스

**버전** — `package.json`의 `version`과 `CHANGELOG.md`가 같은 번호.
- **패치(1.0.x)**: 값 미세 조정, 설명·문서, 대비를 *올리는* 변경.
- **마이너(1.x.0)**: 토큰·컴포넌트·상태 *추가*. 기존 이름은 그대로.
- **메이저(x.0.0)**: 이름 변경·삭제, 눈에 띄는 값 변경. 폐기 토큰은 한 마이너 동안 `$extensions["im.equipe"].deprecated`로 표시하고 다음 메이저에서 지운다.

**토큰을 바꾸는 절차**
1. 티켓에 쓴다: 무엇을·왜·"기존 토큰으로 안 되는 이유"(새 값일 때 필수). 의미 토큰 이름은 용도로 짓는다(`color.ok`, 아님 `color.green`).
2. `tokens/*.json`을 고치고 `npm run check`(대비가 깨지면 여기서 멈춤). 라이트·다크 둘 다.
3. `npm run sheets`로 시트를 다시 렌더해 전/후를 비교.
4. `CHANGELOG.md`에 한 줄(버전·날짜·이유·영향 플랫폼).
5. 생성 파일을 각 저장소·화면에 반영하는 코드 티켓을 플랫폼마다 만든다. **화면이 바뀌면 designer QA 필수.**

## 열린 것 · 미확인
1. `assets/logo/*.svg`는 T-144 §1 규격(Helvetica Neue Bold, 점 `#3F6FF5`)대로 그린 **글자형 SVG**다 — 글꼴이 없는 기기에선 Arial로 보인다. 윤곽선 마스터는 Helvetica Neue 재배포 라이선스 확인 뒤(T-144 §7-1).
2. 그 밖의 미확인은 T-145 그대로: iOS·Android 기본 글자 크기 값 재확인 안 함, 새로 정한 색(`elevation.overlay`·`inverse`·`*-bg` 틴트·`ok`)의 화면 취향 확인 전, Pretendard·Inter 라이선스 원문 미확인, 멤버 아바타 8색 대비 미계산.
