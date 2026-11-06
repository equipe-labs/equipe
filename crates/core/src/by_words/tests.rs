use super::*;
use equipe_protocol::accept::FROM_WORDS;
use equipe_protocol::by_words::{Heard, Reach, Via};
use serde_json::json;

fn ticket(id: TicketId, status: &str, kind: &str) -> Ticket {
    serde_json::from_value(json!({ "id": id, "title": format!("work {id}"), "body": "", "status": status, "kind": kind, "branch": format!("equipe/t-{id}"), "project": "/src/web", "depends_on": [], "created_by": "pm" })).unwrap()
}

/// T-1 code and T-2 research in review, T-3 code being done, T-4 code
/// accepted but not on main, T-5 a proposal, T-6 an idea, T-7 a writing task.
fn board() -> Vec<Ticket> {
    let mut off = ticket(4, "done", "code");
    off.merged = Some(false);
    vec![ticket(1, "review", "code"), ticket(2, "review", "task"), ticket(3, "in_progress", "code"), off, ticket(5, "proposed", "code"), ticket(6, "idea", "task"), ticket(7, "review", "task")]
}

fn approval(id: u64, member: &str, tool: &str, action: &str) -> Approval {
    Approval { id, session: 1, member: member.into(), ticket: Some(3), tool: tool.into(), action: action.into(), reason: Some("to see the tests".into()), preview: None }
}

fn routine(id: u32, title: &str) -> Routine {
    Routine { id, title: title.into(), prompt: "go".into(), when: Schedule::Weekly { weekday: 5, hour: 17, minute: 0 }, on: true, last_run: None, next_run: None }
}

fn said() -> Said {
    Said { via: Via::Typed, utterance: "yes".into() }
}

struct Fixture {
    tickets: Vec<Ticket>,
    approvals: Vec<Approval>,
    accepts: Vec<AcceptProposal>,
    routines: Vec<Routine>,
    proposals: Vec<RoutineProposal>,
    team: Vec<TeamChange>,
}

impl Fixture {
    fn new() -> Self {
        Self { tickets: board(), approvals: Vec::new(), accepts: Vec::new(), routines: vec![routine(1, "Weekly report"), routine(2, "주간 리포트")], proposals: Vec::new(), team: Vec::new() }
    }

    fn waiting(&self) -> Waiting<'_> {
        Waiting { tickets: &self.tickets, approvals: &self.approvals, accepts: &self.accepts, routines: &self.routines, routine_proposals: &self.proposals, team_changes: &self.team, engine_proposals: &[] }
    }
}

fn tickets(verb: Verb, ids: &[TicketId]) -> Vec<Ask> {
    vec![Ask::Tickets { verb, tickets: ids.to_vec() }]
}

#[test]
fn every_request_for_permission_has_its_kind_and_what_equipe_refuses_stays_refused() {
    let kind = |tool: &str, action: &str| approval_kind(&approval(1, "backend", tool, action));
    assert_eq!(kind("Bash", "npm test"), Kind::AllowOnce);
    assert_eq!(kind("Write", "/src/web/a.rs"), Kind::AllowOnce);
    assert_eq!(kind("WebFetch", "https://example.com"), Kind::AllowOnce);
    assert_eq!(kind(permissions::BROWSER_CODE, "await page.click()"), Kind::AllowOnce);
    assert_eq!(kind("Bash", "git push origin main"), Kind::AllowOutward);
    assert_eq!(kind("Bash", "npm test && vercel deploy --prod"), Kind::AllowOutward);
    assert_eq!(kind("Bash", "cargo publish"), Kind::AllowOutward);
    assert_eq!(kind("Bash", "gh release create v1"), Kind::AllowOutward);
    assert_eq!(kind("Read", "/Users/a/.ssh/id_ed25519"), Kind::TouchSecrets);
    assert_eq!(kind("Bash", "cat .env"), Kind::TouchSecrets);
    assert_eq!(kind(crate::runtime::CONFIRM, "Pay $49.00 a month for Vercel Pro"), Kind::ActInName);
    assert_eq!(kind(crate::runtime::SIGN_IN_TOOL, "github.com as abel"), Kind::SignInWithLogin);
    assert_eq!(kind(crate::desktop_gate::SEND, "Messages → Mina: “see you at 5”"), Kind::SendAsPerson);
    assert_eq!(kind(crate::controls::SETTING_TOOL, "Mode: full"), Kind::WidenSetting);
    assert_eq!(kind("Bash", "sudo rm -rf /"), Kind::Blocked);
    assert_eq!(kind("Bash", "git push --force origin main"), Kind::Blocked);
    assert_eq!(Kind::Blocked.reach(), Reach::Never);
    // Only plain commands and sites can become a rule; a member's own question never does.
    assert!(can_always(&approval(1, "b", "Bash", "npm test")));
    assert!(!can_always(&approval(1, "b", "Bash", "npm test && rm x")));
    assert!(!can_always(&approval(1, "b", crate::runtime::CONFIRM, "Sign up")));
    assert!(!can_always(&approval(1, "b", crate::desktop_gate::SEND, "Mail")));
}

#[test]
fn every_ask_has_its_tier() {
    let mut f = Fixture::new();
    f.approvals = vec![approval(10, "backend", "Bash", "npm test"), approval(11, "backend", "Bash", "git push"), approval(12, "pm", crate::runtime::CONFIRM, "Pay $49")];
    f.accepts = vec![AcceptProposal { id: 20, verdict: Verdict::Merge, tickets: vec![1], note: String::new(), summary: String::new(), by: "pm".into(), session: 1, proposed_at: 0 }];
    let w = f.waiting();
    let t = |ask: Ask| tier(&ask, &w);
    assert_eq!(t(Ask::Tickets { verb: Verb::Accept, tickets: vec![2, 7] }), Tier::Low, "research and writing");
    assert_eq!(t(Ask::Tickets { verb: Verb::Accept, tickets: vec![2, 1] }), Tier::High, "code among them");
    assert_eq!(t(Ask::Tickets { verb: Verb::Merge, tickets: vec![1] }), Tier::High);
    assert_eq!(t(Ask::Tickets { verb: Verb::Push, tickets: vec![1] }), Tier::Outward);
    assert_eq!(t(Ask::Tickets { verb: Verb::Idea, tickets: vec![5] }), Tier::Low);
    assert_eq!(t(Ask::Tickets { verb: Verb::TakeOn, tickets: vec![5] }), Tier::Low);
    assert_eq!(t(Ask::Tickets { verb: Verb::KeepOnBranch, tickets: vec![4] }), Tier::Low);
    assert_eq!(t(Ask::RunRoutine(1)), Tier::Low);
    assert_eq!(t(Ask::ApproveRoutine(1)), Tier::High);
    assert_eq!(t(Ask::Engine(1)), Tier::High);
    assert_eq!(t(Ask::Team(1)), Tier::Outward);
    assert_eq!(t(Ask::Approval { id: 10, always: false }), Tier::High);
    assert_eq!(t(Ask::Approval { id: 10, always: true }), Tier::Outward, "widening permissions");
    assert_eq!(t(Ask::Approval { id: 11, always: false }), Tier::Outward);
    assert_eq!(t(Ask::Approval { id: 12, always: false }), Tier::Outward, "paying");
    assert_eq!(t(Ask::Card(20)), Tier::High);
    assert_eq!(t(Ask::Approval { id: 99, always: false }), Tier::Outward, "gone: as refused");
}

#[test]
fn plain_words_name_what_waits_and_anything_less_plain_is_left_to_the_member() {
    let f = Fixture::new();
    let w = f.waiting();
    let read = |text: &str| read(text, &w);
    // Accepting, the way a card was raised before.
    assert_eq!(read("T-2 수락"), tickets(Verb::Accept, &[2]));
    assert_eq!(read("accept T-2 and T-7"), tickets(Verb::Accept, &[2, 7]));
    assert_eq!(read("전부다 수락할게."), tickets(Verb::Accept, &[1, 2, 7]));
    assert_eq!(read("다수락해줘"), tickets(Verb::Accept, &[1, 2, 7]));
    assert_eq!(read("Accept them all!"), tickets(Verb::Accept, &[1, 2, 7]));
    assert_eq!(read("T-1 머지"), tickets(Verb::Merge, &[1]));
    assert_eq!(read("merge T-4"), tickets(Verb::Merge, &[4]));
    assert_eq!(read("merge T-1 and push"), tickets(Verb::Push, &[1]));
    assert_eq!(read("T-1 푸시까지"), tickets(Verb::Push, &[1]));
    assert_eq!(read("전부 머지까지 해줘"), tickets(Verb::Merge, &[1, 4]), "to merge, accepted code off main too; research has nothing to merge");
    assert_eq!(read("leave T-4 on its branch"), tickets(Verb::KeepOnBranch, &[4]));
    assert_eq!(read("T-5 아이디어로 남겨"), tickets(Verb::Idea, &[5]));
    assert_eq!(read("keep T-5 as an idea"), tickets(Verb::Idea, &[5]));
    assert_eq!(read("take on T-6"), tickets(Verb::TakeOn, &[6]));
    assert_eq!(read("T-5 진행"), tickets(Verb::TakeOn, &[5]));
    assert_eq!(read("run the weekly report"), vec![Ask::RunRoutine(1)]);
    assert_eq!(read("주간 리포트 한 번 돌려 줘"), vec![Ask::RunRoutine(2)]);
    // Named but not fit, or nothing named: nothing.
    assert_eq!(read("T-3 수락"), vec![], "being done");
    assert_eq!(read("T-1, T-3, T-99 수락"), tickets(Verb::Accept, &[1]));
    assert_eq!(read("수락할게"), vec![]);
    assert_eq!(read("run it"), vec![], "no routine named");
    for unsure in [
        "전부 수락할까?",
        "T-1 수락하지 마",
        "아직 수락하지 말아줘",
        "테스트 통과하면 다 수락할게",
        "수락 버튼 어디 있어",
        "T-1 수락 버튼이 안 눌려요",
        "이건 반려",
        "T-1 어때",
        "don't accept T-1",
        "accept all after the tests",
        "don't merge T-1",
        "not yet, merge T-1 later",
        "/accept T-1",
        "@pm accept T-1",
        "다 했어요",
        "오늘 한 일 정리해서 내일 아침에 다시 보여줘. 그리고 T-1은 리뷰가 끝나면 수락할 생각이에요, 문제 없으면요",
        "まだ承認しないで",
        "不要接受 T-1",
        "先别合并",
        "no aceptes T-1",
        "n'accepte pas T-1",
        "T-1 nicht akzeptieren",
        "wenn die Tests laufen, alles akzeptieren",
    ] {
        assert_eq!(read(unsure), vec![], "{unsure}");
    }
    // Quoted or fenced words are never the person's.
    assert_eq!(read("> merge T-1\nok, thanks"), vec![]);
    assert_eq!(read("<!-- reply 1 -->\n> accept T-2\n<!-- reply -->\nlooks fine"), vec![]);
    assert_eq!(read("```\nmerge T-1\n```"), vec![]);
}

#[test]
fn plain_words_are_read_in_every_language() {
    let f = Fixture::new();
    let w = f.waiting();
    for (text, verb, ids) in [
        ("T-2 accept", Verb::Accept, vec![2]),
        ("T-2 承認して", Verb::Accept, vec![2]),
        ("全部承認", Verb::Accept, vec![1, 2, 7]),
        ("T-1 マージして", Verb::Merge, vec![1]),
        ("接受 T-2", Verb::Accept, vec![2]),
        ("全部接受", Verb::Accept, vec![1, 2, 7]),
        ("T-1 合并", Verb::Merge, vec![1]),
        ("T-1 合併", Verb::Merge, vec![1]),
        ("aprueba T-2", Verb::Accept, vec![2]),
        ("acepta todo", Verb::Accept, vec![1, 2, 7]),
        ("fusiona T-1", Verb::Merge, vec![1]),
        ("T-2 validé", Verb::Accept, vec![2]),
        ("accepte tout", Verb::Accept, vec![1, 2, 7]),
        ("fusionne T-1", Verb::Merge, vec![1]),
        ("T-2 genehmigen", Verb::Accept, vec![2]),
        ("alles akzeptieren", Verb::Accept, vec![1, 2, 7]),
        ("T-1 zusammenführen", Verb::Merge, vec![1]),
    ] {
        assert_eq!(read(text, &w), tickets(verb, &ids), "{text}");
    }
}

#[test]
fn allowing_in_words_names_what_waits_and_several_are_a_choice() {
    let mut f = Fixture::new();
    f.approvals = vec![approval(10, "backend", "Bash", "npm test"), approval(11, "frontend", "Bash", "git push origin main"), approval(12, "pm", crate::runtime::CONFIRM, "Pay $49")];
    f.accepts = vec![
        AcceptProposal { id: 20, verdict: Verdict::Accept, tickets: vec![1], note: String::new(), summary: String::new(), by: "pm".into(), session: 1, proposed_at: 0 },
        AcceptProposal { id: 21, verdict: Verdict::Accept, tickets: vec![2], note: String::new(), summary: String::new(), by: FROM_WORDS.into(), session: 1, proposed_at: 0 },
    ];
    f.proposals = vec![RoutineProposal { id: 30, routine: routine(0, "Dependency check"), reason: "weekly".into(), by: "pm".into(), session: 1, proposed_at: 0 }];
    let w = f.waiting();
    assert_eq!(read("allow backend", &w), vec![Ask::Approval { id: 10, always: false }]);
    assert_eq!(read("frontend 허용", &w), vec![Ask::Approval { id: 11, always: false }]);
    assert_eq!(read("approve the dependency check", &w), vec![Ask::ApproveRoutine(30)]);
    // Everything waiting, with no name: the person chooses. A card raised from their own words is theirs to click.
    assert_eq!(
        read("승인", &w),
        vec![Ask::Approval { id: 10, always: false }, Ask::Approval { id: 11, always: false }, Ask::Approval { id: 12, always: false }, Ask::Card(20), Ask::ApproveRoutine(30)]
    );
    // "Always": only what can become a rule.
    assert_eq!(read("always allow backend", &w), vec![Ask::Approval { id: 10, always: true }]);
    assert_eq!(read("앞으로 항상 허용", &w), vec![Ask::Approval { id: 10, always: true }, Ask::Approval { id: 11, always: true }]);
    assert_eq!(read("don't allow backend", &w), vec![]);
    assert_eq!(read("allow backend?", &w), vec![]);
    // Nothing waits: nothing to read.
    let empty = Fixture { approvals: vec![], accepts: vec![], proposals: vec![], ..Fixture::new() };
    assert_eq!(read("allow it", &empty.waiting()), vec![]);
}

#[test]
fn doing_it_sends_only_what_its_card_or_the_boards_buttons_send() {
    let mut f = Fixture::new();
    f.approvals = vec![approval(10, "backend", "Bash", "npm test")];
    f.accepts = vec![AcceptProposal { id: 20, verdict: Verdict::Merge, tickets: vec![1, 2], note: String::new(), summary: String::new(), by: "pm".into(), session: 1, proposed_at: 0 }];
    let w = f.waiting();
    let run = |ask: Ask| commands("acme", &ask, &w, &said());
    let s = || "acme".to_owned();
    assert_eq!(run(Ask::Tickets { verb: Verb::Accept, tickets: vec![2, 7] }), [Command::Accept { squad: s(), ticket: 2 }, Command::Accept { squad: s(), ticket: 7 }]);
    assert_eq!(run(Ask::Tickets { verb: Verb::Merge, tickets: vec![1, 4] }), [Command::Accept { squad: s(), ticket: 1 }, Command::Merge { squad: s(), ticket: 1, push: false }, Command::Merge { squad: s(), ticket: 4, push: false }]);
    assert_eq!(run(Ask::Tickets { verb: Verb::Push, tickets: vec![4] }), [Command::Merge { squad: s(), ticket: 4, push: true }]);
    assert_eq!(run(Ask::Tickets { verb: Verb::KeepOnBranch, tickets: vec![4] }), [Command::KeepOnBranch { squad: s(), ticket: 4 }]);
    assert_eq!(run(Ask::Tickets { verb: Verb::Idea, tickets: vec![5] }), [Command::MoveTicket { squad: s(), ticket: 5, status: Some(TicketStatus::Idea), before: None }]);
    assert_eq!(run(Ask::Tickets { verb: Verb::TakeOn, tickets: vec![6] }), [Command::MoveTicket { squad: s(), ticket: 6, status: Some(TicketStatus::Todo), before: None }]);
    assert_eq!(run(Ask::RunRoutine(2)), [Command::RunRoutine { squad: s(), id: 2 }]);
    assert_eq!(run(Ask::Approval { id: 10, always: false }), [Command::Decide { squad: s(), approval: 10, decision: Decision::Allow, words: None, said: Some(said()) }]);
    assert_eq!(run(Ask::Approval { id: 10, always: true }), [Command::Decide { squad: s(), approval: 10, decision: Decision::Always, words: None, said: Some(said()) }]);
    assert_eq!(
        run(Ask::Card(20)),
        [Command::ConfirmAccept { squad: s(), id: 20, tickets: vec![Confirmed { ticket: 1, merge: Some(Land::Merge) }, Confirmed::only(2)], said: Some(said()) }],
        "merged where there is code; research goes into the folder"
    );
    assert_eq!(run(Ask::ApproveRoutine(30)), [Command::ApproveRoutine { squad: s(), id: 30 }]);
    assert_eq!(run(Ask::Team(40)), [Command::ApproveTeamChange { squad: s(), id: 40 }]);
    assert_eq!(run(Ask::Engine(50)), [Command::ApplyEngineProposal { squad: s(), id: 50 }]);
    // Never any other command: nothing that pairs, saves a secret, widens a mode or turns words on.
    for ask in [Ask::Tickets { verb: Verb::Push, tickets: vec![1, 2, 4] }, Ask::Card(20), Ask::Approval { id: 10, always: true }] {
        for c in run(ask) {
            assert!(matches!(c, Command::Accept { .. } | Command::Merge { .. } | Command::ConfirmAccept { .. } | Command::Decide { .. }), "{c:?}");
        }
    }
}

#[test]
fn a_request_still_waits_only_while_what_it_names_takes_it() {
    let mut f = Fixture::new();
    let ask = Ask::Tickets { verb: Verb::Accept, tickets: vec![2] };
    assert!(still_waits(&ask, &f.waiting()));
    f.tickets[1].status = TicketStatus::Done;
    assert!(!still_waits(&ask, &f.waiting()), "accepted on the board meanwhile");
    assert!(!still_waits(&Ask::Approval { id: 10, always: false }, &f.waiting()));
    assert!(still_waits(&Ask::RunRoutine(1), &f.waiting()));
}

#[test]
fn the_read_back_says_the_action_and_its_target_in_equipes_own_words() {
    let mut f = Fixture::new();
    f.approvals = vec![approval(11, "frontend", "Bash", "git push origin main"), approval(12, "pm", crate::runtime::CONFIRM, "Pay $49.00 a month to Vercel")];
    let w = f.waiting();
    let (head, items) = read_back(Lang::En, &Ask::Tickets { verb: Verb::Push, tickets: vec![1] }, &w);
    assert_eq!(head, "Merge T-1 into main and push main to origin");
    assert_eq!(items, ["T-1 “work 1” — equipe/t-1 → main of web"]);
    let (head, items) = read_back(Lang::Ko, &Ask::Tickets { verb: Verb::Merge, tickets: vec![1] }, &w);
    assert_eq!(head, "T-1를 이 Mac에서 main에 머지");
    assert_eq!(items, ["T-1 “work 1” — equipe/t-1 → web의 main"]);
    let (head, items) = read_back(Lang::En, &Ask::Tickets { verb: Verb::Accept, tickets: vec![2] }, &w);
    assert_eq!((head.as_str(), items[0].as_str()), ("Accept T-2", "T-2 “work 2” — into the squad's folder"));
    let (head, items) = read_back(Lang::En, &Ask::Approval { id: 11, always: false }, &w);
    assert_eq!(head, "Let frontend reach outside the squad, once: git push origin main");
    assert_eq!(items, ["On T-3", "frontend says: “to see the tests”"]);
    // The amount as the member asked it, word for word.
    let (head, _) = read_back(Lang::En, &Ask::Approval { id: 12, always: false }, &w);
    assert_eq!(head, "Let pm act in your name: “Pay $49.00 a month to Vercel”");
    let (head, _) = read_back(Lang::Ko, &Ask::Approval { id: 12, always: false }, &w);
    assert_eq!(head, "pm가 내 이름으로 해요: “Pay $49.00 a month to Vercel”");
    assert_eq!(read_back(Lang::En, &Ask::RunRoutine(1), &w).0, "Run the routine “Weekly report” once, now");
    assert_eq!(ask_line(Lang::En, Tier::Outward), "Say yes and it goes in 10 s — say cancel to stop it then.");
    assert_eq!(ask_line(Lang::En, Tier::High), "Say yes to go ahead, or no to leave it on its card.");
}

#[test]
fn a_read_back_runs_only_on_a_clear_yes_and_asks_again_when_unclear() {
    let rb = |text: &str, tier: Tier| answer(Stage::ReadBack, tier, text, 0, 0);
    assert_eq!(rb("yes", Tier::High), (Step::Run, Reply::Yes));
    assert_eq!(rb("ㄱㄱ", Tier::High), (Step::Run, Reply::Yes));
    assert_eq!(rb("고고", Tier::High), (Step::Run, Reply::Yes));
    assert_eq!(rb("ok", Tier::Outward), (Step::Countdown, Reply::Yes), "outward: counts down first");
    assert_eq!(rb("no", Tier::High), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
    assert_eq!(rb("아니 하지 마", Tier::High), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
    assert_eq!(rb("not yet", Tier::Outward), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
    assert_eq!(rb("hmm", Tier::High), (Step::Again, Reply::Unclear));
    assert_eq!(rb("yes?", Tier::High), (Step::Again, Reply::Unclear));
    assert_eq!(rb("> yes", Tier::High), (Step::Again, Reply::Unclear), "a quoted yes is not theirs");
    // Unclear too often: held, nothing done.
    assert_eq!(answer(Stage::ReadBack, Tier::High, "hmm", 0, by_words::ASK_AGAIN), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::Unclear));
    // Moved on to something else entirely: held, and the words go on to the member.
    let long = "actually, first fix the header on the pricing page and then look at the footer links again";
    assert_eq!(rb(long, Tier::High), (Step::Close { outcome: Outcome::Held, pass: true }, Reply::Other));
    // Choosing: a number, else asked again; a no holds.
    assert_eq!(answer(Stage::Which, Tier::High, "2", 3, 0), (Step::Choose(2), Reply::Pick));
    assert_eq!(answer(Stage::Which, Tier::High, "yes", 3, 0), (Step::Again, Reply::Unclear), "a yes chooses nothing");
    assert_eq!(answer(Stage::Which, Tier::High, "none of them, no", 3, 0), (Step::Close { outcome: Outcome::Held, pass: false }, Reply::No));
}

#[test]
fn undo_and_cancel_windows_stop_on_any_no_and_let_the_rest_through() {
    assert_eq!(answer(Stage::Undo, Tier::Low, "undo", 0, 0), (Step::Close { outcome: Outcome::Undone, pass: false }, Reply::Undo));
    assert_eq!(answer(Stage::Undo, Tier::Low, "되돌려", 0, 0).0, Step::Close { outcome: Outcome::Undone, pass: false });
    assert_eq!(answer(Stage::Undo, Tier::Low, "and fix the header", 0, 0), (Step::Pass, Reply::Other));
    assert_eq!(answer(Stage::Countdown, Tier::Outward, "cancel", 0, 0), (Step::Close { outcome: Outcome::Cancelled, pass: false }, Reply::Cancel));
    assert_eq!(answer(Stage::Countdown, Tier::Outward, "잠깐", 0, 0).0, Step::Close { outcome: Outcome::Cancelled, pass: false });
    assert_eq!(answer(Stage::Countdown, Tier::Outward, "ok", 0, 0), (Step::Pass, Reply::Other));
}

fn entry(id: u64, outcome: Outcome, tickets: Vec<TicketId>, closed_at: u64) -> Entry {
    Entry {
        id,
        session: 1,
        kind: Kind::AcceptTask,
        tier: Tier::Low,
        via: Via::Typed,
        all_by_words: false,
        said: "T-2 수락".into(),
        read_back: String::new(),
        items: vec![],
        answers: vec![],
        tickets,
        said_at: closed_at - 5_000,
        read_back_at: None,
        decided_at: None,
        closed_at,
        outcome,
        asked_again: 0,
        chose: false,
        device: None,
        flags: vec![],
    }
}

#[test]
fn the_log_flags_what_was_undone_cancelled_unclear_or_taken_back_later() {
    let back = |ticket: TicketId, at: u64, kind: &str| (ticket, TicketEntry { at, by: "you".into(), kind: kind.into(), text: String::new() });
    let mut unclear = entry(4, Outcome::Done, vec![], 10_000);
    unclear.answers = vec![Heard { words: "hm".into(), read: Reply::Unclear, at: 9_000, via: Via::Typed }];
    unclear.asked_again = 1;
    let entries = vec![entry(1, Outcome::Done, vec![2], 100_000), entry(2, Outcome::Undone, vec![7], 100_000), entry(3, Outcome::Done, vec![7], 100_000), unclear, entry(5, Outcome::Cancelled, vec![], 10_000)];
    let events = [back(2, 200, "reopened"), back(7, 50, "reopened"), back(7, 300, "comment")];
    let flagged = flagged(entries, &events);
    assert_eq!(flagged[0].flags, [Flag::RevertedLater], "reopened after it was accepted in words");
    assert_eq!(flagged[1].flags, [Flag::Undone]);
    assert_eq!(flagged[2].flags, Vec::<Flag>::new(), "reopened before, and only commented after");
    assert_eq!(flagged[3].flags, [Flag::AskedAgain, Flag::Ambiguous]);
    assert_eq!(flagged[4].flags, [Flag::Cancelled]);
    let line = week_line(&flagged, 0).unwrap();
    assert_eq!(line, "Decided in the person's words: 5 (3 done) · undone 1 · cancelled 1 · asked again 1 · ambiguous 1 · held 0 · taken back later 1");
    assert_eq!(week_line(&flagged, 1_000_000), None, "nothing this week: the retro as before");
    let text = flagged.iter().map(|e| serde_json::to_string(e).unwrap()).collect::<Vec<_>>().join("\n") + "\nnot json\n";
    assert_eq!(parse_log(&text).len(), 5);
}

#[test]
fn a_card_is_what_high_words_raise_when_cards_decide() {
    assert_eq!(card_for(&tickets(Verb::Accept, &[1])), Some((Verdict::Accept, vec![1])));
    assert_eq!(card_for(&tickets(Verb::Push, &[1, 4])), Some((Verdict::Merge, vec![1, 4])));
    assert_eq!(card_for(&tickets(Verb::Idea, &[5])), None);
    assert_eq!(card_for(&[Ask::RunRoutine(1)]), None);
    assert_eq!(card_for(&[Ask::RunRoutine(1), Ask::RunRoutine(2)]), None);
}
