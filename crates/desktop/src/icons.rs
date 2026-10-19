//! The app's icons: Lucide's, as GPUI Kit bundles them. Only the ones named
//! here are embedded (plus the components' own set, which GPUI Kit's widgets —
//! the spinner, the jump button, a text box's clear button — draw with).
//! Every icon is drawn at one of two sizes (`theme::ICON`, `theme::ICON_SM`)
//! in the colour of the text around it.

use std::borrow::Cow;

use gpui_kit::*;

gpui_kit::assets::icon_assets!(
    Chosen,
    [
        ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Bot, Brain, Camera, Check, ChevronDown, ChevronRight, ChevronUp, CircleAlert, CornerDownRight, CircleCheck, CircleX, Clock, Copy, Eraser, ExternalLink, FileText, Folder, Globe, Hand, Inbox, Info, Keyboard, Laptop, Lightbulb, Locate, Lock, Maximize2, MessageSquare, Mic, Minimize2, Paperclip, Pencil, Plug, Plus, RotateCcw, RotateCw, Search, Server, Settings, ShieldCheck, Smartphone, Sparkles, SquareSlash, SquareTerminal, Ticket, TriangleAlert, Undo2, Users, X
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// To the latest.
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    /// Send.
    ArrowUp,
    Camera,
    Check,
    ChevronDown,
    ChevronRight,
    ChevronUp,
    /// Recent files.
    Clock,
    Copy,
    /// `/clear`.
    Clear,
    /// `/compact`.
    Compact,
    External,
    File,
    Folder,
    Globe,
    Info,
    Keyboard,
    Laptop,
    /// Follow the conversation's browser.
    Locate,
    Mic,
    Paperclip,
    Plus,
    Reload,
    Retry,
    Search,
    Server,
    Settings,
    /// Permissions.
    Shield,
    /// Commands.
    Slash,
    /// The team.
    Users,
    X,
    /// Something went wrong.
    Alert,
    /// A subagent.
    Bot,
    /// Reasoning.
    Brain,
    /// Open larger.
    Expand,
    FileText,
    /// Handed in.
    Inbox,
    /// Proposed.
    Lightbulb,
    Message,
    Pencil,
    Terminal,
    Ticket,
    /// Cancelled.
    XCircle,
    /// A phone (the equipe app), paired or to pair.
    Phone,
    /// Done: a phone paired.
    Done,
    /// Something went wrong, and what to do.
    Warning,
    /// A secure address.
    Lock,
    /// The person's hand on a member's browser.
    Hand,
    /// Take back what waits, into the box.
    Withdraw,
    /// Send what waits now.
    SendNow,
    /// Settings' AI platforms.
    Plug,
    /// Codex's mark.
    Sparkles,
    /// Claude's own mark, for Claude Code (Simple Icons, CC0; the mark is
    /// Anthropic's, shown to name the platform).
    ClaudeMark,
    /// OpenAI's own mark, for Codex (Simple Icons, CC0; the mark is OpenAI's,
    /// shown to name the platform).
    OpenAiMark,
}

impl Icon {
    #[cfg(test)]
    pub const ALL: [Icon; 56] = [
        Icon::ArrowDown,
        Icon::ArrowLeft,
        Icon::ArrowRight,
        Icon::ArrowUp,
        Icon::Camera,
        Icon::Check,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::ChevronUp,
        Icon::Clock,
        Icon::Copy,
        Icon::Clear,
        Icon::Compact,
        Icon::External,
        Icon::File,
        Icon::Folder,
        Icon::Globe,
        Icon::Info,
        Icon::Keyboard,
        Icon::Laptop,
        Icon::Locate,
        Icon::Mic,
        Icon::Paperclip,
        Icon::Plus,
        Icon::Reload,
        Icon::Retry,
        Icon::Search,
        Icon::Server,
        Icon::Settings,
        Icon::Shield,
        Icon::Slash,
        Icon::Users,
        Icon::Alert,
        Icon::Bot,
        Icon::Brain,
        Icon::Expand,
        Icon::FileText,
        Icon::Inbox,
        Icon::Lightbulb,
        Icon::Message,
        Icon::Pencil,
        Icon::Terminal,
        Icon::Ticket,
        Icon::XCircle,
        Icon::X,
        Icon::Phone,
        Icon::Done,
        Icon::Warning,
        Icon::Lock,
        Icon::Hand,
        Icon::Withdraw,
        Icon::SendNow,
        Icon::Plug,
        Icon::Sparkles,
        Icon::ClaudeMark,
        Icon::OpenAiMark,
    ];

    pub fn path(self) -> &'static str {
        match self {
            Icon::ArrowDown => "icons/arrow-down.svg",
            Icon::ArrowLeft => "icons/arrow-left.svg",
            Icon::ArrowRight => "icons/arrow-right.svg",
            Icon::ArrowUp => "icons/arrow-up.svg",
            Icon::Camera => "icons/camera.svg",
            Icon::Check => "icons/check.svg",
            Icon::ChevronDown => "icons/chevron-down.svg",
            Icon::ChevronRight => "icons/chevron-right.svg",
            Icon::ChevronUp => "icons/chevron-up.svg",
            Icon::Clock => "icons/clock.svg",
            Icon::Copy => "icons/copy.svg",
            Icon::Clear => "icons/eraser.svg",
            Icon::Compact => "icons/minimize-2.svg",
            Icon::External => "icons/external-link.svg",
            Icon::File => "icons/file.svg",
            Icon::Folder => "icons/folder.svg",
            Icon::Globe => "icons/globe.svg",
            Icon::Info => "icons/info.svg",
            Icon::Keyboard => "icons/keyboard.svg",
            Icon::Laptop => "icons/laptop.svg",
            Icon::Locate => "icons/locate.svg",
            Icon::Mic => "icons/mic.svg",
            Icon::Paperclip => "icons/paperclip.svg",
            Icon::Plus => "icons/plus.svg",
            Icon::Reload => "icons/rotate-cw.svg",
            Icon::Retry => "icons/rotate-ccw.svg",
            Icon::Search => "icons/search.svg",
            Icon::Server => "icons/server.svg",
            Icon::Settings => "icons/settings.svg",
            Icon::Shield => "icons/shield-check.svg",
            Icon::Users => "icons/users.svg",
            Icon::Slash => "icons/square-slash.svg",
            Icon::Alert => "icons/circle-alert.svg",
            Icon::Bot => "icons/bot.svg",
            Icon::Brain => "icons/brain.svg",
            Icon::Expand => "icons/maximize-2.svg",
            Icon::FileText => "icons/file-text.svg",
            Icon::Inbox => "icons/inbox.svg",
            Icon::Lightbulb => "icons/lightbulb.svg",
            Icon::Message => "icons/message-square.svg",
            Icon::Pencil => "icons/pencil.svg",
            Icon::Terminal => "icons/square-terminal.svg",
            Icon::Ticket => "icons/ticket.svg",
            Icon::XCircle => "icons/circle-x.svg",
            Icon::X => "icons/x.svg",
            Icon::Phone => "icons/smartphone.svg",
            Icon::Done => "icons/circle-check.svg",
            Icon::Warning => "icons/triangle-alert.svg",
            Icon::Lock => "icons/lock.svg",
            Icon::Hand => "icons/hand.svg",
            Icon::ClaudeMark => BRAND_CLAUDE,
            Icon::OpenAiMark => BRAND_OPENAI,
            Icon::Withdraw => "icons/undo-2.svg",
            Icon::SendNow => "icons/corner-down-right.svg",
            Icon::Plug => "icons/plug.svg",
            Icon::Sparkles => "icons/sparkles.svg",
        }
    }
}

/// An icon at `size` points in `color`.
pub fn icon(name: Icon, size: f32, color: impl Into<Hsla>) -> Svg {
    svg().path(name.path()).size(px(size)).flex_none().text_color(color)
}

const BRAND_CLAUDE: &str = "brand/claude.svg";
const BRAND_OPENAI: &str = "brand/openai.svg";

/// The window's assets: the icons above, then GPUI Kit's own.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            BRAND_CLAUDE => return Ok(Some(Cow::Borrowed(include_bytes!("../assets/brand/claude.svg")))),
            BRAND_OPENAI => return Ok(Some(Cow::Borrowed(include_bytes!("../assets/brand/openai.svg")))),
            _ => {}
        }
        if let Some(bytes) = Chosen.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(Chosen.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppAssets, Icon};
    use gpui_kit::AssetSource as _;

    #[test]
    fn every_icon_the_app_draws_is_embedded() {
        for icon in Icon::ALL {
            let bytes = AppAssets.load(icon.path()).expect("loads").unwrap_or_else(|| panic!("{icon:?} is not embedded"));
            assert!(bytes.starts_with(b"<svg"), "{icon:?}");
        }
    }

    #[test]
    fn the_components_icons_are_there_too() {
        // The jump button's arrow and the spinner draw with these.
        for path in ["icons/arrow-down.svg", "icons/loader.svg", "icons/close.svg"] {
            assert!(AppAssets.load(path).expect("loads").is_some(), "{path}");
        }
    }
}
