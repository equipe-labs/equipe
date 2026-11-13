use super::*;

use std::net::TcpListener;

/// T-336's three templates, as `card.mjs` fills them with their examples.
const TEMPLATES: [(&str, &str); 3] =
    [("board", include_str!("examples/board.html")), ("compare", include_str!("examples/compare.html")), ("figure", include_str!("examples/figure.html"))];

/// The Chrome that draws, or `None` (with a word) where there is none: the
/// tests that need one have nothing to try then. Started once first ([`warm`]).
fn chrome_here() -> Option<PathBuf> {
    let found = chrome();
    match &found {
        Some(chrome) => warm(chrome),
        None => eprintln!("no Chrome here: skipped"),
    }
    found
}

/// The longest Chrome's first start in a test run may take.
const WARMING: Duration = Duration::from_secs(120);

/// Chrome started and drawing once, before any test times a drawing. The
/// first Chrome on a machine whose disk has never read it — a CI runner just
/// booted — reads its few hundred megabytes cold, and every Chrome started
/// meanwhile waits on the same reads: on GitHub's Ubuntu runners the first
/// Chromes of a run all went past [`Limits::time`], while those started a few
/// seconds later drew in about one. What the tests time is a drawing, not
/// that. The other tests wait here until it is done.
fn warm(chrome: &Path) {
    static WARM: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    WARM.get_or_init(|| {
        let started = Instant::now();
        let limits = Limits { time: WARMING, ..Limits::default() };
        match render(chrome, &page("<p>warm</p>", Some(false)), false, &limits) {
            Ok(_) => eprintln!("Chrome's first drawing took {:?}", started.elapsed()),
            Err(e) => eprintln!("Chrome's first drawing failed after {:?}: {e}", started.elapsed()),
        }
    });
}

fn png_size(png: &[u8]) -> (u32, u32) {
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG");
    (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap()))
}

fn temp_home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("equipe-cardtest-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cards_are_found_by_their_fence_and_plain_html_is_code() {
    let text = "Here:\n```html card\n<p>a</p>\n```\n```html\n<p>code</p>\n```\n~~~ HTML  Card\n<p>b</p>\n~~~\n```html card\n<p>still writing";
    assert_eq!(blocks(text), vec!["<p>a</p>".to_owned(), "<p>b</p>".to_owned()]);
    assert!(blocks("```html cards\n<p>x</p>\n```").is_empty());
    assert!(has_blocks(text));
    assert!(!has_blocks("```html\n<p>card</p>\n```"));
}

#[test]
fn the_templates_keep_the_contract() {
    for (name, html) in TEMPLATES {
        assert_eq!(check(html), Ok(()), "{name}");
    }
    assert_eq!(check("<p>see <a>equipe.im</a></p><img src=\"data:image/png;base64,AAAA\" alt=\"\"><div style=\"background:url('data:image/png;base64,AA')\"></div><a href=\"#x\">x</a>"), Ok(()));
}

#[test]
fn what_runs_or_loads_or_is_too_big_is_refused() {
    let refused = [
        "<script>fetch('https://x')</script>",
        "<SCRIPT src=x></SCRIPT>",
        "<img src=\"https://example.com/a.png\">",
        "<img src=//example.com/a.png>",
        "<img srcset=\"a.png 2x\">",
        "<img src=x onerror=\"alert(1)\">",
        "<div onclick=x>a</div>",
        "<svg onload=x></svg>",
        "<iframe src=\"data:text/html,x\"></iframe>",
        "<object data=x></object>",
        "<embed src=x>",
        "<form action=x><button>go</button></form>",
        "<link rel=stylesheet href=https://x/a.css>",
        "<meta http-equiv=refresh content=\"0;url=https://x\">",
        "<base href=https://x/>",
        "<html><body>x</body></html>",
        "<style>@import 'https://x/a.css';</style>",
        "<style>p{background:url(https://x/a.png)}</style>",
        "<p style=\"background: url( 'http://x/a.png' )\">x</p>",
        "<p style=\"background-image:image-set('https://x/a.png' 1x)\">x</p>",
        "<svg><image href=\"https://x/a.png\"/></svg>",
        "<svg><use xlink:href=\"https://x/s.svg#a\"/></svg>",
        "<video poster=https://x/p.png></video>",
        "<a href=x ping=https://x>a</a>",
    ];
    for html in refused {
        assert!(check(html).is_err(), "let through: {html}");
    }
    let big = format!("<p>{}</p>", "\u{ac00}".repeat(SOURCE_MAX / 3 + 1));
    assert!(check(&big).is_err());
}

/// Links a click or a key would follow out of the page (T-357), every way
/// HTML lets them be written.
const LINKS_OUT: [&str; 34] = [
    "<a href=\"https://equipe.im\">equipe.im</a>",
    "<a href=\"http://127.0.0.1:9/\">x</a>",
    "<a href=\"//evil.test/\">x</a>",
    "<a href=\"file:///etc/passwd\">x</a>",
    "<a href=\"FILE:///Users/\">x</a>",
    "<a href=\"javascript:alert(1)\">x</a>",
    "<a href=\"data:text/html,<p>x\">x</a>",
    "<a href=\"data:image/svg+xml,<svg/>\">x</a>",
    "<a href=\"other.html\">x</a>",
    "<a href=\"\">x</a>",
    "<a href>x</a>",
    "<A HREF=\"https://x\">x</A>",
    "<a href=https://x>x</a>",
    "<a href = 'https://x'>x</a>",
    "<a\nhref=\"https://x\">x</a>",
    "<a\x0chref=https://x>x</a>",
    "<a/href=https://x>x</a>",
    "<a title=\"#\" href=\"https://x\">x</a>",
    "<a href=\"#\" href=\"https://x\">x</a>",
    "<a href=\"\tjava\nscript:alert(1)\">x</a>",
    "<a href=\" \u{3000}#x\">x</a>",
    "<a href=\"&#35;x\">x</a>",
    "<a href=\"&#104;ttps://x\">x</a>",
    "<a href=\"https:&#47;&#47;x\">x</a>",
    "<svg><a href=\"https://x\"><text>x</text></a></svg>",
    "<svg><a xlink:href=\"https://x\"><text>x</text></a></svg>",
    "<map name=m><area href=\"https://x\" shape=default></map>",
    "<a href=\"#x\" target=\"_blank\">x</a>",
    "<a href=\"#x\" download>x</a>",
    // Where a tag sits that a browser reads but an eye might not: after a
    // comment `--!>` closes, past a `<style>` an attribute's quote seemed to hold.
    "<!-- <p title=\" --!> <a href=https://x>x</a> \" -->",
    "<!-- <p title=\" --> <a href=https://x>x</a> \" -->",
    "<style><p title=\"</style><a href=https://x>x</a>\"</style>",
    "<textarea><p title=\"</textarea><a href=https://x>x</a>\"",
    "<p title=\"<a href=https://x>\">x</p><math href=\"https://x\">x</math>",
];

#[test]
fn a_link_may_only_go_to_a_place_in_the_card() {
    for html in LINKS_OUT {
        assert!(check(html).is_err(), "let through: {html:?}");
    }
    for html in ["<a href=\"#x\">x</a>", "<a href='#'>top</a>", "<a href=#x>x</a>", "<a href=\" #x \">x</a>", "<a>plain</a>", "<svg><a href=\"#x\"><text>x</text></a></svg>"] {
        assert_eq!(check(html), Ok(()), "{html}");
    }
}

#[test]
fn reading_every_tag_start_stays_quick() {
    // Quotes that never close: each `<` is read to the end, no more.
    let worst = "<a title=\"".repeat(SOURCE_MAX / 10);
    let started = Instant::now();
    let _ = check(&worst);
    // Linear, not quadratic: generous enough for an unoptimised build on a
    // busy CI runner, far below what a quadratic scan would take.
    let bound = if cfg!(debug_assertions) { Duration::from_secs(30) } else { Duration::from_secs(5) };
    assert!(started.elapsed() < bound, "{:?}", started.elapsed());
}

#[test]
fn a_page_kept_under_the_old_contract_is_not_given_out() {
    let home = temp_home("oldpage");
    let old = "<p>see <a href=\"https://evil.test/\">this</a></p>";
    let key = diagram::key(old);
    let page_path = page_file(&home, "acme", &key);
    keep(&page_path, page(old, None).as_bytes()).unwrap();
    for dark in [false, true] {
        keep(&diagram::file(&home, "acme", &key, dark), &tiny_png()).unwrap();
    }
    let text = format!("```html card\n{old}\n```");
    assert!(kept(&home, "acme", &[Entry::Reply { text }]).is_empty());
    let ask = crate::protocol::rpc::ReadImage { squad: "acme".into(), path: page_path.to_string_lossy().into_owned(), max: None };
    let err = crate::visuals::serve_with(&home, &ask, None, &[]).unwrap_err();
    assert!(err.contains("contract"), "{err}");
    assert_eq!(kept_fragment(&page(TEMPLATES[0].1, None)), Ok(TEMPLATES[0].1));
    let _ = std::fs::remove_dir_all(&home);
}

fn tiny_png() -> Vec<u8> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(2, 2).unwrap();
    pixmap.fill(resvg::tiny_skia::Color::BLACK);
    pixmap.encode_png().unwrap()
}

#[test]
fn the_page_holds_the_card_in_the_design_system_and_its_csp_first() {
    let page = page("<p>hi</p>", Some(true));
    let csp = page.find("Content-Security-Policy").unwrap();
    assert!(csp < page.find("<style>").unwrap() && csp < page.find("<p>hi</p>").unwrap());
    assert!(page.contains("default-src 'none'"));
    assert!(page.contains("data-theme=\"dark\""));
    assert!(page.contains("--eq-color-text") && page.contains(".vc-card"));
    assert!(page.contains("<article class=\"vc-card\"><p>hi</p></article>"));
    // The page says the language the card is written in, so its type is that
    // script's: English when it can't tell, never one built in.
    assert!(super::page("<p>hi</p>", None).contains("<html lang=\"en\"><head>"));
    for (card, lang) in [("<h2 class=\"vc-title\">이번 주 스쿼드 숫자</h2>", "ko"), ("<p>今週の数字を<b>確認</b>する</p>", "ja"), ("<p>本周的数字</p>", "zh"), ("<p>Les chiffres de la semaine, revus avec toute l'équipe</p>", "fr")] {
        assert!(super::page(card, Some(false)).contains(&format!("<html lang=\"{lang}\" data-theme=\"light\">")), "{card}: {lang}");
    }
}

#[test]
fn the_three_templates_draw_as_pngs_light_and_dark() {
    let Some(chrome) = chrome_here() else { return };
    // Hosted CI runners have a Chrome but no GPU and little headroom: drawing
    // there times out for reasons unrelated to the templates.
    if std::env::var_os("CI").is_some() && std::env::var_os("EQUIPE_CARD_SHOTS").is_none() {
        return;
    }
    let out = std::env::var_os("EQUIPE_CARD_SHOTS").map(PathBuf::from);
    for (name, html) in TEMPLATES {
        let mut drawn = Vec::new();
        for dark in [false, true] {
            let (png, width, height) = render(&chrome, &page(html, Some(dark)), dark, &Limits::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(png_size(&png), (width, height));
            assert_eq!(width, WIDTH * SCALE, "{name}");
            assert!(height > 200 && height < HEIGHT * SCALE, "{name}: {height} high");
            if let Some(out) = &out {
                std::fs::write(out.join(format!("{name}-{}.png", if dark { "dark" } else { "light" })), &png).unwrap();
            }
            drawn.push(png);
        }
        assert_ne!(drawn[0], drawn[1], "{name}: light and dark alike");
    }
}

#[test]
fn a_card_is_drawn_kept_with_its_page_and_read_back_only_when_on() {
    let Some(_) = chrome_here() else { return };
    let home = temp_home("draw");
    let text = format!("Where things stand:\n```html card\n{}\n```\n", TEMPLATES[2].1.trim());
    // Off until the security review says otherwise.
    assert!(draw(&home, "acme", &text).is_empty());
    std::fs::write(home.join("visuals.json"), r#"{"html_cards": true}"#).unwrap();
    let drawn = draw(&home, "acme", &text);
    assert_eq!(drawn.len(), 1, "{drawn:?}");
    let card = &drawn[0];
    assert_eq!(card.kind, "card");
    assert_eq!(card.key, diagram::key(TEMPLATES[2].1));
    for image in [&card.light, &card.dark] {
        assert!(diagram::is_drawn(&home, "acme", Path::new(&image.path)));
        assert_eq!(png_size(&std::fs::read(&image.path).unwrap()), (image.width, image.height));
    }
    let page_path = card.page.as_deref().unwrap();
    assert!(is_page(&home, "acme", Path::new(page_path)));
    assert!(!is_page(&home, "other", Path::new(page_path)));
    assert!(std::fs::read_to_string(page_path).unwrap().contains("Content-Security-Policy"));
    assert_eq!(kept(&home, "acme", &[Entry::Reply { text: text.clone() }]), drawn);
    // A screen reads the page by its path, as it reads the pictures.
    let ask = |squad: &str, path: &str| crate::protocol::rpc::ReadImage { squad: squad.into(), path: path.into(), max: None };
    let read = crate::visuals::serve_with(&home, &ask("acme", page_path), None, &[]).expect("the page");
    assert_eq!(read.mime, "text/html");
    assert!(crate::visuals::serve_with(&home, &ask("other", page_path), None, &[]).is_err());
    let png = crate::visuals::serve_with(&home, &ask("acme", &card.dark.path), Some(720), &[]).expect("the dark PNG");
    assert_eq!(png.mime, "image/png");
    // A card against the contract isn't drawn: its source shows.
    assert!(draw(&home, "acme", "```html card\n<img src=\"https://x/a.png\">\n```").is_empty());
    let _ = std::fs::remove_dir_all(&home);
}

/// Connections made to `listener` so far, waiting a moment for late ones.
fn knocks(listener: &TcpListener) -> usize {
    listener.set_nonblocking(true).unwrap();
    let mut n = 0;
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        match listener.accept() {
            Ok(_) => n += 1,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    n
}

#[test]
fn a_card_reaching_out_makes_no_request_at_all() {
    let Some(chrome) = chrome_here() else { return };
    // Hosted CI runners draw without a GPU and time out on Chrome's side;
    // the guard itself is also covered by `check` above, which needs no Chrome.
    if std::env::var_os("CI").is_some() && std::env::var_os("EQUIPE_CARD_SHOTS").is_none() {
        return;
    }
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let at = |what: &str| format!("http://127.0.0.1:{port}/{what}");
    // Past the contract's check on purpose: what holds here is the page's
    // CSP and Chrome's network, not the check.
    let card = format!(
        "<p>out</p><img src=\"{img}\"><img src=\"http://localhost:{port}/b.png\"><link rel=\"stylesheet\" href=\"{css}\">\
<style>@import url(\"{imp}\"); p {{ background: url(\"{bg}\") }} @font-face {{ font-family: x; src: url(\"{font}\") }} b {{ font-family: x }}</style>\
<b>font</b><script>fetch(\"{fetch}\"); new Image().src = \"{js}\";</script><iframe src=\"{frame}\"></iframe>\
<video poster=\"{poster}\"></video><object data=\"{object}\"></object>",
        img = at("img.png"),
        css = at("a.css"),
        imp = at("import.css"),
        bg = at("bg.png"),
        font = at("font.woff"),
        fetch = at("fetch"),
        js = at("js.png"),
        frame = at("frame"),
        poster = at("poster.png"),
        object = at("object"),
    );
    // A page that goes elsewhere by itself: what it shows then may not draw.
    let leaving = format!("<meta http-equiv=\"refresh\" content=\"0;url={}\"><p>away</p>", at("refresh"));
    assert!(check(&card).is_err() && check(&leaving).is_err(), "the check refuses them too");
    let limits = Limits { proxy: proxy.local_addr().unwrap().to_string(), ..Limits::default() };
    for dark in [false, true] {
        render(&chrome, &page(&card, Some(dark)), dark, &limits).expect("drawn, without what it wanted");
        let _ = render(&chrome, &page(&leaving, Some(dark)), dark, &limits);
    }
    // Chrome's network holds alone, too: a page without the CSP, its own
    // script running, still reaches nothing.
    let bare = format!("<!doctype html><p>bare</p><img src=\"{}\"><img src=\"http://example.com/c.png\"><script>fetch(\"{}\")</script>", at("bare.png"), at("bare"));
    let _ = render(&chrome, &bare, false, &limits);
    assert_eq!(knocks(&server), 0, "the card's server was reached");
    assert_eq!(knocks(&proxy), 0, "something went out through Chrome's proxy");
}

#[test]
fn a_script_that_never_ends_doesnt_run_and_a_drawing_past_its_time_is_stopped() {
    let Some(chrome) = chrome_here() else { return };
    let looping = "<p>loop</p><script>for(;;){}</script><style>p{color:var(--fg)}</style>";
    assert!(check(looping).is_err());
    let started = Instant::now();
    let (_, _, height) = render(&chrome, &page(looping, Some(false)), false, &Limits::default()).expect("drawn: the script never ran");
    assert!(height > 0 && started.elapsed() < Limits::default().time);
    // Past its time, Chrome is killed and nothing is drawn.
    let started = Instant::now();
    let short = Limits { time: Duration::from_millis(50), ..Limits::default() };
    let err = render(&chrome, &page(TEMPLATES[0].1, Some(false)), false, &short).unwrap_err();
    assert!(err.contains("not drawn in"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
}

#[test]
fn a_huge_card_is_refused_and_a_tall_one_is_cut_at_the_window() {
    let Some(chrome) = chrome_here() else { return };
    let huge = format!("<p>{}</p>", "x".repeat(SOURCE_MAX + 1));
    assert!(check(&huge).is_err());
    let home = temp_home("huge");
    std::fs::write(home.join("visuals.json"), r#"{"html_cards": true}"#).unwrap();
    assert!(draw(&home, "acme", &format!("```html card\n{huge}\n```")).is_empty());
    let _ = std::fs::remove_dir_all(&home);
    let tall = "<div style=\"height: 100000px; background: var(--accent)\">tall</div>";
    assert_eq!(check(tall), Ok(()));
    let started = Instant::now();
    let (png, width, height) = render(&chrome, &page(tall, Some(true)), true, &Limits::default()).expect("drawn");
    assert_eq!((width, height), (WIDTH * SCALE, HEIGHT * SCALE));
    assert_eq!(png_size(&png), (width, height));
    assert!(started.elapsed() < Limits::default().time);
}

/// A stand-in for Chrome: writes `picture` where it is told after `wait`
/// seconds, and notes in `<dir>/log` how many of it were running then
/// (itself included), counting only those still alive.
fn fake_chrome(dir: &Path, wait: f32) -> PathBuf {
    use resvg::tiny_skia::{Color, Pixmap};
    let run = dir.join("run");
    std::fs::create_dir_all(&run).unwrap();
    let mut pixmap = Pixmap::new(8, 8).unwrap();
    pixmap.fill(Color::from_rgba8(10, 20, 30, 255));
    std::fs::write(dir.join("picture.png"), pixmap.encode_png().unwrap()).unwrap();
    let script = dir.join("chrome");
    let body = format!(
        "#!/bin/sh\nfor a; do case \"$a\" in --screenshot=*) out=\"${{a#--screenshot=}}\";; esac; done\n\
touch '{run}/'$$\nn=0; for f in '{run}'/*; do kill -0 \"${{f##*/}}\" 2>/dev/null && n=$((n+1)); done\necho $n >> '{dir}/log'\n\
sleep {wait}\nrm -f '{run}/'$$\ncp '{dir}/picture.png' \"$out\"\n",
        run = run.display(),
        dir = dir.display(),
    );
    std::fs::write(&script, body).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn cards(prefix: &str, n: usize) -> String {
    (0..n).map(|i| format!("```html card\n<p>{prefix} {i}</p>\n```\n")).collect()
}

#[test]
fn a_reply_draws_its_first_cards_in_its_time_and_the_rest_show_their_source() {
    let home = temp_home("budget");
    let chrome = fake_chrome(&home, 0.05);
    let text = cards("budget", PER_REPLY + 2);
    let drawn = draw_within(&home, "acme", &text, &chrome, Budget::reply());
    assert_eq!(drawn.len(), PER_REPLY, "past {PER_REPLY}, the source shows");
    // Drawn before: read back, not counted, even with nothing left to spend.
    let spent = Budget { cards: 0, ..Budget::reply() };
    assert_eq!(draw_within(&home, "acme", &text, &chrome, spent.clone()), drawn);
    // Out of time: nothing new is started.
    let late = Budget { until: Instant::now(), ..Budget::reply() };
    assert!(draw_within(&home, "acme", &cards("late", 2), &chrome, late).is_empty());
    let _ = std::fs::remove_dir_all(&home);
}

/// Until a Chrome has drawn once since the daemon started, a drawing gets
/// Chrome's cold first start's time and a reply room for it; after, the
/// usual ten seconds and thirty.
#[test]
fn chromes_first_start_gets_its_allowance_and_every_drawing_after_the_usual_time() {
    let usual = Limits::default().time;
    assert_eq!(usual, Duration::from_secs(10));
    assert_eq!(allowance(false, usual), FIRST_START, "the first start, cold");
    assert_eq!(allowance(true, usual), usual, "warm: ten seconds, as ever");
    assert_eq!(allowance(true, Duration::from_millis(50)), Duration::from_millis(50), "a shorter limit holds once warm");
    assert_eq!(allowance(false, Duration::from_secs(300)), Duration::from_secs(300), "never less than asked");
    assert_eq!(reply_time(true), REPLY_TIME);
    assert_eq!(reply_time(false), REPLY_TIME + FIRST_START, "the first start doesn't eat the reply's other cards");
    assert!(allowance(false, usual) < reply_time(false));
}

/// A drawing that ends well leaves Chrome warm; with cards off there is
/// nothing to warm, and no Chrome is started.
#[test]
fn a_drawing_warms_chrome_and_cards_off_start_none() {
    let home = temp_home("warm");
    assert!(warm_up(&home).is_none(), "cards are off");
    let chrome = fake_chrome(&home, 0.0);
    render(&chrome, &page("<p>warm</p>", Some(false)), false, &Limits::default()).expect("drawn");
    assert!(is_warm());
    std::fs::write(home.join("visuals.json"), r#"{"html_cards": true}"#).unwrap();
    assert!(warm_up(&home).is_none(), "warm already: nothing to do");
    let _ = std::fs::remove_dir_all(&home);
}

/// Many sessions' replies, each full of cards never drawn before, ending at
/// once: never more than [`CHROMES`] Chromes, no reply past its time, and the
/// squad's `visuals/` within its quota, what is shown kept.
#[test]
fn many_replies_of_new_cards_at_once_stay_within_chromes_time_and_cache() {
    use crate::visuals::cache;
    use std::collections::HashSet;
    let home = temp_home("stress");
    let chrome = fake_chrome(&home, 0.15);
    let (sessions, replies, time) = (6, 3, Duration::from_millis(1500));
    let quota = cache::Quota { keys: 10, bytes: u64::MAX };
    // What each session shows, as the daemon prunes with what all of the
    // squad's open conversations show — not one session's alone, or another
    // session's prune takes away what this one shows ("<key> went"). A
    // reply's keys are known before its cards are drawn; read and pruned with
    // under one lock, so none of a reply shown meanwhile is drawn mid-prune.
    let showing = std::sync::Arc::new(Mutex::new(std::collections::HashMap::<usize, HashSet<String>>::new()));
    let started = Instant::now();
    let threads: Vec<_> = (0..sessions)
        .map(|s| {
            let (home, chrome, showing) = (home.clone(), chrome.clone(), showing.clone());
            std::thread::spawn(move || {
                let mut last = HashSet::new();
                for r in 0..replies {
                    let text = cards(&format!("s{s} r{r}"), 6);
                    showing.lock().unwrap().insert(s, blocks(&text).iter().map(|b| diagram::key(b)).collect());
                    let at = Instant::now();
                    let budget = Budget { until: at + time, ..Budget::reply() };
                    let drawn = draw_within(&home, "acme", &text, &chrome, budget);
                    let took = at.elapsed();
                    assert!(drawn.len() <= PER_REPLY, "{} drawn", drawn.len());
                    assert!(took < time + Duration::from_millis(700), "a reply took {took:?}");
                    last = drawn.iter().map(|d| d.key.clone()).collect();
                    let showing = showing.lock().unwrap();
                    let all: HashSet<String> = showing.values().flatten().cloned().collect();
                    let held = cache::prune(&diagram::dir(&home, "acme"), &all, quota);
                    assert!(held.keys <= quota.keys + all.len(), "{held:?}");
                }
                last
            })
        })
        .collect();
    let shown: HashSet<String> = threads.into_iter().flat_map(|t| t.join().unwrap()).collect();
    assert!(started.elapsed() < (time + Duration::from_millis(700)) * replies, "{:?}", started.elapsed());
    assert!(most_at_once() <= CHROMES, "{} at once", most_at_once());
    let seen: Vec<usize> = std::fs::read_to_string(home.join("log")).unwrap().lines().map(|l| l.trim().parse().unwrap()).collect();
    assert!(!seen.is_empty(), "nothing drawn at all");
    assert!(seen.iter().all(|&n| n <= CHROMES), "Chromes at once: {seen:?}");
    // What the last replies show stays; the rest within the quota.
    let dir = diagram::dir(&home, "acme");
    let held = cache::prune(&dir, &shown, quota);
    assert!(held.keys <= quota.keys.max(shown.len()), "{held:?}");
    for key in &shown {
        assert!(dir.join(format!("{key}-dark.png")).is_file() && dir.join(format!("{key}.html")).is_file(), "{key} went");
    }
    let held = cache::prune(&dir, &HashSet::new(), quota);
    assert!(held.keys <= quota.keys, "{held:?}");
    let on_disk = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".png")).count();
    // A key whose dark tone ran out of time keeps its light one, read back next time.
    assert!(on_disk <= held.keys * 2, "{on_disk} PNGs for {held:?}");
    let _ = std::fs::remove_dir_all(&home);
}

mod panel;
