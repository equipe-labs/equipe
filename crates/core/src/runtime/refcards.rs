//! `show_references` as the daemon runs it (`crate::references`): the call
//! checked, its pictures fetched or copied off the loop into the squad's
//! picture store, then one note in the conversation the member speaks in —
//! the gallery a screen draws as a card — and the member told what showed.
//! And the person keeping a card among a project's references.

use crate::protocol::references::RefCard;
use crate::references::{self, Picture};

use super::*;

/// What a `show_references` call came to, off the loop.
pub(super) struct Shown {
    /// The conversation the note goes in.
    pub session: SessionId,
    pub squad: String,
    pub member: String,
    /// The cards whose pictures were had, in the order asked.
    pub cards: Vec<RefCard>,
    /// The others: each one's title, and why.
    pub failed: Vec<(String, String)>,
    pub reply: oneshot::Sender<ToolResult>,
}

/// Where one item's picture comes from, as worked out on the loop.
enum Source {
    Remote(String),
    Local(PathBuf),
    Refused(String),
}

impl Runtime {
    /// `show_references`: refused at once when the call is wrong; else its
    /// pictures are had off the loop and [`Self::references_shown`] follows.
    pub(super) fn show_references(&mut self, call: ToolCall, reply: oneshot::Sender<ToolResult>) {
        let refuse = |reply: oneshot::Sender<ToolResult>, why: String| drop(reply.send(ToolResult::error(why)));
        let Some(id) = self.caller_id(&call) else {
            return refuse(reply, format!("{} isn't on the squad {}.", call.member, call.squad));
        };
        let role = self.sessions.get(&id).and_then(|s| s.member.as_ref()).map(|m| m.role.clone()).unwrap_or_default();
        if !references::offered(&role) {
            return refuse(reply, "Only the PM, a designer or frontend shows references; tell the PM what you found.".into());
        }
        let asked = match references::parse_args(&call.arguments) {
            Ok(asked) => asked,
            Err(why) => return refuse(reply, why),
        };
        // Said where the person talks with it now: a conversation beside the
        // member's own that is at work, else the one that called.
        let session = if self.sessions.get(&id).is_some_and(Session::is_working) {
            id
        } else {
            self.sessions.iter().find(|(_, s)| s.lane.as_ref().is_some_and(|(of, _)| *of == id) && s.is_working()).map_or(id, |(&lane, _)| lane)
        };
        // A path is found where the member works — its worktree, the squad's
        // folder — or in the squad's own pictures (the store, the diagrams).
        let (home, squad) = (self.config.home.clone(), call.squad.clone());
        let (cwd, mut roots) = self.picture_places(&squad, id);
        roots.push(references::store_dir(&home, &squad));
        roots.push(crate::visuals::diagram::dir(&home, &squad));
        let jobs: Vec<(RefCard, Source)> = asked
            .into_iter()
            .map(|a| {
                let source = match a.picture {
                    Picture::Remote(url) => Source::Remote(url),
                    Picture::Local(given) => match crate::visuals::resolve(&given, &cwd, &roots) {
                        Ok(real) => Source::Local(PathBuf::from(real)),
                        Err(why) => Source::Refused(why),
                    },
                };
                (a.card, source)
            })
            .collect();
        let (internal, member) = (self.internal.clone(), call.member.clone());
        tokio::spawn(async move {
            let mut set = tokio::task::JoinSet::new();
            let n = jobs.len();
            let titles: Vec<String> = jobs.iter().map(|(c, _)| c.title.clone()).collect();
            for (at, (card, source)) in jobs.into_iter().enumerate() {
                let (home, squad) = (home.clone(), squad.clone());
                set.spawn(async move {
                    let bytes = match source {
                        Source::Remote(url) => references::fetch(&url).await,
                        Source::Local(path) => tokio::task::spawn_blocking(move || references::read_local(&path)).await.unwrap_or_else(|_| Err("couldn't be read".into())),
                        Source::Refused(why) => Err(why),
                    };
                    let kept = match bytes {
                        Ok(bytes) => tokio::task::spawn_blocking(move || references::store(&home, &squad, &bytes)).await.unwrap_or_else(|_| Err("couldn't be kept".into())),
                        Err(why) => Err(why),
                    };
                    (at, card, kept)
                });
            }
            let mut done: Vec<Option<(RefCard, Result<PathBuf, String>)>> = (0..n).map(|_| None).collect();
            let _ = tokio::time::timeout(references::ALL_WAIT, async {
                while let Some(Ok((at, card, kept))) = set.join_next().await {
                    done[at] = Some((card, kept));
                }
            })
            .await;
            set.abort_all();
            let (mut cards, mut failed) = (Vec::new(), Vec::new());
            for (at, result) in done.into_iter().enumerate() {
                match result {
                    Some((mut card, Ok(path))) => {
                        card.picture = path.to_string_lossy().into_owned();
                        cards.push(card);
                    }
                    Some((card, Err(why))) => failed.push((card.title, why)),
                    None => failed.push((titles[at].clone(), "it took too long".to_owned())),
                }
            }
            let shown = Shown { session, squad: call.squad, member, cards, failed, reply };
            let _ = internal.send(Internal::References(Box::new(shown)));
        });
    }

    /// The pictures of a `show_references` call are had: the gallery goes in
    /// the conversation as one note, the store is brought within its quota,
    /// and the member is told what showed — or, with none, why.
    pub(super) fn references_shown(&mut self, shown: Shown) {
        let Shown { session, squad, member, cards, failed, reply } = shown;
        let failed_words = failed.iter().map(|(title, why)| format!("{title} ({why})")).collect::<Vec<_>>().join("; ");
        if cards.is_empty() {
            let _ = reply.send(ToolResult::error(format!(
                "No picture could be had, so nothing was shown: {failed_words}. Use the https picture addresses the library's results \
give (no sign-in or key in them), or pictures you saved in your worktree, and call again."
            )));
            return;
        }
        let head = say!(self, "notice.references", name = member, count = cards.len());
        let text = format!("{head}\n{}", references::note_body(&cards));
        if self.sessions.contains_key(&session) {
            self.publish(Event::Notice { undo: None, session, text });
        }
        // The store within its quota: never a picture an open conversation of
        // the squad still shows, nor these.
        let store = references::store_dir(&self.config.home, &squad);
        let prefix = store.to_string_lossy().into_owned();
        let mut keep: HashSet<String> = cards.iter().map(|c| c.picture.clone()).collect();
        for s in self.sessions.values().filter(|s| s.squad.as_deref() == Some(squad.as_str())) {
            for entry in s.transcript.entries() {
                if let Entry::Note { text, .. } = entry
                    && text.contains(&prefix)
                {
                    keep.extend(references::parse_body(text).into_iter().map(|c| c.picture));
                }
            }
        }
        tokio::task::spawn_blocking(move || references::prune(&store, &keep, references::FILES_MAX, references::BYTES_MAX));
        let titles = cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>().join(", ");
        let missed = if failed.is_empty() { String::new() } else { format!(" {} couldn't be had and aren't shown: {failed_words}.", failed.len()) };
        let _ = reply.send(ToolResult::ok(format!(
            "Shown to the person in the conversation as a card of {} reference(s): {titles}.{missed} They can open each big, react to it \
and keep it among the project's references. Now say in a line or two what to look at; don't list or describe them again.",
            cards.len()
        )));
    }

    /// The person tries a card live: its playground page built off the loop
    /// (`crate::playground`) and opened as a tab of theirs in the shared
    /// browser — or, when it can't be run, its source page.
    pub(super) fn try_reference(&mut self, squad: String, project: Option<String>, card: RefCard) {
        if !crate::squads::valid_name(&squad) {
            return;
        }
        let Ok(link) = references::check_link(&card.link) else { return };
        // The project's tokens theme it, read here (a small file, at most 256 KB).
        let tokens = crate::taste::store::design_dir(&self.config.home, &squad, project.as_deref())
            .map(|dir| dir.join(crate::design::folder::TOKENS))
            .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= 256 << 10))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| crate::playground::tokens_css(&t))
            .unwrap_or_default();
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let url = match crate::playground::build(&card, &tokens).await {
                Ok(html) => match crate::playground::Server::get().await {
                    Some(server) => server.put(html),
                    None => link,
                },
                Err(why) => {
                    crate::daemon_log!("equipe: {squad}: \"{}\" opens its source page to try: {why}", card.title);
                    link
                }
            };
            let _ = internal.send(Internal::Apply(Box::new(Command::BrowserNewTab { url: Some(url), profile: None })));
        });
    }

    /// The person keeps a card among `project`'s references (the squad's own
    /// with none): its entry added to the pattern's `notes.md` — the link and
    /// lines, never the picture — and said in `session`; an error said on the
    /// squad's screens when it can't be.
    pub(super) fn keep_reference(&mut self, squad: String, session: Option<SessionId>, project: Option<String>, card: RefCard) {
        if !crate::squads::valid_name(&squad) {
            return;
        }
        let link = match references::check_link(&card.link) {
            Ok(link) => link,
            Err(why) => return self.squad_error(&squad, say!(self, "notice.reference_not_kept", title = card.title, why = why)),
        };
        let card = RefCard { link, picture: String::new(), ..card };
        let Some(dir) = crate::taste::store::design_dir(&self.config.home, &squad, project.as_deref()) else {
            let why = format!("there is no project {}", project.clone().unwrap_or_default());
            return self.squad_error(&squad, say!(self, "notice.reference_not_kept", title = card.title, why = why));
        };
        let path = references::notes_path(&dir, &card);
        let today = squad_memory::date(now_secs());
        let written = (|| -> Result<bool, String> {
            if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
                return Err(format!("{} isn't a plain file", path.display()));
            }
            let before = match std::fs::read_to_string(&path) {
                Ok(text) => Some(text),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.to_string()),
            };
            let Some(after) = references::with_kept(before.as_deref(), &card, &today) else { return Ok(false) };
            std::fs::create_dir_all(path.parent().unwrap_or(&dir)).map_err(|e| e.to_string())?;
            std::fs::write(&path, after).map_err(|e| e.to_string())?;
            Ok(true)
        })();
        let rel = path.strip_prefix(dir.parent().unwrap_or(&dir)).unwrap_or(&path).to_string_lossy().into_owned();
        match written {
            Ok(added) => {
                // The squad's own folder is a repository of equipe's: the change is saved there.
                if added && project.is_none() {
                    let folder = squad_home::path(&self.config.home, &squad);
                    let _ = squad_home::save_in(&folder, squad_home::PERSON, "a reference kept from the conversation", &[&rel]);
                }
                let text = if added { say!(self, "notice.reference_kept", title = card.title, path = rel) } else { say!(self, "notice.reference_known", title = card.title, path = rel) };
                match session.filter(|s| self.sessions.contains_key(s)) {
                    Some(session) => self.publish(Event::Notice { undo: None, session, text }),
                    None => crate::daemon_log!("equipe: {squad}: {text}"),
                }
            }
            Err(why) => self.squad_error(&squad, say!(self, "notice.reference_not_kept", title = card.title, why = why)),
        }
    }
}
