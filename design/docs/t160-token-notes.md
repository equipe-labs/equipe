# T-160 데스크톱 적용에서 드러난 토큰 제안 (원천 `tokens/`에서 고칠 것)

생성 파일은 고치지 않았고, 아래는 데스크톱에 **로컬 보정**으로 남긴 것이다.

1. **`color.err-bg`(라이트 12% 알파)** — 위험(ghost) 버튼을 카드·코드 바탕 위에서 hover하면 `err` 글자가 4.4:1로 AA(4.5)에 못 미친다(sunken이 `#ECEBE6`→`#EAE7E0`으로 어두워진 영향). 데스크톱은 hover 때만 알파를 ×0.65로 낮췄다(`ui.rs` `Kind::Danger`). 제안: 라이트 `err-bg` 알파를 ~8%로, 또는 `button.danger.bg-hover` 컴포넌트 토큰을 따로.
2. **다크 스위치 켬 색(`switch_on` `#4A76A0`)** — 토큰에 스위치 트랙이 없어 로컬로 남겼다. 제안: `switch.on/off/knob` 컴포넌트 토큰.
3. **사이드바·툴팁·popover 면**(`side`, `side_hover`, `side_sel`, `popover`, `tip_*`)과 `wash`(hover 면) — 토큰에 대응이 없어 로컬 유지. `color.hover/pressed`는 있으나 값이 다르다(다크 `0xEFEDE712` 등). 제안: `color.side*`·`color.popover`·`tooltip.*` 추가 또는 의도적으로 로컬임을 문서화.
4. **`prose`·`text2`**(읽기 글자·보조 글자 단계) — 토큰엔 `text`/`muted` 둘뿐. 제안: `color.text-soft`.
5. **멤버·프로젝트 색 8가지** — 토큰 없음(adoption.md §2 위험 ⑤대로 안 건드림).
6. **`ICON_SM` 14→16**(`SIZE_ICON_SM`)은 적용했고 스크린샷에서 사이드바 간격 문제는 없었다. 데스크톱 UI 대부분 아이콘은 16 — 토큰 이름이 `icon-sm`이라 `ICON`과 같은 값이 되어 둘이 겹친다. 제안: 데스크톱 14px 용도가 있으면 `size.icon-xs`.
7. Reduce Motion(`MOTION_DURATION_REDUCED_MS` 100ms 페이드)은 이번에 적용하지 않았다(지금은 끝 상태를 즉시) — 선택 사항이라 후속.

## 2026-11-10 — main과 병합하며 (T-160 재작업)
- main의 데스크톱 팔레트는 T-160 이후 자체 개편됐다(따뜻한 중립 하나, 다크 bg `#151513`·accent `#7EB8F3`·orange `#FF7A45`, 라이트 bg `#FAF9F6`·accent `#2160AB` 등). `design/` 토큰(v1.1.0)은 그 값을 따라가지 않았다(다크 bg `#0A0A0C`, accent `#8FB0FF`). 그대로 토큰으로 채우면 main의 개편이 되돌려지므로 **팔레트는 main 값을 유지**하고, 간격·반지름·크기·글자 크기·줄 높이·모션·그림자만 토큰에서 가져온다.
- 사람이 고른 '다크 주 버튼 = bone'과 v1 파랑은 main의 새 팔레트가 이미 bone 계열 주 버튼(`#ECEAE4`)과 파랑 accent를 쓰므로 충족된 상태다. 값 자체가 v1 토큰과 같지는 않다.
- **토큰 쪽 제안(우선):** `design/tokens/semantic.color.*`를 main 데스크톱 팔레트에 맞게 갱신(또는 팔레트를 토큰에서 다시 만들도록 사람이 결정). 그러면 `theme.rs`의 `Palette`를 `tokens::palette()`로 채울 수 있다.
- `ICON_SM`은 main 화면이 14를 쓰므로 로컬 14로 유지(토큰 `SIZE_ICON_SM`은 16).
- 위험 버튼 hover 로컬 보정은 main의 `danger_bg`(8%)가 이미 해결해 필요 없어졌다.
