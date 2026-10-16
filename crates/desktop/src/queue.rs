//! What waits for a member at work, in the order the person said it: steers
//! (read at its next step, the work carrying on) and memos (read when the
//! turn ends), each with the project the person was looking at. They show in
//! the conversation as the person's own bubbles until the member reads them,
//! they are taken back (`Command::Unqueue`) or sent at once
//! (`Command::SendNow`). The daemon decides; this follows its events. No UI
//! types here, so the rules are tested without a window.

use equipe_core::protocol::SessionInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Read at the member's next step.
    Steer,
    /// Read when its turn ends.
    Memo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub kind: Kind,
    pub text: String,
    /// The project it was said about (`None`: the whole squad).
    pub project: Option<String>,
    /// When it was said, in unix seconds (0: not known — said before the app
    /// attached).
    pub at: u64,
    /// A steer the turn ended on, unread: a memo now, as the daemon is about
    /// to say. Its word keeps it in its place rather than adding another.
    echo: bool,
}

impl Pending {
    fn new(kind: Kind, text: &str, project: Option<String>, at: u64) -> Self {
        Self { kind, text: text.to_owned(), project, at, echo: false }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Queue {
    items: Vec<Pending>,
}

impl Queue {
    /// What waits as a screen attaching finds it: the steers not read, then
    /// the memos (an older daemon: one project for all its memos).
    pub fn from_info(info: &SessionInfo) -> Self {
        let mut items = Vec::new();
        for (i, text) in info.steering.iter().enumerate() {
            items.push(Pending::new(Kind::Steer, text, info.steering_in.get(i).cloned().flatten(), 0));
        }
        let memos_in = if info.queued_in.len() == info.queued.len() { info.queued_in.clone() } else { vec![info.queued_project.clone(); info.queued.len()] };
        for (text, project) in info.queued.iter().zip(memos_in) {
            items.push(Pending::new(Kind::Memo, text, project, 0));
        }
        Self { items }
    }

    pub fn items(&self) -> &[Pending] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether a memo waits (the turn's end brings a message).
    pub fn has_memo(&self) -> bool {
        self.items.iter().any(|p| p.kind == Kind::Memo)
    }

    pub fn memo_count(&self) -> usize {
        self.items.iter().filter(|p| p.kind == Kind::Memo).count()
    }

    /// Said while it works, to be read at its next step.
    pub fn steer(&mut self, text: &str, project: Option<String>, at: u64) {
        self.items.push(Pending::new(Kind::Steer, text, project, at));
    }

    /// Left for when its turn ends. A steer it didn't read comes back as a
    /// memo where it was.
    pub fn memo(&mut self, text: &str, project: Option<String>, at: u64) {
        if let Some(item) = self.items.iter_mut().find(|p| p.echo && p.text == text) {
            item.echo = false;
            item.project = project;
            return;
        }
        self.items.push(Pending::new(Kind::Memo, text, project, at));
    }

    /// The member read a steer: it is the conversation's now.
    pub fn read(&mut self, text: &str) -> bool {
        match self.items.iter().position(|p| p.kind == Kind::Steer && p.text == text) {
            Some(at) => {
                self.items.remove(at);
                true
            }
            None => false,
        }
    }

    /// Taken back: the first memo with these words, else the first steer.
    pub fn withdraw(&mut self, text: &str) -> Option<Pending> {
        let at = self.items.iter().position(|p| p.kind == Kind::Memo && p.text == text).or_else(|| self.items.iter().position(|p| p.text == text))?;
        Some(self.items.remove(at))
    }

    /// Sent at once, as one message: none of it waits any more.
    pub fn sent(&mut self, texts: &[String]) {
        for text in texts {
            if let Some(at) = self.items.iter().position(|p| p.text == *text) {
                self.items.remove(at);
            }
        }
    }

    /// The turn ended: what it didn't read is delivered as memos are.
    pub fn turn_ended(&mut self) {
        for item in self.items.iter_mut().filter(|p| p.kind == Kind::Steer) {
            item.kind = Kind::Memo;
            item.echo = true;
        }
    }

    /// A new turn started with the memos of the first project one was left
    /// about, together: they leave; another project's wait for the next end.
    pub fn delivered(&mut self) {
        let Some(project) = self.items.iter().find(|p| p.kind == Kind::Memo).map(|p| p.project.clone()) else {
            return;
        };
        self.items.retain(|p| p.kind != Kind::Memo || p.project != project);
    }

    /// Every text waiting, in order — what "send now" sends.
    pub fn texts(&self) -> Vec<String> {
        self.items.iter().map(|p| p.text.clone()).collect()
    }

    /// What shows with `slice` in focus (`None`: every project), and how
    /// many others wait about other projects.
    pub fn shown(&self, slice: Option<&str>) -> (Vec<&Pending>, usize) {
        let (here, elsewhere): (Vec<&Pending>, Vec<&Pending>) = self.items.iter().partition(|p| slice.is_none() || p.project.as_deref() == slice);
        (here, elsewhere.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(q: &Queue) -> Vec<(&str, Kind)> {
        q.items().iter().map(|p| (p.text.as_str(), p.kind)).collect()
    }

    #[test]
    fn several_said_while_it_works_stack_in_order() {
        let mut q = Queue::default();
        q.steer("이렇게 딱히 보이지 않아.", Some("site".into()), 10);
        q.steer("??", Some("site".into()), 11);
        q.memo("끝나면 이것도", None, 12);
        assert_eq!(texts(&q), [("이렇게 딱히 보이지 않아.", Kind::Steer), ("??", Kind::Steer), ("끝나면 이것도", Kind::Memo)]);
        assert_eq!(q.texts(), ["이렇게 딱히 보이지 않아.", "??", "끝나면 이것도"]);
        assert!(q.has_memo());
    }

    #[test]
    fn a_steer_read_leaves_and_only_that_one() {
        let mut q = Queue::default();
        q.steer("a", None, 1);
        q.steer("a", None, 2);
        q.memo("a", None, 3);
        assert!(q.read("a"));
        assert_eq!(texts(&q), [("a", Kind::Steer), ("a", Kind::Memo)]);
        assert_eq!(q.items()[0].at, 2, "the first of them was read");
        assert!(q.read("a"));
        assert!(!q.read("a"), "a memo isn't read mid-turn");
    }

    #[test]
    fn taken_back_a_memo_goes_before_a_steer_with_the_same_words() {
        let mut q = Queue::default();
        q.steer("x", None, 1);
        q.memo("x", Some("app".into()), 2);
        let back = q.withdraw("x").unwrap();
        assert_eq!((back.kind, back.project.as_deref()), (Kind::Memo, Some("app")));
        assert_eq!(q.withdraw("x").unwrap().kind, Kind::Steer);
        assert!(q.withdraw("x").is_none());
    }

    #[test]
    fn sent_now_none_of_it_waits() {
        let mut q = Queue::default();
        q.steer("a", None, 1);
        q.memo("b", None, 2);
        q.steer("c", None, 3);
        q.sent(&["a".into(), "b".into(), "c".into()]);
        assert!(q.is_empty());
        // An interrupted turn and the message: nothing comes back.
        q.turn_ended();
        q.delivered();
        assert!(q.is_empty());
    }

    #[test]
    fn unread_at_the_turns_end_a_steer_is_a_memo_in_its_place() {
        let mut q = Queue::default();
        q.memo("m", Some("site".into()), 1);
        q.steer("s", Some("site".into()), 2);
        q.turn_ended();
        // The daemon says so: it stays where it was, once.
        q.memo("s", Some("site".into()), 9);
        assert_eq!(texts(&q), [("m", Kind::Memo), ("s", Kind::Memo)]);
        assert_eq!(q.items()[1].at, 2);
        // Then delivered together, as one message.
        q.delivered();
        assert!(q.is_empty());
    }

    #[test]
    fn delivered_a_project_at_a_time() {
        let mut q = Queue::default();
        q.memo("for site", Some("site".into()), 1);
        q.memo("for app", Some("app".into()), 2);
        q.memo("site again", Some("site".into()), 3);
        q.delivered();
        assert_eq!(q.texts(), ["for app"]);
    }

    #[test]
    fn a_slice_shows_its_own_and_counts_the_rest() {
        let mut q = Queue::default();
        q.steer("mobile한 명 더 추가하는거 승인할게.", None, 1);
        q.steer("우리 웹사이트는 어떻게 되어가는중?", Some("equipe-site".into()), 2);
        q.memo("app memo", Some("app".into()), 3);
        let (here, others) = q.shown(Some("equipe-site"));
        assert_eq!(here.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["우리 웹사이트는 어떻게 되어가는중?"]);
        assert_eq!(others, 2);
        let (all, none) = q.shown(None);
        assert_eq!((all.len(), none), (3, 0));
    }

    #[test]
    fn attaching_mid_turn_finds_the_steers_then_the_memos() {
        let info: SessionInfo = serde_json::from_value(serde_json::json!({
            "session": 1, "project": "/p", "member": null, "engine": "claude", "model": null, "access": "read_only",
            "workspace": null, "state": "working", "title": null, "tokens": 0, "recent": [],
            "queued": ["m1", "m2"], "queued_project": "app", "steering": ["s1"], "steering_in": ["site"]
        }))
        .unwrap();
        let q = Queue::from_info(&info);
        assert_eq!(texts(&q), [("s1", Kind::Steer), ("m1", Kind::Memo), ("m2", Kind::Memo)]);
        assert_eq!(q.items().iter().map(|p| p.project.as_deref()).collect::<Vec<_>>(), [Some("site"), Some("app"), Some("app")], "an older daemon: one project for its memos");
    }
}
