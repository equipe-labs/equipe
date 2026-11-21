//! Internal chores: a member asking the person to do a developer's step — open
//! a terminal, run a command, install software, type a `sudo` password.
//!
//! Where a squad runs on its own for a non-developer
//! ([`equipe_protocol::autonomy::Autonomy::Full`], what a cloud machine
//! defaults to) nobody on the person's side can or should do such a step, and
//! the member that asks has a shell (and, on a cloud machine, `sudo`) itself:
//! it installs the missing browser, dependency or tool and goes on. One such
//! ask stalled a ticket for half an hour behind a person who had no terminal.
//!
//! [`judge`] looks at what `block_on_person` was given: asks that are internal
//! steps are refused back to the member with guidance to do them itself; the
//! asks that remain (a token to issue, an account to connect) are shown
//! without any raw shell command in them ([`scrub`]), since a card must never
//! read as a terminal session to someone who doesn't code.
//!
//! A chore is found by its structure first (a command in backticks, a known
//! installer's command line, `sudo`) and by words second; the words, in each
//! language, live in the locale files (`match.chore_*`), never here. An ask
//! that also reaches outward or concerns a credential (a token to issue, a
//! sign-in, a deploy) stays the person's — except a `sudo` password, which is
//! never the person's.

/// First words of a shell command, as found in backticks.
const COMMANDS: &[&str] = &[
    "sudo", "npx", "npm", "pnpm", "yarn", "bun", "bunx", "brew", "apt", "apt-get", "dnf", "yum", "apk", "snap", "pip", "pip3", "pipx", "python", "python3", "node", "cargo", "rustup", "gem", "go", "dotnet",
    "curl", "wget", "chmod", "chown", "mkdir", "cd", "ls", "git", "docker", "make", "bash", "sh", "zsh", "xcode-select", "playwright", "softwareupdate",
];

/// Command lines of installers and runners that are recognizable in running text.
const PHRASES: &[&str] = &[
    "npx ", "npm install", "npm i ", "npm run", "pnpm install", "pnpm add", "yarn add", "yarn install", "brew install", "apt-get", "apt install", "pip install", "pip3 install", "playwright install", "cargo install",
    "xcode-select", "sudo ",
];

fn mentions(lower: &str, key: &str) -> bool {
    crate::i18n::every(key).iter().any(|w| lower.contains(w))
}

/// The backtick spans of `text` (inline or fenced), without their fences.
fn spans(text: &str) -> Vec<&str> {
    text.split('`').enumerate().filter(|(i, s)| i % 2 == 1 && !s.trim().is_empty()).map(|(_, s)| s).collect()
}

/// Whether `span` is a shell command: it starts with a program a shell runs
/// (after a `$` prompt or a language tag a fence may carry).
fn is_command(span: &str) -> bool {
    let span = span.trim().trim_start_matches(['$', '>', '#']).trim_start();
    let span = span.strip_prefix("bash").or_else(|| span.strip_prefix("sh\n")).or_else(|| span.strip_prefix("shell")).map_or(span, str::trim_start);
    let first = span.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    // A bare path (`outputs/T-12/shots/`) or a single word is not a command line.
    COMMANDS.contains(&first.as_str()) && span.split_whitespace().nth(1).is_some()
}

/// Whether the text asks the person to type a `sudo` password.
fn wants_sudo(lower: &str) -> bool {
    lower.contains("sudo") || mentions(lower, "match.chore_password_words")
}

/// Whether `ask` is an internal step: a command to run, a terminal to open,
/// something to install, a `sudo` password to type. An ask that also reaches
/// outward (a credential, a sign-in, a deploy) is not, unless it is for `sudo`.
pub fn is_chore(ask: &str) -> bool {
    let lower = ask.to_lowercase();
    if wants_sudo(&lower) {
        return true;
    }
    let internal = spans(&ask.replace("```", "`")).into_iter().any(is_command) || PHRASES.iter().any(|p| lower.contains(p)) || mentions(&lower, "match.chore_terminal_words");
    internal && !mentions(&lower, "match.chore_outward_words")
}

/// `text` without its shell commands: backtick spans that are commands are
/// dropped, and the gaps they leave closed up.
pub fn scrub(text: &str) -> String {
    let text = text.replace("```", "`");
    let mut out = String::new();
    for (i, part) in text.split('`').enumerate() {
        if i % 2 == 1 {
            if is_command(part) {
                continue;
            }
            out.push('`');
            out.push_str(part);
            out.push('`');
        } else {
            out.push_str(part);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What `block_on_person` does with its asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    /// What the person is shown, without raw commands.
    pub asks: Vec<String>,
    /// What to tell the member when some of its asks were turned away.
    pub told: Option<String>,
}

/// The line a member is told when it asks the person for an internal step.
pub fn refusal(refused: &[&str]) -> String {
    let list = refused.iter().map(|r| format!("\"{}\"", r.chars().take(100).collect::<String>())).collect::<Vec<_>>().join(", ");
    format!(
        "Not put to the person: {list} is an internal step, and this person is not a developer — never ask them to open a terminal, run a \
command, install anything or type a sudo password. You have a shell, and `sudo` where the machine needs it: do it yourself now (install \
the browser, dependency or tool with its own installer — for a browser, `npx playwright install --with-deps chrome` — and go on with the \
ticket). If it truly can't be done from here, put the ticket down with one line in plain, non-technical words on what is blocked and \
what the person has to do outside a terminal (grant access, issue a key) — no commands."
    )
}

/// Judge what a member put down for the person: refuse the internal steps,
/// scrub the rest. Errs when nothing is left to ask.
pub fn judge(needs: &[String]) -> Result<Judged, String> {
    let (refused, kept): (Vec<&String>, Vec<&String>) = needs.iter().partition(|n| is_chore(n));
    if refused.is_empty() {
        let asks: Vec<String> = needs.iter().map(|n| scrub(n)).filter(|n| !n.is_empty()).collect();
        return if asks.is_empty() { Err(refusal(&needs.iter().map(String::as_str).collect::<Vec<_>>())) } else { Ok(Judged { asks, told: None }) };
    }
    let refused: Vec<&str> = refused.into_iter().map(String::as_str).collect();
    let asks: Vec<String> = kept.into_iter().map(|n| scrub(n)).filter(|n| !n.is_empty()).collect();
    if asks.is_empty() {
        return Err(refusal(&refused));
    }
    Ok(Judged { asks, told: Some(refusal(&refused)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stalled_ask_is_an_internal_step() {
        assert!(is_chore("Run `npx playwright install --with-deps chrome` in a terminal and enter your sudo password."));
        assert!(is_chore("Run npx playwright install --with-deps chrome"));
        assert!(is_chore("Open a terminal and install Node."));
        assert!(is_chore("Enter your sudo password when it asks."));
        assert!(is_chore("`brew install ffmpeg`"));
    }

    #[test]
    fn the_translated_asks_are_internal_steps_too() {
        assert!(is_chore("터미널을 열고 크롬을 설치해 주세요."));
        assert!(is_chore("관리자 비밀번호를 입력해 주세요."));
        assert!(is_chore("Ouvrez un terminal et installez Chrome."));
    }

    #[test]
    fn what_reaches_outward_or_is_a_credential_stays_the_persons() {
        assert!(!is_chore("Issue a GitHub token and paste it in the card."));
        assert!(!is_chore("Run `npx vercel login` and sign in to your account."));
        assert!(!is_chore("Share the 3 sheets with the service account."));
        assert!(!is_chore("Approve the DNS change for the domain."));
        assert!(!is_chore("Look at `outputs/T-12/shots/` and tell me which you like."));
        assert!(!is_chore("Connect your Stripe account."));
    }

    #[test]
    fn a_sudo_password_is_never_the_persons_even_beside_a_credential() {
        assert!(is_chore("Sign in to your account, then enter your sudo password."));
    }

    #[test]
    fn scrub_drops_commands_and_keeps_paths() {
        assert_eq!(scrub("Open `outputs/T-12/shots/` and run `npm run dev` to see it."), "Open `outputs/T-12/shots/` and run to see it.");
        assert_eq!(scrub("`npx playwright install chrome`"), "");
        assert_eq!(scrub("Check ```bash\nnpm install\n``` first"), "Check first");
        assert_eq!(scrub("Issue a token for `GITHUB_TOKEN`."), "Issue a token for `GITHUB_TOKEN`.");
    }

    #[test]
    fn judge_refuses_all_chores_with_guidance() {
        let err = judge(&["Run `npx playwright install --with-deps chrome` in a terminal and enter your sudo password.".into()]).unwrap_err();
        assert!(err.contains("do it yourself") && err.contains("never ask them to open a terminal"));
    }

    #[test]
    fn judge_keeps_real_asks_and_tells_the_member_about_the_rest() {
        let j = judge(&["Install Chrome with `sudo apt install chromium`.".into(), "Issue a Stripe API key.".into()]).unwrap();
        assert_eq!(j.asks, ["Issue a Stripe API key."]);
        assert!(j.told.unwrap().contains("do it yourself"));
    }

    #[test]
    fn judge_scrubs_a_command_from_a_real_ask() {
        let j = judge(&["Issue a token at the dashboard (`curl -H auth https://api.example.com`).".into()]).unwrap();
        assert!(!j.asks[0].contains("curl"));
        assert!(j.told.is_none());
    }
}
