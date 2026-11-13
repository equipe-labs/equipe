//! On macOS, `equipe` carries an Info.plist in its `__TEXT,__info_plist`
//! section, as a command-line tool can: its identifier (the same one its
//! release signature uses, so macOS keeps one identity for it across
//! updates), its version, and the reasons macOS shows when it asks the person
//! something on equipe's behalf — Local Network, for instance, when a member's
//! command reaches a machine on the network.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("Info.plist");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>equipe</string>
	<key>CFBundleName</key>
	<string>equipe</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>NSLocalNetworkUsageDescription</key>
	<string>equipe's members reach machines on your network when their work needs it — a dev server, a device, another Mac of yours.</string>
</dict>
</plist>
"#
    );
    std::fs::write(&out, plist).expect("Info.plist written");
    println!("cargo:rustc-link-arg-bin=equipe=-Wl,-sectcreate,__TEXT,__info_plist,{}", out.display());
}
