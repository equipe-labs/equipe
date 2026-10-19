# 대비 수치 — 세 팔레트 전 조합

계산: `src/palettes.py`(WCAG 2.x 상대 휘도 공식, sRGB). 기준: 글자 4.5:1, 그래픽(포커스 링·브랜드 점) 3:1. 이 표는 `python3 src/palettes.py`/`gen_docs.py`로 다시 만들 수 있다.

## A. Signal blue — 따뜻한 중립 + 파란 한 점

| 모드 | 전경 | 배경 | 대비 | 기준 | 판정 | 용도 |
|---|---|---|---|---|---|---|
| light | `text` #15161A | `bg` #F6F5F1 | 16.57:1 | 4.5:1 | ✅ | 본문 글자 |
| light | `text` #15161A | `surface` #FFFFFF | 18.08:1 | 4.5:1 | ✅ | 카드 위 글자 |
| light | `text` #15161A | `sunken` #EAE7E0 | 14.64:1 | 4.5:1 | ✅ | 코드·눌린 면 위 글자 |
| light | `muted` #4F5159 | `bg` #F6F5F1 | 7.25:1 | 4.5:1 | ✅ | 보조 글자 |
| light | `muted` #4F5159 | `surface` #FFFFFF | 7.91:1 | 4.5:1 | ✅ | 카드 위 보조 글자 |
| light | `muted` #4F5159 | `sunken` #EAE7E0 | 6.41:1 | 4.5:1 | ✅ | 눌린 면 위 보조 글자 |
| light | `accent` #1D4ED8 | `bg` #F6F5F1 | 6.14:1 | 4.5:1 | ✅ | 링크·텍스트 버튼 |
| light | `accent` #1D4ED8 | `surface` #FFFFFF | 6.70:1 | 4.5:1 | ✅ | 카드 위 링크 |
| light | `onAccent` #FFFFFF | `accent` #1D4ED8 | 6.70:1 | 4.5:1 | ✅ | 채운 버튼 글자 |
| light | `focus` #1D4ED8 | `bg` #F6F5F1 | 6.14:1 | 3.0:1 | ✅ | 포커스 링(그래픽) |
| light | `dot` #3F6FF5 | `bg` #F6F5F1 | 4.01:1 | 3.0:1 | ✅ | 브랜드 점 on 바탕(그래픽) |
| light | `ok` #1F6B33 | `bg` #F6F5F1 | 6.00:1 | 4.5:1 | ✅ | 완료·성공 글자 |
| light | `work` #825600 | `bg` #F6F5F1 | 5.87:1 | 4.5:1 | ✅ | 진행 중 글자 |
| light | `wait` #A04610 | `bg` #F6F5F1 | 5.70:1 | 4.5:1 | ✅ | 내 차례·허락 대기 글자 |
| light | `err` #AD2E27 | `bg` #F6F5F1 | 6.02:1 | 4.5:1 | ✅ | 오류 글자 |
| light | `ok` #1F6B33 | `surface` #FFFFFF | 6.54:1 | 4.5:1 | ✅ | 카드 위 성공 |
| light | `err` #AD2E27 | `surface` #FFFFFF | 6.57:1 | 4.5:1 | ✅ | 카드 위 오류 |
| light | `wait` #A04610 | `surface` #FFFFFF | 6.22:1 | 4.5:1 | ✅ | 카드 위 대기 |
| light | `work` #825600 | `surface` #FFFFFF | 6.40:1 | 4.5:1 | ✅ | 카드 위 진행 |
| light | `dot` #3F6FF5 | `아이콘 타일 void` #0A0A0C | 4.52:1 | 3.0:1 | ✅ | 앱 아이콘의 점(그래픽) |
| dark | `text` #EFEDE7 | `bg` #0A0A0C | 16.90:1 | 4.5:1 | ✅ | 본문 글자 |
| dark | `text` #EFEDE7 | `surface` #16171A | 15.31:1 | 4.5:1 | ✅ | 카드 위 글자 |
| dark | `text` #EFEDE7 | `sunken` #111114 | 16.10:1 | 4.5:1 | ✅ | 코드·눌린 면 위 글자 |
| dark | `muted` #A3A5AB | `bg` #0A0A0C | 8.03:1 | 4.5:1 | ✅ | 보조 글자 |
| dark | `muted` #A3A5AB | `surface` #16171A | 7.28:1 | 4.5:1 | ✅ | 카드 위 보조 글자 |
| dark | `muted` #A3A5AB | `sunken` #111114 | 7.65:1 | 4.5:1 | ✅ | 눌린 면 위 보조 글자 |
| dark | `accent` #8FB0FF | `bg` #0A0A0C | 9.25:1 | 4.5:1 | ✅ | 링크·텍스트 버튼 |
| dark | `accent` #8FB0FF | `surface` #16171A | 8.38:1 | 4.5:1 | ✅ | 카드 위 링크 |
| dark | `onAccent` #0B1226 | `accent` #8FB0FF | 8.70:1 | 4.5:1 | ✅ | 채운 버튼 글자 |
| dark | `focus` #8FB0FF | `bg` #0A0A0C | 9.25:1 | 3.0:1 | ✅ | 포커스 링(그래픽) |
| dark | `dot` #3F6FF5 | `bg` #0A0A0C | 4.52:1 | 3.0:1 | ✅ | 브랜드 점 on 바탕(그래픽) |
| dark | `ok` #7FCB8E | `bg` #0A0A0C | 10.21:1 | 4.5:1 | ✅ | 완료·성공 글자 |
| dark | `work` #E2B865 | `bg` #0A0A0C | 10.62:1 | 4.5:1 | ✅ | 진행 중 글자 |
| dark | `wait` #EC9A62 | `bg` #0A0A0C | 8.82:1 | 4.5:1 | ✅ | 내 차례·허락 대기 글자 |
| dark | `err` #EE7B74 | `bg` #0A0A0C | 7.27:1 | 4.5:1 | ✅ | 오류 글자 |
| dark | `ok` #7FCB8E | `surface` #16171A | 9.25:1 | 4.5:1 | ✅ | 카드 위 성공 |
| dark | `err` #EE7B74 | `surface` #16171A | 6.59:1 | 4.5:1 | ✅ | 카드 위 오류 |
| dark | `wait` #EC9A62 | `surface` #16171A | 7.99:1 | 4.5:1 | ✅ | 카드 위 대기 |
| dark | `work` #E2B865 | `surface` #16171A | 9.63:1 | 4.5:1 | ✅ | 카드 위 진행 |
| dark | `dot` #3F6FF5 | `아이콘 타일 void` #0A0A0C | 4.52:1 | 3.0:1 | ✅ | 앱 아이콘의 점(그래픽) |

미달 조합: 없음

## B. Steel chrome — 크롬 회색 + 차가운 강철 파랑

| 모드 | 전경 | 배경 | 대비 | 기준 | 판정 | 용도 |
|---|---|---|---|---|---|---|
| light | `text` #15161A | `bg` #F6F5F1 | 16.57:1 | 4.5:1 | ✅ | 본문 글자 |
| light | `text` #15161A | `surface` #FFFFFF | 18.08:1 | 4.5:1 | ✅ | 카드 위 글자 |
| light | `text` #15161A | `sunken` #ECEBE6 | 15.15:1 | 4.5:1 | ✅ | 코드·눌린 면 위 글자 |
| light | `muted` #5E6068 | `bg` #F6F5F1 | 5.75:1 | 4.5:1 | ✅ | 보조 글자 |
| light | `muted` #5E6068 | `surface` #FFFFFF | 6.27:1 | 4.5:1 | ✅ | 카드 위 보조 글자 |
| light | `muted` #5E6068 | `sunken` #ECEBE6 | 5.25:1 | 4.5:1 | ✅ | 눌린 면 위 보조 글자 |
| light | `accent` #2E5C8A | `bg` #F6F5F1 | 6.39:1 | 4.5:1 | ✅ | 링크·텍스트 버튼 |
| light | `accent` #2E5C8A | `surface` #FFFFFF | 6.97:1 | 4.5:1 | ✅ | 카드 위 링크 |
| light | `onAccent` #F6F5F1 | `accent` #2E5C8A | 6.39:1 | 4.5:1 | ✅ | 채운 버튼 글자 |
| light | `focus` #2E5C8A | `bg` #F6F5F1 | 6.39:1 | 3.0:1 | ✅ | 포커스 링(그래픽) |
| light | `dot` #7F8591 | `bg` #F6F5F1 | 3.40:1 | 3.0:1 | ✅ | 브랜드 점 on 바탕(그래픽) |
| light | `ok` #1F6B33 | `bg` #F6F5F1 | 6.00:1 | 4.5:1 | ✅ | 완료·성공 글자 |
| light | `work` #825600 | `bg` #F6F5F1 | 5.87:1 | 4.5:1 | ✅ | 진행 중 글자 |
| light | `wait` #A04610 | `bg` #F6F5F1 | 5.70:1 | 4.5:1 | ✅ | 내 차례·허락 대기 글자 |
| light | `err` #AD2E27 | `bg` #F6F5F1 | 6.02:1 | 4.5:1 | ✅ | 오류 글자 |
| light | `ok` #1F6B33 | `surface` #FFFFFF | 6.54:1 | 4.5:1 | ✅ | 카드 위 성공 |
| light | `err` #AD2E27 | `surface` #FFFFFF | 6.57:1 | 4.5:1 | ✅ | 카드 위 오류 |
| light | `wait` #A04610 | `surface` #FFFFFF | 6.22:1 | 4.5:1 | ✅ | 카드 위 대기 |
| light | `work` #825600 | `surface` #FFFFFF | 6.40:1 | 4.5:1 | ✅ | 카드 위 진행 |
| light | `dot` #7F8591 | `아이콘 타일 void` #0A0A0C | 5.34:1 | 3.0:1 | ✅ | 앱 아이콘의 점(그래픽) |
| dark | `text` #EFEDE7 | `bg` #0A0A0C | 16.90:1 | 4.5:1 | ✅ | 본문 글자 |
| dark | `text` #EFEDE7 | `surface` #16171A | 15.31:1 | 4.5:1 | ✅ | 카드 위 글자 |
| dark | `text` #EFEDE7 | `sunken` #111114 | 16.10:1 | 4.5:1 | ✅ | 코드·눌린 면 위 글자 |
| dark | `muted` #8A8C93 | `bg` #0A0A0C | 5.89:1 | 4.5:1 | ✅ | 보조 글자 |
| dark | `muted` #8A8C93 | `surface` #16171A | 5.34:1 | 4.5:1 | ✅ | 카드 위 보조 글자 |
| dark | `muted` #8A8C93 | `sunken` #111114 | 5.61:1 | 4.5:1 | ✅ | 눌린 면 위 보조 글자 |
| dark | `accent` #B4CDE6 | `bg` #0A0A0C | 12.07:1 | 4.5:1 | ✅ | 링크·텍스트 버튼 |
| dark | `accent` #B4CDE6 | `surface` #16171A | 10.94:1 | 4.5:1 | ✅ | 카드 위 링크 |
| dark | `onAccent` #0A0A0C | `accent` #B4CDE6 | 12.07:1 | 4.5:1 | ✅ | 채운 버튼 글자 |
| dark | `focus` #B4CDE6 | `bg` #0A0A0C | 12.07:1 | 3.0:1 | ✅ | 포커스 링(그래픽) |
| dark | `dot` #8A909B | `bg` #0A0A0C | 6.16:1 | 3.0:1 | ✅ | 브랜드 점 on 바탕(그래픽) |
| dark | `ok` #7FCB8E | `bg` #0A0A0C | 10.21:1 | 4.5:1 | ✅ | 완료·성공 글자 |
| dark | `work` #E2B865 | `bg` #0A0A0C | 10.62:1 | 4.5:1 | ✅ | 진행 중 글자 |
| dark | `wait` #EC9A62 | `bg` #0A0A0C | 8.82:1 | 4.5:1 | ✅ | 내 차례·허락 대기 글자 |
| dark | `err` #EE7B74 | `bg` #0A0A0C | 7.27:1 | 4.5:1 | ✅ | 오류 글자 |
| dark | `ok` #7FCB8E | `surface` #16171A | 9.25:1 | 4.5:1 | ✅ | 카드 위 성공 |
| dark | `err` #EE7B74 | `surface` #16171A | 6.59:1 | 4.5:1 | ✅ | 카드 위 오류 |
| dark | `wait` #EC9A62 | `surface` #16171A | 7.99:1 | 4.5:1 | ✅ | 카드 위 대기 |
| dark | `work` #E2B865 | `surface` #16171A | 9.63:1 | 4.5:1 | ✅ | 카드 위 진행 |
| dark | `dot` #8A909B | `아이콘 타일 void` #0A0A0C | 6.16:1 | 3.0:1 | ✅ | 앱 아이콘의 점(그래픽) |

미달 조합: 없음

## C. Mono ink — 색 없음, 잉크가 곧 강조

| 모드 | 전경 | 배경 | 대비 | 기준 | 판정 | 용도 |
|---|---|---|---|---|---|---|
| light | `text` #0A0A0C | `bg` #F6F5F1 | 18.13:1 | 4.5:1 | ✅ | 본문 글자 |
| light | `text` #0A0A0C | `surface` #FFFFFF | 19.78:1 | 4.5:1 | ✅ | 카드 위 글자 |
| light | `text` #0A0A0C | `sunken` #EFEDE7 | 16.90:1 | 4.5:1 | ✅ | 코드·눌린 면 위 글자 |
| light | `muted` #5B5E66 | `bg` #F6F5F1 | 5.95:1 | 4.5:1 | ✅ | 보조 글자 |
| light | `muted` #5B5E66 | `surface` #FFFFFF | 6.49:1 | 4.5:1 | ✅ | 카드 위 보조 글자 |
| light | `muted` #5B5E66 | `sunken` #EFEDE7 | 5.54:1 | 4.5:1 | ✅ | 눌린 면 위 보조 글자 |
| light | `accent` #0A0A0C | `bg` #F6F5F1 | 18.13:1 | 4.5:1 | ✅ | 링크·텍스트 버튼 |
| light | `accent` #0A0A0C | `surface` #FFFFFF | 19.78:1 | 4.5:1 | ✅ | 카드 위 링크 |
| light | `onAccent` #F6F5F1 | `accent` #0A0A0C | 18.13:1 | 4.5:1 | ✅ | 채운 버튼 글자 |
| light | `focus` #2F5E8C | `bg` #F6F5F1 | 6.21:1 | 3.0:1 | ✅ | 포커스 링(그래픽) |
| light | `dot` #5B5E66 | `bg` #F6F5F1 | 5.95:1 | 3.0:1 | ✅ | 브랜드 점 on 바탕(그래픽) |
| light | `ok` #1F6B33 | `bg` #F6F5F1 | 6.00:1 | 4.5:1 | ✅ | 완료·성공 글자 |
| light | `work` #825600 | `bg` #F6F5F1 | 5.87:1 | 4.5:1 | ✅ | 진행 중 글자 |
| light | `wait` #A04610 | `bg` #F6F5F1 | 5.70:1 | 4.5:1 | ✅ | 내 차례·허락 대기 글자 |
| light | `err` #A32A24 | `bg` #F6F5F1 | 6.61:1 | 4.5:1 | ✅ | 오류 글자 |
| light | `ok` #1F6B33 | `surface` #FFFFFF | 6.54:1 | 4.5:1 | ✅ | 카드 위 성공 |
| light | `err` #A32A24 | `surface` #FFFFFF | 7.21:1 | 4.5:1 | ✅ | 카드 위 오류 |
| light | `wait` #A04610 | `surface` #FFFFFF | 6.22:1 | 4.5:1 | ✅ | 카드 위 대기 |
| light | `work` #825600 | `surface` #FFFFFF | 6.40:1 | 4.5:1 | ✅ | 카드 위 진행 |
| light | `dot` #5B5E66 | `아이콘 타일 void` #0A0A0C | 3.05:1 | 3.0:1 | ✅ | 앱 아이콘의 점(그래픽) |
| dark | `text` #EFEDE7 | `bg` #0A0A0C | 16.90:1 | 4.5:1 | ✅ | 본문 글자 |
| dark | `text` #EFEDE7 | `surface` #16171A | 15.31:1 | 4.5:1 | ✅ | 카드 위 글자 |
| dark | `text` #EFEDE7 | `sunken` #111114 | 16.10:1 | 4.5:1 | ✅ | 코드·눌린 면 위 글자 |
| dark | `muted` #A3A5AB | `bg` #0A0A0C | 8.03:1 | 4.5:1 | ✅ | 보조 글자 |
| dark | `muted` #A3A5AB | `surface` #16171A | 7.28:1 | 4.5:1 | ✅ | 카드 위 보조 글자 |
| dark | `muted` #A3A5AB | `sunken` #111114 | 7.65:1 | 4.5:1 | ✅ | 눌린 면 위 보조 글자 |
| dark | `accent` #EFEDE7 | `bg` #0A0A0C | 16.90:1 | 4.5:1 | ✅ | 링크·텍스트 버튼 |
| dark | `accent` #EFEDE7 | `surface` #16171A | 15.31:1 | 4.5:1 | ✅ | 카드 위 링크 |
| dark | `onAccent` #0A0A0C | `accent` #EFEDE7 | 16.90:1 | 4.5:1 | ✅ | 채운 버튼 글자 |
| dark | `focus` #B4CDE6 | `bg` #0A0A0C | 12.07:1 | 3.0:1 | ✅ | 포커스 링(그래픽) |
| dark | `dot` #7C7E85 | `bg` #0A0A0C | 4.88:1 | 3.0:1 | ✅ | 브랜드 점 on 바탕(그래픽) |
| dark | `ok` #7FCB8E | `bg` #0A0A0C | 10.21:1 | 4.5:1 | ✅ | 완료·성공 글자 |
| dark | `work` #E2B865 | `bg` #0A0A0C | 10.62:1 | 4.5:1 | ✅ | 진행 중 글자 |
| dark | `wait` #EC9A62 | `bg` #0A0A0C | 8.82:1 | 4.5:1 | ✅ | 내 차례·허락 대기 글자 |
| dark | `err` #E8837D | `bg` #0A0A0C | 7.51:1 | 4.5:1 | ✅ | 오류 글자 |
| dark | `ok` #7FCB8E | `surface` #16171A | 9.25:1 | 4.5:1 | ✅ | 카드 위 성공 |
| dark | `err` #E8837D | `surface` #16171A | 6.81:1 | 4.5:1 | ✅ | 카드 위 오류 |
| dark | `wait` #EC9A62 | `surface` #16171A | 7.99:1 | 4.5:1 | ✅ | 카드 위 대기 |
| dark | `work` #E2B865 | `surface` #16171A | 9.63:1 | 4.5:1 | ✅ | 카드 위 진행 |
| dark | `dot` #7C7E85 | `아이콘 타일 void` #0A0A0C | 4.88:1 | 3.0:1 | ✅ | 앱 아이콘의 점(그래픽) |

미달 조합: 없음

