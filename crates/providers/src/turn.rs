//! The outcome of one model turn, whichever engine ran it, and what it shows
//! while it runs.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Input read from the prompt cache or written to it: most of what a
    /// long conversation sends.
    pub cache_tokens: Option<u64>,
    /// How full the conversation's context was at the turn's last request.
    pub context_tokens: Option<u64>,
    /// The model's context window.
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnResult {
    /// The assistant's text for this turn.
    pub text: String,
    /// Terminal status reported by the engine, e.g. `completed` or `incomplete`.
    pub status: String,
    pub usage: Option<TurnUsage>,
}

/// Something worth showing while a turn runs.
#[derive(Debug, Clone, PartialEq)]
pub enum Activity {
    /// A fragment of the answer.
    Text(String),
    /// The engine is using a tool, e.g. `Edit` on a file. Codex's are named
    /// as Claude Code's are — `Bash`, `Edit`, `Write`, `mcp__equipe__board` —
    /// so every screen shows both alike.
    Tool { name: String, target: Option<String> },
    /// A tool use came back (the `Tool` with this `name` and `target`).
    ToolDone(ToolDone),
    /// A fragment of the model's reasoning, as the engine shows it — not
    /// part of the answer.
    Thinking(String),
    /// The model the engine is running, reported at startup.
    Model(String),
    /// The Claude plan's usage windows: five-hour and seven-day, each as the
    /// share used (0–1) and when it resets (Unix seconds).
    Plan { five_hour: Option<(f64, i64)>, seven_day: Option<(f64, i64)> },
    /// The Codex plan's usage limits, as Codex last reported them.
    Limits(Limits),
    /// Output the turn has produced so far, in tokens.
    Output(u64),
    /// The member handed `what` to a subagent (Claude Code's Agent tool).
    SubagentStarted { id: String, what: String },
    /// A subagent's latest step: a tool and what it acts on.
    SubagentStep { id: String, step: String },
    /// A subagent finished and handed back what it found.
    SubagentEnded { id: String },
    /// A message sent while the turn ran was read, at this point of it.
    Steered(String),
}

/// How a tool use ended, as the engine answered it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDone {
    pub name: String,
    pub target: Option<String>,
    pub ok: bool,
    /// A command's exit code, when it said one.
    pub exit: Option<i32>,
    pub ms: u64,
    /// What it printed (or answered), whole.
    pub output: String,
    /// An edit's or a write's change, as a unified diff headed `diff --git`.
    pub diff: Option<String>,
}


/// A plan's usage limits, as Codex reports them: its windows, of whatever
/// length, and the plan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Limits {
    pub windows: Vec<LimitWindow>,
    /// `plus`, `pro`, `free`…
    pub plan_type: Option<String>,
    /// Why the plan stopped answering, when it did: `rate_limit_reached`…
    pub reached: Option<String>,
    /// Credits beyond the plan: whether there are any, and the balance.
    pub credits: Option<(bool, Option<String>)>,
}

/// One usage window: how much is used (0–100), how long it is, and when it
/// starts over (Unix seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimitWindow {
    pub used_percent: f64,
    pub minutes: Option<i64>,
    pub resets_at: Option<i64>,
}

impl Limits {
    /// The window this many minutes long — by length, never by place.
    pub fn window(&self, minutes: i64) -> Option<&LimitWindow> {
        self.windows.iter().find(|w| w.minutes == Some(minutes))
    }

    /// Fold a sparse update in: what it says replaces, what it leaves out stays.
    pub fn merge(&mut self, update: Limits) {
        // A window is known by its length: an update about one leaves the others.
        for window in update.windows {
            match self.windows.iter_mut().find(|w| w.minutes == window.minutes) {
                Some(kept) => *kept = window,
                None => self.windows.push(window),
            }
        }
        if update.plan_type.is_some() {
            self.plan_type = update.plan_type;
        }
        if update.credits.is_some() {
            self.credits = update.credits;
        }
        self.reached = update.reached;
    }
}

/// Five hours and seven days, as windows are measured.
pub const FIVE_HOURS: i64 = 300;
pub const SEVEN_DAYS: i64 = 10_080;
