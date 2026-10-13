type RelayEnv = import("../src/env").Env;

declare namespace Cloudflare {
  interface Env extends RelayEnv {
    TEST_APNS_PUBLIC_KEY?: string;
    TEST_FCM_PUBLIC_KEY?: string;
  }
  interface GlobalProps {
    mainModule: typeof import("../src/index");
    durableNamespaces: "Daemon" | "RateLimit";
  }
}
