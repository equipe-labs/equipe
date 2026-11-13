# Releasing

Releases are cut by maintainers. A release is a tag `vX.Y.Z` that matches the
workspace `version` in `Cargo.toml`; pushing it runs
[`.github/workflows/release.yml`](../.github/workflows/release.yml).

## Normal release

1. Run the checks:

   ```sh
   cargo test --workspace --exclude equipe-desktop --locked
   cargo test -p equipe-desktop
   cargo clippy --workspace --exclude equipe-desktop --all-targets --locked -- -D warnings
   ```

2. Bump `version` in `Cargo.toml`, then run `cargo update -w` so `Cargo.lock` follows.
3. Commit `Release X.Y.Z`, tag `vX.Y.Z`, and push both:

   ```sh
   git push origin HEAD:main vX.Y.Z
   ```

The workflow builds the command line for four targets, builds, signs and
notarizes the desktop app (`equipe.dmg`, with its Sparkle `appcast.xml`),
installs the archives on clean macOS and Linux machines, then publishes the
GitHub Release and updates the Homebrew formula and cask. It usually takes
about 25 minutes. Most of that is the desktop app and Apple's notarization.

## Fast release

For an urgent command-line fix, put `[fast]` in the release commit's message:

```sh
git commit -m "Release X.Y.Z [fast]"
```

With `[fast]`, only the command line is built and published, which usually
takes about 10 minutes. The workflow skips:

- **The desktop app.** It is not rebuilt, signed or notarized. The release
  carries the previous release's `equipe.dmg`, `equipe.dmg.sha256` and
  `appcast.xml` unchanged. The appcast still names the previous version, so
  the app offers no update and downloads keep working. The Homebrew cask is
  left alone, and the formula is updated as usual.
- **The clean-machine install check** (`try-install`). Each build still runs
  its own `equipe --version` smoke test.

Use it only when the change doesn't need a new app: a change under
`crates/desktop` (or anything the app shows) reaches people with the next
normal release. The checks in step 1 still apply.

## Signing the command line on macOS

The macOS archives' `equipe` is signed in the `build` job with the same
Developer ID as the app (`MACOS_CERTIFICATE`, `MACOS_CERTIFICATE_PASSWORD`),
with Hardened Runtime and the identifier `equipe` — the same identity as the
copy the app carries (`equipe.app/Contents/Helpers/equipe`). Fast releases
sign it too.

Why it matters: macOS remembers the person's answers per code identity. An
ad-hoc binary's identity is its hash, so every update was a stranger: the
Keychain asked again for equipe's secrets ("Always Allow" didn't last), App
Management and Local Network answers were lost, and the login item was
announced again ("equipe can run in the background"). Signed, the designated
requirement is `identifier "equipe" and anchor apple generic and … certificate
leaf[subject.OU] = <team>` for every release, and one answer lasts.

Without the secrets (a fork), the job warns and ships the binary ad hoc;
everything still works, with macOS asking again after updates. Builds from
source are ad hoc as well: sign your own with
`codesign --force --options runtime -s "<identity>" -i equipe ~/.local/bin/equipe`
if the prompts bother you.

The binary also carries an Info.plist (`crates/cli/build.rs`, in its
`__TEXT,__info_plist` section) with its identifier, version and the reason
macOS shows when it asks about the local network on equipe's behalf.

Notarizing the bare binary isn't needed: `install.sh` and `equipe update`
download it without the quarantine flag, so Gatekeeper doesn't assess it.
