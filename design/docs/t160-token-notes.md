# T-160 데스크톱 적용에서 드러난 토큰 제안 (원천 `tokens/`에서 고칠 것)

생성 파일은 고치지 않았고, 아래는 데스크톱에 **로컬 보정**으로 남긴 것이다.

1. **`color.err-bg`(라이트 12% 알파)** — 위험(ghost) 버튼을 카드·코드 바탕 위에서 hover하면 `err` 글자가 4.4:1로 AA(4.5)에 못 미친다(sunken이 `#ECEBE6`→`#EAE7E0`으로 어두워진 영향). 데스크톱은 hover 때만 알파를 ×0.65로 낮췄다(`ui.rs` `Kind::Danger`). 제안: 라이트 `err-bg` 알파를 ~8%로, 또는 `button.danger.bg-hover` 컴포넌트 토큰을 따로.
2. **다크 스위치 켬 색(`switch_on` `#4A76A0`)** — 토큰에 스위치 트랙이 없어 로컬로 남겼다. 제안: `switch.on/off/knob` 컴포넌트 토큰.
3. **사이드바·툴팁·popover 면**(`side`, `side_hover`, `side_sel`, `popover`, `tip_*`)과 `wash`(hover 면) — 토큰에 대응이 없어 로컬 유지. `color.hover/pressed`는 있으나 값이 다르다(다크 `0xEFEDE712` 등). 제안: `color.side*`·`color.popover`·`tooltip.*` 추가 또는 의도적으로 로컬임을 문서화.
4. **`prose`·`text2`**(읽기 글자·보조 글자 단계) — 토큰엔 `text`/`muted` 둘뿐. 제안: `color.text-soft`.
5. **멤버·프로젝트 색 8가지** — 토큰 없음(adoption.md §2 위험 ⑤대로 안 건드림).
6. **`ICON_SM` 14→16**(`SIZE_ICON_SM`)은 적용했고 스크린샷에서 사이드바 간격 문제는 없었다. 데스크톱 UI 대부분 아이콘은 16 — 토큰 이름이 `icon-sm`이라 `ICON`과 같은 값이 되어 둘이 겹친다. 제안: 데스크톱 14px 용도가 있으면 `size.icon-xs`.
7. Reduce Motion(`MOTION_DURATION_REDUCED_MS` 100ms 페이드)은 이번에 적용하지 않았다(지금은 끝 상태를 즉시) — 선택 사항이라 후속.
