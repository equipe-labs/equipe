# T-582 긴 대화 스크롤 프로파일

2026-11-09, macOS, release + snapshot, 1200×750 light, animation off
(`FAKE_WORKING=0`, `FAKE_ASKS=0`), hidden manual draw를 `EQUIPE_PERF=2`에
포함했다. 각 fixture는 3초 settle 뒤 older/newer 방향으로 wheel 240회씩
실행했다.

## 결정적 카운터

| 상태 | fixture | 방향 | root render | conversation render | derived rebuild | chapter build |
|---|---:|---|---:|---:|---:|---:|
| 전 | 50/500/2,000 | 각 방향 | 240 | 240 | 240 | 720 |
| 후 | 50/500/2,000 | 각 방향 | 240 | 240 | 0 | 0 |

스크롤 1회당 conversation/root render 비율은 모든 구간에서 1.000이다.
파생 상태와 chapter는 초기 revision에서 한 번 만들어진 뒤 두 스크롤 구간
동안 카운터가 그대로였다.

## CPU와 메모리

quiet 재측정에서 양방향 480 frame을 합친 값:

| fixture | CPU/frame 평균 | CPU p95 | CPU max | settle 메모리 | 왕복 뒤 메모리 |
|---:|---:|---:|---:|---:|---:|
| 50 | 5.726 ms | 8.347 ms | 13.947 ms | 78.4 MB | 64.9 MB |
| 2,000 | 3.499 ms | 7.565 ms | 11.363 ms | 69.6 MB | 68.5 MB |

2,000-entry는 p95 8ms와 max 16.7ms 예산 안이며, 50-entry 대비 평균
CPU/frame이 증가하지 않았다. physical footprint 차이는 +30MB 안이고 왕복 뒤
지속 증가하지 않았다.

첫 측정에서는 2,000-entry older 방향 p95가 전 10.159ms에서 후 8.500ms로
줄었지만 머신 load average가 14여서 판정에는 쓰지 않았다. 위 표는 load average
3.23의 재측정이다.

원자료는 `baseline/app-{50,500,2000}.log`, `after/app-{50,500,2000}.log`,
`quiet/app-{50,2000}.log`에 있다.

최신 `origin/main` rebase 뒤 같은 release·hidden draw 조건으로 2,000-entry를
다시 확인했다. 480 frame의 CPU/frame은 평균 5.612ms, p95 7.151ms,
max 9.461ms였다. 각 240 wheel 구간에서 conversation render는 정확히 240회,
derived rebuild와 chapter build는 0회였고, 메모리는 settle 68.0MB에서 왕복 뒤
64.0MB였다. 원자료는 `rebased-app-2000.log`에 있다.
