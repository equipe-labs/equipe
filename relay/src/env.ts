import type { Daemon } from "./daemon";
import type { RateLimit } from "./rate-limit";

export interface Env {
  DAEMONS: DurableObjectNamespace<Daemon>;
  RATE_LIMITS: DurableObjectNamespace<RateLimit>;

  // Push secrets; each platform is off until all of its secrets are set.
  APNS_KEY?: string;
  APNS_KEY_ID?: string;
  APNS_TEAM_ID?: string;
  APNS_TOPIC?: string;
  FCM_SERVICE_ACCOUNT?: string;
}
