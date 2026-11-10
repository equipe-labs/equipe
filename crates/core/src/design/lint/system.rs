//! Product code held to building from the project's design system, where
//! it has one (`Kit`): values its tokens hold written out, raw elements
//! re-making a registry component, imports around the registry. The
//! registry's own components, and the theme's stylesheet, aren't product
//! code and aren't held to it.

use super::source::{Kind, Source};
use super::{Finding, Kit, System};
use crate::design::system::{catalog, registry};

/// UI kits and primitives a project builds its registry on, or beside it:
/// imported directly in product code, they go around the system. With the
/// registry component that wraps each, where there is one.
const WRAPPED: [(&str, &str); 12] = [
    ("vaul", "drawer"),
    ("cmdk", "command"),
    ("sonner", "sonner"),
    ("input-otp", "input-otp"),
    ("embla-carousel-react", "carousel"),
    ("react-day-picker", "calendar"),
    ("react-resizable-panels", "resizable"),
    ("recharts", "chart"),
    ("@tanstack/react-table", "data-table"),
    ("react-hook-form", "form"),
    ("next-themes", ""),
    ("@radix-ui/react-slot", ""),
];

/// Other component libraries: a second system beside the project's own.
const OTHER_KITS: [&str; 22] = [
    "@mui/", "@material-ui/", "antd", "@chakra-ui/", "react-bootstrap", "@mantine/", "@headlessui/", "@nextui-org/", "@heroui/", "primereact", "semantic-ui-react", "@fluentui/", "@ark-ui/", "react-aria-components", "@react-aria/", "@base-ui-components/", "@base-ui/", "bits-ui", "reka-ui", "radix-vue", "@kobalte/", "radix-ui",
];

/// Packages a registry depends on that aren't components: product code may use them.
const UTILITIES: [&str; 9] = ["clsx", "tailwind-merge", "class-variance-authority", "lucide-react", "tw-animate-css", "tailwindcss-animate", "next-themes", "@radix-ui/react-slot", "zod"];

/// A component's name as code writes it: `alert-dialog` → `AlertDialog`.
pub fn pascal(name: &str) -> String {
    name.split('-').map(|p| {
        let mut c = p.chars();
        c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str())
    }).collect()
}

/// The registry component a package's primitives are wrapped by.
fn wrapper_of(pkg: &str, kit: &Kit) -> Option<String> {
    if let Some(rest) = pkg.strip_prefix("@radix-ui/react-") {
        let name = catalog::normal(rest);
        return kit.has(&name).then_some(name);
    }
    WRAPPED.iter().find(|(p, _)| *p == pkg).and_then(|(_, c)| (!c.is_empty() && kit.has(c)).then(|| (*c).to_owned()))
}

pub fn check(source: &Source, system: &System, kit: &Kit, out: &mut Vec<Finding>) {
    let file = &source.path;
    // A value written out that a token holds: the finishing note becomes a rule here.
    for f in out.iter_mut().filter(|f| f.rule == "literal-on-scale") {
        *f = Finding::new("system-token", file, f.line, f.evidence.clone());
    }
    tokens(source, system, file, out);
    if source.kind != Kind::Css {
        components(source, kit, file, out);
    }
    imports(source, kit, file, out);
}

fn px(text: &str) -> Option<f64> {
    let t = text.trim();
    t.strip_suffix("px").and_then(|n| n.parse().ok()).or_else(|| t.strip_suffix("rem").and_then(|n| n.parse::<f64>().ok()).map(|r| r * 16.))
}

fn radius_scale(system: &System) -> String {
    system.radii.iter().take(6).map(|(n, v)| format!("{n} {v}px")).collect::<Vec<_>>().join(", ")
}

fn tokens(source: &Source, system: &System, file: &str, out: &mut Vec<Finding>) {
    for d in &source.decls {
        if d.prop.starts_with("--") || d.value.contains("var(") || d.value.contains("theme(") {
            continue;
        }
        let radius = d.prop == "border-radius" || d.prop.starts_with("border-") && d.prop.ends_with("-radius");
        if radius && !system.radii.is_empty() {
            for part in d.value.split_whitespace() {
                let Some(v) = px(part) else { continue };
                if v <= 0. || v >= 999. {
                    continue;
                }
                let said = match system.radius_token(v) {
                    Some(t) => format!("{}: {part} is `{t}`", d.prop),
                    None => format!("{}: {part} is off the radius scale ({})", d.prop, radius_scale(system)),
                };
                out.push(Finding::new("system-token", file, Some(d.line), said));
            }
        }
        if d.prop == "box-shadow" && !system.shadows.is_empty() && !matches!(d.value.trim(), "none" | "0" | "inherit" | "initial" | "unset") {
            out.push(Finding::new("system-token", file, Some(d.line), format!("box-shadow: {} — the system's shadows are {}", d.value, system.shadows.iter().take(5).cloned().collect::<Vec<_>>().join(", "))));
        }
    }
    for tag in source.nodes.iter().filter_map(|n| if let super::source::Node::Open(t) = n { Some(t) } else { None }) {
        for class in tag.classes() {
            let base = class.rsplit(':').next().unwrap_or(class).trim_start_matches('!');
            if let Some(inner) = base.strip_prefix("rounded-[").or_else(|| base.split_once("-[").filter(|(p, _)| p.starts_with("rounded-")).map(|(_, r)| r)) {
                if system.radii.is_empty() || inner.contains("var(") {
                    continue;
                }
                let said = match px(inner.trim_end_matches(']')).and_then(|v| system.radius_token(v)) {
                    Some(t) => format!("{base} is `{t}`"),
                    None => format!("{base} is off the radius scale ({})", radius_scale(system)),
                };
                out.push(Finding::new("system-token", file, Some(tag.line), said));
            }
            if base.starts_with("shadow-[") && !system.shadows.is_empty() && !base.contains("var(") {
                out.push(Finding::new("system-token", file, Some(tag.line), format!("{base} — the system's shadows are {}", system.shadows.iter().take(5).cloned().collect::<Vec<_>>().join(", "))));
            }
        }
    }
}

fn components(source: &Source, kit: &Kit, file: &str, out: &mut Vec<Finding>) {
    let cat = catalog::catalog();
    let present: Vec<&catalog::Component> = cat.components.iter().filter(|c| kit.has(&c.name)).collect();
    if present.is_empty() {
        return;
    }
    for tag in source.nodes.iter().filter_map(|n| if let super::source::Node::Open(t) = n { Some(t) } else { None }) {
        // A component (capitalised in JSX) is already the system's, or another's import.
        if tag.name.chars().next().is_some_and(char::is_uppercase) || tag.name.contains('.') {
            continue;
        }
        let classes = tag.classes();
        let styled = !classes.is_empty() || tag.attr("style").is_some_and(|s| !s.trim().is_empty()) || tag.has(":style");
        let attr = |n: &str| tag.attr(n).map(str::to_owned);
        let Some(comp) = present.iter().find(|c| c.matchers().iter().any(|m| m.matches(&tag.name, &attr, &classes, styled))) else { continue };
        let what = match (tag.attr("role"), classes.first()) {
            (Some(role), _) => format!("<{} role=\"{role}\">", tag.name),
            (None, Some(c)) => format!("<{} class=\"{c}…\">", tag.name),
            (None, None) => format!("<{}>", tag.name),
        };
        out.push(Finding::new(
            "system-component",
            file,
            Some(tag.line),
            format!("{what} re-makes the registry's {}: use <{}> from `{}/{}`", pascal(&comp.name), pascal(&comp.name), kit.ui.trim_end_matches('/'), comp.name),
        ));
    }
}

fn imports(source: &Source, kit: &Kit, file: &str, out: &mut Vec<Finding>) {
    let ui = kit.ui.trim_end_matches('/');
    for (n, line) in source.text.lines().enumerate() {
        for spec in registry::imports(line) {
            let at = Some(n as u32 + 1);
            if let Some(pkg) = registry::package(&spec) {
                if UTILITIES.contains(&pkg.as_str()) {
                    continue;
                }
                if let Some(comp) = wrapper_of(&pkg, kit) {
                    out.push(Finding::new("system-import", file, at, format!("`{spec}` imported directly: the registry's {} wraps it — import from `{ui}/{comp}`", pascal(&comp))));
                } else if OTHER_KITS.iter().any(|k| if k.ends_with('/') { pkg.starts_with(k) } else { pkg == *k }) {
                    out.push(Finding::new("system-import", file, at, format!("`{spec}` is another component library beside the project's own: build it from the registry (`{ui}/…`)")));
                } else if kit.wrapped.contains(&pkg) {
                    out.push(Finding::new("system-import", file, at, format!("`{spec}` is what the registry's components are built on: use them (`{ui}/…`), not it directly")));
                }
            } else if spec.contains("/registry/") && !ui.contains("registry") {
                out.push(Finding::new("system-import", file, at, format!("`{spec}` reaches into the registry's source: import the installed component from `{ui}/…`")));
            }
        }
    }
}
