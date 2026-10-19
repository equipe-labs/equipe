# equipe 브랜드 전략 (한 장) — v1 제안

상태: **제안** — 사람이 확인하기 전 초안(playbook B2 ★). 사실은 출처를 달았고, 근거 없는 판단은 "제안"으로 표시했다.
사실 출처: `equipe-site/design/copy.md`(사이트 정본 카피), `equipe/docs/BRAND.md`, `equipe-app/docs/mvp-flows.md` §1, squad 메모리 `site-primary-cta-macos-app`·`design-system-and-brand`.

## 1. 포지셔닝

**누구를 위한 것**: Codex나 Claude Code를 이미 쓰고, 에이전트를 하나씩 시키는 대신 **작은 팀처럼 이끌고 싶은 한 사람**(개발자·창업자). 출처: copy.md "Who equipe is for".
**무엇을 대신해 주는가**: 역할별 에이전트(PM·디자이너·프런트·백엔드·모바일…)가 보드 위에서 병렬로 일하고, 각자 자기 브랜치에서 작업하며, **머지는 사람이 승인한 뒤에만** 된다.
**대안 대비**: 또 하나의 에이전트 루프가 아니라, 이미 있는 에이전트 엔진 위의 **팀 계층**이다(copy.md "Support line").

> **한 문장 (제안)**: equipe는 한 사람이 이끄는 AI 소프트웨어 스쿼드다 — 에이전트는 병렬로 일하고, 결정은 항상 당신 몫이다.
> EN: *equipe is an AI software squad led by one person — the agents work in parallel, the decisions stay yours.*

**표면별 역할 (사람 결정 2026-10-16 반영)**: 주 행동 = macOS 앱 다운로드, CLI 설치는 보조. 폰 앱은 *승인·검토용 리모컨*(앱스토어 문구 "내 컴퓨터의 AI 스쿼드를 폰에서 승인").

**하나 더 — 사이트 H1이 둘이다.** `copy.md`는 "Your AI software squad, led by you.", 라이브는 "Lead a team of AI agents. You approve every merge."(audit V1). 이 전략의 문장과 둘 다 일치하는 핵심은 *"이끈다(lead)"* 와 *"승인은 당신"*. 어느 쪽을 정본으로 할지는 사람 결정 → `guidelines.md` §5.3에 적어 둔다.

## 2. 성격 (제안 — 4개) 와 "아닌 것" 3개

| 성격 | 뜻 | 근거 |
|---|---|---|
| **차분한 (calm)** | 큰소리·과장·느낌표 없음. 일은 에이전트가 하고 사람은 조용히 결정한다 | copy.md "plain, concrete, a little dry. No hype words"; BRAND.md "silence" |
| **정확한 (precise)** | 구체적인 명사·숫자·명령어. "빠르다" 대신 무엇이 어떻게 | copy.md "Nothing below goes beyond [the README]" |
| **정돈된 (editorial)** | 여백과 위계, 장식 없음. 만든 이의 손이 느껴지는 절제 | BRAND.md "Editorial, restrained… lots of negative space" |
| **사람 편 (on your side)** | 권한은 항상 사람에게. 막히면 사람이 해야 할 일을 정확히 알려준다 | "Your approval" 섹션, 앱 `mvp-flows.md` §1 "Never act on stale or unclear information" |

**아닌 것**: ① 화려한·"혁신적인" (hype) ② 귀여운·마스코트·이모지 (BRAND.md "never cute… no emoji") ③ 차가운 기계 — 'AI가 알아서 다 한다'는 인상(권한을 가져가는 말투).
참고: BRAND.md는 "cold-luxury, a little uncanny"까지 말하는데, 제품 화면(사이트·앱)에서 실제로 쓰이는 건 차분함·정돈됨이다. '패션하우스' 무드는 소셜 카드·아바타 같은 *브랜드 표면*에만 남기고 UI에는 가져오지 않는 것을 제안한다.

## 3. 목소리·톤

**공통 규칙**: 짧게, 능동태, 구체적으로. 사람이 할 일을 먼저. 이름은 문장 안에서 항상 소문자 `equipe`(점 없음). 명령어·파일 경로는 코드 서식.

| 상황 | 톤 | EN | KO |
|---|---|---|---|
| 소개·히어로 | 담담하게 단언 | "Lead a team of AI agents. You approve every merge." | "AI 에이전트 팀을 이끌고, 머지는 당신이 승인해요." |
| 승인 요청 | 직접적, 무엇이 필요한지만 | "Allow editing files?" · "Deny" | "파일 수정을 허락할까요?" · "거절" |
| 사람이 막힌 일 | 해야 할 일 + 이유 | "frontend is waiting — once you send, it picks up where it left off." | "frontend가 기다리고 있어요. 보내면 하던 일을 이어가요." |
| 오류 | 사과 대신 사실 + 다음 행동 | "Your Mac is offline. Check that equipe is running." | "Mac이 오프라인이에요. equipe가 켜져 있는지 확인해 주세요." |
| 빈 상태 | 안내, 한 걸음 | "No squads yet. Create one on your Mac with `equipe`." | "아직 스쿼드가 없어요. Mac에서 `equipe`로 만들어 보세요." |
| 알림 | 누가·무엇이 필요한지만(명령·내용 없음 — T-62) | "backend needs you" | "backend가 당신을 기다려요" |
| 출시 소식 | 사실만, 날짜와 함께 | "v0.30 — Desktop app updates itself." | "v0.30 — 데스크톱 앱이 스스로 업데이트해요." |

예문 출처: "frontend is waiting…"은 데스크톱 화면(`desktop-tickets.png`)에서 가져왔고, 출시 소식의 버전 번호는 **예시**다. 영어 나머지는 `mvp-flows`·앱 영어 문자열·copy.md 표현을 따라 쓴 **제안 문장**(그대로 인용 아님). 안드로이드 영어 오류 문구 "Your Mac is offline. Check that equipe is running."은 `values/strings.xml`에서 그대로 가져왔다.

**하지 말 것 (Don't)**
- 과장어: *revolutionary, 10x, magic, supercharge / 혁신적인, 압도적인, 마법 같은*
- 느낌표, 이모지, ALL CAPS 장식 문구(`APPROVE` 같은 카피)
- 의인화·농담("Oops!", "앗, 문제가 생겼어요 😅")
- "AI가 알아서"처럼 사람의 결정을 가리는 표현 (권한은 사람에게)
- 모호한 약속("곧 만나요")과 없는 기능 홍보(스토어 규칙 2.3.1과도 충돌) — 준비 중은 "준비 중"이라고 쓴다
- 같은 개념에 다른 이름: squad(스쿼드), member(멤버), ticket(티켓), board(보드), approve(승인) — 용어는 `equipe-site/design/i18n/glossary.md`가 정본(8개 언어)

**한국어 말투 (제안)**: **해요체로 통일** — 사이트·스토어가 이미 해요체, 앱은 해요·합쇼 혼용(audit V3). 근거: "plain, a little dry"가 합쇼체의 딱딱함 없이도 지켜지고, 폰 화면의 짧은 문장에 맞음. 2인칭 "당신"은 홍보·소개 글에서만, 앱 UI는 주어 생략을 기본으로. 사람 결정 대기.

## 4. 시각 언어 한 줄 (자세한 건 `guidelines.md`)

**따뜻한 중립(bone·void) 위에 파란 점 하나.** 색은 거의 쓰지 않고, 쓰는 색은 의미(상태)와 단 하나의 브랜드 파랑뿐이다. 장식 대신 여백과 위계.
