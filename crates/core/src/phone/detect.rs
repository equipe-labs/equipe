//! Whether a squad has a mobile app to test, and for which platforms: read
//! from its projects' files, a few folders deep, never the whole tree.
//!
//! - iOS: an Xcode project or workspace (`*.xcodeproj`, `*.xcworkspace`) — at
//!   the root, under `ios/` (React Native, Expo after prebuild, Flutter) or in
//!   an app folder of a monorepo (`apps/mobile/ios`).
//! - Android: a Gradle module that applies `com.android.application`
//!   (`app/build.gradle(.kts)`, `android/app/…`), or an `AndroidManifest.xml`
//!   in one.
//! - Both: an Expo app without native folders yet (`app.json` with an `expo`
//!   key, or `expo` in `package.json`'s dependencies), or a Flutter app
//!   (`pubspec.yaml` naming `flutter`) — the person may run either.

use std::path::Path;

/// The platforms a squad's apps run on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Platforms {
    pub ios: bool,
    pub android: bool,
}

impl Platforms {
    pub fn any(self) -> bool {
        self.ios || self.android
    }

    fn or(self, other: Platforms) -> Platforms {
        Platforms { ios: self.ios || other.ios, android: self.android || other.android }
    }
}

/// Folders never looked in: dependencies, build output, version control.
const SKIP: [&str; 12] = ["node_modules", "Pods", ".git", "build", "DerivedData", "target", ".gradle", "dist", ".next", "vendor", ".build", ".expo"];

/// How deep below a project's root an app is looked for (`apps/mobile/ios/X.xcodeproj` is 3).
const DEPTH: usize = 3;
/// And through how many folders at most, all told.
const FOLDERS: usize = 400;

/// The platforms of every project together. On a machine that can't run an
/// iOS simulator (not a Mac), iOS doesn't count.
pub fn platforms(projects: &[std::path::PathBuf]) -> Platforms {
    let found = projects.iter().fold(Platforms::default(), |all, p| all.or(of_project(p)));
    Platforms { ios: found.ios && cfg!(target_os = "macos"), ..found }
}

/// How long a project's platforms are taken as read: each turn asks, and a
/// project rarely becomes a mobile app between two turns.
const FRESH: std::time::Duration = std::time::Duration::from_secs(120);

/// [`platforms`], each project read again at most every two minutes.
pub fn platforms_cached(projects: &[std::path::PathBuf]) -> Platforms {
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};
    use std::time::Instant;
    static READ: LazyLock<Mutex<HashMap<std::path::PathBuf, (Instant, Platforms)>>> = LazyLock::new(Default::default);
    let found = projects.iter().fold(Platforms::default(), |all, p| {
        let cached = READ.lock().ok().and_then(|m| m.get(p).copied()).filter(|(at, _)| at.elapsed() < FRESH).map(|(_, f)| f);
        let f = cached.unwrap_or_else(|| {
            let f = of_project(p);
            if let Ok(mut m) = READ.lock() {
                m.insert(p.clone(), (Instant::now(), f));
            }
            f
        });
        all.or(f)
    });
    Platforms { ios: found.ios && cfg!(target_os = "macos"), ..found }
}

/// One project's platforms.
pub fn of_project(root: &Path) -> Platforms {
    let mut found = Platforms::default();
    let mut queue = std::collections::VecDeque::from([(root.to_owned(), 0usize)]);
    let mut looked = 0;
    while let Some((dir, depth)) = queue.pop_front() {
        looked += 1;
        if looked > FOLDERS || (found.ios && found.android) {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if is_dir && (name.ends_with(".xcodeproj") || name.ends_with(".xcworkspace")) {
                // An Xcode project of a Mac-only or a library target still counts:
                // the tools say so when the simulator has nothing to run.
                found.ios = true;
                continue;
            }
            match name.as_str() {
                "build.gradle" | "build.gradle.kts" if applies_android_app(&path) => found.android = true,
                "AndroidManifest.xml" if depth > 0 => found.android = true,
                "app.json" if std::fs::read_to_string(&path).is_ok_and(|t| t.contains("\"expo\"")) => found = found.or(Platforms { ios: true, android: true }),
                "package.json" if std::fs::read_to_string(&path).is_ok_and(|t| t.contains("\"expo\":") || t.contains("\"react-native\":")) => {
                    found = found.or(Platforms { ios: true, android: true });
                }
                "pubspec.yaml" if std::fs::read_to_string(&path).is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("flutter:"))) => {
                    found = found.or(Platforms { ios: true, android: true });
                }
                _ => {}
            }
            if is_dir && depth < DEPTH && !name.starts_with('.') && !SKIP.contains(&name.as_str()) {
                queue.push_back((path, depth + 1));
            }
        }
    }
    found
}

fn applies_android_app(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| t.contains("com.android.application") || t.contains("libs.plugins.android.application"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile_lite::Dir {
        let dir = tempfile_lite::Dir::new();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            if path.extension().is_some_and(|e| e == "xcodeproj" || e == "xcworkspace") {
                std::fs::create_dir_all(&path).unwrap();
            } else {
                std::fs::write(&path, text).unwrap();
            }
        }
        dir
    }

    #[test]
    fn an_xcode_project_is_ios_and_a_gradle_app_is_android() {
        let ios = project(&[("Shop.xcodeproj", ""), ("Shop/ContentView.swift", "")]);
        assert_eq!(of_project(ios.path()), Platforms { ios: true, android: false });
        let android = project(&[("settings.gradle.kts", ""), ("app/build.gradle.kts", "plugins { id(\"com.android.application\") }")]);
        assert_eq!(of_project(android.path()), Platforms { ios: false, android: true });
        let library = project(&[("lib/build.gradle", "apply plugin: 'com.android.library'")]);
        assert!(!of_project(library.path()).any(), "a library has nothing to launch");
    }

    #[test]
    fn react_native_expo_flutter_and_monorepos_count() {
        let rn = project(&[("package.json", "{\"dependencies\":{\"react-native\":\"0.76\"}}"), ("ios/Shop.xcworkspace", ""), ("android/app/build.gradle", "apply plugin: \"com.android.application\"")]);
        assert_eq!(of_project(rn.path()), Platforms { ios: true, android: true });
        let expo = project(&[("app.json", "{\"expo\":{\"name\":\"shop\"}}")]);
        assert_eq!(of_project(expo.path()), Platforms { ios: true, android: true });
        let mono = project(&[("apps/mobile/ios/Shop.xcodeproj", ""), ("apps/web/package.json", "{\"dependencies\":{\"next\":\"15\"}}")]);
        assert_eq!(of_project(mono.path()), Platforms { ios: true, android: false });
        let flutter = project(&[("pubspec.yaml", "name: shop\ndependencies:\n  flutter:\n    sdk: flutter\n")]);
        assert!(of_project(flutter.path()).ios);
    }

    #[test]
    fn a_web_or_server_project_is_no_mobile_project() {
        let web = project(&[("package.json", "{\"dependencies\":{\"next\":\"15\",\"react\":\"19\"}}"), ("src/app/page.tsx", "")]);
        assert!(!of_project(web.path()).any());
        let deps = project(&[("node_modules/react-native/package.json", "{\"react-native\": \"x\"}"), ("node_modules/x/ios/X.xcodeproj", ""), ("Cargo.toml", "")]);
        assert!(!of_project(deps.path()).any(), "what dependencies carry isn't the squad's app");
        assert!(!platforms(&[web.path().to_owned(), deps.path().to_owned()]).any());
    }

    /// A temporary folder, removed when dropped.
    mod tempfile_lite {
        pub struct Dir(std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let dir = std::env::temp_dir().join(format!("equipe-phone-detect-{}-{n}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).unwrap();
                Self(dir)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
