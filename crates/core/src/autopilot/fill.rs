//! What equipe runs in a page to finish a sign-in by itself — a code, a
//! "remember me" box, a passkey button, a device-code approval. Each script
//! first checks the page is where its guard lets it act (the login's exact
//! host, https: [`crate::vault::FillGuard`]) and answers what it did as JSON
//! after the fill mark (`vault::fill_answer` reads it) — never a value it
//! typed. The browser host runs it with the member's tools held off the tab.

use crate::vault::FillGuard;

/// A regular expression's alternatives from the locale files' words.
fn alternatives(key: &str, extra: &[&str]) -> String {
    let escape = |w: &str| {
        w.chars().fold(String::new(), |mut out, c| {
            if "\\^$.*+?()[]{}|/-".contains(c) {
                out.push('\\');
            }
            out.push(c);
            out
        })
    };
    let mut words: Vec<String> = crate::i18n::every(key).iter().filter(|w| !w.is_empty()).map(|w| escape(w)).collect();
    words.extend(extra.iter().map(|w| escape(w)));
    words.sort();
    words.dedup();
    words.join("|")
}

/// A JS function `(root) => n`: ticks the visible, unticked boxes in `root`
/// whose label says "remember me", "keep me signed in", "stay signed in",
/// "trust this device" (in any of equipe's languages), as a click would;
/// how many it ticked. Never a box about mail, terms or anything else.
pub fn remember_js() -> String {
    let words = serde_json::to_string(&alternatives("match.remember_words", &["remember me", "keep me signed in", "stay signed in", "keep me logged in", "trust this device", "trust this browser", "remember this device", "remember this browser", "don't ask again on this"])).unwrap_or_default();
    format!(
        r#"(root) => {{
  const RE = new RegExp({words}, 'i');
  const seen = (e) => {{ const r = e.getBoundingClientRect(); const s = getComputedStyle(e); return (r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none') || !!(e.labels && e.labels[0]); }};
  const label = (b) => [b.getAttribute('aria-label'), b.name, b.id, ...(b.labels ? Array.from(b.labels).map((l) => l.innerText) : []), (b.closest('label') || {{}}).innerText, b.parentElement && b.parentElement.innerText && b.parentElement.innerText.length < 120 ? b.parentElement.innerText : ''].filter(Boolean).join(' ');
  let ticked = 0;
  for (const b of Array.from((root || document).querySelectorAll('input[type=checkbox], [role=checkbox]'))) {{
    const on = b.checked === true || b.getAttribute('aria-checked') === 'true';
    if (on || b.disabled || !seen(b) || !RE.test(label(b))) continue;
    (b.labels && b.labels[0] && !seen(b) ? b.labels[0] : b).click();
    ticked++;
  }}
  return ticked;
}}"#
    )
}

/// The guard's checks, opening a script: refuse where it may not act.
fn guard_js(guard: &FillGuard) -> String {
    let hosts = serde_json::to_string(&guard.hosts).unwrap_or_default();
    let (mark_a, mark_b) = crate::vault::ANSWER_MARK.split_at(6);
    format!(
        r#"  const MARK = {mark_a:?} + {mark_b:?};
  const say = (what) => MARK + JSON.stringify(what);
  const HOSTS = {hosts}, SUB = {sub}, HTTP = {http}, REMEMBER = {remember};
  const host = location.hostname.toLowerCase().replace(/^www\./, '');
  const here = HOSTS.some((h) => host === h || (SUB && (host.endsWith('.' + h) || h.endsWith('.' + host))));
  if (!here) return say({{ refused: 'host', filled: [], submitted: false }});
  if (location.protocol !== 'https:' && !(HTTP && location.protocol === 'http:')) return say({{ refused: 'insecure', filled: [], submitted: false }});
  const shown = (el) => {{ const r = el.getBoundingClientRect(); const cs = getComputedStyle(el); return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && !el.disabled && !el.readOnly; }};
  const set = (el, v) => {{
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(el, v);
    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  }};
  const submit = (el) => {{
    const form = el && el.form;
    const button = (form || document).querySelector('button[type=submit], input[type=submit], button:not([type])');
    if (form && form.requestSubmit) {{ try {{ form.requestSubmit(button || undefined); return true; }} catch (e) {{}} }}
    if (button) {{ button.click(); return true; }}
    return false;
  }};
"#,
        sub = guard.subdomains,
        http = guard.http,
        remember = guard.remember,
    )
}

/// Type a one-time `code` into the page's code field — one field, or a row
/// of one-character boxes — tick "trust this device" where the login lets
/// it, and submit. Answers `step: "code"`.
pub fn code_script(code: &str, guard: &FillGuard) -> String {
    let c = serde_json::to_string(code).unwrap_or_default();
    format!(
        r#"() => {{
{guard}
  const C = {c};
  const inputs = Array.from(document.querySelectorAll('input')).filter(shown).filter((i) => ['text', 'tel', 'number', ''].includes(i.type));
  const named = (i) => i.autocomplete === 'one-time-code' || /(^|[^a-z])(otp|totp|mfa|2fa|code|token|pin|verification)([^a-z]|$)/i.test(`${{i.name}} ${{i.id}} ${{i.getAttribute('aria-label') || ''}} ${{i.placeholder || ''}}`);
  const boxes = inputs.filter((i) => i.maxLength === 1);
  let filled = [];
  let last = null;
  if (boxes.length === C.length && C.length > 1) {{
    boxes.forEach((b, k) => {{ b.focus(); set(b, C[k]); }});
    last = boxes[boxes.length - 1];
    filled.push('code');
  }} else {{
    const field = inputs.find(named) || inputs.find((i) => (i.maxLength >= 4 && i.maxLength <= 10) || i.inputMode === 'numeric');
    if (!field) return say({{ filled: [], submitted: false, step: 'code' }});
    field.focus(); set(field, C); last = field; filled.push('code');
  }}
  const remembered = REMEMBER ? ({remember})(last.form || document) : 0;
  const submitted = submit(last);
  return say({{ filled, submitted, step: 'code', remembered }});
}}"#,
        guard = guard_js(guard),
        remember = remember_js(),
    )
}

/// Press the page's "Sign in with a passkey" / "Use security key" button
/// (equipe's virtual authenticator, attached for the moment, answers it).
/// Answers `step: "passkey"`.
pub fn passkey_script(guard: &FillGuard) -> String {
    let words = serde_json::to_string(&alternatives("match.passkey_words", &["passkey", "security key", "use your device", "webauthn"])).unwrap_or_default();
    format!(
        r#"() => {{
{guard}
  const RE = new RegExp({words}, 'i');
  const button = Array.from(document.querySelectorAll('button, a[role=button], [role=button], input[type=submit], input[type=button]')).filter(shown).find((b) => RE.test(`${{b.innerText || ''}} ${{b.value || ''}} ${{b.getAttribute('aria-label') || ''}}`));
  if (!button) return say({{ filled: [], submitted: false, step: 'passkey' }});
  const remembered = REMEMBER ? ({remember})(button.form || document) : 0;
  button.click();
  return say({{ filled: ['passkey'], submitted: true, step: 'passkey', remembered }});
}}"#,
        guard = guard_js(guard),
        remember = remember_js(),
    )
}

/// A device-code approval page, on one of the hosts the person named for
/// it: type `code` where the page asks for one; else, where the page shows
/// that very code (or no code was given: a sign-in equipe's own CLI opened
/// in this browser), press its approve button. Answers `step: "device"`.
pub fn device_script(code: Option<&str>, guard: &FillGuard) -> String {
    let c = serde_json::to_string(&code.unwrap_or_default()).unwrap_or_default();
    let approve = serde_json::to_string(&alternatives("match.approve_words", &["authorize", "allow", "approve", "confirm", "continue", "yes", "accept"])).unwrap_or_default();
    format!(
        r#"() => {{
{guard}
  const C = {c};
  const flat = (t) => String(t || '').toUpperCase().replace(/[^A-Z0-9]/g, '');
  const inputs = Array.from(document.querySelectorAll('input')).filter(shown).filter((i) => ['text', 'tel', ''].includes(i.type));
  const boxes = inputs.filter((i) => i.maxLength === 1);
  const want = flat(C);
  if (want && boxes.length >= want.length - 1 && boxes.length > 1 && boxes.every((b) => !b.value)) {{
    const chars = boxes.length === want.length ? want : C.replace(/\s/g, '');
    boxes.forEach((b, k) => {{ b.focus(); set(b, chars[k] || ''); }});
    return say({{ filled: ['code'], submitted: submit(boxes[boxes.length - 1]), step: 'device' }});
  }}
  const field = inputs.find((i) => /code|user_code|otc/i.test(`${{i.name}} ${{i.id}} ${{i.placeholder || ''}} ${{i.getAttribute('aria-label') || ''}}`) && !i.value);
  if (want && field) {{
    field.focus(); set(field, C);
    return say({{ filled: ['code'], submitted: submit(field), step: 'device' }});
  }}
  // Approve only what this sign-in asked: the page shows its code, or equipe's own CLI opened this page.
  const page = flat(document.body && document.body.innerText);
  if (want && !page.includes(want)) return say({{ filled: [], submitted: false, step: 'device', refused: 'code' }});
  const RE = new RegExp('^\\s*(' + {approve} + ')', 'i');
  const button = Array.from(document.querySelectorAll('button, input[type=submit], [role=button]')).filter(shown).find((b) => RE.test(`${{b.innerText || b.value || ''}}`) && !/cancel|deny|reject/i.test(`${{b.innerText || b.value || ''}}`));
  if (!button) return say({{ filled: [], submitted: false, step: 'device' }});
  button.click();
  return say({{ filled: ['approve'], submitted: true, step: 'device' }});
}}"#,
        guard = guard_js(guard),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> FillGuard {
        FillGuard { hosts: vec!["github.com".into()], subdomains: false, http: false, remember: true }
    }

    #[test]
    fn every_script_checks_the_host_before_it_touches_anything_and_never_carries_the_mark() {
        for script in [code_script("123456", &guard()), passkey_script(&guard()), device_script(Some("ABCD-1234"), &guard())] {
            let host_check = script.find("refused: 'host'").unwrap();
            assert!(host_check < script.find("querySelectorAll").unwrap(), "{script}");
            assert!(!script.contains(crate::vault::ANSWER_MARK), "the mark is put together as it runs");
            assert!(script.contains(r#"const HOSTS = ["github.com"], SUB = false, HTTP = false, REMEMBER = true;"#));
        }
        assert!(code_script("12\"34", &guard()).contains(r#"const C = "12\"34";"#), "the code goes in as JSON");
        assert!(device_script(None, &guard()).contains(r#"const C = "";"#));
    }

    /// Each script is JavaScript a browser takes (checked with Node where this machine has it).
    #[test]
    fn every_script_parses() {
        use std::io::Write as _;
        let login = crate::protocol::vault::Login { id: 1, site: "github.com".into(), url: None, username: "a".into(), note: None, allowed: Vec::new(), auto: true, stopped: None, created_at: 0, password_at: None, uses: Vec::new(), autopilot: Default::default() };
        let scripts = [code_script("123456", &guard()), passkey_script(&guard()), device_script(Some("ABCD-1234"), &guard()), device_script(None, &guard()), crate::vault::fill_script("a", "p", &FillGuard::exact(&login)), remember_js()];
        for script in scripts {
            let Ok(mut node) = std::process::Command::new("node").args(["-e", "new Function('return (' + require('fs').readFileSync(0, 'utf8') + ')')"]).stdin(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn() else { return };
            node.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
            let out = node.wait_with_output().unwrap();
            assert!(out.status.success(), "{}\n{script}", String::from_utf8_lossy(&out.stderr));
        }
    }

    #[test]
    fn remember_me_is_found_in_every_language_and_only_those_boxes() {
        let js = remember_js();
        assert!(js.contains("remember me") && js.contains("trust this device"), "{js}");
        assert!(!js.contains("newsletter"));
    }
}
