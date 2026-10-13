//! Images for messages: from this machine's clipboard, from a laptop over SSH
//! (`equipe send-image` drops them in an inbox here), or uploaded through
//! iTerm2 when the console runs over SSH.

use std::path::{Path, PathBuf};

/// Where attached images are kept: a temp folder only the person can read,
/// which members may read without asking.
pub fn pastes_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = std::env::temp_dir().join("equipe-pastes");
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    Ok(dir)
}

/// Where `equipe send-image` on another machine drops images for the console
/// here to attach.
pub fn inbox() -> PathBuf {
    equipe_providers::credentials::equipe_home().join("inbox")
}

fn stamp() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Save this machine's clipboard image as a PNG, ready to attach.
pub fn clipboard_image() -> Result<PathBuf, String> {
    let path = pastes_dir()?.join(format!("paste-{}.png", stamp()));
    let target = path.to_string_lossy().into_owned();
    let has = |program: &str| std::process::Command::new("which").arg(program).output().is_ok_and(|o| o.status.success());
    let shell = |script: &str| std::process::Command::new("sh").arg("-c").arg(script).arg("sh").arg(&target).status();
    let _ = if cfg!(target_os = "macos") {
        if has("pngpaste") {
            shell("pngpaste \"$1\" 2>/dev/null")
        } else {
            shell(r#"osascript -e "set f to (open for access POSIX file \"$1\" with write permission)" -e "try" -e "write (the clipboard as «class PNGf») to f" -e "end try" -e "close access f" >/dev/null 2>&1"#)
        }
    } else if has("wl-paste") {
        shell("wl-paste --type image/png > \"$1\" 2>/dev/null")
    } else {
        shell("xclip -selection clipboard -t image/png -o > \"$1\" 2>/dev/null")
    };
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
        return Ok(path);
    }
    let _ = std::fs::remove_file(&path);
    Err("No image on the clipboard. Copy a screenshot (⌃⇧⌘4 on a Mac), then Ctrl-V.".into())
}

/// Images that arrived in the inbox, moved to where attachments are kept.
pub fn take_inbox() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(inbox()) else {
        return Vec::new();
    };
    let Ok(dir) = pastes_dir() else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|from| {
            let to = dir.join(from.file_name()?);
            std::fs::rename(&from, &to).or_else(|_| std::fs::copy(&from, &to).map(|_| ()).and_then(|()| std::fs::remove_file(&from))).ok()?;
            Some(to)
        })
        .collect()
}

/// `equipe send-image <host>`: this machine's clipboard image, into the
/// inbox of the equipe on `host` — whose console attaches it.
pub fn send_image(host: &str) -> Result<(), String> {
    let image = clipboard_image()?;
    let name = format!("from-{}.png", stamp());
    let remote = format!("mkdir -p ~/.equipe/inbox && cat > ~/.equipe/inbox/{name}.part && mv ~/.equipe/inbox/{name}.part ~/.equipe/inbox/{name}");
    let file = std::fs::File::open(&image).map_err(|err| err.to_string())?;
    let status = std::process::Command::new("ssh")
        .arg(host)
        .arg(remote)
        .stdin(file)
        .status()
        .map_err(|err| format!("Couldn't run ssh: {err}"))?;
    let _ = std::fs::remove_file(&image);
    if status.success() { Ok(()) } else { Err(format!("ssh to {host} didn't take the image")) }
}

/// The escape sequence asking iTerm2 to let the person pick files on their
/// own Mac and send them here, as a tar.gz in base64 (iTerm2's `it2ul`).
pub fn upload_request() -> String {
    use base64::Engine as _;
    let tar = std::process::Command::new("tar").arg("--version").output().map(|o| o.stdout).unwrap_or_default();
    let first = String::from_utf8_lossy(&tar).lines().next().unwrap_or_default().to_owned();
    let version = base64::engine::general_purpose::STANDARD.encode(first);
    format!("\x1b]1337;RequestUpload=format=tgz;version={version}\x07")
}

/// Unpack what iTerm2 uploaded and return the images in it, moved to where
/// attachments are kept.
pub fn unpack_upload(base64_data: &str) -> Result<Vec<PathBuf>, String> {
    use base64::Engine as _;
    use std::io::Write as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data.trim())
        .map_err(|err| format!("The upload didn't decode: {err}"))?;
    let into = pastes_dir()?.join(format!("upload-{}", stamp()));
    std::fs::create_dir_all(&into).map_err(|err| err.to_string())?;
    let mut tar = std::process::Command::new("tar")
        .args(["-x", "-z", "-C"])
        .arg(&into)
        .args(["-f", "-"])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| err.to_string())?;
    tar.stdin.take().expect("piped").write_all(&bytes).map_err(|err| err.to_string())?;
    if !tar.wait().is_ok_and(|s| s.success()) {
        return Err("The upload didn't unpack".into());
    }
    let mut images = Vec::new();
    collect_images(&into, &mut images);
    images.sort();
    Ok(images)
}

fn collect_images(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_images(&path, out);
        } else if crate::tui::is_image(&path) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_upload_is_unpacked_to_its_images() {
        use base64::Engine as _;
        let src = std::env::temp_dir().join(format!("equipe-upload-src-{}", std::process::id()));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("Screen Shot.png"), b"png").unwrap();
        std::fs::write(src.join("notes.txt"), b"not an image").unwrap();
        let tgz = std::process::Command::new("tar").arg("-czf").arg("-").arg("-C").arg(&src).args(["Screen Shot.png", "notes.txt"]).output().unwrap().stdout;
        let encoded = base64::engine::general_purpose::STANDARD.encode(tgz);
        // iTerm2 sends it in lines.
        let lines: String = encoded.as_bytes().chunks(76).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("");
        let images = unpack_upload(&lines).unwrap();
        assert_eq!(images.len(), 1);
        assert!(images[0].ends_with("Screen Shot.png"));
        assert!(upload_request().starts_with("\x1b]1337;RequestUpload=format=tgz;version="));
        std::fs::remove_dir_all(&src).unwrap();
    }
}
