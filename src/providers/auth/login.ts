/** Interactive ChatGPT/Codex OAuth login: loopback callback + browser open. */

import { spawn } from "node:child_process";
import { createServer, type Server } from "node:http";
import type { AuthCredentials } from "../../core/types.js";
import {
  CALLBACK_PATH,
  CALLBACK_PORT,
  createAuthorizationRequest,
  exchangeCode,
} from "./codex-oauth.js";
import { saveCredentials } from "./store.js";

const LOGIN_TIMEOUT_MS = 5 * 60_000;

export interface LoginOptions {
  openBrowser?: boolean;
  onUrl?: (url: string) => void;
  timeoutMs?: number;
}

function openInBrowser(url: string): void {
  const platform = process.platform;
  const cmd = platform === "darwin" ? "open" : platform === "win32" ? "cmd" : "xdg-open";
  const args = platform === "win32" ? ["/c", "start", "", url] : [url];
  try {
    const child = spawn(cmd, args, { stdio: "ignore", detached: true });
    child.on("error", () => {}); // opener missing → user uses the printed URL
    child.unref();
  } catch {
    // ignore — the URL is printed for manual use
  }
}

function html(title: string, body: string): string {
  return `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title><style>
    body{font:16px system-ui;margin:0;display:grid;place-items:center;height:100vh;background:#0b0b0c;color:#eaeaea}
    .card{max-width:420px;padding:32px;text-align:center}
    h1{font-weight:500;font-size:20px}p{color:#a0a0a0}
  </style></head><body><div class="card"><h1>${title}</h1><p>${body}</p></div></body></html>`;
}

/** Run the full Codex OAuth login and persist the credentials. Returns them. */
export function runCodexLogin(options: LoginOptions = {}): Promise<AuthCredentials> {
  const req = createAuthorizationRequest();
  const timeoutMs = options.timeoutMs ?? LOGIN_TIMEOUT_MS;

  return new Promise<AuthCredentials>((resolve, reject) => {
    let settled = false;
    let server: Server | undefined;
    let timer: NodeJS.Timeout | undefined;

    const finish = (fn: () => void): void => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      server?.close();
      fn();
    };

    server = createServer((httpReq, res) => {
      const url = new URL(httpReq.url ?? "/", `http://localhost:${CALLBACK_PORT}`);
      if (url.pathname !== CALLBACK_PATH) {
        res.writeHead(404).end();
        return;
      }
      if (url.searchParams.get("state") !== req.state) {
        res.writeHead(400, { "Content-Type": "text/html" });
        res.end(html("Login failed", "State mismatch — please retry <code>equipe login</code>."));
        return;
      }
      const code = url.searchParams.get("code");
      if (!code) {
        const errDesc = url.searchParams.get("error_description") ?? "Missing authorization code.";
        res.writeHead(400, { "Content-Type": "text/html" });
        res.end(html("Login failed", errDesc));
        finish(() => reject(new Error(`OAuth callback error: ${errDesc}`)));
        return;
      }
      res.writeHead(200, { "Content-Type": "text/html" });
      res.end(html("You're signed in", "Return to your terminal — Equipe has your session."));
      exchangeCode(code, req.verifier, req.redirectUri)
        .then((cred) => {
          saveCredentials(cred);
          finish(() => resolve(cred));
        })
        .catch((err: unknown) => finish(() => reject(err instanceof Error ? err : new Error(String(err)))));
    });

    server.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "EADDRINUSE") {
        finish(() =>
          reject(
            new Error(
              `Port ${CALLBACK_PORT} is in use. Close whatever holds it (another login?) and retry.`,
            ),
          ),
        );
        return;
      }
      finish(() => reject(err));
    });

    server.listen(CALLBACK_PORT, "127.0.0.1", () => {
      options.onUrl?.(req.url);
      if (options.openBrowser !== false) openInBrowser(req.url);
    });

    timer = setTimeout(
      () => finish(() => reject(new Error("Login timed out. Retry `equipe login`."))),
      timeoutMs,
    );
  });
}
