# T-617 · 문서 집중 보기 구현 계약

선택: B 복귀 우선. 기존 `Doc` surface와 `ScrollHandle`을 복제하지 않고 레이아웃 루트에서 같은 surface의 배치만 바꾼다.

## 상태와 전이

- `document.focus = off | on { origin, previous_layout, previous_focus }`.
- `origin`: `conversation { title }` 또는 `board { title, selected_ticket }`. 헤더 문구는 각각 `← {title} 대화로 돌아가기` / `← {title} 보드로 돌아가기`; 영어는 `← Back to {title} conversation` / `← Back to {title} board`. 목적지는 집중 보기 중 바뀌지 않는다. 일반 명칭인 `PM 대화`·`보드`만 쓰지 않는다.
- 진입: 기본 문서 헤더의 `집중 보기` / `Focus view` 버튼. 단축키 `⇧⌘F`. 문서 식별자, 스크롤 handle, 현재 선택, 분할 폭, 키보드 포커스를 저장하고 집중 상태로 전환한다. macOS 전체화면은 건드리지 않는다.
- 종료: 헤더의 복귀 버튼, `Esc`, `⇧⌘F`가 같은 toggle 명령을 호출한다. 분할 폭·선택 문서·스크롤 위치·선택 티켓과 진입 버튼의 키보드 포커스를 복원한다.
- `Esc` 우선순위: 모달 → 팝오버/더보기 → 문서 안 선택/검색 상태 → 집중 보기 → 기존 앱 계층. 한 번에 한 계층만 닫는다.
- 문서 닫기: 더보기 안의 일반 명령이다. 집중 중 닫으면 집중 상태도 해제하고, 원래 맥락의 문서를 열기 전 포커스로 돌아간다. 문서 전환은 새 문서 식별자로 집중을 유지하되 새 문서의 저장된 스크롤 또는 0에서 시작한다. 문서가 없어지면 집중 상태를 해제한다.

## 헤더

- 집중 상태는 항상 두 행이다. 1행 왼쪽은 복귀 버튼, 오른쪽은 문서 도구. 2행은 파일 아이콘, 전체 파일명, 짧은 폴더 경로다.
- 1행: `space.3` 인라인 여백, `size.row` 최소 높이. 복귀 버튼은 `type.small`, semibold, `color.text`; `Esc`는 `type.meta`, `color.muted`.
- 2행: `space.4` 인라인, `space.2` 블록 여백. 파일명은 `type.small` semibold, 경로는 `type.meta`/`color.muted`.
- 760pt 또는 150%에서는 새로 읽기·Finder에서 파일 열기·전체 경로 복사·문서 닫기를 `더보기` 하나로 접는다. 복귀 버튼과 더보기는 항상 첫 행에 남는다.
- 파일명은 2줄 제한이나 말줄임표를 쓰지 않는다. 한국어는 `word-break: keep-all` 뒤 긴 경로·영문에만 `overflow-wrap: anywhere`. 헤더 높이가 늘고 본문이 아래에서 스크롤된다. 전체 경로는 hover tooltip과 접근성 설명에도 제공한다.
- 본문은 `layout.measure`(680pt)를 최대 폭으로 가운데 두며 760pt에서 양쪽 `layout.gutter`를 유지한다. 150%에서도 gutter의 시각 크기는 확대하지 않는다. 문서 제목은 `type.headline`, 절 제목은 `type.title`, 본문은 `type.ui`로 폭과 확대 상태에 관계없이 위계를 유지한다. Markdown과 Mermaid는 기존 renderer를 그대로 쓴다. 카드형 Mermaid는 노드 최소 읽기 폭을 확보하도록 지원 범위인 1200pt 이하에서 노드와 화살표를 세로 흐름으로 바꾼다. viewport가 아니라 문서 surface 폭을 기준으로 한다. 노드 제목만 semibold이고 설명은 regular이며 한국어는 `word-break: keep-all`로 한 글자 고아행을 만들지 않는다.

## 기본 문서 헤더

- 파일명·경로와 도구를 두 행으로 허용한다. `집중 보기 ⇧⌘F`를 문서 닫기와 분리해 표시한다.
- 760pt/150%에서는 새로 읽기·열기·복사를 더보기로 접고 집중 보기와 문서 닫기만 남긴다. 두 행동은 각각 `Focus view`와 `Close document`로 이름이 다르다.

## 움직임

- 같은 surface의 폭·위치만 `motion.duration.slide` 180ms / `motion.easing.soft`로 바꾼다. fade·scale·장식 효과 없음.
- Reduce Motion에서는 즉시 재배치한다. 스크롤 위치는 애니메이션하지 않는다.

## 접근성

- 접근 가능한 이름: `{title} 대화로 돌아가기, Escape`, `{title} 보드로 돌아가기, Escape`, `집중 보기, Shift Command F`, `더 많은 문서 작업`, `문서 닫기`.
- 집중 진입 직후 포커스는 복귀 버튼에 둔다. 포커스 순서: 복귀 → 다시 읽기 → 파일 열기 → 더보기 → 문서 본문. 760pt/150%에서는 복귀 → 더보기 → 본문.
- 더보기는 `aria-expanded`; 열리면 헤더에만 `Esc 메뉴 닫기` / `Esc closes menu`를 표시하고 첫 메뉴 항목에 포커스를 둔다. 메뉴는 두 행짜리 헤더 아래에 열려 파일명을 가리지 않는다. `Esc`로 닫으면 더보기 버튼, 집중 종료 뒤에는 진입 버튼/원래 선택으로 포커스를 돌린다.
- 대비(기존 계산 `design/docs/contrast.md`): 본문/배경 16.57:1 light, 16.90:1 dark; muted/배경 7.25:1 light, 8.03:1 dark; focus/배경 6.14:1 light, 9.25:1 dark. 모두 WCAG AA 이상.

## 문구

| 상황 | 한국어 | English |
|---|---|---|
| 진입 | 집중 보기 | Focus view |
| 대화 복귀 | {대화 제목} 대화로 돌아가기 | Back to {conversation title} conversation |
| 보드 복귀 | {보드 제목} 보드로 돌아가기 | Back to {board title} board |
| 키 안내 | Esc | Esc |
| 메뉴 열린 키 안내 | Esc 메뉴 닫기 | Esc closes menu |
| 더보기 | 더 많은 문서 작업 | More document actions |
| 닫기 | 문서 닫기 | Close document |

## QA 계약

- 760/1200pt × light/dark × 100/150% × ko/en에서 파일명·복귀·더보기·본문 시작과 Mermaid가 가로로 잘리지 않는다.
- 대화 진입과 보드 진입을 각각 검증한다. 집중 전후 같은 문단, 분할 폭, 선택 문서/티켓, 포커스 대상을 비교한다.
- 더보기 열린 상태에서 첫 `Esc`는 메뉴만 닫고 더보기로 포커스를 돌리며, 두 번째 `Esc`가 집중 보기를 종료하고 원래 진입 버튼으로 포커스를 돌린다. UI map의 `focus-menu` → `focus` → `focus-restored`가 이 순서를 고정한다.
- 집중 중 문서 닫기·전환, macOS 전체화면 진입/해제 뒤 빈 surface나 고아 focus 상태가 없어야 한다.
