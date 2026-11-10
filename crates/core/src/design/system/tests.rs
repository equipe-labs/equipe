//! A design system found in each shape a project keeps one, its coverage
//! and token contrast; a component's matrix planned and judged from
//! measured results; a registry checked and installed offline; and product
//! code held to building from the system — only where there is one.

use std::path::{Path, PathBuf};

use super::*;
use crate::design::lint::{self, Tier};

fn project(files: &[(&str, &str)]) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("equipe-system-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // A repository of its own: nothing above it is its project.
    std::fs::create_dir_all(root.join(".git")).unwrap();
    for (name, body) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    root
}

const GLOBALS: &str = r#"@import "tailwindcss";
:root {
  --radius: 0.625rem;
  --background: oklch(1 0 0);
  --foreground: oklch(0.145 0 0);
  --primary: oklch(0.205 0 0);
  --primary-foreground: oklch(0.985 0 0);
  --muted: oklch(0.97 0 0);
  --muted-foreground: oklch(0.85 0 0);
  --ring: oklch(0.708 0 0);
}
.dark {
  --background: oklch(0.145 0 0);
  --foreground: oklch(0.985 0 0);
  --primary: oklch(0.922 0 0);
  --primary-foreground: oklch(0.205 0 0);
  --muted: oklch(0.269 0 0);
  --muted-foreground: oklch(0.708 0 0);
  --ring: oklch(0.556 0 0);
}
@theme inline {
  --color-background: var(--background);
  --color-primary: var(--primary);
  --radius-lg: var(--radius);
}
"#;

const COMPONENTS_JSON: &str = r#"{
  "$schema": "https://ui.shadcn.com/schema.json",
  "style": "new-york",
  "tailwind": { "config": "", "css": "app/globals.css", "baseColor": "neutral", "cssVariables": true },
  "aliases": { "components": "@/components", "ui": "@/components/ui", "utils": "@/lib/utils", "lib": "@/lib", "hooks": "@/hooks" },
  "registries": { "@acme": "https://acme.example/r/{name}.json" }
}"#;

const BUTTON: &str = r#"import * as React from "react"
import { Slot } from "@radix-ui/react-slot"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"

const buttonVariants = cva(
  "inline-flex items-center rounded-md text-sm font-medium focus-visible:ring-ring/50 disabled:opacity-50",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        destructive: "bg-destructive text-white",
        outline: "border bg-background",
        "ghost": "hover:bg-accent",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 px-3",
        lg: "h-10 px-6",
        icon: "size-9",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  }
)

function Button({ className, variant, size, asChild = false, ...props }) {
  const Comp = asChild ? Slot : "button"
  return <Comp data-slot="button" className={cn(buttonVariants({ variant, size, className }))} {...props} />
}

export { Button, buttonVariants }
"#;

const DIALOG: &str = "import * as DialogPrimitive from \"@radix-ui/react-dialog\"\nimport { cn } from \"@/lib/utils\"\nexport function Dialog(props) { return <DialogPrimitive.Root {...props} /> }\n";
const UTILS: &str = "import { clsx } from \"clsx\"\nimport { twMerge } from \"tailwind-merge\"\nexport function cn(...i) { return twMerge(clsx(i)) }\n";

fn registry_json(button_deps: &str) -> String {
    format!(
        r#"{{
  "$schema": "https://ui.shadcn.com/schema/registry.json",
  "name": "acme",
  "homepage": "https://acme.example",
  "items": [
    {{ "name": "utils", "type": "registry:lib", "dependencies": ["clsx", "tailwind-merge"], "files": [{{ "path": "registry/lib/utils.ts", "type": "registry:lib" }}], "meta": {{ "source": "https://ui.shadcn.com/docs/installation", "license": "MIT" }} }},
    {{ "name": "button", "type": "registry:ui", "title": "Button", "dependencies": ["@radix-ui/react-slot", "class-variance-authority"], "registryDependencies": [{button_deps}], "files": [{{ "path": "registry/ui/button.tsx", "type": "registry:ui" }}], "meta": {{ "source": "https://ui.shadcn.com/docs/components/button", "license": "MIT" }} }},
    {{ "name": "dialog", "type": "registry:ui", "title": "Dialog", "dependencies": ["@radix-ui/react-dialog"], "registryDependencies": ["@acme/utils"], "files": [{{ "path": "registry/ui/dialog.tsx", "type": "registry:ui" }}], "meta": {{ "source": "https://ui.shadcn.com/docs/components/dialog", "license": "MIT" }} }}
  ]
}}"#
    )
}

/// A shadcn-style project with a registry: tokens as CSS, components.json, the
/// components' source, a docs page and the built items.
fn shadcn_project(extra: &[(&str, &str)]) -> PathBuf {
    let reg = registry_json("\"https://acme.example/r/utils.json\"");
    let mut files: Vec<(&str, &str)> = vec![
        ("app/globals.css", GLOBALS),
        ("components.json", COMPONENTS_JSON),
        ("registry.json", reg.as_str()),
        ("registry/ui/button.tsx", BUTTON),
        ("registry/ui/dialog.tsx", DIALOG),
        ("registry/lib/utils.ts", UTILS),
        ("components/ui/button.tsx", BUTTON),
        ("components/ui/dialog.tsx", DIALOG),
        ("lib/utils.ts", UTILS),
        ("app/docs/components/button/page.tsx", "<div data-example=\"variant=default\"><Button>Save</Button></div>"),
    ];
    files.extend_from_slice(extra);
    let root = project(&files);
    build(&root);
    root
}

/// What `shadcn build` writes, for the tests: each item with its files' contents.
fn build(root: &Path) {
    let reg = registry::Registry::read(&root.join("registry.json")).unwrap();
    std::fs::create_dir_all(root.join("public/r")).unwrap();
    for item in &reg.items {
        let mut v = serde_json::to_value(item).unwrap();
        for (i, f) in item.files.iter().enumerate() {
            v["files"][i]["content"] = serde_json::Value::String(std::fs::read_to_string(root.join(&f.path)).unwrap_or_default());
        }
        std::fs::write(root.join(format!("public/r/{}.json", item.name)), v.to_string()).unwrap();
    }
}

#[test]
fn a_shadcn_project_is_found_whole_tokens_registry_components_docs_and_build() {
    let root = shadcn_project(&[]);
    let found = detect(&root);
    assert!(found.has_system());
    assert_eq!(found.tokens.len(), 1, "{:#?}", found.tokens);
    assert_eq!(found.tokens[0].shapes, [Shape::CssVariables, Shape::TailwindTheme]);
    assert!(found.tokens[0].dark);
    assert_eq!(found.components_json.as_ref().unwrap().css.as_deref(), Some("app/globals.css"));
    assert_eq!(found.registry.as_ref().unwrap().items.len(), 3);
    assert!(found.components_dir.as_ref().unwrap().ends_with("components/ui"));
    assert_eq!(found.components, ["button", "dialog"]);
    assert_eq!(found.doc("button").map(|d| d.path.ends_with("button/page.tsx")), Some(true));
    assert!(found.built.as_ref().unwrap().ends_with("public/r"));
    let cov = coverage(&found);
    assert_eq!(cov.line().split(',').next(), Some("2/49 components"));
    assert!(cov.missing.contains(&"accordion".to_owned()) && !cov.missing.contains(&"button".to_owned()));
    assert_eq!(cov.undocumented, ["dialog"]);
    let said = missing(&found).join("\n");
    assert!(said.contains("2/49 components, missing: accordion, alert,") && said.contains("No docs page for: dialog"), "{said}");
    assert!(!said.contains("No registry.json") && !said.contains("No design tokens"), "{said}");
    let text = report(&found);
    assert!(text.contains("✓ tokens: app/globals.css (CSS variables, Tailwind @theme)") && text.contains("docs pages: 1/2"), "{text}");
}

#[test]
fn each_shape_of_tokens_is_found_where_the_project_keeps_it() {
    // DTCG in design/, a Tailwind theme in src/, and components under src/ by their alias.
    let dtcg = r##"{ "color": { "$type": "color", "bg": { "$value": "#ffffff", "$extensions": { "mode": { "dark": "#000000" } } }, "text": { "$value": "#111111" }, "brand": { "$value": "#1d4ed8" } } }"##;
    let theme = "@theme {\n  --color-bg: #fff;\n  --color-text: #111;\n  --color-brand: #1d4ed8;\n}\n";
    let root = project(&[
        ("design/tokens.json", dtcg),
        ("src/styles/theme.css", theme),
        ("components.json", r#"{ "tailwind": { "css": "src/styles/theme.css" }, "aliases": { "ui": "@/components/ui" } }"#),
        ("src/components/ui/tabs.tsx", "export const Tabs = 1"),
        ("src/components/ui/input-otp.tsx", "export const InputOTP = 1"),
        ("stories/Tabs.stories.tsx", "export default {}"),
    ]);
    let found = detect(&root);
    let shapes: Vec<Vec<Shape>> = found.tokens.iter().map(|t| t.shapes.clone()).collect();
    assert!(shapes.contains(&vec![Shape::Dtcg]) && shapes.contains(&vec![Shape::TailwindTheme]), "{shapes:?}");
    assert!(found.tokens.iter().find(|t| t.shapes == [Shape::Dtcg]).unwrap().dark);
    assert!(found.components_dir.as_ref().unwrap().ends_with("src/components/ui"));
    assert_eq!(found.components, ["input-otp", "tabs"]);
    assert!(found.doc("tabs").is_some(), "a story is a docs page");
    assert!(missing(&found).iter().any(|m| m.contains("No registry.json")));
}

#[test]
fn a_project_with_nothing_is_told_where_to_start() {
    let root = project(&[("index.html", "<h1>Hi</h1>"), ("app.css", ":root { --x: 1px; }")]);
    let found = detect(&root);
    assert!(!found.has_system() && found.tokens.is_empty(), "a few properties of its own aren't a theme");
    let said = missing(&found).join("\n");
    assert!(said.contains("No design tokens") && said.contains("No components.json") && said.contains("No registry.json"), "{said}");
    assert!(!said.contains("/49"), "no coverage asked of a project with no system");
}

#[test]
fn the_tokens_text_on_its_surface_is_held_to_aa_in_each_mode() {
    let root = shadcn_project(&[]);
    let found = detect(&root);
    let said = token_contrast(&found.all_tokens()).join("\n");
    assert!(said.contains("`muted-foreground` on `muted` is") && said.contains("in light"), "{said}");
    assert!(!said.contains("`primary-foreground` on `primary`"), "{said}");
    assert!(said.contains("the focus ring `ring` on `background`"), "{said}");
    assert_eq!(super::surface_of("color.on-primary").as_deref(), Some("color.primary"));
    assert_eq!(super::surface_of("foreground").as_deref(), Some("background"));
}

// ── the matrix ──

#[test]
fn a_components_matrix_is_planned_from_its_pattern_and_its_declared_variants() {
    let cat = catalog::catalog();
    let themes = vec!["light".to_owned(), "dark".to_owned()];
    let plan = matrix::plan(cat.get("button").unwrap(), Some(BUTTON), "http://localhost:3000/docs/components/button", &[390, 1280], &themes);
    assert_eq!(plan.variants, [("variant".to_owned(), vec!["default".to_owned(), "destructive".into(), "outline".into(), "ghost".into()]), ("size".to_owned(), vec!["default".to_owned(), "sm".into(), "lg".into(), "icon".into()])]);
    assert_eq!(plan.states, ["default", "hover", "focus-visible", "active", "disabled"]);
    assert_eq!(matrix::per_example(&plan), 5 * 2 * 2);
    let dialog = matrix::plan(cat.get("dialog").unwrap(), None, "x.html", &[1280], &themes);
    assert_eq!(dialog.open.as_ref().map(|o| o.how.as_str()), Some("click"));
    assert!(dialog.keyboard.contains(&"focus-trapped".to_owned()) && dialog.variants.is_empty());
    // tailwind-variants writes the same shape.
    let tv = matrix::variants("const badge = tv({ base: 'x', variants: { tone: { neutral: 'a', danger: 'b' } } })");
    assert_eq!(tv, [("tone".to_owned(), vec!["neutral".to_owned(), "danger".into()])]);
}

const MEASURED: &str = r#"{
  "url": "http://localhost:3000/docs/components/button",
  "sheets": ["/x/sheet-390.png"],
  "runs": [
    { "width": 390, "theme": "light",
      "examples": [
        { "example": "variant=default", "target": true, "states": [
          { "state": "default", "applied": true, "picture": "/x/1-default.png", "look": "a", "texts": [{ "text": "Save", "fg": "rgba(250, 250, 250, 1)", "bg": "rgb(23, 23, 23)", "size": 14, "weight": 500 }] },
          { "state": "hover", "applied": true, "picture": "/x/1-hover.png", "look": "a", "texts": [] },
          { "state": "focus-visible", "applied": true, "picture": "/x/1-focus.png", "look": "a", "texts": [], "focus_shows": false },
          { "state": "active", "applied": true, "picture": "/x/1-active.png", "look": "b", "texts": [] },
          { "state": "disabled", "applied": true, "picture": "/x/1-disabled.png", "look": "c", "texts": [{ "text": "Save", "fg": "rgba(200, 200, 200, 1)", "bg": "rgb(255, 255, 255)", "size": 14, "weight": 500 }] }
        ] },
        { "example": "variant=outline", "target": true, "states": [
          { "state": "default", "applied": true, "picture": "/x/2-default.png", "look": "d", "texts": [{ "text": "Cancel", "fg": "rgba(160, 160, 160, 1)", "bg": "rgb(255, 255, 255)", "size": 14, "weight": 500 }] },
          { "state": "hover", "applied": true, "picture": "/x/2-hover.png", "look": "e" },
          { "state": "focus-visible", "applied": true, "picture": "/x/2-focus.png", "look": "f", "focus_shows": true },
          { "state": "active", "applied": true, "picture": "/x/2-active.png", "look": "g" },
          { "state": "disabled", "applied": false, "why": "its part has no disabled state to set" }
        ] }
      ],
      "axe": { "ran": true, "violations": [
        { "id": "button-name", "impact": "critical", "help": "Buttons must have discernible text", "helpUrl": "https://dequeuniversity.com/rules/axe/4.10/button-name", "nodes": [{ "target": ["[data-example=\"variant=outline\"] > button"], "failureSummary": "Fix any of the following" }] },
        { "id": "region", "impact": "moderate", "help": "All page content should be contained by landmarks", "nodes": [] }
      ] },
      "keyboard": [
        { "example": "variant=default", "check": "tab-reaches", "pass": true, "detail": "" },
        { "example": "variant=default", "check": "enter-activates", "pass": false, "detail": "nothing happened" },
        { "example": "variant=default", "check": "disabled-skipped", "pass": true, "detail": "" }
      ] }
  ]
}"#;

#[test]
fn measured_states_axe_and_keys_are_judged_in_plain_words() {
    let plan = matrix::plan(catalog::catalog().get("button").unwrap(), Some(BUTTON), "http://localhost:3000/docs/components/button", &[390], &["light".to_owned()]);
    let measured: matrix::Measured = serde_json::from_str(MEASURED).unwrap();
    let r = matrix::judge(&plan, &measured, 7);
    let text = r.text();
    let has = |s: &str| r.findings.iter().any(|f| f.words.contains(s));
    assert!(!r.passes());
    assert_eq!(r.examples, ["variant=default", "variant=outline"]);
    assert_eq!(r.shown, (9, 2 * 5));
    assert!(has("the docs page doesn't show variant=destructive") && has("size=sm"), "{text}");
    assert!(!has("variant=default") && !has("size=default"), "shown, and the default size is what an example without one shows: {text}");
    assert!(has("focus doesn't show when the keyboard reaches it") && has("Reached by the keyboard looks the same as at rest"), "{text}");
    assert!(r.findings.iter().any(|f| f.words.contains("The pointer over it looks the same") && f.level == matrix::Level::Note), "a hover that changes nothing is noted, not failed");
    assert!(has("couldn't show it disabled: its part has no disabled state to set"), "{text}");
    assert!(has("“Cancel” is 2.6:1") && !has("“Save” is"), "disabled text is left out of contrast, the default's passes: {text}");
    assert!(r.findings.iter().any(|f| f.check == "axe:button-name" && f.level == matrix::Level::Fail && f.words.contains("Buttons must have discernible text")));
    assert!(r.findings.iter().any(|f| f.check == "axe:region" && f.level == matrix::Level::Note));
    assert!(has("Enter activates it — it didn't: nothing happened"), "{text}");
    assert!(has("Space activates or toggles it wasn't tried"), "{text}");
    assert!(r.axe && text.contains("axe run") && text.contains("sheet: /x/sheet-390.png"));
    assert!(r.mark_line().starts_with(matrix::MARK));
    // Kept, and read back.
    let root = project(&[]);
    let file = matrix::write(&root.join("design"), &r).unwrap();
    assert!(file.ends_with("design/checks/components/button.json"));
    assert_eq!(matrix::read(&root.join("design"), "Button"), Some(r));
}

#[test]
fn a_page_with_no_examples_says_how_to_mark_them() {
    let plan = matrix::plan(catalog::catalog().get("tabs").unwrap(), None, "docs/tabs.html", &[1280], &["light".to_owned()]);
    let r = matrix::judge(&plan, &matrix::Measured { url: "x".into(), runs: vec![matrix::Run { width: 1280, theme: "light".into(), ..Default::default() }], sheets: vec![] }, 1);
    assert!(r.findings.iter().any(|f| f.words.contains("data-example=")), "{}", r.text());
    assert!(r.findings.iter().any(|f| f.words.contains("accessibility rules weren't run")));
}

// ── the registry ──

#[test]
fn a_registry_is_checked_for_files_imports_origins_and_other_brands_assets() {
    let bad_button = BUTTON.to_owned() + "\nimport { Loader } from \"lucide-react\"\nimport { Dialog } from \"@/components/ui/dialog\"\nexport const avatar = \"https://github.com/shadcn.png\"\n";
    let reg = r#"{
  "name": "acme",
  "items": [
    { "name": "button", "type": "registry:ui", "dependencies": ["@radix-ui/react-slot", "class-variance-authority", "left-pad"], "registryDependencies": ["dialog"], "files": [{ "path": "registry/ui/button.tsx", "type": "registry:ui" }], "meta": { "source": "https://ui.shadcn.com/docs/components/button" } },
    { "name": "dialog", "type": "registry:ui", "files": [{ "path": "registry/ui/dialog.tsx", "type": "registry:ui" }, { "path": "registry/ui/gone.tsx", "type": "registry:ui" }] },
    { "name": "logo", "type": "registry:file", "files": [{ "path": "registry/assets/vercel-logo.svg", "type": "registry:file" }], "meta": { "source": "original" } },
    { "name": "button", "type": "registry:widget", "files": [] }
  ]
}"#;
    let root = project(&[("registry.json", reg), ("registry/ui/button.tsx", bad_button.as_str()), ("registry/ui/dialog.tsx", DIALOG), ("registry/assets/vercel-logo.svg", "<svg/>")]);
    let r = registry::Registry::read(&root.join("registry.json")).unwrap();
    let problems = registry::check(&r, None);
    let said: Vec<String> = problems.iter().map(registry::Problem::line).collect();
    let has = |s: &str| said.iter().any(|l| l.contains(s));
    assert!(has("no `homepage`"), "{said:#?}");
    assert!(has("button: doesn't say its source's licence"), "{said:#?}");
    assert!(has("dialog: doesn't say where it came from"), "{said:#?}");
    assert!(has("button: imports `lucide-react` without declaring it"), "{said:#?}");
    assert!(has("dialog: imports `@radix-ui/react-dialog` without declaring it"), "{said:#?}");
    assert!(has("button: declares `left-pad`, which none of its files imports"), "{said:#?}");
    assert!(has("needs `dialog` by its bare name, which installs shadcn's own `dialog`, not this registry's: write `@acme/dialog`"), "{said:#?}");
    assert!(has("dialog: registry/ui/gone.tsx isn't there"), "{said:#?}");
    assert!(has("dialog: registry/ui/dialog.tsx imports `@/lib/utils` but `registryDependencies` doesn't name `utils`"), "{said:#?}");
    assert!(has("https://github.com/shadcn.png: another brand's picture"), "{said:#?}");
    assert!(has("registry/assets/vercel-logo.svg is named for vercel"), "{said:#?}");
    assert!(has("is a registry:file with no `target`"), "{said:#?}");
    assert!(has("button: listed twice") && has("`type` is `registry:widget`"), "{said:#?}");
    assert!(!has("button.tsx imports `@/components/ui/dialog` but"), "named in registryDependencies, if by its bare name: {said:#?}");
}

#[test]
fn a_clean_registry_passes_and_its_build_is_held_to_its_source() {
    let root = shadcn_project(&[]);
    let r = registry::Registry::read(&root.join("registry.json")).unwrap();
    let problems = registry::check(&r, Some(&root.join("public/r")));
    assert!(problems.iter().all(|p| p.level == registry::Level::Warn), "{problems:#?}");
    std::fs::write(root.join("registry/ui/dialog.tsx"), format!("{DIALOG}// changed\n")).unwrap();
    let problems = registry::check(&r, Some(&root.join("public/r")));
    assert!(problems.iter().any(|p| p.words.contains("registry/ui/dialog.tsx changed after it was built")), "{problems:#?}");
    std::fs::remove_file(root.join("public/r/utils.json")).unwrap();
    assert!(registry::check_built(&r, &root.join("public/r")).iter().any(|p| p.item.as_deref() == Some("utils") && p.words.contains("isn't built")));
}

#[test]
fn a_sample_install_resolves_offline_and_says_what_it_couldnt() {
    let root = shadcn_project(&[]);
    let r = registry::Registry::read(&root.join("registry.json")).unwrap();
    let scratch = project(&[]);
    let aliases = vec![("ui".to_owned(), "@/components/ui".to_owned()), ("lib".to_owned(), "@/lib".to_owned())];
    let done = registry::install(&r, &root.join("public/r"), "dialog", &scratch, &aliases);
    assert!(done.resolves(), "{done:#?}");
    assert_eq!(done.items, ["dialog", "utils"]);
    assert!(scratch.join("components/ui/dialog.tsx").is_file() && scratch.join("lib/utils.ts").is_file());
    assert!(done.packages.contains(&"@radix-ui/react-dialog".to_owned()) && done.packages.contains(&"clsx".to_owned()));
    // An item that needs one of its own by its bare name: shadcn's would come instead.
    let bare = project(&[]);
    std::fs::write(root.join("registry.json"), registry_json("\"utils\"")).unwrap();
    build(&root);
    let r = registry::Registry::read(&root.join("registry.json")).unwrap();
    let done = registry::install(&r, &root.join("public/r"), "button", &bare, &aliases);
    assert!(!done.resolves());
    assert!(done.unresolved.iter().any(|u| u.contains("by its bare name")), "{done:#?}");
    assert!(done.broken.iter().any(|b| b.contains("`@/lib/utils`")), "the import it would leave broken: {done:#?}");
}

// ── building from the system ──

fn lint_file(root: &Path, file: &str) -> lint::Report {
    let path = root.join(file);
    let design = lint::design_dir_of(&path).unwrap_or_else(|| root.join("design"));
    let system = lint::System::read(&design).with_kit(&path);
    let mut report = lint::lint_sources(&[path], &system);
    lint::apply_waivers(&mut report, &system.decisions_home(), None);
    report
}

fn system_rules(r: &lint::Report) -> Vec<(String, String)> {
    r.findings.iter().filter(|f| f.tier == Tier::System && f.open()).map(|f| (f.rule.clone(), f.evidence.clone())).collect()
}

const PRODUCT: &str = r#"import { Button } from "@/components/ui/button"
import * as Dialog from "@radix-ui/react-dialog"
import { cn } from "@/lib/utils"
import { Menu } from "@headlessui/react"
import { Thing } from "../../registry/ui/button"

export function Checkout() {
  return (
    <main className="p-4">
      <button className="rounded-md bg-primary px-4 text-sm">Pay</button>
      <Button>Save</Button>
      <div role="dialog" aria-label="Confirm">Sure?</div>
      <div className="rounded-[10px] bg-[#171717] text-primary">Total</div>
      <input type="checkbox" aria-label="Agree" />
      <button type="submit">Plain</button>
    </main>
  )
}
"#;

#[test]
fn product_code_is_held_to_building_from_the_system() {
    let root = shadcn_project(&[("app/checkout/page.tsx", PRODUCT)]);
    let r = lint_file(&root, "app/checkout/page.tsx");
    let found = system_rules(&r);
    let has = |rule: &str, s: &str| found.iter().any(|(id, e)| id == rule && e.contains(s));
    assert!(has("system-component", "<button class=\"rounded-md…\"> re-makes the registry's Button: use <Button> from `@/components/ui/button`"), "{}", r.text());
    assert!(has("system-component", "<div role=\"dialog\"> re-makes the registry's Dialog"), "{}", r.text());
    assert!(!has("system-component", "Plain") && !found.iter().any(|(_, e)| e.contains("Checkbox")), "an unstyled button, and a component the registry hasn't got, aren't flagged: {found:#?}");
    assert!(has("system-import", "`@radix-ui/react-dialog` imported directly: the registry's Dialog wraps it — import from `@/components/ui/dialog`"), "{found:#?}");
    assert!(has("system-import", "`@headlessui/react` is another component library"), "{found:#?}");
    assert!(has("system-import", "`../../registry/ui/button` reaches into the registry's source"), "{found:#?}");
    assert!(!found.iter().any(|(_, e)| e.contains("@/lib/utils") || e.starts_with("`@/components/ui/button`")), "{found:#?}");
    assert!(has("system-token", "rounded-[10px] is `radius`"), "{found:#?}");
    assert!(has("system-token", "bg-[#171717] is `primary`"), "a colour a token holds: {found:#?}");
    let s = r.summary();
    assert!(!s.passes() && s.floor.0 == s.floor.1, "the floor passes, the system doesn't: {s:?}");
    assert!(r.text().contains("Build from the system — fix before handing in"));
    // `--tier floor` keeps what blocks like it.
    let mut only = r.clone();
    only.only(Tier::Floor);
    assert!(only.findings.iter().any(|f| f.tier == Tier::System));
}

#[test]
fn the_systems_own_code_and_a_project_without_one_are_left_alone() {
    let root = shadcn_project(&[]);
    for own in ["components/ui/button.tsx", "registry/ui/dialog.tsx", "app/globals.css"] {
        let r = lint_file(&root, own);
        assert!(system_rules(&r).is_empty(), "{own}: {}", r.text());
    }
    // The same screen in a project with no registry and no components.json.
    let bare = project(&[("app/checkout/page.tsx", PRODUCT)]);
    let r = lint_file(&bare, "app/checkout/page.tsx");
    assert!(r.findings.iter().all(|f| f.tier != Tier::System), "{}", r.text());
    assert!(!r.checked.iter().chain(r.skipped.iter().map(|(id, _)| id)).any(|id| id.starts_with("system-")), "not even said as skipped");
    assert!(r.summary().system.is_empty());
}

#[test]
fn a_real_exception_is_kept_with_its_reason_scoped_and_the_floor_still_is_not() {
    let decision = "# Old checkout\nWaive system-component (app/legacy): the old checkout stays until T-40 moves it to Dialog\nWaive token-colour: we like it\n";
    let root = shadcn_project(&[("app/legacy/page.tsx", "<div role=\"dialog\" aria-label=\"Old\">x</div>\n"), ("app/new/page.tsx", "<div role=\"dialog\" aria-label=\"New\">x</div>\n"), ("design/decisions/2026-11-08-legacy.md", decision)]);
    let kept = lint_file(&root, "app/legacy/page.tsx");
    assert!(system_rules(&kept).is_empty() && kept.summary().passes(), "{}", kept.text());
    assert_eq!(kept.summary().waived, [("system-component".to_owned(), "the old checkout stays until T-40 moves it to Dialog".to_owned())]);
    assert!(kept.refused.iter().any(|r| r.contains("`token-colour` is a floor rule")));
    let new = lint_file(&root, "app/new/page.tsx");
    assert_eq!(system_rules(&new).len(), 1, "the waiver holds for its scope only");
    assert!(lint::waivers::decision("system-import", Some("app/legacy"), "the legacy menu until T-41 replaces it", None).is_ok());
}

#[test]
fn the_edit_time_check_hands_back_what_isnt_built_from_the_system() {
    let root = shadcn_project(&[("app/pay/page.tsx", "export const A = () => <main className=\"p-4\">x</main>\n")]);
    let file = root.join("app/pay/page.tsx");
    let written = "  <button className=\"bg-primary rounded-md\">Pay</button>\n";
    std::fs::write(&file, format!("export const A = () => (\n{written})\n")).unwrap();
    let input = serde_json::json!({ "tool_name": "Edit", "tool_input": { "file_path": file.to_string_lossy(), "new_string": written } });
    let said = lint::hook::run(&input, false).expect("handed back");
    assert!(said.contains("isn't built from the project's design system") && said.contains("system-component") && said.contains("<Button>"), "{said}");
    // Kept on purpose for this file: nothing handed back.
    std::fs::create_dir_all(root.join("design/decisions")).unwrap();
    std::fs::write(root.join("design/decisions/x.md"), "Waive system-component (app/pay): the payment provider's own button is required here by contract\n").unwrap();
    assert!(lint::hook::run(&input, false).is_none());
    // A project without a system: nothing about it.
    let bare = project(&[("app/pay/page.tsx", "")]);
    let file = bare.join("app/pay/page.tsx");
    std::fs::write(&file, format!("export const A = () => (\n{written})\n")).unwrap();
    let input = serde_json::json!({ "tool_name": "Edit", "tool_input": { "file_path": file.to_string_lossy(), "new_string": written } });
    assert!(lint::hook::run(&input, false).is_none());
}
