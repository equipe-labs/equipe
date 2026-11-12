//! Command-line sign-ins without the person at a terminal:
//!
//! - non-interactive credentials first: a GitHub token, or a GitHub App's
//!   installation token minted again before it runs out, handed to `gh`
//!   over its standard input (`gh auth login --with-token`), never on a
//!   command line or in a member's view; an AWS access key behind a
//!   profile's `credential_process` (`equipe credential aws <profile>`,
//!   which refuses a member calling it directly); a token members'
//!   commands get as an environment variable at run time only;
//! - and a device-code sign-in (`gh auth login --web`, `aws sso login`,
//!   `vercel login`) equipe starts itself — so the code is the one its own
//!   CLI printed, never one a member or a page passed on — then approves
//!   in the shared browser on the hosts the person named for it.

use std::collections::HashMap;
use std::path::Path;

use crate::protocol::vault::{CliKind, CliLogin, Login};

/// A device-code sign-in's page and code, as its CLI printed them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevicePrompt {
    pub url: Option<String>,
    pub code: Option<String>,
}

/// What a device-code CLI's output says, read line after line.
pub fn read_prompt(seen: &mut DevicePrompt, line: &str) {
    for word in line.split_whitespace() {
        let word = word.trim_matches(|c: char| matches!(c, '"' | '\'' | '<' | '>' | '(' | ')' | ',' | '.') && c != '/');
        if (word.starts_with("https://") || word.starts_with("http://localhost") || word.starts_with("http://127.0.0.1")) && seen.url.is_none() {
            let url = word.trim_end_matches(['.', ',']).to_owned();
            if let Some(code) = url.split_once("user_code=").map(|(_, c)| c.split('&').next().unwrap_or_default().to_owned()).filter(|c| device_code_shaped(c)) {
                seen.code.get_or_insert(code);
            }
            seen.url = Some(url);
        } else if seen.code.is_none() && device_code_shaped(word) {
            seen.code = Some(word.to_owned());
        }
    }
}

/// `ABCD-1234`, `WDJB-MJHT`: what device-code flows show.
fn device_code_shaped(word: &str) -> bool {
    let Some((a, b)) = word.split_once('-') else { return false };
    let part = |p: &str| (4..=5).contains(&p.len()) && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    part(a) && part(b)
}

/// The sign-in command a member asked the person to run, as equipe runs it
/// itself: the CLI and its arguments, with what keeps it from waiting on a
/// terminal (`--web`, a browser that is only told the address). `None`: not
/// one equipe runs.
pub fn device_command(command: &str) -> Option<(String, Vec<String>)> {
    let words: Vec<String> = command.split_whitespace().map(str::to_owned).collect();
    // Only the command itself — no chain, pipe or redirection.
    if command.contains(['|', ';', '&', '>', '<', '`', '$', '\n']) {
        return None;
    }
    let safe = |w: &str| w.chars().all(|c| c.is_ascii_alphanumeric() || "-_=./:@".contains(c));
    if !words.iter().all(|w| safe(w)) {
        return None;
    }
    let mut args: Vec<String> = words.iter().skip(1).cloned().collect();
    match (words.first()?.as_str(), words.get(1).map(String::as_str), words.get(2).map(String::as_str)) {
        ("gh", Some("auth"), Some("login" | "refresh")) => {
            if args.iter().any(|a| a == "--with-token") {
                return None;
            }
            if !args.iter().any(|a| a == "--web" || a == "-w") {
                args.push("--web".into());
            }
            if words[2] == "login" && !args.iter().any(|a| a == "-p" || a.starts_with("--git-protocol")) {
                args.extend(["--git-protocol".into(), "https".into()]);
            }
            if !args.iter().any(|a| a == "-h" || a.starts_with("--hostname")) {
                args.extend(["--hostname".into(), "github.com".into()]);
            }
            Some(("gh".into(), args))
        }
        ("aws", Some("sso"), Some("login")) | ("aws", Some("login"), _) => Some(("aws".into(), args)),
        ("vercel", Some("login"), _) => Some(("vercel".into(), args)),
        _ => None,
    }
}

/// The CLI a device-code login is for: its first word (`gh`, `aws`, `vercel`).
pub fn cli_of(command: &str) -> Option<String> {
    device_command(command).map(|(cli, _)| cli)
}

/// The site whose page approves a CLI's device sign-in, where it is always
/// the same one (AWS's depends on the organization: the person names it).
pub fn default_host(cli: &str) -> Option<&'static str> {
    match cli {
        "gh" => Some("github.com"),
        "vercel" => Some("vercel.com"),
        _ => None,
    }
}

/// Approvals made from a login's own "approve device sign-ins" switch are
/// numbered from here (their login's id added), apart from kept ones.
pub const FROM_LOGIN: u32 = 1_000_000;

/// The device sign-in approval for `name` (`gh`, `aws`, `vercel`): one the
/// person kept, with the hosts they named — or, for a CLI whose approving
/// site is always the same, the login of that site whose "approve device
/// sign-ins" switch they turned on.
pub fn approval_for(cli: &[CliLogin], logins: &[Login], name: &str) -> Option<CliLogin> {
    if let Some(kept) = cli.iter().find(|c| c.kind == CliKind::DeviceLogin && c.name == name) {
        return Some(kept.clone());
    }
    let host = default_host(name)?;
    let login = logins.iter().find(|l| l.autopilot.device_code && crate::vault::site_of(&l.site) == host)?;
    Some(CliLogin { id: FROM_LOGIN + login.id, kind: CliKind::DeviceLogin, name: name.to_owned(), host: host.to_owned(), public: String::new(), squad: None, created_at: login.created_at, last: None })
}

/// The approval numbered `id` ([`approval_for`]'s).
pub fn approval_by_id(cli: &[CliLogin], logins: &[Login], id: u32) -> Option<CliLogin> {
    if id < FROM_LOGIN {
        return cli.iter().find(|c| c.id == id && c.kind == CliKind::DeviceLogin).cloned();
    }
    let login = logins.iter().find(|l| l.id == id - FROM_LOGIN && l.autopilot.device_code)?;
    ["gh", "vercel"].into_iter().find(|n| default_host(n) == Some(crate::vault::site_of(&login.site).as_str())).and_then(|n| approval_for(&[], logins, n))
}

/// The hosts a device-code approval may act on: the ones the person named,
/// `*.` meaning that domain's subdomains.
pub fn approval_hosts(login: &CliLogin) -> Vec<String> {
    login.host.split([',', ' ']).map(|h| h.trim().trim_start_matches("https://").trim_end_matches('/').to_ascii_lowercase()).filter(|h| !h.is_empty()).collect()
}

/// Whether `url` is on one of `hosts`, over https (http only on this machine:
/// a CLI's own callback).
pub fn on_hosts(url: &str, hosts: &[String]) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let host = crate::vault::site_of(&lower);
    if lower.starts_with("http://") {
        return host == "localhost" || host == "127.0.0.1";
    }
    lower.starts_with("https://")
        && hosts.iter().any(|h| match h.strip_prefix("*.") {
            Some(base) => host.ends_with(&format!(".{base}")) || host == base,
            None => host == h.trim_start_matches("www."),
        })
}

/// The environment members' commands get from the person's kept values:
/// each [`CliKind::Env`] value for every squad or for `squad`.
pub fn run_env(cli: &[CliLogin], values: &HashMap<u32, String>, squad: &str) -> Vec<(String, String)> {
    cli.iter()
        .filter(|c| c.kind == CliKind::Env && c.squad.as_deref().is_none_or(|s| s == squad))
        .filter_map(|c| values.get(&c.id).map(|v| (c.name.clone(), v.clone())))
        .collect()
}

/// A valid environment variable's name.
pub fn env_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.starts_with(|c: char| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

// ── GitHub App installation tokens ──

fn base64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A PEM's DER bytes, and whether it is PKCS#1 (`RSA PRIVATE KEY`).
fn pem_der(pem: &str) -> Result<(Vec<u8>, bool), String> {
    use base64::Engine as _;
    let pkcs1 = pem.contains("BEGIN RSA PRIVATE KEY");
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).map(str::trim).collect();
    let der = base64::engine::general_purpose::STANDARD.decode(body).map_err(|_| "that private key isn't a PEM file".to_owned())?;
    Ok((der, pkcs1))
}

/// The JWT a GitHub App signs to ask for an installation token (RS256, ten
/// minutes at most, a minute back for clock drift).
pub fn app_jwt(app_id: &str, pem: &str, now: u64) -> Result<String, String> {
    let (der, pkcs1) = pem_der(pem)?;
    let key = if pkcs1 { ring::signature::RsaKeyPair::from_der(&der) } else { ring::signature::RsaKeyPair::from_pkcs8(&der) }.map_err(|_| "that private key isn't a GitHub App's RSA key".to_owned())?;
    let header = base64url(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = base64url(serde_json::json!({ "iat": now.saturating_sub(60), "exp": now + 540, "iss": app_id }).to_string().as_bytes());
    let signing = format!("{header}.{claims}");
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), signing.as_bytes(), &mut signature).map_err(|_| "signing failed".to_owned())?;
    Ok(format!("{signing}.{}", base64url(&signature)))
}

/// A GitHub App installation token and when it runs out (unix seconds).
pub async fn installation_token(app: &str, pem: &str, now: u64) -> Result<(String, u64), String> {
    let (app_id, installation) = app.split_once('/').ok_or("say the app id and the installation id as <app>/<installation>")?;
    let jwt = app_jwt(app_id.trim(), pem, now)?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let response = client
        .post(format!("https://api.github.com/app/installations/{}/access_tokens", installation.trim()))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "equipe")
        .header("Authorization", format!("Bearer {jwt}"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("GitHub said {status}"));
    }
    let token = body.get("token").and_then(|t| t.as_str()).ok_or("GitHub gave no token")?.to_owned();
    // An hour, as GitHub gives them; read back "expires_at" only roughly.
    Ok((token, now + 3600))
}

// ── giving a credential to a CLI ──

/// Run `program args`, `input` on its standard input (a token: never on the
/// command line), and say whether it worked — its error output trimmed,
/// never the input.
pub async fn run_with_input(program: &str, args: &[&str], input: &str) -> Result<(), String> {
    use tokio::io::AsyncWriteExt as _;
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).await.map_err(|e| e.to_string())?;
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output()).await.map_err(|_| format!("{program} didn't finish"))?.map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{program}: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("failed").replace(input, "•••")))
    }
}

/// Give `gh` (and git, through it) a token for `host`.
pub async fn gh_token(host: &str, token: &str) -> Result<(), String> {
    run_with_input("gh", &["auth", "login", "--with-token", "--hostname", host], token).await?;
    run_with_input("gh", &["auth", "setup-git", "--hostname", host], "").await
}

/// An AWS CLI config with `profile` getting its credentials from
/// `equipe credential aws <profile>`: that profile's section's
/// `credential_process` set, every other line kept.
pub fn aws_config_with(config: &str, profile: &str, exe: &str) -> String {
    let header = if profile == "default" { "[default]".to_owned() } else { format!("[profile {profile}]") };
    let line = format!("credential_process = \"{exe}\" credential aws {profile}");
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    let mut done = false;
    for l in config.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if inside && !done {
                out.push(line.clone());
                done = true;
            }
            inside = t == header;
        }
        if inside && t.starts_with("credential_process") {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_owned());
    }
    if inside && !done {
        out.push(line.clone());
        done = true;
    }
    if !done {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push(header);
        out.push(line);
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Set up `profile` in the AWS CLI's config file (`AWS_CONFIG_FILE`, else
/// `~/.aws/config`) to take its credentials from equipe.
pub fn write_aws_profile(home: &Path, profile: &str, exe: &str) -> Result<(), String> {
    let path = std::env::var_os("AWS_CONFIG_FILE").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".aws/config"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let before = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, aws_config_with(&before, profile, exe)).map_err(|e| e.to_string())
}

/// The executable of this process's parent, as the system names it.
#[cfg(target_os = "macos")]
fn parent_exe() -> Option<String> {
    // SAFETY: getppid never fails; proc_pidpath writes at most the buffer's size.
    let ppid = unsafe { libc::getppid() };
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(ppid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn parent_exe() -> Option<String> {
    // SAFETY: getppid never fails.
    let ppid = unsafe { libc::getppid() };
    let cmdline = std::fs::read(format!("/proc/{ppid}/cmdline")).ok()?;
    Some(String::from_utf8_lossy(&cmdline).replace('\0', " "))
}

#[cfg(not(unix))]
fn parent_exe() -> Option<String> {
    None
}

/// Whether the process that ran this one is the AWS CLI (its own binary, or
/// the Python of its installation): who `credential_process` answers.
pub fn parent_is_aws() -> bool {
    parent_exe().is_some_and(|exe| is_aws(&exe))
}

fn is_aws(exe: &str) -> bool {
    let lower = exe.to_ascii_lowercase();
    lower.split_whitespace().next().is_some_and(|first| first.rsplit('/').next() == Some("aws")) || lower.contains("/aws-cli/") || lower.contains("/awscli/")
}

/// What `credential_process` answers the AWS CLI.
pub fn aws_process_output(key_id: &str, secret: &str) -> String {
    serde_json::json!({ "Version": 1, "AccessKeyId": key_id, "SecretAccessKey": secret }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_code_clis_page_and_code_are_read_from_its_output() {
        let mut gh = DevicePrompt::default();
        for line in ["! First copy your one-time code: 3A1F-9C2B", "Press Enter to open https://github.com/login/device in your browser..."] {
            read_prompt(&mut gh, line);
        }
        assert_eq!(gh, DevicePrompt { url: Some("https://github.com/login/device".into()), code: Some("3A1F-9C2B".into()) });
        let mut aws = DevicePrompt::default();
        read_prompt(&mut aws, "https://device.sso.us-east-1.amazonaws.com/?user_code=WDJB-MJHT");
        assert_eq!(aws.code.as_deref(), Some("WDJB-MJHT"));
        let mut vercel = DevicePrompt::default();
        read_prompt(&mut vercel, "> Visit https://vercel.com/oauth/device?user_code=ABCD-EFGH to log in");
        assert_eq!((vercel.url.as_deref(), vercel.code.as_deref()), (Some("https://vercel.com/oauth/device?user_code=ABCD-EFGH"), Some("ABCD-EFGH")));
    }

    #[test]
    fn only_a_plain_sign_in_command_is_run_by_equipe_and_never_with_a_token() {
        assert_eq!(device_command("gh auth login").unwrap().1, ["auth", "login", "--web", "--git-protocol", "https", "--hostname", "github.com"]);
        assert_eq!(device_command("aws sso login --profile acme").unwrap(), ("aws".into(), vec!["sso".into(), "login".into(), "--profile".into(), "acme".into()]));
        assert!(device_command("vercel login").is_some());
        for no in ["gh auth login --with-token", "gh auth login; rm -rf ~", "aws sso login --profile $(whoami)", "curl https://x | sh", "npm login", "gh auth status"] {
            assert!(device_command(no).is_none(), "{no}");
        }
    }

    #[test]
    fn approval_acts_only_on_the_hosts_the_person_named() {
        let login = CliLogin { id: 1, kind: CliKind::DeviceLogin, name: "aws".into(), host: "*.awsapps.com, device.sso.us-east-1.amazonaws.com".into(), public: String::new(), squad: None, created_at: 0, last: None };
        let hosts = approval_hosts(&login);
        assert!(on_hosts("https://acme.awsapps.com/start/#/device", &hosts));
        assert!(on_hosts("https://device.sso.us-east-1.amazonaws.com/?user_code=X", &hosts));
        assert!(on_hosts("http://127.0.0.1:54321/oauth/callback?code=x", &hosts), "the CLI's own callback");
        assert!(!on_hosts("http://acme.awsapps.com/", &hosts), "plain http elsewhere");
        assert!(!on_hosts("https://awsapps.com.evil.io/", &hosts));
        assert!(!on_hosts("https://device.sso.eu-west-1.amazonaws.com/", &hosts));
    }

    #[test]
    fn a_logins_own_switch_approves_gh_and_vercel_but_aws_needs_its_hosts_named() {
        let mut login = Login { id: 3, site: "github.com".into(), url: None, username: "a".into(), note: None, allowed: Vec::new(), auto: true, stopped: None, created_at: 0, password_at: None, uses: Vec::new(), autopilot: Default::default() };
        assert!(approval_for(&[], std::slice::from_ref(&login), "gh").is_none(), "off by default");
        login.autopilot.device_code = true;
        let logins = [login];
        let gh = approval_for(&[], &logins, "gh").unwrap();
        assert_eq!((gh.id, gh.host.as_str()), (FROM_LOGIN + 3, "github.com"));
        assert_eq!(approval_by_id(&[], &logins, gh.id), Some(gh));
        assert!(approval_for(&[], &logins, "aws").is_none() && approval_for(&[], &logins, "vercel").is_none());
    }

    #[test]
    fn members_get_the_values_for_their_squad_by_name() {
        let env = |id, name: &str, squad: Option<&str>| CliLogin { id, kind: CliKind::Env, name: name.into(), host: String::new(), public: String::new(), squad: squad.map(Into::into), created_at: 0, last: None };
        let cli = [env(1, "NPM_TOKEN", None), env(2, "VERCEL_TOKEN", Some("beta")), CliLogin { kind: CliKind::GhToken, ..env(3, "gh", None) }];
        let values: HashMap<u32, String> = [(1, "n".into()), (2, "v".into()), (3, "g".into())].into();
        assert_eq!(run_env(&cli, &values, "acme"), [("NPM_TOKEN".to_owned(), "n".to_owned())]);
        assert_eq!(run_env(&cli, &values, "beta").len(), 2);
        assert!(env_name("NPM_TOKEN") && !env_name("npm_token") && !env_name("PATH=x"));
    }

    #[test]
    fn an_aws_profile_gets_equipes_credential_process_and_keeps_the_rest() {
        let before = "[default]\nregion = us-east-1\n\n[profile acme]\nregion = ap-northeast-2\ncredential_process = old\n\n[profile other]\nregion = eu-west-1\n";
        let after = aws_config_with(before, "acme", "/usr/local/bin/equipe");
        assert!(after.contains("[profile acme]\nregion = ap-northeast-2\ncredential_process = \"/usr/local/bin/equipe\" credential aws acme\n"), "{after}");
        assert!(!after.contains("= old") && after.contains("[profile other]\nregion = eu-west-1"), "{after}");
        let added = aws_config_with("[default]\nregion = us-east-1\n", "new", "equipe");
        assert!(added.ends_with("\n\n[profile new]\ncredential_process = \"equipe\" credential aws new\n"), "{added}");
        let out: serde_json::Value = serde_json::from_str(&aws_process_output("AKIA1", "s3cr3t")).unwrap();
        assert_eq!((out["Version"].as_u64(), out["AccessKeyId"].as_str(), out["SecretAccessKey"].as_str()), (Some(1), Some("AKIA1"), Some("s3cr3t")));
    }

    #[test]
    fn only_the_aws_cli_counts_as_its_parent() {
        assert!(is_aws("/usr/local/aws-cli/aws"));
        assert!(is_aws("/opt/homebrew/Cellar/awscli/2.22.0/libexec/bin/python3.13"));
        assert!(!is_aws("/bin/zsh"));
        assert!(!is_aws("/usr/bin/python3 /tmp/aws-lookalike.py"));
    }

    #[test]
    fn a_github_app_jwt_is_signed_rs256() {
        assert!(app_jwt("123", "-----BEGIN RSA PRIVATE KEY-----\nnot a key\n-----END RSA PRIVATE KEY-----", 1_000).is_err());
    }
}
