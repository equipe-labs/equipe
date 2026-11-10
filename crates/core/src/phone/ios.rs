//! The iOS Simulator: `xcrun simctl` makes, boots, installs, launches, takes
//! screenshots and records; AXe reads the accessibility tree and touches,
//! types and presses (see `axe.rs` for why AXe, and how it gets here).

use std::path::Path;
use std::time::Duration;

use image::RgbImage;
use serde_json::Value;

use super::exec::{Exec, Running, args};
use super::tree::{self, Screen};
use super::{Device, Key};

/// The simulator model used when a member names none, if this Xcode has it.
pub const DEFAULT_MODEL: &str = "iPhone 17 Pro";

fn simctl(exec: &dyn Exec, rest: &[&str], timeout: Duration) -> super::exec::Out {
    let mut a = vec!["simctl".to_owned()];
    a.extend(rest.iter().map(|s| (*s).to_owned()));
    exec.run("xcrun", &a, None, timeout)
}

/// A simulator, as `simctl list devices -j` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sim {
    pub name: String,
    pub udid: String,
    pub booted: bool,
}

/// Every simulator this Mac has.
pub fn list(exec: &dyn Exec) -> Result<Vec<Sim>, String> {
    let out = simctl(exec, &["list", "devices", "-j"], Duration::from_secs(60));
    if !out.ok {
        return Err(format!("The iOS Simulator isn't available here ({}). It comes with Xcode.", out.why()));
    }
    let json: Value = serde_json::from_slice(&out.stdout).map_err(|_| "simctl's list was unreadable".to_owned())?;
    Ok(json["devices"]
        .as_object()
        .into_iter()
        .flat_map(|runtimes| runtimes.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|d| Some(Sim { name: d["name"].as_str()?.to_owned(), udid: d["udid"].as_str()?.to_owned(), booted: d["state"].as_str() == Some("Booted") }))
        .collect())
}

/// The newest iOS runtime installed, and the device models it runs.
fn runtime(exec: &dyn Exec) -> Result<(String, Vec<String>), String> {
    let out = simctl(exec, &["list", "runtimes", "-j"], Duration::from_secs(60));
    let json: Value = serde_json::from_slice(&out.stdout).map_err(|_| format!("simctl's runtimes were unreadable ({})", out.why()))?;
    let newest = json["runtimes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["platform"].as_str() == Some("iOS") && r["isAvailable"].as_bool() != Some(false))
        .max_by(|a, b| version(a["version"].as_str().unwrap_or("0")).cmp(&version(b["version"].as_str().unwrap_or("0"))))
        .ok_or("No iOS simulator runtime is installed. The person adds one in Xcode › Settings › Components (it takes several GB): ask them first.")?;
    let models = newest["supportedDeviceTypes"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str().map(str::to_owned)).collect();
    Ok((newest["identifier"].as_str().unwrap_or_default().to_owned(), models))
}

fn version(v: &str) -> Vec<u32> {
    v.split('.').filter_map(|p| p.parse().ok()).collect()
}

/// The model to make: the one asked for if this runtime runs it, else the
/// default, else the newest-numbered "iPhone … Pro" (not a Max), else any iPhone.
pub fn choose_model(models: &[String], asked: Option<&str>) -> Result<String, String> {
    if let Some(asked) = asked.map(str::trim).filter(|a| !a.is_empty()) {
        return models
            .iter()
            .find(|m| m.eq_ignore_ascii_case(asked))
            .cloned()
            .ok_or_else(|| format!("This Mac's newest iOS runtime has no {asked}. It has: {}.", models.iter().filter(|m| m.starts_with("iP")).cloned().collect::<Vec<_>>().join(", ")));
    }
    if models.iter().any(|m| m == DEFAULT_MODEL) {
        return Ok(DEFAULT_MODEL.into());
    }
    let number = |m: &str| m.split_whitespace().nth(1).and_then(|n| n.trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<u32>().ok()).unwrap_or(0);
    let iphones: Vec<&String> = models.iter().filter(|m| m.starts_with("iPhone")).collect();
    iphones
        .iter()
        .filter(|m| m.ends_with(" Pro"))
        .max_by_key(|m| number(m))
        .or_else(|| iphones.iter().max_by_key(|m| number(m)))
        .map(|m| (*m).clone())
        .ok_or_else(|| "This Mac's iOS runtime runs no iPhone.".to_owned())
}

/// The ticket's simulator for this member, made if there is none: its udid.
pub fn ensure(exec: &dyn Exec, name_prefix: &str, model: Option<&str>) -> Result<(Sim, bool), String> {
    let sims = list(exec)?;
    let ours = |s: &&Sim| s.name.strip_prefix(name_prefix).is_some_and(|rest| rest.starts_with(' ')) && model.is_none_or(|m| s.name.ends_with(m));
    if let Some(sim) = sims.iter().find(ours) {
        return Ok((sim.clone(), false));
    }
    let (runtime, models) = runtime(exec)?;
    let model = choose_model(&models, model)?;
    let name = format!("{name_prefix} {model}");
    let out = simctl(exec, &["create", &name, &model, &runtime], Duration::from_secs(120));
    if !out.ok {
        return Err(format!("The simulator couldn't be made: {}", out.why()));
    }
    let udid = out.text().trim().to_owned();
    Ok((Sim { name, udid, booted: false }, true))
}

/// Boot it and wait until it is ready.
pub fn boot(exec: &dyn Exec, udid: &str) -> Result<(), String> {
    let out = simctl(exec, &["boot", udid], Duration::from_secs(120));
    if !out.ok && !out.why().contains("Booted") {
        return Err(format!("The simulator didn't start: {}", out.why()));
    }
    let ready = simctl(exec, &["bootstatus", udid, "-b"], Duration::from_secs(240));
    if !ready.ok {
        return Err(format!("The simulator didn't finish starting: {}", ready.why()));
    }
    Ok(())
}

pub fn shutdown(exec: &dyn Exec, udid: &str) {
    simctl(exec, &["shutdown", udid], Duration::from_secs(60));
}

/// An app bundle's id, from its `Info.plist`.
pub fn bundle_id(exec: &dyn Exec, app: &Path) -> Option<String> {
    let plist = app.join("Info.plist");
    let out = exec.run("plutil", &["-extract".into(), "CFBundleIdentifier".into(), "raw".into(), "-o".into(), "-".into(), plist.to_string_lossy().into_owned()], None, Duration::from_secs(10));
    let id = out.text().trim().to_owned();
    (out.ok && !id.is_empty()).then_some(id)
}

/// One booted simulator, driven.
pub struct Ios<'a> {
    pub exec: &'a dyn Exec,
    pub udid: String,
    /// AXe, when it is here; without it only screenshots, launching and recording work.
    pub axe: Option<String>,
}

impl Ios<'_> {
    fn axe(&self, rest: &[&str], stdin: Option<&[u8]>) -> Result<String, String> {
        let axe = self.axe.as_deref().ok_or(super::NO_AXE)?;
        let mut a: Vec<String> = rest.iter().map(|s| (*s).to_owned()).collect();
        a.extend(["--udid".to_owned(), self.udid.clone()]);
        let out = self.exec.run(axe, &a, stdin, Duration::from_secs(30));
        if out.ok { Ok(out.text()) } else { Err(out.why()) }
    }
}

fn num(v: f64) -> String {
    format!("{:.1}", v)
}

impl Device for Ios<'_> {
    fn screenshot(&self) -> Result<RgbImage, String> {
        let file = std::env::temp_dir().join(format!("equipe-phone-{}-{}.png", self.udid, std::process::id()));
        let out = simctl(self.exec, &["io", &self.udid, "screenshot", "--type=png", &file.to_string_lossy()], Duration::from_secs(30));
        let read = std::fs::read(&file);
        let _ = std::fs::remove_file(&file);
        if !out.ok {
            return Err(format!("No screenshot: {}", out.why()));
        }
        let bytes = read.map_err(|e| format!("No screenshot: {e}"))?;
        image::load_from_memory(&bytes).map(|i| i.to_rgb8()).map_err(|e| format!("The screenshot was unreadable: {e}"))
    }

    fn tree(&self) -> Result<Screen, String> {
        tree::from_axe(&self.axe(&["describe-ui"], None)?)
    }

    fn can_touch(&self) -> bool {
        self.axe.is_some()
    }

    fn tap(&self, x: f64, y: f64) -> Result<(), String> {
        self.axe(&["tap", "-x", &num(x), "-y", &num(y)], None).map(|_| ())
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        self.axe(&["type", "--stdin"], Some(text.as_bytes())).map(|_| ())
    }

    fn swipe(&self, from: (f64, f64), to: (f64, f64), seconds: f64) -> Result<(), String> {
        self.axe(
            &["swipe", "--start-x", &num(from.0), "--start-y", &num(from.1), "--end-x", &num(to.0), "--end-y", &num(to.1), "--duration", &format!("{seconds:.2}")],
            None,
        )
        .map(|_| ())
    }

    fn key(&self, key: Key) -> Result<(), String> {
        match key {
            Key::Home => self.axe(&["button", "home"], None),
            Key::Back => self.axe(&["gesture", "swipe-from-left-edge"], None),
            Key::Enter => self.axe(&["key", "40"], None),
            Key::Delete => self.axe(&["key", "42"], None),
            Key::Tab => self.axe(&["key", "43"], None),
            Key::Escape => self.axe(&["key", "41"], None),
        }
        .map(|_| ())
    }

    fn install(&self, app: &Path) -> Result<Option<String>, String> {
        if app.extension().is_none_or(|e| e != "app") {
            return Err(format!("{} isn't an iOS app: give the `.app` your simulator build made (…/Build/Products/Debug-iphonesimulator/<Name>.app).", app.display()));
        }
        let out = simctl(self.exec, &["install", &self.udid, &app.to_string_lossy()], Duration::from_secs(180));
        if !out.ok {
            return Err(format!("The app didn't install: {}", out.why()));
        }
        Ok(bundle_id(self.exec, app))
    }

    fn launch(&self, bundle: &str) -> Result<(), String> {
        let out = simctl(self.exec, &["launch", "--terminate-running-process", &self.udid, bundle], Duration::from_secs(60));
        if out.ok { Ok(()) } else { Err(format!("{bundle} didn't launch: {}", out.why())) }
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        let out = simctl(self.exec, &["openurl", &self.udid, url], Duration::from_secs(30));
        if out.ok { Ok(()) } else { Err(format!("{url} didn't open: {}", out.why())) }
    }

    fn record(&self, video: &Path) -> Result<Box<dyn Running>, String> {
        let mut a = args(["simctl", "io"]);
        a.extend([self.udid.clone(), "recordVideo".into(), "--codec=h264".into(), "--force".into(), video.to_string_lossy().into_owned()]);
        self.exec.spawn("xcrun", &a)
    }

    fn finish(&self, mut running: Box<dyn Running>, _video: &Path) -> Result<(), String> {
        if running.finish(Duration::from_secs(20)) { Ok(()) } else { Err("The recording didn't finish writing in time.".into()) }
    }

    fn points_per_pixel(&self, image: &RgbImage) -> f64 {
        // The tree speaks points; a screenshot is pixels: an iPhone is 3× (2× for a few).
        match self.tree() {
            Ok(screen) if screen.size.w > 0. => screen.size.w / f64::from(image.width().max(1)),
            _ => 1. / 3.,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone::exec::fake::Fake;

    const LIST: &str = r#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-26-5":[
        {"name":"iPhone 17 Pro","udid":"AAA","state":"Booted"},
        {"name":"acme T12 qa iPhone 17 Pro","udid":"BBB","state":"Shutdown"}]}}"#;
    const RUNTIMES: &str = r#"{"runtimes":[
        {"platform":"iOS","version":"18.2","identifier":"rt.18","isAvailable":true,"supportedDeviceTypes":[{"name":"iPhone 16 Pro"}]},
        {"platform":"iOS","version":"26.5","identifier":"rt.26","isAvailable":true,"supportedDeviceTypes":[{"name":"iPhone 17 Pro"},{"name":"iPhone 17 Pro Max"},{"name":"iPad Air"}]},
        {"platform":"watchOS","version":"27.0","identifier":"rt.w","isAvailable":true,"supportedDeviceTypes":[]}]}"#;

    #[test]
    fn the_tickets_own_simulator_is_used_or_made_never_another() {
        let fake = Fake::default();
        fake.on("simctl list devices -j", LIST).on("simctl list runtimes -j", RUNTIMES).on("simctl create", "NEW-UDID\n");
        let (sim, made) = ensure(&fake, "acme T12 qa", None).unwrap();
        assert_eq!((sim.udid.as_str(), made), ("BBB", false));
        let (sim, made) = ensure(&fake, "acme T13 qa", None).unwrap();
        assert_eq!((sim.name.as_str(), sim.udid.as_str(), made), ("acme T13 qa iPhone 17 Pro", "NEW-UDID", true));
        assert!(fake.called("xcrun simctl create acme T13 qa iPhone 17 Pro iPhone 17 Pro rt.26"), "{:?}", fake.calls());
        assert!(!fake.calls().iter().any(|c| c.contains("AAA")), "the person's own simulator is never touched");
    }

    #[test]
    fn a_model_is_chosen_from_what_the_runtime_runs() {
        let models: Vec<String> = ["iPhone 17", "iPhone 17 Pro", "iPhone 17 Pro Max", "iPhone 16e"].map(String::from).to_vec();
        assert_eq!(choose_model(&models, None).unwrap(), "iPhone 17 Pro");
        assert_eq!(choose_model(&models, Some("iphone 16e")).unwrap(), "iPhone 16e");
        assert!(choose_model(&models, Some("Pixel 9")).unwrap_err().contains("iPhone 17 Pro Max"));
        let later: Vec<String> = ["iPhone 18", "iPhone 18 Pro", "iPhone 18 Pro Max", "iPhone 19 Pro"].map(String::from).to_vec();
        assert_eq!(choose_model(&later, None).unwrap(), "iPhone 19 Pro");
    }

    #[test]
    fn touches_go_through_axe_to_this_simulator_only() {
        let fake = Fake::default();
        let ios = Ios { exec: &fake, udid: "BBB".into(), axe: Some("/x/axe".into()) };
        ios.tap(201., 725.).unwrap();
        ios.type_text("it's \"quoted\" -n").unwrap();
        ios.key(Key::Home).unwrap();
        let calls = fake.calls();
        assert_eq!(calls[0], "/x/axe tap -x 201.0 -y 725.0 --udid BBB");
        assert_eq!(calls[1], "/x/axe type --stdin --udid BBB <<< it's \"quoted\" -n", "text on stdin: nothing in it is read as a flag");
        assert_eq!(calls[2], "/x/axe button home --udid BBB");
        let without = Ios { exec: &fake, udid: "BBB".into(), axe: None };
        assert_eq!(without.tap(1., 1.).unwrap_err(), crate::phone::NO_AXE);
    }
}
