export type Route =
  | { kind: "register" }
  | { kind: "daemon"; id: string }
  | { kind: "device"; id: string; device: string }
  | { kind: "pair"; id: string; slot: string };

const HEX32 = "([0-9a-f]{32})";
const DAEMON = new RegExp(`^/v1/daemons/${HEX32}/connect$`);
const DEVICE = new RegExp(`^/v1/daemons/${HEX32}/devices/${HEX32}/connect$`);
const PAIR = new RegExp(`^/v1/daemons/${HEX32}/pair/${HEX32}$`);

export function route(pathname: string): Route | null {
  if (pathname === "/v1/daemons") return { kind: "register" };
  let m = DAEMON.exec(pathname);
  if (m) return { kind: "daemon", id: m[1]! };
  m = DEVICE.exec(pathname);
  if (m) return { kind: "device", id: m[1]!, device: m[2]! };
  m = PAIR.exec(pathname);
  if (m) return { kind: "pair", id: m[1]!, slot: m[2]! };
  return null;
}

export function bearer(request: Request): string | null {
  const m = /^Bearer\s+(\S+)$/i.exec(request.headers.get("Authorization") ?? "");
  return m ? m[1]! : null;
}

export function plain(status: number, message: string): Response {
  return new Response(message + "\n", { status, headers: { "content-type": "text/plain" } });
}
