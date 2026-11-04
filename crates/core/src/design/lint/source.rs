//! A UI source read loosely, the way the lint needs it: its CSS declarations
//! (from a stylesheet, `<style>` blocks, `style` attributes, styled
//! templates and inline style objects), its elements in order with their
//! attributes, and the text between them. Not a parser: forgiving of what it
//! doesn't know, and never fails.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A stylesheet: `.css`, `.scss`, `.less`.
    Css,
    /// Markup with its style and script inside: `.html`, `.vue`, `.svelte`.
    Markup,
    /// Script with markup inside: `.tsx`, `.jsx`.
    Script,
}

impl Kind {
    pub fn of(path: &Path) -> Option<Kind> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "css" | "scss" | "less" | "pcss" => Some(Kind::Css),
            "html" | "htm" | "vue" | "svelte" | "astro" => Some(Kind::Markup),
            "tsx" | "jsx" => Some(Kind::Script),
            _ => None,
        }
    }
}

/// `prop: value`, the property in kebab case.
#[derive(Debug, Clone, PartialEq)]
pub struct Decl {
    pub prop: String,
    pub value: String,
    pub line: u32,
    /// The rule's selector it is in (a stylesheet's); `None` inline.
    pub selector: Option<String>,
    /// Inside `@media (prefers-reduced-motion…)`.
    pub reduced: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    /// Lower case for HTML's elements; a component keeps its capital.
    pub name: String,
    pub attrs: Vec<(String, Option<String>)>,
    pub line: u32,
    pub self_closing: bool,
}

impl Tag {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_deref().unwrap_or(""))
    }

    pub fn has(&self, name: &str) -> bool {
        self.attrs.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    /// Its classes, from `class`, `className` or `:class`.
    pub fn classes(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for (n, v) in &self.attrs {
            if matches!(n.as_str(), "class" | "className" | ":class" | "class:list") {
                if let Some(v) = v {
                    out.extend(v.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '{' | '}' | ',' | '(' | ')')).filter(|c| !c.is_empty()));
                }
            }
        }
        out
    }

    /// Whether any attribute's name begins so (`on`, `@`, `v-on:`).
    pub fn has_prefix(&self, prefix: &str) -> bool {
        self.attrs.iter().any(|(n, _)| n.starts_with(prefix))
    }

    pub fn is_heading(&self) -> Option<u8> {
        let b = self.name.as_bytes();
        (b.len() == 2 && b[0] == b'h' && (b'1'..=b'6').contains(&b[1])).then(|| b[1] - b'0')
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Open(Tag),
    Close { name: String, line: u32 },
    /// Text between elements; `expr` when it held an expression (`{label}`).
    Text { text: String, line: u32, expr: bool },
    Comment { text: String, line: u32 },
}

#[derive(Debug, Clone)]
pub struct Source {
    pub path: String,
    pub kind: Kind,
    pub text: String,
    pub nodes: Vec<Node>,
    pub decls: Vec<Decl>,
    starts: Vec<usize>,
}

/// Elements that never hold anything.
const VOID: [&str; 14] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr", "param"];

pub fn is_void(name: &str) -> bool {
    VOID.contains(&name)
}

impl Source {
    pub fn read(path: &Path, text: String) -> Option<Source> {
        let kind = Kind::of(path)?;
        Some(Source::new(path.to_string_lossy().into_owned(), kind, text))
    }

    pub fn new(path: String, kind: Kind, text: String) -> Source {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        let mut source = Source { path, kind, text, nodes: Vec::new(), decls: Vec::new(), starts };
        match kind {
            Kind::Css => {
                let text = source.text.clone();
                source.decls = css(&text, 0, &source.starts);
            }
            Kind::Markup | Kind::Script => source.markup(),
        }
        source
    }

    /// The line (from 1) at byte `at`.
    pub fn line_at(&self, at: usize) -> u32 {
        line_of(&self.starts, at)
    }

    fn markup(&mut self) {
        let text = self.text.clone();
        let bytes = text.as_bytes();
        let script = self.kind == Kind::Script;
        let mut nodes = Vec::new();
        let mut decls = Vec::new();
        let mut depth: i32 = 0;
        let mut i = 0;
        let mut text_from = 0;
        let flush = |from: usize, to: usize, depth: i32, nodes: &mut Vec<Node>, starts: &[usize]| {
            if to <= from || (script && depth <= 0) {
                return;
            }
            let raw = &text[from..to];
            let (words, expr) = without_expressions(raw, script);
            if !words.trim().is_empty() || expr {
                nodes.push(Node::Text { text: words, line: line_of(starts, from), expr });
            }
        };
        while i < bytes.len() {
            if bytes[i] != b'<' {
                i += 1;
                continue;
            }
            if text[i..].starts_with("<!--") {
                flush(text_from, i, depth, &mut nodes, &self.starts);
                let end = text[i..].find("-->").map_or(text.len(), |e| i + e + 3);
                nodes.push(Node::Comment { text: text[i + 4..end.saturating_sub(3).max(i + 4)].to_owned(), line: line_of(&self.starts, i) });
                i = end;
                text_from = i;
                continue;
            }
            let next = bytes.get(i + 1).copied().unwrap_or(b' ');
            // Inside markup, any `<a` or `</` is markup; outside it, only where an expression starts.
            if script && !(depth > 0 && (next == b'/' || next.is_ascii_alphabetic())) && !opens_markup(bytes, i) {
                i += 1;
                continue;
            }
            if next == b'/' {
                let name_end = text[i + 2..].find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_'))).map_or(text.len(), |e| i + 2 + e);
                let name = &text[i + 2..name_end];
                let Some(close) = text[name_end..].find('>') else { break };
                flush(text_from, i, depth, &mut nodes, &self.starts);
                if !name.is_empty() {
                    nodes.push(Node::Close { name: normal_name(name), line: line_of(&self.starts, i) });
                    depth -= 1;
                } else {
                    depth -= 1;
                }
                i = name_end + close + 1;
                text_from = i;
                continue;
            }
            if next == b'>' {
                // A fragment.
                flush(text_from, i, depth, &mut nodes, &self.starts);
                depth += 1;
                i += 2;
                text_from = i;
                continue;
            }
            if !next.is_ascii_alphabetic() {
                i += 1;
                continue;
            }
            let Some((tag, end)) = open_tag(&text, i, &self.starts) else {
                i += 1;
                continue;
            };
            flush(text_from, i, depth, &mut nodes, &self.starts);
            let name = tag.name.clone();
            let closes_itself = tag.self_closing || (!script && is_void(&name));
            for (n, v) in &tag.attrs {
                if n == "style"
                    && let Some(v) = v
                {
                    decls.extend(inline(v, tag.line, script));
                }
            }
            nodes.push(Node::Open(tag));
            i = end;
            if !closes_itself {
                depth += 1;
            }
            // What a script or style holds is theirs, not text.
            if !script && (name == "style" || name == "script") {
                let close = format!("</{name}");
                let inner_end = text[i..].to_ascii_lowercase().find(&close).map_or(text.len(), |e| i + e);
                if name == "style" {
                    decls.extend(css(&text[i..inner_end], i, &self.starts));
                } else {
                    decls.extend(js_styles(&text[i..inner_end], i, &self.starts));
                }
                i = inner_end;
            }
            text_from = i;
        }
        flush(text_from, text.len(), depth, &mut nodes, &self.starts);
        if script {
            decls.extend(js_styles(&text, 0, &self.starts));
            decls.extend(templates(&text, &self.starts));
        }
        self.nodes = nodes;
        self.decls = decls;
    }

    /// The text of every node, joined: what a page says.
    pub fn words(&self) -> String {
        self.nodes.iter().filter_map(|n| if let Node::Text { text, .. } = n { Some(text.as_str()) } else { None }).collect::<Vec<_>>().join(" ")
    }
}

fn line_of(starts: &[usize], at: usize) -> u32 {
    (starts.partition_point(|&s| s <= at)) as u32
}

fn normal_name(name: &str) -> String {
    if name.chars().next().is_some_and(char::is_uppercase) || name.contains('.') { name.to_owned() } else { name.to_ascii_lowercase() }
}

/// In a script, `<` opens markup only where an expression could start —
/// after `(`, `=`, `return`, `?`, `:`, `&&`, `,`, `[`, `{`, `>` or a line's
/// start — never in `a < b` or `useState<string>`.
fn opens_markup(bytes: &[u8], at: usize) -> bool {
    let next = bytes.get(at + 1).copied().unwrap_or(b' ');
    if !(next.is_ascii_alphabetic() || next == b'/' || next == b'>') {
        return false;
    }
    let mut j = at;
    while j > 0 {
        j -= 1;
        let c = bytes[j];
        if c == b' ' || c == b'\t' || c == b'\r' {
            continue;
        }
        if c == b'\n' {
            return true;
        }
        if matches!(c, b'(' | b'=' | b'?' | b':' | b'&' | b'|' | b',' | b'[' | b'{' | b'>' | b'}') {
            return true;
        }
        // `return <div>`
        return bytes[..=j].ends_with(b"return");
    }
    true
}

/// An opening tag at `at` (`<name …>`): the tag, and where it ends.
fn open_tag(text: &str, at: usize, starts: &[usize]) -> Option<(Tag, usize)> {
    let bytes = text.as_bytes();
    let mut i = at + 1;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'.' | b':' | b'_')) {
        i += 1;
    }
    let name = normal_name(&text[at + 1..i]);
    let mut attrs = Vec::new();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let c = *bytes.get(i)?;
        if c == b'>' {
            return Some((Tag { name, attrs, line: line_of(starts, at), self_closing: false }, i + 1));
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'>') {
            return Some((Tag { name, attrs, line: line_of(starts, at), self_closing: true }, i + 2));
        }
        if c == b'{' {
            // `{...props}`
            i = balanced(bytes, i)?;
            continue;
        }
        let from = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        if i == from {
            i += 1;
            continue;
        }
        let key = text[from..i].to_owned();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            attrs.push((key, None));
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = match *bytes.get(i)? {
            q @ (b'"' | b'\'') => {
                let end = text[i + 1..].find(q as char)? + i + 1;
                let v = text[i + 1..end].to_owned();
                i = end + 1;
                v
            }
            b'{' => {
                let end = balanced(bytes, i)?;
                let v = text[i + 1..end - 1].to_owned();
                i = end;
                v
            }
            _ => {
                let from = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                text[from..i].to_owned()
            }
        };
        attrs.push((key, Some(value)));
    }
}

/// Past the `{…}` (with strings inside) opening at `at`.
fn balanced(bytes: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = at;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(_) if c == b'\\' => i += 1,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                b'"' | b'\'' | b'`' => quote = Some(c),
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// Text with its `{…}` expressions taken out (a script's); whether it had one.
/// A string literal inside an expression stays, as it is shown.
fn without_expressions(raw: &str, script: bool) -> (String, bool) {
    if !script && !raw.contains("{{") {
        return (decode(raw), false);
    }
    let bytes = raw.as_bytes();
    let mut out = String::new();
    let mut expr = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let end = balanced(bytes, i).unwrap_or(bytes.len());
            let inner = raw[i..end].trim_matches(|c| c == '{' || c == '}').trim();
            // `{"Save"}` and `{'Save'}` are words; anything else an expression.
            if let Some(words) = inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')).or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))) {
                out.push_str(words);
            } else if !inner.is_empty() && !inner.starts_with("/*") {
                expr = true;
                out.push(' ');
            }
            i = end;
            continue;
        }
        let ch = raw[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    (decode(&out), expr)
}

fn decode(text: &str) -> String {
    text.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'")
}

/// The declarations of a stylesheet (or a `<style>` block at byte `offset`).
pub fn css(text: &str, offset: usize, starts: &[usize]) -> Vec<Decl> {
    let mut out = Vec::new();
    // The selectors open around where we are, and whether each is reduced motion's.
    let mut stack: Vec<(String, bool)> = Vec::new();
    let mut from = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let end = text[i + 2..].find("*/").map_or(text.len(), |e| i + 2 + e + 2);
            i = end;
            from = i;
            continue;
        }
        match c {
            b'"' | b'\'' => quote = Some(c),
            b'{' => {
                let selector = text[from..i].trim().to_owned();
                let reduced = stack.iter().any(|(_, r)| *r) || (selector.starts_with("@media") && selector.contains("prefers-reduced-motion") && selector.contains("reduce"));
                stack.push((selector, reduced));
                from = i + 1;
            }
            b';' | b'}' => {
                let piece = &text[from..i];
                if let Some((prop, value)) = piece.split_once(':')
                    && !prop.trim().is_empty()
                    && !prop.contains('{')
                    && stack.last().is_some_and(|(s, _)| !s.starts_with('@') || s.starts_with("@media") || s.starts_with("@supports"))
                {
                    let lead = piece.len() - piece.trim_start().len();
                    let selector = stack.iter().rev().map(|(s, _)| s.as_str()).find(|s| !s.starts_with('@')).map(str::to_owned);
                    out.push(Decl {
                        prop: kebab(prop.trim()),
                        value: value.trim().trim_end_matches("!important").trim().to_owned(),
                        line: line_of(starts, offset + from + lead),
                        selector,
                        reduced: stack.iter().any(|(_, r)| *r),
                    });
                }
                if c == b'}' {
                    stack.pop();
                }
                from = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// A `style` attribute: `color: red; padding: 4px` in markup, a style
/// object in a script (`{ color: 'red', padding: 4 }`).
fn inline(value: &str, line: u32, script: bool) -> Vec<Decl> {
    let pieces: Vec<&str> = if script { value.trim().trim_start_matches('{').trim_end_matches('}').split(',').collect() } else { value.split(';').collect() };
    pieces
        .into_iter()
        .filter_map(|p| {
            let (k, v) = p.split_once(':')?;
            let v = v.trim().trim_matches(|c| c == '\'' || c == '"' || c == '`').trim();
            (!k.trim().is_empty() && !v.is_empty()).then(|| Decl { prop: kebab(k.trim().trim_matches(|c| c == '\'' || c == '"')), value: px_number(v, script), line, selector: None, reduced: false })
        })
        .collect()
}

/// A bare number in a style object is pixels.
fn px_number(v: &str, script: bool) -> String {
    if script && v.parse::<f64>().is_ok() && v != "0" { format!("{v}px") } else { v.to_owned() }
}

/// Style properties as a script writes them, camel-cased, that the rules read.
const STYLE_PROPS: [&str; 30] = [
    "color", "background", "backgroundColor", "backgroundImage", "borderColor", "fill", "stroke", "fontFamily", "fontSize", "margin", "marginTop", "marginBottom", "marginLeft", "marginRight", "marginInline", "marginBlock", "padding", "paddingTop", "paddingBottom", "paddingLeft",
    "paddingRight", "paddingInline", "paddingBlock", "gap", "rowGap", "columnGap", "outline", "textTransform", "letterSpacing", "textAlign",
];

/// `prop: 'value'` and `prop: 12` in a script's style objects, wherever they are.
fn js_styles(text: &str, offset: usize, starts: &[usize]) -> Vec<Decl> {
    let mut out = Vec::new();
    for prop in STYLE_PROPS {
        let mut at = 0;
        while let Some(found) = text[at..].find(prop) {
            let start = at + found;
            at = start + prop.len();
            let before = text[..start].chars().next_back().unwrap_or(' ');
            if before.is_ascii_alphanumeric() || before == '-' || before == '_' || before == '.' {
                continue;
            }
            let rest = text[at..].trim_start();
            let Some(rest) = rest.strip_prefix(':') else { continue };
            let rest = rest.trim_start();
            let value = match rest.chars().next() {
                Some(q @ ('\'' | '"' | '`')) => rest[1..].find(q).map(|e| rest[1..1 + e].to_owned()),
                Some(c) if c.is_ascii_digit() => Some(rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>()).map(|n| if n == "0" { n } else { format!("{n}px") }),
                _ => None,
            };
            if let Some(value) = value.filter(|v| !v.contains("${")) {
                out.push(Decl { prop: kebab(prop), value, line: line_of(starts, offset + start), selector: None, reduced: false });
            }
        }
    }
    out
}

/// Styled templates in a script: styled.div`…`, css`…`, `<style jsx>{`…`}`.
fn templates(text: &str, starts: &[usize]) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find('`') {
        let open = at + found;
        let Some(len) = text[open + 1..].find('`') else { break };
        let close = open + 1 + len;
        let before = text[..open].trim_end();
        let styled = before.ends_with("css") || before.ends_with(')') && before.contains("styled") || before.rsplit(['\n', ' ', '=', '(']).next().is_some_and(|w| w.starts_with("styled.") || w.starts_with("keyframes") || w == "createGlobalStyle");
        let inner = &text[open + 1..close];
        if styled || (before.ends_with('{') && inner.contains('{') && inner.contains(':')) {
            let body = if inner.contains('{') { inner.to_owned() } else { format!("&{{{inner}}}") };
            let shift = if inner.contains('{') { 0 } else { 2 };
            for mut d in css(&body, open + 1, starts) {
                if shift > 0 {
                    d.selector = None;
                }
                out.push(d);
            }
        }
        at = close + 1;
    }
    out
}

/// `backgroundColor` → `background-color`.
pub fn kebab(prop: &str) -> String {
    if prop.starts_with("--") {
        return prop.to_owned();
    }
    let mut out = String::new();
    for c in prop.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(name: &str, text: &str) -> Source {
        Source::read(Path::new(name), text.to_owned()).unwrap()
    }

    #[test]
    fn a_stylesheet_gives_each_declaration_its_selector_line_and_motion_context() {
        let s = read("a.css", ".card {\n  color: #123456;\n  padding: 13px !important;\n}\n@media (prefers-reduced-motion: reduce) {\n  .card { animation: none; }\n}\n/* a: b; */");
        let props: Vec<(&str, &str, u32, bool)> = s.decls.iter().map(|d| (d.prop.as_str(), d.value.as_str(), d.line, d.reduced)).collect();
        assert_eq!(props, vec![("color", "#123456", 2, false), ("padding", "13px", 3, false), ("animation", "none", 6, true)]);
        assert_eq!(s.decls[0].selector.as_deref(), Some(".card"));
    }

    #[test]
    fn markup_gives_elements_attributes_text_and_its_style() {
        let s = read("a.html", "<html><style>h1 { color: red }</style>\n<h1 class=\"big title\">Hello &amp; welcome</h1>\n<img src=x.png><button style=\"padding: 3px\"></button><!-- source: crm --></html>");
        let opens: Vec<&str> = s.nodes.iter().filter_map(|n| if let Node::Open(t) = n { Some(t.name.as_str()) } else { None }).collect();
        assert_eq!(opens, vec!["html", "style", "h1", "img", "button"]);
        let h1 = s.nodes.iter().find_map(|n| if let Node::Open(t) = n { (t.name == "h1").then_some(t) } else { None }).unwrap();
        assert_eq!(h1.classes(), vec!["big", "title"]);
        assert_eq!(h1.line, 2);
        assert!(s.words().contains("Hello & welcome"));
        assert!(s.decls.iter().any(|d| d.prop == "color" && d.value == "red"));
        assert!(s.decls.iter().any(|d| d.prop == "padding" && d.value == "3px"));
        assert!(s.nodes.iter().any(|n| matches!(n, Node::Comment { text, .. } if text.contains("source: crm"))));
    }

    #[test]
    fn a_script_finds_its_markup_and_style_objects_but_not_its_comparisons_or_generics() {
        let s = read(
            "A.tsx",
            "const [a, setA] = useState<string>('');\nif (a < b) {}\nexport function A() {\n  return (\n    <div className=\"card\" style={{ color: '#fff', padding: 12 }} {...rest}>\n      <Button onClick={() => go()}>{t('save')}</Button>\n      <span>{\"Hi\"} there</span>\n    </div>\n  );\n}\nconst Box = styled.div`\n  margin: 7px;\n`;",
        );
        let opens: Vec<String> = s.nodes.iter().filter_map(|n| if let Node::Open(t) = n { Some(t.name.clone()) } else { None }).collect();
        assert_eq!(opens, vec!["div", "Button", "span"]);
        assert!(s.decls.iter().any(|d| d.prop == "color" && d.value == "#fff"), "{:?}", s.decls);
        assert!(s.decls.iter().any(|d| d.prop == "padding" && d.value == "12px"));
        assert!(s.decls.iter().any(|d| d.prop == "margin" && d.value == "7px" && d.line == 12), "{:?}", s.decls);
        assert!(s.nodes.iter().any(|n| matches!(n, Node::Text { text, expr: true, .. } if text.trim().is_empty())), "the button's label is an expression");
        assert!(s.words().contains("Hi there"));
        assert!(!s.words().contains("useState"));
    }

    #[test]
    fn kebab_case_and_kinds() {
        assert_eq!(kebab("backgroundColor"), "background-color");
        assert_eq!(kebab("--Brand"), "--Brand");
        assert_eq!(Kind::of(Path::new("a.svelte")), Some(Kind::Markup));
        assert_eq!(Kind::of(Path::new("a.rs")), None);
    }
}
