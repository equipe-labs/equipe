//! The Android emulator, through the SDK's own tools: `emulator` boots an AVD
//! made for the ticket (its two `.ini` files written here, as `avdmanager`
//! would, so no Java is needed), `adb` installs, launches, reads the screen
//! (`screencap`, `uiautomator dump`), touches (`input`) and records
//! (`screenrecord`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use image::RgbImage;

use super::exec::{Exec, Out, Running};
use super::tree::{self, Screen};
use super::{Device, Key};

/// Where the Android SDK is: `ANDROID_HOME`, `ANDROID_SDK_ROOT`, or where
/// Android Studio puts it.
pub fn sdk() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .chain([home.join("Library/Android/sdk"), home.join("Android/Sdk")])
        .find(|p| p.join("platform-tools").is_dir())
}

pub fn adb_path() -> Option<String> {
    sdk().map(|s| s.join("platform-tools/adb")).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).or_else(|| on_path("adb"))
}

pub fn emulator_path() -> Option<String> {
    sdk().map(|s| s.join("emulator/emulator")).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).or_else(|| on_path("emulator"))
}

fn on_path(name: &str) -> Option<String> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
}

/// Where AVDs are kept.
pub fn avd_home() -> PathBuf {
    std::env::var_os("ANDROID_AVD_HOME").map(PathBuf::from).unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".android/avd"))
}

/// An AVD's name for a ticket's member: letters, digits, `_`, `.`, `-` only.
pub fn avd_name(prefix: &str) -> String {
    prefix.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '_' }).collect()
}

/// The newest system image installed for this Mac's processor: `(android-35, google_apis, arm64-v8a)`.
pub fn system_image(sdk: &Path) -> Option<(String, String, String)> {
    let abi = if cfg!(target_arch = "aarch64") { "arm64-v8a" } else { "x86_64" };
    let mut best: Option<(u32, u8, String, String)> = None;
    for platform in std::fs::read_dir(sdk.join("system-images")).ok()?.flatten() {
        let name = platform.file_name().to_string_lossy().into_owned();
        let Some(level) = name.strip_prefix("android-").and_then(|n| n.parse::<u32>().ok()) else { continue };
        for tag in std::fs::read_dir(platform.path()).into_iter().flatten().flatten() {
            let tag_name = tag.file_name().to_string_lossy().into_owned();
            if !tag.path().join(abi).join("system.img").is_file() {
                continue;
            }
            let rank = match tag_name.as_str() {
                "google_apis" => 3,
                "default" => 2,
                "google_apis_playstore" => 1,
                _ => 0,
            };
            if best.as_ref().is_none_or(|b| (level, rank) > (b.0, b.1)) {
                best = Some((level, rank, name.clone(), tag_name));
            }
        }
    }
    best.map(|(_, _, platform, tag)| (platform, tag, abi.to_owned()))
}

/// The AVD's two files, as `avdmanager create avd` writes them: `<name>.ini`
/// and `<name>.avd/config.ini`, a phone-sized screen.
pub fn avd_files(name: &str, home: &Path, image: &(String, String, String)) -> [(PathBuf, String); 2] {
    let (platform, tag, abi) = image;
    let dir = home.join(format!("{name}.avd"));
    let ini = format!("avd.ini.encoding=UTF-8\npath={}\npath.rel=avd/{name}.avd\ntarget={platform}\n", dir.display());
    let arch = if abi.starts_with("arm64") { "arm64" } else { "x86_64" };
    let config = format!(
        "AvdId={name}\navd.ini.displayname={name}\nabi.type={abi}\nhw.cpu.arch={arch}\nimage.sysdir.1=system-images/{platform}/{tag}/{abi}/\ntag.id={tag}\n\
hw.lcd.width=1080\nhw.lcd.height=2400\nhw.lcd.density=420\nhw.keyboard=yes\nhw.ramSize=2048\ndisk.dataPartition.size=4G\n\
PlayStore.enabled=false\nshowDeviceFrame=no\nfastboot.forceColdBoot=yes\n"
    );
    [(home.join(format!("{name}.ini")), ini), (dir.join("config.ini"), config)]
}

/// Make the AVD if it isn't there.
pub fn ensure_avd(name: &str) -> Result<bool, String> {
    let home = avd_home();
    if home.join(format!("{name}.ini")).is_file() {
        return Ok(false);
    }
    let sdk = sdk().ok_or(NO_SDK)?;
    let image = system_image(&sdk).ok_or(
        "No Android system image for this Mac is installed. The person adds one in Android Studio › SDK Manager › SDK Platforms (several GB): ask them first.",
    )?;
    for (path, text) in avd_files(name, &home, &image) {
        std::fs::create_dir_all(path.parent().unwrap_or(&home)).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("The emulator's files couldn't be written: {e}"))?;
    }
    Ok(true)
}

pub const NO_SDK: &str = "The Android SDK isn't installed here (no adb). The person installs Android Studio, or the SDK's command-line tools with platform-tools and the emulator.";

fn adb(exec: &dyn Exec, adb: &str, serial: Option<&str>, rest: &[&str], timeout: Duration) -> Out {
    let mut a: Vec<String> = Vec::new();
    if let Some(s) = serial {
        a.extend(["-s".to_owned(), s.to_owned()]);
    }
    a.extend(rest.iter().map(|s| (*s).to_owned()));
    exec.run(adb, &a, None, timeout)
}

/// The emulators running: their serials.
pub fn running(exec: &dyn Exec, adb_path: &str) -> Vec<String> {
    adb(exec, adb_path, None, &["devices"], Duration::from_secs(20))
        .text()
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|s| s.starts_with("emulator-"))
        .map(str::to_owned)
        .collect()
}

/// The serial of the emulator running AVD `name`, if one is.
pub fn serial_of(exec: &dyn Exec, adb_path: &str, name: &str) -> Option<String> {
    running(exec, adb_path).into_iter().find(|s| adb(exec, adb_path, Some(s), &["emu", "avd", "name"], Duration::from_secs(10)).text().lines().next().map(str::trim) == Some(name))
}

/// Start AVD `name` without a window and wait until Android has booted: its serial.
pub fn boot(exec: &dyn Exec, adb_path: &str, emulator: &str, name: &str) -> Result<String, String> {
    if let Some(serial) = serial_of(exec, adb_path, name) {
        return Ok(serial);
    }
    let taken = running(exec, adb_path);
    let port = (5554..5682).step_by(2).find(|p| !taken.contains(&format!("emulator-{p}"))).ok_or("No free port for another emulator.")?;
    let a: Vec<String> = ["-avd", name, "-port", &port.to_string(), "-no-window", "-no-audio", "-no-boot-anim", "-no-snapshot-save"].iter().map(|s| (*s).to_owned()).collect();
    exec.detach(emulator, &a)?;
    let serial = format!("emulator-{port}");
    let waited = adb(exec, adb_path, Some(&serial), &["wait-for-device"], Duration::from_secs(180));
    if !waited.ok {
        return Err(format!("The emulator didn't come up: {}", waited.why()));
    }
    for _ in 0..120 {
        if adb(exec, adb_path, Some(&serial), &["shell", "getprop", "sys.boot_completed"], Duration::from_secs(10)).text().trim() == "1" {
            return Ok(serial);
        }
        exec.sleep(Duration::from_secs(2));
    }
    Err("Android didn't finish booting in four minutes.".into())
}

pub fn shutdown(exec: &dyn Exec, adb_path: &str, serial: &str) {
    adb(exec, adb_path, Some(serial), &["emu", "kill"], Duration::from_secs(20));
}

/// `input text`'s argument: spaces as `%s`, the rest quoted for the device's shell.
pub fn input_text(text: &str) -> Result<String, String> {
    if !text.is_ascii() || text.contains('\n') {
        return Err("Android's `input text` types plain ASCII on one line only: type the rest with the on-screen keyboard (tap its keys), or paste it in.".into());
    }
    Ok(format!("'{}'", text.replace(' ', "%s").replace('\'', "'\\''")))
}

/// One booted emulator, driven.
pub struct Android<'a> {
    pub exec: &'a dyn Exec,
    pub adb: String,
    pub serial: String,
}

impl Android<'_> {
    fn shell(&self, rest: &[&str]) -> Result<String, String> {
        let mut a = vec!["shell"];
        a.extend_from_slice(rest);
        let out = adb(self.exec, &self.adb, Some(&self.serial), &a, Duration::from_secs(30));
        if out.ok { Ok(out.text()) } else { Err(out.why()) }
    }

    /// Pixels per dp: `wm density`'s dpi over 160.
    pub fn scale(&self) -> f64 {
        let said = self.shell(&["wm", "density"]).unwrap_or_default();
        let dpi = said.lines().rev().find_map(|l| l.rsplit(':').next()?.trim().parse::<f64>().ok()).unwrap_or(160.);
        dpi / 160.
    }

    fn px(&self, v: f64) -> String {
        format!("{}", (v * self.scale()).round() as i64)
    }
}

impl Device for Android<'_> {
    fn screenshot(&self) -> Result<RgbImage, String> {
        let out = adb(self.exec, &self.adb, Some(&self.serial), &["exec-out", "screencap", "-p"], Duration::from_secs(30));
        if !out.ok {
            return Err(format!("No screenshot: {}", out.why()));
        }
        image::load_from_memory(&out.stdout).map(|i| i.to_rgb8()).map_err(|e| format!("The screenshot was unreadable: {e}"))
    }

    fn tree(&self) -> Result<Screen, String> {
        self.shell(&["uiautomator", "dump", "/sdcard/equipe-ui.xml"])?;
        let xml = adb(self.exec, &self.adb, Some(&self.serial), &["exec-out", "cat", "/sdcard/equipe-ui.xml"], Duration::from_secs(20)).text();
        tree::from_uiautomator(&xml, self.scale())
    }

    fn can_touch(&self) -> bool {
        true
    }

    fn tap(&self, x: f64, y: f64) -> Result<(), String> {
        self.shell(&["input", "tap", &self.px(x), &self.px(y)]).map(|_| ())
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        self.shell(&["input", "text", &input_text(text)?]).map(|_| ())
    }

    fn swipe(&self, from: (f64, f64), to: (f64, f64), seconds: f64) -> Result<(), String> {
        let ms = ((seconds * 1000.).round() as i64).to_string();
        self.shell(&["input", "swipe", &self.px(from.0), &self.px(from.1), &self.px(to.0), &self.px(to.1), &ms]).map(|_| ())
    }

    fn key(&self, key: Key) -> Result<(), String> {
        let code = match key {
            Key::Home => "KEYCODE_HOME",
            Key::Back => "KEYCODE_BACK",
            Key::Enter => "KEYCODE_ENTER",
            Key::Delete => "KEYCODE_DEL",
            Key::Tab => "KEYCODE_TAB",
            Key::Escape => "KEYCODE_ESCAPE",
        };
        self.shell(&["input", "keyevent", code]).map(|_| ())
    }

    fn install(&self, app: &Path) -> Result<Option<String>, String> {
        if app.extension().is_none_or(|e| e != "apk") {
            return Err(format!("{} isn't an Android app: give the `.apk` your debug build made (app/build/outputs/apk/debug/app-debug.apk).", app.display()));
        }
        let out = adb(self.exec, &self.adb, Some(&self.serial), &["install", "-r", "-t", &app.to_string_lossy()], Duration::from_secs(240));
        if !out.ok || out.text().contains("Failure") {
            return Err(format!("The app didn't install: {}", out.why()));
        }
        Ok(package_of(self.exec, app))
    }

    fn launch(&self, package: &str) -> Result<(), String> {
        self.shell(&["am", "force-stop", package])?;
        let said = self.shell(&["monkey", "-p", package, "-c", "android.intent.category.LAUNCHER", "1"])?;
        if said.contains("No activities found") { Err(format!("{package} has no screen to launch (is it installed?).")) } else { Ok(()) }
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.shell(&["am", "start", "-a", "android.intent.action.VIEW", "-d", &format!("'{}'", url.replace('\'', "'\\''"))]).map(|_| ())
    }

    fn record(&self, video: &Path) -> Result<Box<dyn Running>, String> {
        let remote = remote_video(video);
        // Android's own limit is three minutes a recording.
        let a: Vec<String> = ["-s", &self.serial, "shell", "screenrecord", "--time-limit", "180", &remote].iter().map(|s| (*s).to_owned()).collect();
        self.exec.spawn(&self.adb, &a)
    }

    fn finish(&self, mut running: Box<dyn Running>, video: &Path) -> Result<(), String> {
        let remote = remote_video(video);
        // Stopped on the device, so the file is closed properly; then copied over.
        let _ = self.shell(&["pkill", "-INT", "screenrecord"]);
        running.finish(Duration::from_secs(10));
        self.exec.sleep(Duration::from_millis(800));
        let out = adb(self.exec, &self.adb, Some(&self.serial), &["pull", &remote, &video.to_string_lossy()], Duration::from_secs(120));
        let _ = self.shell(&["rm", "-f", &remote]);
        if out.ok { Ok(()) } else { Err(format!("The recording couldn't be copied from the emulator: {}", out.why())) }
    }

    fn points_per_pixel(&self, _image: &RgbImage) -> f64 {
        1. / self.scale()
    }
}

fn remote_video(video: &Path) -> String {
    format!("/sdcard/equipe-{}", video.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "recording.mp4".into()))
}

/// An APK's package, from the SDK's `aapt2` (or `aapt`), when one is installed.
pub fn package_of(exec: &dyn Exec, apk: &Path) -> Option<String> {
    let tools = sdk()?.join("build-tools");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&tools).ok()?.flatten().map(|e| e.path()).collect();
    versions.sort();
    let aapt = versions.iter().rev().flat_map(|v| [v.join("aapt2"), v.join("aapt")]).find(|p| p.is_file())?;
    let out = exec.run(&aapt.to_string_lossy(), &["dump".into(), "badging".into(), apk.to_string_lossy().into_owned()], None, Duration::from_secs(30));
    let text = out.text();
    let at = text.find("package: name='")? + "package: name='".len();
    Some(text[at..].split('\'').next()?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone::exec::fake::Fake;

    #[test]
    fn an_avd_is_written_as_avdmanager_would() {
        let image = ("android-35".to_owned(), "google_apis".to_owned(), "arm64-v8a".to_owned());
        let [(ini_path, ini), (config_path, config)] = avd_files("acme_T12_qa", Path::new("/h/.android/avd"), &image);
        assert_eq!(ini_path, Path::new("/h/.android/avd/acme_T12_qa.ini"));
        assert!(ini.contains("path=/h/.android/avd/acme_T12_qa.avd\n") && ini.contains("target=android-35"));
        assert_eq!(config_path, Path::new("/h/.android/avd/acme_T12_qa.avd/config.ini"));
        assert!(config.contains("image.sysdir.1=system-images/android-35/google_apis/arm64-v8a/"));
        assert!(config.contains("hw.cpu.arch=arm64"));
        assert_eq!(avd_name("acme T12 qa"), "acme_T12_qa");
    }

    #[test]
    fn text_is_typed_as_android_s_input_takes_it() {
        assert_eq!(input_text("it's me").unwrap(), "'it'\\''s%sme'");
        assert!(input_text("안녕").is_err(), "not ASCII: said plainly");
        assert!(input_text("a\nb").is_err());
    }

    #[test]
    fn touches_are_in_dp_and_reach_this_emulator() {
        let fake = Fake::default();
        fake.on("wm density", "Physical density: 420\n");
        let phone = Android { exec: &fake, adb: "adb".into(), serial: "emulator-5560".into() };
        phone.tap(100., 200.).unwrap();
        phone.key(Key::Back).unwrap();
        assert!(fake.called("adb -s emulator-5560 shell input tap 263 525"), "{:?}", fake.calls());
        assert!(fake.called("adb -s emulator-5560 shell input keyevent KEYCODE_BACK"));
    }

    #[test]
    fn an_emulator_is_found_by_its_avd_or_started_on_a_free_port() {
        let fake = Fake::default();
        fake.on("devices", "List of devices attached\nemulator-5554\tdevice\nemulator-5556\tdevice\n")
            .on("-s emulator-5554 emu avd name", "Pixel_9\nOK\n")
            .on("-s emulator-5556 emu avd name", "someone_else\nOK\n")
            .on("getprop sys.boot_completed", "1\n");
        let serial = boot(&fake, "adb", "emulator", "acme_T12_qa").unwrap();
        assert_eq!(serial, "emulator-5558");
        assert!(fake.called("detach emulator -avd acme_T12_qa -port 5558 -no-window"), "{:?}", fake.calls());
        fake.on("-s emulator-5558 emu avd name", "acme_T12_qa\nOK\n").on("devices", "emulator-5554\tdevice\nemulator-5556\tdevice\nemulator-5558\tdevice\n");
        assert_eq!(serial_of(&fake, "adb", "acme_T12_qa").as_deref(), Some("emulator-5558"));
    }
}
