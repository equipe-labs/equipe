# T-642 · 대화 이름 바꾸기 — 구현 계약

PM 대화 하나의 제목(자동 제목 T-578 포함)을 사용자가 바꾼다. 편집은 **헤더 제목 아래에 붙는 작은 카드 하나**에서만 하고, 진입 경로가 셋이어도 편집하는 곳은 같다. 대화 화면은 뒤에 그대로 보인다(모달 아님).

- 그림: UI map `conversation/rename` 12상태 × 760/1200 × 라이트/다크 (`design/flows/conversation/rename/`), 8개 언어 카드 `langs-sheet.png`.
- 원본: `prototype.html`(?state=…&lang=…&mode=…&w=…&zoom=…) 위에 실제 앱 스냅샷(`base/`, T-578 빌드)을 깔고 바뀌는 부분만 다시 그렸다. 다시 그리기: `node design/specs/rename-conversation/render.mjs`, `langs.mjs`.
- 목업 색·글자는 디자인 시스템 토큰의 가장 가까운 값이다. 구현은 `theme.rs` Palette와 기존 컴포넌트를 쓴다(아래 매핑).

## 고르지 않은 안

- 사이드바 행 안에서 바로 편집(ChatGPT·Claude.ai 방식): 사이드바가 접혀 있거나 760×600처럼 행이 화면 밖이면 편집할 곳이 없다. 150%에서 행 폭 244pt로는 오류 문구를 둘 자리가 없다.
- 헤더 제목 자리에서 바로 편집(Finder 방식): 1200에서는 가장 직접적이지만 760·150%에서는 헤더에 남는 폭이 120pt 안팎이라 입력·글자 수·오류·버튼을 둘 수 없다. 넓을 때와 좁을 때 두 가지 배치를 만들어야 한다.
- 고른 안: 헤더 제목에 붙는 카드. 폭과 상관없이 배치가 하나이고, 오류·재시도까지 같은 자리에서 끝난다.

## 진입

| 경로 | 포인터 | 키보드 |
|---|---|---|
| 헤더 제목 | 제목이 버튼이 된다. hover 시 `p.wash` 배경과 제목 뒤 연필(12pt, `p.muted`, 간격 8 — hover 때만 자리를 차지하고, 키보드 포커스에는 포커스 링만). 툴팁 `이름 바꾸기`. 제목이 잘렸으면 첫 줄에 전체 제목(좁아서 프로젝트를 접었으면 `프로젝트 / 제목`), 둘째 줄에 `이름 바꾸기` | Tab 순서: 프로젝트 → **제목** → ⋯. Return·Space로 연다 |
| ‘이 대화’ ⋯ | 메뉴 **첫 항목** `이름 바꾸기`(연필 아이콘), 그 아래 구분선, 기존 `다른 프로젝트로 옮기기`·프로젝트들·구분선·`대화 치우기` | 메뉴는 ↑↓로 움직이고 Return으로 고른다. 열면 첫 항목에 포커스 |
| 사이드바 대화 행 | hover 시 오른쪽에 연필·× 두 버튼(각 24pt, 간격 4). 연필 툴팁 `이름 바꾸기`. 누르면 그 대화를 열고 헤더 카드를 연다 | — (행에서 Return은 지금처럼 대화를 연다) |

- 전역 단축키는 두지 않는다(메뉴 항목에도 키 표시 없음).
- 사이드바 행의 제목이 잘렸으면, 행 hover(아이콘 밖) 툴팁에 전체 제목을 보인다(T-641 지적 2).
- 아직 시작하지 않은 대화(`draft_talk`)와 PM 자신의 대화(`with_pm`, 메뉴 없음)에는 진입점이 없다. ⋯ 메뉴가 있는 곳에만 있다. 읽기 전용 연결은 `대화 옮기기`와 같은 수준으로 처리한다(보낸 명령이 거절되면 아래 ‘실패’).

## 카드

- 위치: 헤더 제목 버튼 바로 아래 6pt, 왼쪽 끝은 **언제나 제목 버튼의 왼쪽 끝**(단 창 오른쪽에 240pt가 안 남으면 그만큼 왼쪽으로, 사이드바 경계 + 8pt보다 왼쪽으로는 가지 않음). 폭 `min(360, 창 오른쪽 끝 − 8 − 왼쪽 끝)`. 대화 영역 밖으로 나가지 않는다. `deferred` popover이고 모양은 기존 `talk-menu-list`와 같다: `p.raised`, 1px `p.hair`, `R_MD`(8), `shadow_lg`, 안쪽 12.
- 열려 있는 동안 헤더 제목 버튼은 `p.wash` 배경(무엇을 바꾸는지 표시).
- 위에서부터:
  1. 라벨 `대화 이름` (12pt, `p.muted`). 입력의 접근성 이름이기도 하다.
  2. 입력: 한 줄 `Input`, 높이 `FIELD`(36), `R_SM`, `p.code_bg`, 1px `p.hair`. 포커스면 테두리 `p.accent`, 오류면 `p.danger()`. 열 때 **현재 제목 전체가 선택**되고 커서는 끝에 있다. 30~40자일 때 입력 안 오른쪽에 글자 수 `n/40`(12pt, tabular, `p.muted`). 40자를 넘으면 글자 수는 메시지 줄 오른쪽으로 옮기고 `p.danger()`(입력 폭을 빼앗지 않게). 글이 입력보다 길어 왼쪽이 밀려나면 왼쪽 끝 8pt는 비우고 8→48pt에 걸쳐 페이드로 가린다(반쯤 잘린 글자를 깨진 글자로 보이지 않게).
  3. 메시지 줄: 있을 때만 생긴다(카드가 아래로 한 줄 길어지고 위는 움직이지 않는다). 12pt `p.danger()` + `CircleAlert` 13pt.
  4. 버튼(오른쪽 정렬): `취소 Esc`(Secondary) · `저장 ↩`(Primary, `data-primary`). 저장할 수 없는 값이면 Primary는 비활성 모양.
- 카드 폭이 300pt보다 좁으면(760·150%): 키 힌트(`Esc`·`↩`)를 빼서 두 버튼이 한 줄에 남게 하고, 글자 수는 메시지 줄 오른쪽에 둔다. 키 동작과 접근성 이름은 그대로. 그래도 한 줄에 안 들어가면(de 등) 두 버튼을 카드 폭으로 세로로 쌓고 **주 버튼을 위**에 둔다.
- 키 힌트 색: Secondary 안은 `p.muted`(흰 카드 위 6.29:1, dark 6.37:1), Primary 안은 `p.primary_key()`.
- 카드가 열려 있는 동안 좁은 헤더(760·150%)에서는 프로젝트 표시와 `/`를 먼저 숨겨 제목에 자리를 준다.

## 값과 검사

- 최대 **40자**(T-578 자동 제목 상한과 같다). 글자 수는 grapheme 단위.
- 저장 직전에 줄바꿈·탭은 공백으로, 연속 공백은 하나로, 앞뒤 공백은 지운다. 화면의 입력값은 바꾸지 않는다.
- 비었거나 공백뿐: 지우는 동안은 저장 비활성만. Return·바깥 클릭으로 저장하려 할 때 메시지 `이름을 입력해 주세요.`와 입력 테두리 오류색을 보인다(UI map `empty`는 Return 뒤).
- 40자 초과(타이핑이든 붙여넣기든): **자르지 않는다.** 41번째 글자부터 `p.danger_bg()` 배경 + `p.danger()` 글자로 표시, 메시지 `40자까지 쓸 수 있어요. {n}자를 줄여 주세요.`, 저장 비활성.
- 정리한 값이 지금 제목과 같으면 저장은 명령 없이 카드만 닫는다.
- 붙여넣기는 여러 줄도 받는다(위 규칙으로 한 줄이 됨).

## 키와 포커스

- 카드가 열리면 포커스는 입력. Tab 순서: 입력 → 취소 → 저장 → 입력(카드 안에서 돈다).
- Return(또는 ⌘Return): 저장. 저장할 수 없는 값이면 아무것도 보내지 않고 메시지를 보인다. 카드는 열린 채.
- Esc: 취소 — 카드를 닫고 원래 제목 그대로. 포커스는 카드를 연 곳으로 돌아간다: 헤더 제목·⋯ 메뉴에서 열었으면 **헤더 제목 버튼**, 사이드바 연필에서 열었으면 헤더 제목 버튼(그 대화가 열려 있으므로).
- **한글 IME**: 조합 중(marked text가 있을 때) Return은 조합 확정만, Esc는 조합 취소만 한다. 저장·취소는 조합이 없을 때만. 조합 중 글자도 글자 수에 넣어 보이지만, 검사는 확정된 값으로 한다.
- **바깥 클릭 = Return**: 바뀐 게 없으면 닫는다. 바뀌었고 저장할 수 있으면 저장한다. 저장할 수 없으면 카드는 열린 채 이유를 보인다(입력을 버리지 않는다).
- 다른 대화·화면으로 옮기면(사이드바 클릭, ⌘N, 뒤로 등) 바깥 클릭과 같다. 단 저장할 수 없는 값이면 그 초안은 버리고 원래 제목을 둔다(헤더가 다른 대화로 바뀌기 때문).
- 카드가 열린 동안 ⋯ 메뉴·멤버 popover는 열리지 않는다(popover는 한 번에 하나, 기존 `board.popover` 규칙).

## 저장·성공·실패

- 저장 순서: 정리한 값으로 `RenameConversation { session, title }` → **곧바로** 헤더·사이드바·보관 목록이 새 제목을 보인다(낙관적 표시) → 카드 닫힘 → 포커스는 헤더 제목 버튼(포커스 링이 보임, `saved` 상태).
- 데몬 확인 이벤트가 오면 그대로 둔다. 다른 화면(다른 창, 폰)도 같은 이벤트로 바뀐다.
- 실패(명령 거절, 연결 끊김, 없는 세션): 헤더·사이드바를 **원래 제목으로 되돌리고**, 카드를 다시 열어 사용자가 쳤던 값을 그대로 둔 채(선택 없이, 커서는 끝) 메시지를 보인다. Primary 버튼 문구는 `다시 시도 ↩`. 글자 수는 길이와 상관없이 숨긴다(메시지에 폭을 준다).
  - 연결이 끊겼을 때: `Mac과 연결이 끊겨 저장하지 못했어요. 입력한 이름은 그대로 있어요.`
  - 그 밖: `이름을 저장하지 못했어요. 입력한 이름은 그대로 있어요.`
- 실패 상태에서 Esc는 위 취소와 같다(원래 제목 유지).

## 헤더가 좁을 때 (T-651과 함께)

제목이 먼저 자리를 갖는다. 제목이 잘리는 동안 순서대로 양보한다: ① `컨텍스트 31%` 숨김(잘린 글자로 보이지 않음) → ② 프로젝트 이름을 그 표시(프로젝트 색 표시, 프로젝트 밖이면 빈 원)로 접고 이름은 툴팁으로 → ③ 그래도 넘치면(760·150%) 프로젝트 표시와 `/`도 숨김. 제목은 최소 96pt를 지키고, ⋯·터미널·브라우저 버튼은 항상 남는다. 이 순서는 T-651(150%·760 상단 바 겹침)의 수정 기준이기도 하다.

## 접근성

- 헤더 제목: role button, 이름 = 제목, 설명 = `이름 바꾸기`, `aria-expanded` = 카드 열림.
- 카드: role dialog, 이름 `대화 이름 바꾸기`. 입력의 이름 `대화 이름`. 오류 메시지는 입력의 설명으로 연결하고, 생길 때 알린다(실패는 role alert).
- 저장 성공 시 알림: `이름을 바꿨어요: {제목}`. 취소는 알리지 않는다.
- 사이드바 연필·×: 이름 `이름 바꾸기` / `대화 치우기`, 24pt 이상. 아이콘만 있는 버튼은 툴팁으로 이름을 보인다.
- 대비(계산값, WCAG 2 상대 휘도): 오류 글자 `red`/`raised` light 6.84:1, dark 6.68:1 · 라벨·글자 수 `muted`/`raised` 6.29:1, 6.37:1 · 입력 안 글자 수 `muted`/`code_bg` 5.56:1, 6.65:1. 모두 AA(4.5:1) 이상. 목업의 토큰 값도 오류 6.57:1 / 6.26:1.
- Reduce Motion: 카드는 움직임 없이 나타난다(기존 popover와 같음).

## 문구 (새 키 `talk.rename_*`, 기존 키는 그대로)

| 키 | ko | en | ja | zh-Hans | zh-Hant | de | fr | es |
|---|---|---|---|---|---|---|---|---|
| rename (메뉴·툴팁·설명) | 이름 바꾸기 | Rename | 名前を変更 | 重命名 | 重新命名 | Umbenennen | Renommer | Cambiar nombre |
| rename_dialog | 대화 이름 바꾸기 | Rename conversation | 会話の名前を変更 | 重命名对话 | 重新命名對話 | Gespräch umbenennen | Renommer la conversation | Cambiar nombre de la conversación |
| rename_label | 대화 이름 | Conversation name | 会話の名前 | 对话名称 | 對話名稱 | Name des Gesprächs | Nom de la conversation | Nombre de la conversación |
| 취소 / 저장 | `peek.cancel` / `board.save` | | | | | | | |
| rename_retry | 다시 시도 | Try again | 再試行 | 重试 | 重試 | Erneut versuchen | Réessayer | Reintentar |
| rename_empty | 이름을 입력해 주세요. | Enter a name. | 名前を入力してください。 | 请输入名称。 | 請輸入名稱。 | Gib einen Namen ein. | Saisissez un nom. | Escribe un nombre. |
| rename_long {max}{over} | {max}자까지 쓸 수 있어요. {over}자를 줄여 주세요. | Up to {max} characters. Remove {over}. | {max}文字までです。あと{over}文字減らしてください。 | 最多 {max} 个字符，请删去 {over} 个。 | 最多 {max} 個字元，請刪去 {over} 個。 | Höchstens {max} Zeichen. Entferne {over}. | {max} caractères au maximum. Retirez-en {over}. | Máximo {max} caracteres. Quita {over}. |
| rename_offline | Mac과 연결이 끊겨 저장하지 못했어요. 입력한 이름은 그대로 있어요. | Lost the connection to your Mac, so the name wasn’t saved. What you typed is kept. | Macとの接続が切れたため保存できませんでした。入力した名前はそのままです。 | 与 Mac 的连接已断开，未能保存名称。已保留你输入的内容。 | 與 Mac 的連線已中斷，未能儲存名稱。已保留你輸入的內容。 | Keine Verbindung zum Mac – Name nicht gesichert. Deine Eingabe bleibt erhalten. | Connexion au Mac perdue : nom non enregistré. Votre saisie est conservée. | Se perdió la conexión con el Mac y no se guardó el nombre. Lo que escribiste se conserva. |
| rename_failed | 이름을 저장하지 못했어요. 입력한 이름은 그대로 있어요. | Couldn’t save the name. What you typed is kept. | 名前を保存できませんでした。入力した名前はそのままです。 | 未能保存名称，已保留你输入的内容。 | 未能儲存名稱，已保留你輸入的內容。 | Name nicht gesichert. Deine Eingabe bleibt erhalten. | Nom non enregistré. Votre saisie est conservée. | No se guardó el nombre. Lo que escribiste se conserva. |
| renamed {title} (알림) | 이름을 바꿨어요: {title} | Renamed: {title} | 名前を変更しました: {title} | 已重命名：{title} | 已重新命名：{title} | Umbenannt: {title} | Renommée : {title} | Nombre cambiado: {title} |
| rename_count {n}{max} | {n}/{max} | | | | | | | |

- ja 메시지는 구절 단위로 끊는다(`保存で / きませんでした` 같은 낱말 중간 끊김 금지). GPUI 줄바꿈이 구절을 모르면 문장 끝(。) 뒤에서만 끊기도록 문장을 나눠 그린다 — GPUI에서 되는지는 미확인.
- es: 기존 `sidebar.put_away`가 `Guardar`(=저장)와 같은 낱말이라 같은 메뉴·카드 근처에서 헷갈린다. 이 티켓 범위 밖이라 바꾸지 않았다 — 구현 때 `Quitar`/`Archivar` 같은 다른 낱말 검토를 권한다(미확정).

## UI map 상태 (`conversation/rename`)

| 상태 | kind | 무엇 |
|---|---|---|
| entry-title | default | 헤더 제목 hover: 연필 + 툴팁 (연필은 보일 때만 자리를 차지한다) |
| entry-menu | default | ⋯ 메뉴, 첫 항목 `이름 바꾸기`에 포커스 |
| entry-sidebar | default | 사이드바 행 hover: 연필·×, 연필 툴팁 |
| editing | default | 카드 열림, 제목 전체 선택 |
| composing | default | 한글 조합 중(`카` 확정 + `피` 조합) — Return은 확정만 |
| empty | error | 빈 값으로 Return 뒤: 메시지, 저장 비활성 |
| too-long | error | 붙여넣은 48자: 넘친 8자 표시, 저장 비활성 |
| saved | success | 헤더·사이드바 같은 새 제목, 포커스는 헤더 제목 |
| failed | error | 연결 끊김: 원래 제목, 입력값 유지, `다시 시도` |
| long | long | 독일어 긴 제목: 헤더 양보 순서, 사이드바 잘린 행의 전체 제목 툴팁 |
| large | default | 150%: 760에서 프로젝트 표시까지 양보, 카드가 대화 영역 안 |
| large-failed | error | 150% 영어 실패(연결 끊김): 좁은 카드, 키 힌트 없이 한 줄 버튼 |

760은 760×760 창이다(600 높이에서는 사이드바 대화 행이 화면 밖이라 TP-1의 ‘같은 제목’을 볼 수 없다).

## QA 계약 (T-644)

- TP-1: entry-title → editing → saved에서 헤더·사이드바 제목이 같은 프레임에 바뀐다. 다른 창의 같은 대화도 이벤트로 바뀐다.
- TP-2: 마우스 없이 Tab → 제목 → Return → 입력 → Return 저장 / Esc 취소, 포커스가 헤더 제목으로 돌아온다. VoiceOver가 `대화 이름 바꾸기` 대화상자, `대화 이름` 입력, 오류 문구를 읽는다.
- TP-3: 빈 값·공백뿐·41자 이상·여러 줄 붙여넣기·한글 조합 중 Return/Esc에서 제목도 입력도 잃지 않는다.
- TP-5: 데몬을 멈추고 저장 → 원래 제목으로 돌아오고 카드에 입력값과 `failed` 문구가 남는다. 다시 켜고 `다시 시도`로 저장된다.
- TP-6: 8개 언어 × 760/1200 × 라이트/다크 × 100/150%에서 헤더 양보 순서대로 줄고 ⋯가 사라지지 않으며, 카드가 대화 영역을 넘지 않는다.

## 구현 메모

- 편집 상태는 하나(`Shell.rename: Option<{ session, input, origin, error }>`). 사이드바 연필은 대화를 열고 같은 상태를 연다.
- 낙관적 표시는 `Lane.title`을 바꾸고 이전 값을 들고 있다가 실패 때 되돌린다. 자동 제목(`set_title_if_empty`)은 수동 제목을 덮지 않는다(계획서).
- 목업 토큰 → 앱 값: 표면 `--eq-color-raised`→`p.raised`, 선 `--eq-color-line`→`p.hair`, hover `--eq-color-hover`→`p.wash`, 포커스 `--eq-color-focus`→`p.accent`, 오류 `--eq-color-err(-bg)`→`p.danger()`/`p.danger_bg()`, 입력 바탕 `--eq-color-bg`→`p.code_bg`, Primary `--eq-color-action`→`Kind::Primary`, 툴팁 `--eq-color-inverse`→`p.tip_bg()`. 글자 14→`UI`/`SMALL`(헤더 제목은 지금처럼 14 semibold), 12→`SMALL − 1`.
