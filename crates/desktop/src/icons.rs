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
        ArrowDown, ArrowLeft, ArrowRight, ArrowUp, AtSign, Camera, Check, ChevronDown, ChevronRight, ChevronUp, Clock, Copy, Eraser, ExternalLink, FolderOpen, Gauge, Laptop, Locate, Mic, Minimize2, Paperclip, Plus, RotateCcw, RotateCw, Search, Server, ShieldCheck, Sparkles, Square, SquareSlash, X
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    /// Send.
    ArrowUp,
    At,
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
    /// The effort picker.
    Effort,
    External,
    File,
    Folder,
    Laptop,
    /// Follow the conversation's browser.
    Locate,
    Mic,
    /// The mode picker.
    Mode,
    /// The model picker.
    Model,
    Paperclip,
    Plus,
    Reload,
    Retry,
    Search,
    Server,
    /// Commands.
    Slash,
    /// Stop.
    Stop,
    X,
}

impl Icon {
    pub const ALL: [Icon; 32] = [
        Icon::ArrowDown,
        Icon::ArrowLeft,
        Icon::ArrowRight,
        Icon::ArrowUp,
        Icon::At,
        Icon::Camera,
        Icon::Check,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::ChevronUp,
        Icon::Clock,
        Icon::Copy,
        Icon::Clear,
        Icon::Compact,
        Icon::Effort,
        Icon::External,
        Icon::File,
        Icon::Folder,
        Icon::Laptop,
        Icon::Locate,
        Icon::Mic,
        Icon::Mode,
        Icon::Model,
        Icon::Paperclip,
        Icon::Plus,
        Icon::Reload,
        Icon::Retry,
        Icon::Search,
        Icon::Server,
        Icon::Slash,
        Icon::Stop,
        Icon::X,
    ];

    pub fn path(self) -> &'static str {
        match self {
            Icon::ArrowDown => "icons/arrow-down.svg",
            Icon::ArrowLeft => "icons/arrow-left.svg",
            Icon::ArrowRight => "icons/arrow-right.svg",
            Icon::ArrowUp => "icons/arrow-up.svg",
            Icon::At => "icons/at-sign.svg",
            Icon::Camera => "icons/camera.svg",
            Icon::Check => "icons/check.svg",
            Icon::ChevronDown => "icons/chevron-down.svg",
            Icon::ChevronRight => "icons/chevron-right.svg",
            Icon::ChevronUp => "icons/chevron-up.svg",
            Icon::Clock => "icons/clock.svg",
            Icon::Copy => "icons/copy.svg",
            Icon::Clear => "icons/eraser.svg",
            Icon::Compact => "icons/minimize-2.svg",
            Icon::Effort => "icons/gauge.svg",
            Icon::External => "icons/external-link.svg",
            Icon::File => "icons/file.svg",
            Icon::Folder => "icons/folder-open.svg",
            Icon::Laptop => "icons/laptop.svg",
            Icon::Locate => "icons/locate.svg",
            Icon::Mic => "icons/mic.svg",
            Icon::Mode => "icons/shield-check.svg",
            Icon::Model => "icons/sparkles.svg",
            Icon::Paperclip => "icons/paperclip.svg",
            Icon::Plus => "icons/plus.svg",
            Icon::Reload => "icons/rotate-cw.svg",
            Icon::Retry => "icons/rotate-ccw.svg",
            Icon::Search => "icons/search.svg",
            Icon::Server => "icons/server.svg",
            Icon::Slash => "icons/square-slash.svg",
            Icon::Stop => "icons/square.svg",
            Icon::X => "icons/x.svg",
        }
    }
}

/// An icon at `size` points in `color`.
pub fn icon(name: Icon, size: f32, color: impl Into<Hsla>) -> Svg {
    svg().path(name.path()).size(px(size)).flex_none().text_color(color)
}

/// The window's assets: the icons above, then GPUI Kit's own.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
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
