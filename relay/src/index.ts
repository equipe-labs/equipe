// The equipe relay: a mailman between `equipe daemon` and paired phones. It forwards
// sealed frames and sends pushes; it never holds a key that opens them.
// See docs/remote-protocol.md.

import { randomBytes, sha256Hex, toB64url, toHex } from "./encoding";
import type { Env } from "./env";
import { plain, route } from "./routes";

export { Daemon } from "./daemon";
export { RateLimit } from "./rate-limit";

export const REGISTRATIONS_PER_HOUR = 20;
const HOUR_MS = 60 * 60 * 1000;

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const target = route(new URL(request.url).pathname);
    if (!target) return plain(404, "not found");

    if (target.kind === "register") {
      return request.method === "POST" ? register(request, env) : plain(405, "use POST");
    }
    if (request.method !== "GET") return plain(405, "use GET");
    return env.DAEMONS.get(env.DAEMONS.idFromName(target.id)).fetch(request);
  },
} satisfies ExportedHandler<Env>;

async function register(request: Request, env: Env): Promise<Response> {
  const ip = request.headers.get("CF-Connecting-IP") ?? "unknown";
  const limiter = env.RATE_LIMITS.get(env.RATE_LIMITS.idFromName(ip));
  if (!(await limiter.hit(REGISTRATIONS_PER_HOUR, HOUR_MS))) {
    return plain(429, "too many registrations from this address; try again later");
  }

  const id = toHex(randomBytes(16));
  const token = toB64url(randomBytes(32));
  await env.DAEMONS.get(env.DAEMONS.idFromName(id)).register(await sha256Hex(token));
  return Response.json({ id, token }, { status: 201 });
}
