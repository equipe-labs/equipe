# 대비 표 — 의미 토큰 조합 (WCAG 2.x AA)

`npm run contrast`로 다시 만든다 (`scripts/contrast.mjs`, 토큰 `tokens/*.json`에서 직접 읽음). 글자 4.5:1, 그래픽(포커스 링·테두리·점·아이콘) 3:1. 반투명 면은 아래층 위에 합성해서 계산한다 ("바탕" 열의 `a + b`는 a 위에 b를 얹은 것).
비활성 상태(`opacity.disabled` 0.4)는 WCAG 1.4.3 예외라 계산에서 뺐다.

## 라이트

| 전경 | 바탕 | 대비 | 기준 | 판정 | 용도 |
|---|---|---|---|---|---|
| `text` #15161A | `bg` #F6F5F1 | 16.57:1 | 4.5:1 | ✅ | 본문 글자 on 바탕 |
| `text` #15161A | `surface` #FFFFFF | 18.08:1 | 4.5:1 | ✅ | 카드·필드 위 글자 |
| `text` #15161A | `raised` #FFFFFF | 18.08:1 | 4.5:1 | ✅ | 팝오버·시트 위 글자 |
| `text` #15161A | `sunken` #EAE7E0 | 14.64:1 | 4.5:1 | ✅ | 코드·눌린 면 위 글자 |
| `muted` #4F5159 | `bg` #F6F5F1 | 7.25:1 | 4.5:1 | ✅ | 보조 글자 on 바탕 |
| `muted` #4F5159 | `surface` #FFFFFF | 7.91:1 | 4.5:1 | ✅ | 보조 글자·플레이스홀더 on 카드/필드 |
| `muted` #4F5159 | `raised` #FFFFFF | 7.91:1 | 4.5:1 | ✅ | 보조 글자 on 시트 |
| `muted` #4F5159 | `sunken` #EAE7E0 | 6.41:1 | 4.5:1 | ✅ | 보조 글자 on 눌린 면(중립 배지) |
| `accent` #1D4ED8 | `bg` #F6F5F1 | 6.14:1 | 4.5:1 | ✅ | 링크·텍스트 버튼 on 바탕 |
| `accent` #1D4ED8 | `surface` #FFFFFF | 6.70:1 | 4.5:1 | ✅ | 링크 on 카드 |
| `accent` #1D4ED8 | `raised` #FFFFFF | 6.70:1 | 4.5:1 | ✅ | 링크 on 시트 |
| `accent` #1D4ED8 | `surface` + `accent-bg` #E8EDFB | 5.72:1 | 4.5:1 | ✅ | 선택된 칩·행 글자(accent on accent-bg) |
| `text` #15161A | `surface` + `accent-bg` #E8EDFB | 15.44:1 | 4.5:1 | ✅ | 선택된 행 본문 글자 |
| `muted` #4F5159 | `surface` + `accent-bg` #E8EDFB | 6.76:1 | 4.5:1 | ✅ | 선택된 행 보조 글자 |
| `on-accent` #FFFFFF | `accent` #1D4ED8 | 6.70:1 | 4.5:1 | ✅ | 채운 브랜드 버튼 글자 (웹 마케팅) |
| `on-action` #F6F5F1 | `action` #15161A | 16.57:1 | 4.5:1 | ✅ | 주 버튼 글자 |
| `on-action` #F6F5F1 | `action-hover` #2A2C33 | 12.78:1 | 4.5:1 | ✅ | 주 버튼 호버 |
| `on-action` #F6F5F1 | `action-pressed` #3A3C44 | 10.08:1 | 4.5:1 | ✅ | 주 버튼 눌림 |
| `on-err` #FFFFFF | `err` #AD2E27 | 6.57:1 | 4.5:1 | ✅ | 위험 버튼 글자 |
| `text` #15161A | `bg` + `hover` #EAE9E5 | 14.89:1 | 4.5:1 | ✅ | 행 호버 위 글자 |
| `text` #15161A | `surface` + `hover` #F3F3F3 | 16.22:1 | 4.5:1 | ✅ | 카드 안 행 호버 위 글자 |
| `muted` #4F5159 | `surface` + `hover` #F3F3F3 | 7.10:1 | 4.5:1 | ✅ | 행 호버 위 보조 글자 |
| `text` #15161A | `bg` + `pressed` #E1E0DC | 13.66:1 | 4.5:1 | ✅ | 눌린 행 위 글자 |
| `accent` #1D4ED8 | `bg` + `pressed` #E1E0DC | 5.06:1 | 4.5:1 | ✅ | 눌린 텍스트 버튼 |
| `ok` #1F6B33 | `bg` #F6F5F1 | 6.00:1 | 4.5:1 | ✅ | 완료 글자 on 바탕 |
| `work` #825600 | `bg` #F6F5F1 | 5.87:1 | 4.5:1 | ✅ | 진행 중 글자 on 바탕 |
| `wait` #A04610 | `bg` #F6F5F1 | 5.70:1 | 4.5:1 | ✅ | 내 차례 글자 on 바탕 |
| `err` #AD2E27 | `bg` #F6F5F1 | 6.02:1 | 4.5:1 | ✅ | 오류 글자 on 바탕 |
| `ok` #1F6B33 | `surface` #FFFFFF | 6.54:1 | 4.5:1 | ✅ | 완료 글자 on 카드 |
| `work` #825600 | `surface` #FFFFFF | 6.40:1 | 4.5:1 | ✅ | 진행 중 글자 on 카드 |
| `wait` #A04610 | `surface` #FFFFFF | 6.22:1 | 4.5:1 | ✅ | 내 차례 글자 on 카드 |
| `err` #AD2E27 | `surface` #FFFFFF | 6.57:1 | 4.5:1 | ✅ | 오류 글자 on 카드 |
| `ok` #1F6B33 | `raised` #FFFFFF | 6.54:1 | 4.5:1 | ✅ | 완료 on 시트 |
| `wait` #A04610 | `raised` #FFFFFF | 6.22:1 | 4.5:1 | ✅ | 내 차례 on 시트 |
| `err` #AD2E27 | `raised` #FFFFFF | 6.57:1 | 4.5:1 | ✅ | 오류 on 시트(필드 오류 글자) |
| `ok` #1F6B33 | `surface` + `ok-bg` #E4EDE6 | 5.47:1 | 4.5:1 | ✅ | 배지 ok (카드 위) |
| `work` #825600 | `surface` + `work-bg` #F0EAE0 | 5.36:1 | 4.5:1 | ✅ | 배지 work (카드 위) |
| `wait` #A04610 | `surface` + `wait-bg` #F3E9E2 | 5.19:1 | 4.5:1 | ✅ | 배지 wait (카드 위) |
| `err` #AD2E27 | `surface` + `err-bg` #F5E6E5 | 5.41:1 | 4.5:1 | ✅ | 배지 err (카드 위) |
| `ok` #1F6B33 | `bg` + `ok-bg` #DCE4DA | 5.04:1 | 4.5:1 | ✅ | 배지 ok (바탕 위) |
| `work` #825600 | `bg` + `work-bg` #E8E2D4 | 4.94:1 | 4.5:1 | ✅ | 배지 work (바탕 위) |
| `wait` #A04610 | `bg` + `wait-bg` #ECE0D6 | 4.78:1 | 4.5:1 | ✅ | 배지 wait (바탕 위) |
| `err` #AD2E27 | `bg` + `err-bg` #EDDDD8 | 4.98:1 | 4.5:1 | ✅ | 배지·배너 err (바탕 위) |
| `on-inverse` #F6F5F1 | `inverse` #15161A | 16.57:1 | 4.5:1 | ✅ | 토스트 글자 |
| `inverse-accent` #8FB0FF | `inverse` #15161A | 8.46:1 | 4.5:1 | ✅ | 토스트 액션 |
| `focus` #1D4ED8 | `bg` #F6F5F1 | 6.14:1 | 3:1 | ✅ | 포커스 링 on 바탕 |
| `focus` #1D4ED8 | `surface` #FFFFFF | 6.70:1 | 3:1 | ✅ | 포커스 링 on 카드 |
| `focus` #1D4ED8 | `raised` #FFFFFF | 6.70:1 | 3:1 | ✅ | 포커스 링 on 시트 |
| `focus` #1D4ED8 | `sunken` #EAE7E0 | 5.43:1 | 3:1 | ✅ | 포커스 링 on 눌린 면 |
| `border-strong` #74767D | `bg` #F6F5F1 | 4.16:1 | 3:1 | ✅ | 필드·보조 버튼 테두리 on 바탕 |
| `border-strong` #74767D | `surface` #FFFFFF | 4.54:1 | 3:1 | ✅ | 필드 테두리 on 카드 |
| `border-strong` #74767D | `raised` #FFFFFF | 4.54:1 | 3:1 | ✅ | 시트 손잡이·필드 on 시트 |
| `accent` #1D4ED8 | `surface` + `accent-bg` #E8EDFB | 5.72:1 | 3:1 | ✅ | 선택된 칩 테두리 |
| `wait` #A04610 | `surface` #FFFFFF | 6.22:1 | 3:1 | ✅ | 결정 카드 테두리(wait) |
| `err` #AD2E27 | `surface` #FFFFFF | 6.57:1 | 3:1 | ✅ | 오류 필드 테두리 |
| `dot` #3F6FF5 | `bg` #F6F5F1 | 4.01:1 | 3:1 | ✅ | 브랜드 점 on 바탕 (그래픽·로고 전용) |
| `dot` #3F6FF5 | `surface` #FFFFFF | 4.37:1 | 3:1 | ✅ | 브랜드 점 on 카드 |
| `dot` #3F6FF5 | `raw.neutral.950` #0A0A0C | 4.52:1 | 3:1 | ✅ | 브랜드 점 on 아이콘 타일(void) |

## 다크

| 전경 | 바탕 | 대비 | 기준 | 판정 | 용도 |
|---|---|---|---|---|---|
| `text` #EFEDE7 | `bg` #0A0A0C | 16.90:1 | 4.5:1 | ✅ | 본문 글자 on 바탕 |
| `text` #EFEDE7 | `surface` #16171A | 15.31:1 | 4.5:1 | ✅ | 카드·필드 위 글자 |
| `text` #EFEDE7 | `raised` #1B1C20 | 14.54:1 | 4.5:1 | ✅ | 팝오버·시트 위 글자 |
| `text` #EFEDE7 | `sunken` #111114 | 16.10:1 | 4.5:1 | ✅ | 코드·눌린 면 위 글자 |
| `muted` #A3A5AB | `bg` #0A0A0C | 8.03:1 | 4.5:1 | ✅ | 보조 글자 on 바탕 |
| `muted` #A3A5AB | `surface` #16171A | 7.28:1 | 4.5:1 | ✅ | 보조 글자·플레이스홀더 on 카드/필드 |
| `muted` #A3A5AB | `raised` #1B1C20 | 6.91:1 | 4.5:1 | ✅ | 보조 글자 on 시트 |
| `muted` #A3A5AB | `sunken` #111114 | 7.65:1 | 4.5:1 | ✅ | 보조 글자 on 눌린 면(중립 배지) |
| `accent` #8FB0FF | `bg` #0A0A0C | 9.25:1 | 4.5:1 | ✅ | 링크·텍스트 버튼 on 바탕 |
| `accent` #8FB0FF | `surface` #16171A | 8.38:1 | 4.5:1 | ✅ | 링크 on 카드 |
| `accent` #8FB0FF | `raised` #1B1C20 | 7.96:1 | 4.5:1 | ✅ | 링크 on 시트 |
| `accent` #8FB0FF | `surface` + `accent-bg` #252A36 | 6.75:1 | 4.5:1 | ✅ | 선택된 칩·행 글자(accent on accent-bg) |
| `text` #EFEDE7 | `surface` + `accent-bg` #252A36 | 12.32:1 | 4.5:1 | ✅ | 선택된 행 본문 글자 |
| `muted` #A3A5AB | `surface` + `accent-bg` #252A36 | 5.86:1 | 4.5:1 | ✅ | 선택된 행 보조 글자 |
| `on-accent` #0B1226 | `accent` #8FB0FF | 8.70:1 | 4.5:1 | ✅ | 채운 브랜드 버튼 글자 (웹 마케팅) |
| `on-action` #0A0A0C | `action` #EFEDE7 | 16.90:1 | 4.5:1 | ✅ | 주 버튼 글자 |
| `on-action` #0A0A0C | `action-hover` #DAD7CF | 13.75:1 | 4.5:1 | ✅ | 주 버튼 호버 |
| `on-action` #0A0A0C | `action-pressed` #C9C6BE | 11.59:1 | 4.5:1 | ✅ | 주 버튼 눌림 |
| `on-err` #0A0A0C | `err` #EE7B74 | 7.27:1 | 4.5:1 | ✅ | 위험 버튼 글자 |
| `text` #EFEDE7 | `bg` + `hover` #1A1A1B | 14.84:1 | 4.5:1 | ✅ | 행 호버 위 글자 |
| `text` #EFEDE7 | `surface` + `hover` #252628 | 12.91:1 | 4.5:1 | ✅ | 카드 안 행 호버 위 글자 |
| `muted` #A3A5AB | `surface` + `hover` #252628 | 6.14:1 | 4.5:1 | ✅ | 행 호버 위 보조 글자 |
| `text` #EFEDE7 | `bg` + `pressed` #262627 | 12.97:1 | 4.5:1 | ✅ | 눌린 행 위 글자 |
| `accent` #8FB0FF | `bg` + `pressed` #262627 | 7.10:1 | 4.5:1 | ✅ | 눌린 텍스트 버튼 |
| `ok` #7FCB8E | `bg` #0A0A0C | 10.21:1 | 4.5:1 | ✅ | 완료 글자 on 바탕 |
| `work` #E2B865 | `bg` #0A0A0C | 10.62:1 | 4.5:1 | ✅ | 진행 중 글자 on 바탕 |
| `wait` #EC9A62 | `bg` #0A0A0C | 8.82:1 | 4.5:1 | ✅ | 내 차례 글자 on 바탕 |
| `err` #EE7B74 | `bg` #0A0A0C | 7.27:1 | 4.5:1 | ✅ | 오류 글자 on 바탕 |
| `ok` #7FCB8E | `surface` #16171A | 9.25:1 | 4.5:1 | ✅ | 완료 글자 on 카드 |
| `work` #E2B865 | `surface` #16171A | 9.63:1 | 4.5:1 | ✅ | 진행 중 글자 on 카드 |
| `wait` #EC9A62 | `surface` #16171A | 7.99:1 | 4.5:1 | ✅ | 내 차례 글자 on 카드 |
| `err` #EE7B74 | `surface` #16171A | 6.59:1 | 4.5:1 | ✅ | 오류 글자 on 카드 |
| `ok` #7FCB8E | `raised` #1B1C20 | 8.78:1 | 4.5:1 | ✅ | 완료 on 시트 |
| `wait` #EC9A62 | `raised` #1B1C20 | 7.59:1 | 4.5:1 | ✅ | 내 차례 on 시트 |
| `err` #EE7B74 | `raised` #1B1C20 | 6.26:1 | 4.5:1 | ✅ | 오류 on 시트(필드 오류 글자) |
| `ok` #7FCB8E | `surface` + `ok-bg` #232D28 | 7.35:1 | 4.5:1 | ✅ | 배지 ok (카드 위) |
| `work` #E2B865 | `surface` + `work-bg` #2F2B23 | 7.60:1 | 4.5:1 | ✅ | 배지 work (카드 위) |
| `wait` #EC9A62 | `surface` + `wait-bg` #302723 | 6.51:1 | 4.5:1 | ✅ | 배지 wait (카드 위) |
| `err` #EE7B74 | `surface` + `err-bg` #302325 | 5.53:1 | 4.5:1 | ✅ | 배지 err (카드 위) |
| `ok` #7FCB8E | `bg` + `ok-bg` #18211C | 8.47:1 | 4.5:1 | ✅ | 배지 ok (바탕 위) |
| `work` #E2B865 | `bg` + `work-bg` #241F17 | 8.77:1 | 4.5:1 | ✅ | 배지 work (바탕 위) |
| `wait` #EC9A62 | `bg` + `wait-bg` #251C16 | 7.47:1 | 4.5:1 | ✅ | 배지 wait (바탕 위) |
| `err` #EE7B74 | `bg` + `err-bg` #261819 | 6.31:1 | 4.5:1 | ✅ | 배지·배너 err (바탕 위) |
| `on-inverse` #0A0A0C | `inverse` #EFEDE7 | 16.90:1 | 4.5:1 | ✅ | 토스트 글자 |
| `inverse-accent` #1D4ED8 | `inverse` #EFEDE7 | 5.72:1 | 4.5:1 | ✅ | 토스트 액션 |
| `focus` #8FB0FF | `bg` #0A0A0C | 9.25:1 | 3:1 | ✅ | 포커스 링 on 바탕 |
| `focus` #8FB0FF | `surface` #16171A | 8.38:1 | 3:1 | ✅ | 포커스 링 on 카드 |
| `focus` #8FB0FF | `raised` #1B1C20 | 7.96:1 | 3:1 | ✅ | 포커스 링 on 시트 |
| `focus` #8FB0FF | `sunken` #111114 | 8.82:1 | 3:1 | ✅ | 포커스 링 on 눌린 면 |
| `border-strong` #7C7E85 | `bg` #0A0A0C | 4.88:1 | 3:1 | ✅ | 필드·보조 버튼 테두리 on 바탕 |
| `border-strong` #7C7E85 | `surface` #16171A | 4.42:1 | 3:1 | ✅ | 필드 테두리 on 카드 |
| `border-strong` #7C7E85 | `raised` #1B1C20 | 4.20:1 | 3:1 | ✅ | 시트 손잡이·필드 on 시트 |
| `accent` #8FB0FF | `surface` + `accent-bg` #252A36 | 6.75:1 | 3:1 | ✅ | 선택된 칩 테두리 |
| `wait` #EC9A62 | `surface` #16171A | 7.99:1 | 3:1 | ✅ | 결정 카드 테두리(wait) |
| `err` #EE7B74 | `surface` #16171A | 6.59:1 | 3:1 | ✅ | 오류 필드 테두리 |
| `dot` #3F6FF5 | `bg` #0A0A0C | 4.52:1 | 3:1 | ✅ | 브랜드 점 on 바탕 (그래픽·로고 전용) |
| `dot` #3F6FF5 | `surface` #16171A | 4.10:1 | 3:1 | ✅ | 브랜드 점 on 카드 |
| `dot` #3F6FF5 | `raw.neutral.950` #0A0A0C | 4.52:1 | 3:1 | ✅ | 브랜드 점 on 아이콘 타일(void) |

**합계: 116개 조합 중 미달 0개.**
