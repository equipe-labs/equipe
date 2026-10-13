import { generateKeyPairSync } from "node:crypto";
import { cloudflareTest } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

// Throwaway keys made per run, so the push tests can check real signatures without
// any Apple or Google credentials.
const apns = generateKeyPairSync("ec", { namedCurve: "P-256" });
const fcm = generateKeyPairSync("rsa", { modulusLength: 2048 });
const pem = (key: { export(options: object): string | Buffer }, type: string) =>
  key.export({ type, format: "pem" }).toString();

const pushSecrets = {
  APNS_KEY: pem(apns.privateKey, "pkcs8"),
  APNS_KEY_ID: "TESTKEY123",
  APNS_TEAM_ID: "TESTTEAM12",
  APNS_TOPIC: "im.equipe.app",
  FCM_SERVICE_ACCOUNT: JSON.stringify({
    type: "service_account",
    project_id: "equipe-test",
    client_email: "relay@equipe-test.iam.gserviceaccount.com",
    private_key: pem(fcm.privateKey, "pkcs8"),
  }),
  TEST_APNS_PUBLIC_KEY: pem(apns.publicKey, "spki"),
  TEST_FCM_PUBLIC_KEY: pem(fcm.publicKey, "spki"),
};

const worker = (bindings: Record<string, string>) =>
  cloudflareTest({ wrangler: { configPath: "./wrangler.toml" }, miniflare: { bindings } });

export default defineConfig({
  test: {
    projects: [
      {
        plugins: [worker(pushSecrets)],
        test: { name: "relay", include: ["test/**/*.test.ts"], exclude: ["test/unconfigured.test.ts"] },
      },
      {
        plugins: [worker({})],
        test: { name: "unconfigured", include: ["test/unconfigured.test.ts"] },
      },
    ],
  },
});
