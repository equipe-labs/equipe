/**
 * Equipe core domain types.
 *
 * The model is deliberately small: a **squad** of role-bound **agents** pull
 * **tickets** off a shared **board**. Each agent executes through a swappable
 * **engine** (ACP-driven coding agent, local CLI, or a thin LLM call). An
 * **orchestrator** (the director) plans, assigns, and gates work with humans.
 */

// ── Roles ────────────────────────────────────────────────────────────────────

/** The built-in squad roles. A project rarely needs more than these. */
export const ROLES = [
  "po", // product owner — turns intent into specs and tickets
  "designer", // UX/UI — flows, wireframes, design tokens
  "frontend", // frontend engineer
  "backend", // backend engineer
  "qa", // quality — tests, review, acceptance
  "devops", // build, deploy, infra
] as const;

export type RoleId = (typeof ROLES)[number];

/** A role definition: who the agent is and what it is allowed to touch. */
export interface RoleSpec {
  id: RoleId;
  title: string;
  /** System prompt fragment defining the role's mandate and voice. */
  charter: string;
  /** Toolset ids this role may use (maps to engine capabilities). */
  toolsets: string[];
  /** Whether this role edits code in a git worktree, or only produces artifacts. */
  writesCode: boolean;
  /** Preferred engine for this role; falls back to the squad default. */
  engine?: EngineRef;
}

// ── Engines (the "worker") ─────────────────────────────────────────────────────

/**
 * How an agent's turns actually execute. Equipe does not ship its own agent
 * loop for MVP — it drives mature engines and owns only the orchestration.
 */
export type EngineKind =
  | "acp" // drive an external agent over the Agent Client Protocol
  | "cli" // spawn a local coding-agent CLI process
  | "llm"; // a thin provider call (for non-coding roles)

export interface EngineRef {
  kind: EngineKind;
  /** e.g. "codex", "claude-code", "gemini" — the concrete backend. */
  backend: string;
  /** Provider/auth profile id this engine authenticates with. */
  provider: ProviderId;
  /** Model ref, e.g. "openai/gpt-5.6". Empty → resolved from provider default. */
  model?: string;
}

// ── Providers & auth ───────────────────────────────────────────────────────────

export type ProviderId = "openai" | "anthropic" | "google" | "local" | (string & {});

export type AuthMethod = "oauth-subscription" | "api-key" | "none";

/**
 * Normalized credentials. OAuth-subscription is the headline path for MVP:
 * the user logs in with their own ChatGPT Plus/Pro (Codex) subscription.
 */
export interface AuthCredentials {
  provider: ProviderId;
  method: AuthMethod;
  /** Short-lived access token (JWT for OAuth). */
  access: string;
  /** Refresh token, when the method supports it. */
  refresh?: string;
  /** Unix epoch seconds at which `access` expires. */
  expires?: number;
  /** For OAuth: the account id extracted from the token (e.g. chatgpt_account_id). */
  accountId?: string;
}

// ── Board (single source of truth) ───────────────────────────────────────────

export type TicketStatus =
  | "backlog"
  | "todo"
  | "in_progress"
  | "in_review"
  | "blocked"
  | "done"
  | "cancelled";

export interface Ticket {
  id: string;
  title: string;
  body: string;
  status: TicketStatus;
  /** Role expected to own this ticket. */
  role: RoleId | null;
  /** Agent instance currently holding it, if any. */
  assignee: string | null;
  /** Ticket ids that must be `done` before this one can start. */
  dependsOn: string[];
  /** Git branch / worktree associated with the work, if code. */
  branch: string | null;
  createdAt: number;
  updatedAt: number;
}

/** An immutable decision/handoff record — how agents communicate via the board. */
export interface BoardEvent {
  id: string;
  ticketId: string | null;
  /** Agent or "orchestrator" or "human". */
  actor: string;
  kind: "created" | "assigned" | "note" | "handoff" | "status" | "approval" | "artifact";
  message: string;
  at: number;
}

// ── Squad configuration ────────────────────────────────────────────────────────

export interface SquadConfig {
  /** Project name/slug for this squad's workspace. */
  project: string;
  /** Absolute path to the git repo the squad builds in. */
  repo: string;
  /** Default engine when a role does not specify one. */
  defaultEngine: EngineRef;
  /** Which roles are active for this project. */
  roles: RoleId[];
  /** Human-in-the-loop: statuses that require human approval before advancing. */
  approvalGates: TicketStatus[];
}
