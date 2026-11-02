# 대화 속 그림 대비 표 (WCAG 2.x AA)

`node contrast.mjs`로 다시 만든다. 글자 4.5:1, 그래픽(선·테두리·점) 3:1. 반투명 면은 아래층 위에 합성해서 계산했다.

## 라이트

| 전경 | 바탕 | 대비 | 기준 | 판정 | 용도 |
|---|---|---|---|---|---|
| text | surface | 18.08 | 4.5 | 통과 | 카드: 제목·본문 |
| muted | surface | 7.91 | 4.5 | 통과 | 카드: 보조 글자·표 라벨·눈금 |
| muted | sunken | 6.41 | 4.5 | 통과 | 카드: 프로젝트 줄·표 첫 열·중립 배지·Mermaid 레인 제목 |
| text | sunken | 14.64 | 4.5 | 통과 | 카드: 프로젝트 이름 |
| ok | surface + ok-bg | 5.47 | 4.5 | 통과 | 배지 완료 |
| work | surface + work-bg | 5.36 | 4.5 | 통과 | 배지 진행 중 |
| wait | surface + wait-bg | 5.19 | 4.5 | 통과 | 배지 사람 차례 |
| err | surface + err-bg | 5.41 | 4.5 | 통과 | 배지 막힘 |
| accent | surface + accent-bg | 5.72 | 4.5 | 통과 | 추천 배지·열 |
| text | surface + accent-bg | 15.44 | 4.5 | 통과 | 추천 열 글자 |
| accent | surface | 6.70 | 4.5 | 통과 | 추천 이유 강조 |
| ok | surface | 6.54 | 4.5 | 통과 | 증감 ▲ 좋음 |
| err | surface | 6.57 | 4.5 | 통과 | 증감 ▼ 나쁨 |
| accent | surface | 6.70 | 3 | 통과 | 스파크라인 선·끝점 |
| text | surface + work-bg | 15.15 | 4.5 | 통과 | Mermaid: 진행 중 노드 글자 |
| work | surface | 6.40 | 3 | 통과 | Mermaid: 진행 중 노드 테두리 |
| text | surface + wait-bg | 15.09 | 4.5 | 통과 | Mermaid: 사람 차례 노드 글자 |
| wait | surface | 6.22 | 3 | 통과 | Mermaid: 사람 차례 노드 테두리 |
| text | surface + ok-bg | 15.11 | 4.5 | 통과 | Mermaid: 완료 노드 글자 |
| ok | surface | 6.54 | 3 | 통과 | Mermaid: 완료 테두리 |
| text | surface + err-bg | 14.88 | 4.5 | 통과 | Mermaid: 막힘 노드 글자 |
| err | surface | 6.57 | 3 | 통과 | Mermaid: 막힘 테두리 |
| border-strong | surface | 4.54 | 3 | 통과 | Mermaid: 다음 노드 테두리 |
| muted | surface | 7.91 | 3 | 통과 | Mermaid: 선·화살표 |
| accent | surface | 6.70 | 3 | 통과 | Mermaid: 핵심 줄기 |
| on-action | action | 16.57 | 4.5 | 통과 | Mermaid: 강조 노드 글자 |
| action | surface | 18.08 | 3 | 통과 | Mermaid: 강조 노드 면 |
| text | bg | 16.57 | 4.5 | 통과 | Mermaid: 알약 노드 글자 on 바탕색 면 |
| border-strong | bg | 4.16 | 3 | 통과 | Mermaid: 알약 테두리 |
| on-accent | accent | 6.70 | 4.5 | 통과 | Mermaid 원그래프: 조각(accent) 위 글자 |
| on-accent | ok | 6.54 | 4.5 | 통과 | Mermaid 원그래프: 조각(ok) 위 글자 |
| on-accent | work | 6.40 | 4.5 | 통과 | Mermaid 원그래프: 조각(work) 위 글자 |
| on-accent | wait | 6.22 | 4.5 | 통과 | Mermaid 원그래프: 조각(wait) 위 글자 |
| on-accent | err | 6.57 | 4.5 | 통과 | Mermaid 원그래프: 조각(err) 위 글자 |
| on-accent | border-strong | 4.54 | 4.5 | 통과 | Mermaid 원그래프: 조각(border-strong) 위 글자 |
| accent | surface | 6.70 | 3 | 통과 | Mermaid 원그래프: 조각(accent) 면 vs 카드 |
| ok | surface | 6.54 | 3 | 통과 | Mermaid 원그래프: 조각(ok) 면 vs 카드 |
| work | surface | 6.40 | 3 | 통과 | Mermaid 원그래프: 조각(work) 면 vs 카드 |
| wait | surface | 6.22 | 3 | 통과 | Mermaid 원그래프: 조각(wait) 면 vs 카드 |
| err | surface | 6.57 | 3 | 통과 | Mermaid 원그래프: 조각(err) 면 vs 카드 |
| border-strong | surface | 4.54 | 3 | 통과 | Mermaid 원그래프: 조각(border-strong) 면 vs 카드 |

## 다크

| 전경 | 바탕 | 대비 | 기준 | 판정 | 용도 |
|---|---|---|---|---|---|
| text | surface | 15.31 | 4.5 | 통과 | 카드: 제목·본문 |
| muted | surface | 7.28 | 4.5 | 통과 | 카드: 보조 글자·표 라벨·눈금 |
| muted | sunken | 7.65 | 4.5 | 통과 | 카드: 프로젝트 줄·표 첫 열·중립 배지·Mermaid 레인 제목 |
| text | sunken | 16.10 | 4.5 | 통과 | 카드: 프로젝트 이름 |
| ok | surface + ok-bg | 7.35 | 4.5 | 통과 | 배지 완료 |
| work | surface + work-bg | 7.60 | 4.5 | 통과 | 배지 진행 중 |
| wait | surface + wait-bg | 6.51 | 4.5 | 통과 | 배지 사람 차례 |
| err | surface + err-bg | 5.53 | 4.5 | 통과 | 배지 막힘 |
| accent | surface + accent-bg | 6.75 | 4.5 | 통과 | 추천 배지·열 |
| text | surface + accent-bg | 12.32 | 4.5 | 통과 | 추천 열 글자 |
| accent | surface | 8.38 | 4.5 | 통과 | 추천 이유 강조 |
| ok | surface | 9.25 | 4.5 | 통과 | 증감 ▲ 좋음 |
| err | surface | 6.59 | 4.5 | 통과 | 증감 ▼ 나쁨 |
| accent | surface | 8.38 | 3 | 통과 | 스파크라인 선·끝점 |
| text | surface + work-bg | 12.09 | 4.5 | 통과 | Mermaid: 진행 중 노드 글자 |
| work | surface | 9.63 | 3 | 통과 | Mermaid: 진행 중 노드 테두리 |
| text | surface + wait-bg | 12.47 | 4.5 | 통과 | Mermaid: 사람 차례 노드 글자 |
| wait | surface | 7.99 | 3 | 통과 | Mermaid: 사람 차례 노드 테두리 |
| text | surface + ok-bg | 12.16 | 4.5 | 통과 | Mermaid: 완료 노드 글자 |
| ok | surface | 9.25 | 3 | 통과 | Mermaid: 완료 테두리 |
| text | surface + err-bg | 12.85 | 4.5 | 통과 | Mermaid: 막힘 노드 글자 |
| err | surface | 6.59 | 3 | 통과 | Mermaid: 막힘 테두리 |
| border-strong | surface | 4.42 | 3 | 통과 | Mermaid: 다음 노드 테두리 |
| muted | surface | 7.28 | 3 | 통과 | Mermaid: 선·화살표 |
| accent | surface | 8.38 | 3 | 통과 | Mermaid: 핵심 줄기 |
| on-action | action | 16.90 | 4.5 | 통과 | Mermaid: 강조 노드 글자 |
| action | surface | 15.31 | 3 | 통과 | Mermaid: 강조 노드 면 |
| text | bg | 16.90 | 4.5 | 통과 | Mermaid: 알약 노드 글자 on 바탕색 면 |
| border-strong | bg | 4.88 | 3 | 통과 | Mermaid: 알약 테두리 |
| bg | accent | 9.25 | 4.5 | 통과 | Mermaid 원그래프: 조각(accent) 위 글자 |
| bg | ok | 10.21 | 4.5 | 통과 | Mermaid 원그래프: 조각(ok) 위 글자 |
| bg | work | 10.62 | 4.5 | 통과 | Mermaid 원그래프: 조각(work) 위 글자 |
| bg | wait | 8.82 | 4.5 | 통과 | Mermaid 원그래프: 조각(wait) 위 글자 |
| bg | err | 7.27 | 4.5 | 통과 | Mermaid 원그래프: 조각(err) 위 글자 |
| bg | border-strong | 4.88 | 4.5 | 통과 | Mermaid 원그래프: 조각(border-strong) 위 글자 |
| accent | surface | 8.38 | 3 | 통과 | Mermaid 원그래프: 조각(accent) 면 vs 카드 |
| ok | surface | 9.25 | 3 | 통과 | Mermaid 원그래프: 조각(ok) 면 vs 카드 |
| work | surface | 9.63 | 3 | 통과 | Mermaid 원그래프: 조각(work) 면 vs 카드 |
| wait | surface | 7.99 | 3 | 통과 | Mermaid 원그래프: 조각(wait) 면 vs 카드 |
| err | surface | 6.59 | 3 | 통과 | Mermaid 원그래프: 조각(err) 면 vs 카드 |
| border-strong | surface | 4.42 | 3 | 통과 | Mermaid 원그래프: 조각(border-strong) 면 vs 카드 |
