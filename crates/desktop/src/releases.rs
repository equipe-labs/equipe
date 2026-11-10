//! Each project's latest release — its newest tag — on whichever machine the
//! projects are, for the head of a project's page (`views/project.rs`).

use std::path::{Path, PathBuf};

use crate::target::Endpoint;

/// A project's latest release: its newest tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    /// Unix seconds.
    pub at: u64,
}

/// What finds each project's newest tag, on whichever machine runs it.
fn releases_script(projects: &[PathBuf]) -> String {
    let dirs: Vec<String> = projects.iter().map(|p| crate::ssh::quote(&p.to_string_lossy())).collect();
    format!(
        "for d in {}; do t=$(git -C \"$d\" for-each-ref --sort=-creatordate --count=1 --format='%(refname:short)\t%(creatordate:unix)' refs/tags 2>/dev/null); printf '%s\\t%s\\n' \"$d\" \"$t\"; done",
        dirs.join(" ")
    )
}

/// `<path>\t<tag>\t<unix>` a line; a project without tags has its path alone.
pub fn parse_releases(text: &str) -> Vec<(PathBuf, Release)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let (path, tag, at) = (parts.next()?, parts.next()?.trim(), parts.next()?.trim().parse().ok()?);
            (!tag.is_empty()).then(|| (PathBuf::from(path), Release { tag: tag.to_owned(), at }))
        })
        .collect()
}

/// Each project's newest tag, here or on the daemon's machine. Blocks: call
/// it on a thread.
pub fn releases(endpoint: &Endpoint, projects: &[PathBuf]) -> Result<Vec<(PathBuf, Release)>, String> {
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    let script = releases_script(projects);
    let text = match endpoint {
        Endpoint::Local { .. } => {
            let out = std::process::Command::new("sh").arg("-c").arg(&script).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
        Endpoint::Ssh(host) => {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            let bytes = runtime.block_on(crate::ssh::Launcher::system(None).run(host, &script)).map_err(|f| f.guide(endpoint).short)?;
            String::from_utf8_lossy(&bytes).into_owned()
        }
    };
    Ok(parse_releases(&text))
}

/// The release of the project at `path`, among those found.
pub fn release_of<'a>(found: &'a [(PathBuf, Release)], path: &Path) -> Option<&'a Release> {
    found.iter().find(|(p, _)| p == path).map(|(_, r)| r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_projects_newest_tag_is_its_latest_release() {
        let text = "/c/web\tv1.2.0\t1700000000\n/c/api\t\n/c/odd\tv1\tsoon\n";
        let found = parse_releases(text);
        assert_eq!(found, [(PathBuf::from("/c/web"), Release { tag: "v1.2.0".into(), at: 1_700_000_000 })]);
        assert_eq!(release_of(&found, Path::new("/c/web")).map(|r| r.tag.as_str()), Some("v1.2.0"));
        assert!(release_of(&found, Path::new("/c/api")).is_none(), "no tag, no release");
        let script = releases_script(&["/c/my app".into()]);
        assert!(script.contains("'/c/my app'"), "{script}");
    }
}
