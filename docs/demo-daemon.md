# The demo daemon (store review)

App Review and Google Play reviewers have no Mac to pair with. `equipe demo serve` gives them
one: a daemon of its own, reachable through the relay like any other, with a pairing code that
never expires and a scripted squad behind it. Its rules are in
[remote-protocol.md › Demo pairing](remote-protocol.md#demo-pairing-store-review).

What a reviewer's phone gets, on its own copy:

- squad `demo`: PM, frontend and backend; projects `website` and `server`;
- a board with a ticket in each state — done, **in review** (accept or send back), in progress,
  **waiting** on the person (resume), to do, proposed;
- a **request for permission** waiting (frontend asks to run `npm run build`). Allowed, the
  member goes on and hands the landing page in; denied, it says so. Another request follows
  20 seconds after each answer — one waits at a time;
- the **PM answers** what it's told (in Korean when written to in Korean); asked for work
  ("add a FAQ page"), it puts a ticket on the board. Other members answer briefly.

No agent, engine or model runs: nothing costs anything and nothing a phone sends goes anywhere
but its own copy of the script. At most 200 phones are kept (the oldest goes first).

## Commands

```sh
equipe demo serve [--home DIR]      # run it, in the foreground; prints the code
equipe demo code  [--home DIR]      # the code again: QR and address
equipe demo reset [--home DIR]      # forget every phone, start the script over
equipe demo reset --new-code        # …and a new code: the old one stops working
```

`--home` defaults to `$EQUIPE_HOME/demo` (`~/.equipe/demo`). It holds `remote.json` (its relay
registration and keys, 0600), `demo.json` (the code's secret, 0600), `pairing-url.txt` and
`pairing-qr.svg` (the code, for the review forms), and `demo.sock` (for `reset`). The first
`serve` registers with the relay (`EQUIPE_RELAY`, else `https://relay.equipe.im`).

The code survives restarts and upgrades; only `reset --new-code` or deleting `demo.json`
changes it. After that, the review notes must get the new one.

## Where to run it

It has to be up for as long as a review can take (days, and again on every update), so it
runs as a service:

1. **The always-on Mac mini (recommended to start).** No cost, already up around the clock.
   The demo has its own home and relay identity and never talks to `equipe daemon`, so it can't
   reach the person's squads. A LaunchAgent keeps it running:

   ```xml
   <!-- ~/Library/LaunchAgents/im.equipe.demo.plist -->
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0"><dict>
     <key>Label</key><string>im.equipe.demo</string>
     <key>ProgramArguments</key><array>
       <string>/opt/homebrew/bin/equipe</string><string>demo</string><string>serve</string>
     </array>
     <key>RunAtLoad</key><true/>
     <key>KeepAlive</key><true/>
     <key>StandardOutPath</key><string>/tmp/equipe-demo.log</string>
     <key>StandardErrorPath</key><string>/tmp/equipe-demo.log</string>
   </dict></plist>
   ```

   `launchctl load ~/Library/LaunchAgents/im.equipe.demo.plist`; stop with `launchctl unload …`.
   (Point `ProgramArguments` at wherever `which equipe` says.)

2. **A small Linux VM** (any VPS or Fly.io machine; 256 MB is plenty) when it should be off the
   person's machine altogether. A systemd unit:

   ```ini
   # /etc/systemd/system/equipe-demo.service
   [Unit]
   Description=equipe demo daemon (store review)
   After=network-online.target
   [Service]
   ExecStart=/usr/local/bin/equipe demo serve --home /var/lib/equipe-demo
   Restart=always
   DynamicUser=yes
   StateDirectory=equipe-demo
   [Install]
   WantedBy=multi-user.target
   ```

   Not tried on Linux yet: build and check it there first.

Either way, keep `demo.json` and `remote.json` backed up with the home — losing them means a
new code in the review notes.

## Is it up?

- The log says `demo: connected to the relay at …` after each (re)connection.
- Pair a phone with the code from `equipe demo code`; then `equipe demo reset` so reviewers
  start from the beginning.
- The whole way, without a phone, against a local relay (`npx wrangler dev` in `relay/`) and a
  demo pointed at it:
  `EQUIPE_E2E_RELAY=http://localhost:8787 EQUIPE_E2E_HOME=<its home> cargo test -p equipe e2e_demo -- --ignored`
  pairs two phones with the one code, allows the request and talks to the PM.

## For the review forms

- **App Store Connect › App Review Information › Notes:** "equipe pairs with the user's own
  computer. For review, scan this QR code (or paste the address into Settings › Pair with a
  computer): <code>. It connects to a demo computer with a sample team; no sign-in is needed."
  Attach `pairing-qr.svg` (as a PNG) or a link to it.
- **Play Console › App content › App access:** "All or some functionality is restricted";
  instructions as above, with the address from `pairing-url.txt` — Play asks for QR codes as an
  address.

Run `equipe demo reset` before each submission so a reviewer finds the script at its start.
