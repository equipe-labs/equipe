//! `/help` and ⌘/: the keys of this window and every command, from the list
//! completion uses (`commands::COMMANDS`) — so the sheet can't drift from what
//! `/` offers. Plain data.

use equipe_core::protocol::Mode;

use crate::commands::COMMANDS;

pub const INTRO: &str = "PM이 주 연락처입니다: 원하는 것을 말하면 PM이 보드에 일을 계획하고, equipe가 티켓마다 담당자에게 넘깁니다. 창은 모든 프로젝트와 스쿼드 홈을 한꺼번에 보여 주고, 헤더의 프로젝트 칩(⌘0–⌘9)이나 /project <이름>이 한 프로젝트로 좁힙니다.";

pub struct Section {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

fn rows(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(k, w)| ((*k).to_owned(), (*w).to_owned())).collect()
}

pub fn sections() -> Vec<Section> {
    vec![
        Section {
            title: "대화".into(),
            rows: rows(&[
                ("⏎", "입력창 위에 적힌 멤버에게 보냅니다 — 일하는 중이면 다음 단계에서 읽고 하던 일은 계속합니다"),
                ("⇥ (Tab)", "일하는 멤버에게 쓴 글을 메모로 — 이번 턴이 끝난 뒤 읽습니다"),
                ("@이름 …", "다른 멤버에게 보냅니다 — PM도 알게 됩니다"),
                ("⌘↑ ⌘↓ · ⌥⌘1–9", "대화할 멤버 고르기"),
                ("⇧⏎ · ⌥⏎ · ⌃J", "줄바꿈"),
                ("Esc · ⌘.", "일하는 멤버를 멈춥니다 — 아직 못 읽은 글은 바로 들어갑니다"),
                ("⌘V · 드롭", "클립보드의 이미지나 이미지 파일을 붙입니다"),
                ("→", "빈 입력창에서 흐린 추측을 받습니다"),
                ("⌘K · /", "명령 목록 — 입력하면 걸러지고, 인자도 이어서 고릅니다"),
            ]),
        },
        Section {
            title: "둘러보기".into(),
            rows: rows(&[
                ("⌘B", "보드와 대화 사이를 오갑니다"),
                ("⌘0 · ⌘1–9", "모든 프로젝트 · 그 프로젝트만"),
                ("⌘⇧O", "도구 단계 펼치기/접기"),
                ("⌘U", "사용량 — 플랜 한도, 멤버별 컨텍스트와 토큰"),
                ("⌘/", "이 도움말"),
                ("⌘⇧K", "연결 대상 — 이 맥 또는 SSH 호스트"),
                ("T-n 링크", "눌러서 보드에서 엽니다"),
            ]),
        },
        Section {
            title: "답하기".into(),
            rows: rows(&[("y · a · n", "요청을 허락 · 항상 허락 · 거절 (입력창이 비었을 때)"), ("보드에서", "a 수락 · r 바꿔 달라고 보내기 · m 병합 · d 변경 보기 · g 의존 보기 · f 찾기 · ⏎ 담당자와 대화")]),
        },
        Section { title: "모드 — 멤버가 알아서 하는 정도 · /mode [멤버] <모드>".into(), rows: Mode::ALL.iter().map(|m| (m.label().to_owned(), m.summary().to_owned())).collect() },
        Section {
            title: "명령 — / 를 누르면 완성됩니다".into(),
            rows: COMMANDS.iter().map(|c| (format!("/{} {}", c.name, c.args).trim_end().to_owned(), c.about.to_owned())).collect(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_completion_offers_is_in_the_help() {
        let help = sections().pop().unwrap();
        assert_eq!(help.rows.len(), COMMANDS.len());
        assert!(help.rows.iter().any(|(k, _)| k.starts_with("/usage")));
        assert!(help.rows.iter().all(|(k, about)| k.starts_with('/') && !about.is_empty()));
    }

    #[test]
    fn the_four_modes_are_explained() {
        let modes = &sections()[3];
        assert_eq!(modes.rows.len(), Mode::ALL.len());
    }
}
