//! The window's side of notifications (`crate::notify`): telling what came in
//! while the window was out of sight, the sheet that says why equipe asks to
//! notify before macOS asks, the Dock's number, and `--notify-test`.

use equipe_core::t;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::Shell;
use crate::icons::{Icon, icon};
use crate::notify::{self, Alert, Permission};
use crate::theme::{ICON_SM, R_MD, SMALL};
use crate::ui::{self, Kind};

const WIDTH: f32 = 440.;

/// "창 밖에서도 알려 드릴까요?": why, before macOS asks.
pub fn sheet(shell: &Shell, cx: &mut Context<Shell>) -> Option<AnyElement> {
    if !shell.bell.sheet {
        return None;
    }
    let p = shell.palette;
    let badge = div().mt(px(2.)).size(px(32.)).flex_none().rounded(px(R_MD)).bg(p.wash).border_1().border_color(p.hair).flex().items_center().justify_center().child(icon(Icon::Inbox, 17., p.text));
    let header = h_flex().gap_3().items_start().child(badge).child(
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(div().text_size(px(17.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(t!("notify.sheet_title")))
            .child(div().text_size(px(SMALL)).line_height(px(20.)).text_color(p.muted).child(t!("notify.sheet_body"))),
    );
    let private = h_flex()
        .gap_2()
        .items_start()
        .border_t_1()
        .border_color(p.hair)
        .pt_3()
        .child(div().mt(px(2.)).child(icon(Icon::Lock, ICON_SM, p.muted)))
        .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(t!("notify.sheet_private")));
    let buttons = h_flex()
        .gap_2()
        .justify_end()
        .child(ui::button(p, "notify-later", Kind::Ghost, t!("notify.sheet_later")).on_click(cx.listener(|this, _, window, cx| this.notify_later(window, cx))))
        .child(ui::button(p, "notify-allow", Kind::Primary, t!("notify.sheet_allow")).on_click(cx.listener(|this, _, window, cx| this.allow_notifications(window, cx))));
    Some(crate::views::connect::modal_w(p, "notify-sheet", WIDTH, v_flex().gap_4().child(header).child(private).child(buttons)))
}

impl Shell {
    /// Follow what macOS allows, and whether the window — its squad — is in
    /// front of the person. Once, as the window opens. (Where clicks go is
    /// the app's: `main::follow_notice_clicks`.)
    pub fn start_bell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        notify::set_in_sight(self.seat().route(), window.is_window_active());
        let mut permission = notify::permission();
        self.bell.permission = *permission.borrow_and_update();
        cx.spawn_in(window, async move |this, cx| {
            while permission.changed().await.is_ok() {
                let now = *permission.borrow_and_update();
                if this
                    .update_in(cx, |this, _, cx| {
                        this.bell.permission = now;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        // Back in front: the person may have changed what macOS allows meanwhile.
        cx.observe_window_activation(window, |this, window, _| {
            let active = window.is_window_active();
            notify::set_in_sight(this.seat().route(), active);
            if active {
                notify::refresh();
            }
        })
        .detach();
    }

    /// What the model saw happen: told while the window is out of sight, only
    /// the kinds the person wants — a reply only to what they last wrote. The
    /// first time there is something to tell, the sheet waits for them,
    /// saying why before macOS asks.
    pub fn announce(&mut self, alerts: Vec<Alert>, window: &Window, cx: &mut Context<Self>) {
        // With several windows open nothing comes twice: a squad's notices
        // from the first window on that machine and squad, the app's own
        // (an update) from the app's first window.
        let (first, tells) = (crate::windows::first(self.window_id), crate::windows::tells(self.window_id));
        let pings = self.remembered.pings;
        let alerts: Vec<Alert> = notify::only_awaited(alerts, &self.bell.awaiting).into_iter().filter(|a| pings.wants(a) && if a.is_the_apps() { first } else { tells }).collect();
        // In sight, the cards say it — this window, or another on the same
        // squad, in front — but a browser waiting for the person's hands is
        // told even then, unless that very page is on screen: the person may
        // be reading elsewhere while a member waits on them.
        let route = self.seat().route();
        let browser = self.browser.read(cx);
        let alerts: Vec<Alert> = alerts
            .into_iter()
            .filter(|a| {
                let seen = window.is_window_active() || notify::in_sight(if a.is_the_apps() { None } else { Some(route) });
                !seen || matches!(a, Alert::Browser { page, .. } if !browser.showing(*page))
            })
            .collect();
        if alerts.is_empty() {
            return;
        }
        if notify::explain_now(self.bell.permission, self.remembered.notify_later) && !self.overlay_open() {
            self.bell.sheet = true;
            cx.notify();
        }
        let (apps, squads): (Vec<Alert>, Vec<Alert>) = alerts.into_iter().partition(Alert::is_the_apps);
        for (alerts, route) in [(apps, None), (squads, Some(route))] {
            for toast in notify::toasts(&alerts, &self.model.squad, true) {
                notify::send(&toast, route, pings.sound);
            }
        }
    }

    /// The person wrote to `session`: its answer is the one to tell.
    pub fn await_reply(&self, command: &equipe_core::protocol::Command) {
        use equipe_core::protocol::Command;
        if let Command::Submit { session, .. } | Command::SubmitLater { session, .. } = command {
            self.bell.awaiting.set(Some(*session));
        }
    }

    /// The sheet, when macOS was never asked (a switch turned on, "알림 켜기").
    pub fn offer_notifications(&mut self, cx: &mut Context<Self>) {
        if self.bell.permission == Permission::NotAsked {
            self.bell.sheet = true;
            cx.notify();
        }
    }

    /// "알림 켜기": macOS's own question, now.
    pub fn allow_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.bell.sheet = false;
        notify::ask();
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// "나중에": not asked again on its own; settings still offers it.
    pub fn notify_later(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.bell.sheet = false;
        self.remembered.notify_later = true;
        self.remembered.save(&equipe_providers::credentials::equipe_home());
        self.composer.focus(window, cx);
        cx.notify();
    }

    /// The Dock's number: what waits for the person (나에게), when they want
    /// it — each machine's squad once, however many windows show it.
    pub fn sync_badge(&mut self) {
        if let Some(want) = crate::windows::badge(self.window_id, self.model.needs_you_count(), &self.remembered.pings) {
            notify::set_badge(want);
        }
    }
}

/// `equipe-desktop --notify-test`, from the app bundle: no window, no daemon.
/// Says what macOS allows, asks if it never was, posts two notices in one
/// thread and the update's, puts 3 on the Dock, and waits two minutes for a
/// click (ten seconds when macOS doesn't allow them) — each step on stderr (`open --stderr <file> equipe.app --args --notify-test`).
pub fn notify_test() {
    gpui_kit::application().run(|cx| {
        #[cfg(target_os = "macos")]
        notify::system::VERBOSE.store(true, std::sync::atomic::Ordering::Relaxed);
        notify::start();
        let (clicks, mut clicked) = tokio::sync::mpsc::unbounded_channel::<notify::Click>();
        notify::follow_clicks(clicks);
        let say = |what: String| eprintln!("notify-test: {what}");
        say(format!("bundle id {:?}", notify_bundle()));
        cx.spawn(async move |cx| {
            let wait = |ms| cx.background_executor().timer(std::time::Duration::from_millis(ms));
            let mut permission = notify::permission();
            for _ in 0..50 {
                if *permission.borrow_and_update() != Permission::Unknown {
                    break;
                }
                wait(100).await;
            }
            say(format!("permission {:?}", *permission.borrow()));
            if *permission.borrow() == Permission::NotAsked {
                say("asking macOS".into());
                notify::ask();
                // Three minutes for the person to answer macOS's question.
                for _ in 0..1800 {
                    if !matches!(*permission.borrow_and_update(), Permission::NotAsked | Permission::Unknown) {
                        break;
                    }
                    wait(100).await;
                }
                say(format!("permission now {:?}", *permission.borrow()));
            }
            cx.update(|_| notify::set_badge(Some(3)));
            say("dock badge 3".into());
            // Posted whatever macOS allows: refused, macOS says why.
            let alerts = [
                Alert::Decision { session: 1, member: "pm".into(), count: 2 },
                Alert::Reply { session: 1, member: "pm".into(), line: t!("notify.test_line").into() },
                Alert::Update { version: env!("CARGO_PKG_VERSION").into() },
            ];
            for toast in notify::toasts(&alerts, "equipe", true) {
                say(format!("posting {:?} / {:?} in thread {}", toast.title, toast.body, toast.thread));
                notify::send(&toast, None, true);
            }
            if *permission.borrow() == Permission::Allowed {
                say("waiting 120 s for a click".into());
                let click = futures_timeout(&mut clicked, cx, 120_000).await;
                say(format!("clicked: {click:?}"));
            } else {
                say(format!("not allowed: System Settings › Notifications › equipe ({})", notify::settings_url()));
                wait(10_000).await;
            }
            cx.update(|cx| {
                notify::set_badge(None);
                cx.quit();
            });
        })
        .detach();
    });
}

fn notify_bundle() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        notify::system::bundle_id()
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// The next click, or `None` after `ms`.
async fn futures_timeout(clicked: &mut tokio::sync::mpsc::UnboundedReceiver<notify::Click>, cx: &AsyncApp, ms: u64) -> Option<notify::Click> {
    for _ in 0..ms / 100 {
        if let Ok(go) = clicked.try_recv() {
            return Some(go);
        }
        cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
    }
    None
}
