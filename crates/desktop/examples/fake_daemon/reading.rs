//! `FAKE_READING=1`: the PM's last reply is a long Korean one with every kind
//! of text a reply has — headings, a numbered list with bullets under it,
//! bold, ticket numbers, a link, inline code and a code block — to compare
//! how the conversation reads before and after a change to its colours or type.

pub fn on() -> bool {
    std::env::var("FAKE_READING").is_ok_and(|v| v == "1")
}

pub const ASKED: &str = "로그인 고친 거 어디까지 됐어? 정리해 줘";

pub const REPLY: &str = "로그인 문제는 **원인을 찾았고, 고치는 중**이에요. 지금까지 한 일과 남은 일을 정리했어요.

## 원인

세션 쿠키가 `SameSite=Strict`로 설정돼 있어서, 결제 페이지에서 돌아올 때 로그인이 풀렸어요. [T-42](ticket:42)에서 처음 보고된 문제와 같은 원인이에요.

## 진행 상황

1. **백엔드**: 쿠키 설정을 `Lax`로 바꿨어요.
   - 테스트 `cargo test -p auth`가 모두 통과했어요
   - 스테이징에 올려서 확인했어요
2. **프론트엔드**: 결제 후 돌아오는 경로를 고쳤어요. [T-117](ticket:117)에서 같이 처리했어요.
3. **QA**: 사파리와 크롬에서 다시 확인하는 중이에요.

## 남은 일

- 모바일 앱 웹뷰에서도 같은 문제가 있는지 확인해야 해요
- 배포 전에 [보안 체크리스트](https://owasp.org/www-project-top-ten/)를 한 번 더 볼게요

```sh
cargo test -p auth -- session
```

QA 확인이 끝나면 **오늘 저녁에 배포**할 수 있어요. 진행할까요?";
