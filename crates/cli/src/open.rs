//! `equipe open <link>`: open a shared link (`equipe://open/…`, or its
//! `https://equipe.im/o/#…` form, as pasted from a chat app) in the desktop
//! app — launching it, or bringing it forward on the place the link names.
//! The app registers the `equipe` URL scheme, so the system does the
//! launching; the app decides which of its machines and squads the link is
//! about (`crates/desktop/src/deeplink.rs`).

use std::process::Command;

use equipe_core::protocol::deeplink;
use equipe_core::t;

/// The `equipe://` form of whatever link `text` holds, in either form.
pub fn normalize(text: &str) -> Result<String, String> {
    deeplink::parse(text).map(|link| link.uri()).ok_or_else(|| t!("open.bad_link", text = text.trim()))
}

/// The program and arguments that hand `uri` to the desktop app.
fn launcher(uri: &str) -> Option<(&'static str, Vec<String>)> {
    if cfg!(target_os = "macos") {
        Some(("/usr/bin/open", vec![uri.to_owned()]))
    } else if cfg!(target_os = "linux") {
        Some(("xdg-open", vec![uri.to_owned()]))
    } else {
        None
    }
}

/// Open `text`; with `print`, only say the link in its `equipe://` form.
pub fn command(text: &str, print: bool) -> Result<(), String> {
    let uri = normalize(text)?;
    if print {
        println!("{uri}");
        return Ok(());
    }
    let Some((program, args)) = launcher(&uri) else { return Err(t!("open.unsupported", link = uri)) };
    let status = Command::new(program).args(&args).status().map_err(|_| t!("open.no_app", link = uri))?;
    if status.success() {
        println!("{}", t!("open.opening", link = uri));
        Ok(())
    } else {
        Err(t!("open.no_app", link = uri))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_forms_open_as_the_scheme_form() {
        assert_eq!(normalize("equipe://open/web/ticket/T-5?host=mini").as_deref(), Ok("equipe://open/web/ticket/T-5?host=mini"));
        assert_eq!(normalize("<https://equipe.im/o/#web/ticket/5?host=mini>.").as_deref(), Ok("equipe://open/web/ticket/T-5?host=mini"));
        assert_eq!(normalize("T-5 some title · equipe://open/web/status").as_deref(), Ok("equipe://open/web/status"));
    }

    #[test]
    fn a_word_that_is_no_link_is_refused_saying_so() {
        assert!(normalize("hello").is_err());
        assert!(normalize("https://example.com/").is_err());
    }

    #[test]
    fn the_system_launches_the_app() {
        if cfg!(target_os = "macos") {
            assert_eq!(launcher("equipe://open/web/status"), Some(("/usr/bin/open", vec!["equipe://open/web/status".to_owned()])));
        }
    }
}
