//! Windows desktop backend: Windows Graphics Capture, UI Automation, and
//! focus-safe `SendInput` events. The common controller still owns aiming,
//! scene revalidation, the send gate, and per-member coordinates.

use std::mem::size_of;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};
use serde_json::json;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame as CaptureFrame;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window as CaptureWindow;

use crate::controller::{
    App, Controller, Desktop, Display, Hit, Node, Permissions, Pointer, Window,
};
use crate::geometry::{Frame, Pt};
use crate::keys::{self, Combo};

/// A UIA element is COM reference counted and remains valid between an
/// `elements` response and the member's next action.
#[derive(Clone)]
pub struct Element(IUIAutomationElement);

pub struct WindowsDesktop {
    automation: IUIAutomation,
}

impl WindowsDesktop {
    fn new() -> Result<Self, windows::core::Error> {
        // The helper serves every request serially on this thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let automation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            Ok(Self { automation })
        }
    }

    fn element_for(&self, pid: i32, window: Option<&Window>) -> Option<IUIAutomationElement> {
        let hwnd = window.map(|w| hwnd_from_id(w.id)).or_else(|| {
            self.windows()
                .into_iter()
                .find(|w| w.pid == pid)
                .map(|w| hwnd_from_id(w.id))
        })?;
        unsafe { self.automation.ElementFromHandle(hwnd).ok() }
    }

    fn node_of(element: &IUIAutomationElement, depth: usize) -> Node {
        let control = unsafe { element.CurrentControlType().ok() };
        let password = unsafe { element.CurrentIsPassword().is_ok_and(|v| v.as_bool()) };
        let role = role(control, password).to_owned();
        let title = unsafe {
            element
                .CurrentName()
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        let frame = unsafe { element.CurrentBoundingRectangle().ok() }.and_then(frame_from_rect);
        let focused = unsafe { element.CurrentHasKeyboardFocus().is_ok_and(|v| v.as_bool()) };
        let value = value_of(element);
        let actions = unsafe {
            element
                .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                .ok()
                .map(|_| vec!["AXPress".to_owned()])
                .unwrap_or_default()
        };
        Node {
            depth,
            role,
            title,
            value,
            frame,
            actions,
            focused,
        }
    }

    fn walk(
        walker: &IUIAutomationTreeWalker,
        element: IUIAutomationElement,
        depth: usize,
        max: usize,
        out: &mut Vec<(Element, Node)>,
    ) {
        if out.len() >= max {
            return;
        }
        out.push((Element(element.clone()), Self::node_of(&element, depth)));
        if out.len() >= max {
            return;
        }
        let mut child = unsafe { walker.GetFirstChildElement(&element).ok() };
        while let Some(current) = child {
            Self::walk(walker, current.clone(), depth + 1, max, out);
            if out.len() >= max {
                return;
            }
            child = unsafe { walker.GetNextSiblingElement(&current).ok() };
        }
    }
}

impl Desktop for WindowsDesktop {
    type Element = Element;

    fn permissions(&self) -> Permissions {
        // Unlike macOS, Windows has no per-app Screen Recording or
        // Accessibility grant. UIPI failures are reported by the operation.
        Permissions {
            screen: true,
            accessibility: true,
            post_events: true,
        }
    }

    fn display(&self) -> Option<Display> {
        let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        (w > 0 && h > 0).then_some(Display {
            frame: Frame {
                x: 0.0,
                y: 0.0,
                w: f64::from(w),
                h: f64::from(h),
            },
            // This process is per-monitor DPI aware, so UIA and capture both use
            // physical pixels and no second scaling is needed.
            backing: 1.0,
        })
    }

    fn capture(&self, window: Option<u32>, width: usize, height: usize) -> Result<Vec<u8>, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        match window {
            Some(id) => {
                let item = CaptureWindow::enumerate()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|w| id_from_hwnd(HWND(w.as_raw_hwnd())) == id)
                    .ok_or_else(|| format!("window w{id} is no longer on screen"))?;
                Screenshot::start(capture_settings(item, tx, width, height))
                    .map_err(|e| e.to_string())?;
            }
            None => {
                let item = Monitor::primary().map_err(|e| e.to_string())?;
                Screenshot::start(capture_settings(item, tx, width, height))
                    .map_err(|e| e.to_string())?;
            }
        }
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| "Windows Graphics Capture produced no frame".to_owned())?
    }

    fn apps(&self) -> Vec<App> {
        let mut apps = Vec::new();
        for window in CaptureWindow::enumerate().unwrap_or_default() {
            let Ok(pid) = window.process_id() else {
                continue;
            };
            if apps.iter().any(|a: &App| a.pid == pid as i32) {
                continue;
            }
            let name = window
                .process_name()
                .unwrap_or_else(|_| window.title().unwrap_or_else(|_| format!("pid {pid}")))
                .trim_end_matches(".exe")
                .to_owned();
            apps.push(App {
                pid: pid as i32,
                name,
                bundle_id: None,
            });
        }
        apps
    }

    fn open(&self, app: &str) -> Result<(), String> {
        let app = HSTRING::from(app);
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &app,
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        ((result.0 as isize) > 32).then_some(()).ok_or_else(|| {
            format!(
                "Windows couldn't open that app (ShellExecute error {})",
                result.0 as isize
            )
        })
    }

    fn windows(&self) -> Vec<Window> {
        CaptureWindow::enumerate()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|window| {
                let rect = window.rect().ok()?;
                let pid = window.process_id().ok()? as i32;
                let title = window.title().ok()?;
                let app = window
                    .process_name()
                    .unwrap_or_else(|_| format!("pid {pid}"))
                    .trim_end_matches(".exe")
                    .to_owned();
                let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
                (w > 0 && h > 0).then(|| Window {
                    id: id_from_hwnd(HWND(window.as_raw_hwnd())),
                    pid,
                    app,
                    title,
                    frame: Frame {
                        x: rect.left.into(),
                        y: rect.top.into(),
                        w: w.into(),
                        h: h.into(),
                    },
                })
            })
            .collect()
    }

    fn raise(&self, pid: i32, window: Option<&Window>) -> Result<(), String> {
        let hwnd = window
            .map(|w| hwnd_from_id(w.id))
            .or_else(|| {
                self.windows()
                    .into_iter()
                    .find(|w| w.pid == pid)
                    .map(|w| hwnd_from_id(w.id))
            })
            .ok_or_else(|| format!("pid {pid} has no window"))?;
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            if SetForegroundWindow(hwnd).as_bool() {
                Ok(())
            } else {
                Err("Windows refused to move that window to the foreground".to_owned())
            }
        }
    }

    fn focused_app(&self) -> Option<i32> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        (pid != 0).then_some(pid as i32)
    }

    fn focused_element(&self) -> Option<Self::Element> {
        unsafe { self.automation.GetFocusedElement().ok().map(Element) }
    }

    fn hit(&self, at: Pt) -> Option<Hit> {
        let element = unsafe {
            self.automation
                .ElementFromPoint(POINT {
                    x: at.x.round() as i32,
                    y: at.y.round() as i32,
                })
                .ok()?
        };
        let node = Self::node_of(&element, 0);
        Some(Hit {
            pid: unsafe { element.CurrentProcessId().ok()? },
            role: node.role,
            title: node.title,
        })
    }

    fn elements(
        &self,
        pid: i32,
        window: Option<&Window>,
        max: usize,
    ) -> Vec<(Self::Element, Node)> {
        let Some(root) = self.element_for(pid, window) else {
            return Vec::new();
        };
        let Ok(walker) = (unsafe { self.automation.ControlViewWalker() }) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        Self::walk(&walker, root, 0, max, &mut out);
        out
    }

    fn same(&self, a: &Self::Element, b: &Self::Element) -> bool {
        unsafe {
            self.automation
                .CompareElements(&a.0, &b.0)
                .is_ok_and(|v| v.as_bool())
        }
    }

    fn frame(&self, element: &Self::Element) -> Option<Frame> {
        unsafe { element.0.CurrentBoundingRectangle().ok() }.and_then(frame_from_rect)
    }

    fn value(&self, element: &Self::Element) -> Option<String> {
        value_of(&element.0)
    }

    fn node(&self, element: &Self::Element) -> Option<Node> {
        Some(Self::node_of(&element.0, 0))
    }

    fn press(&self, element: &Self::Element) -> Result<(), String> {
        unsafe {
            element
                .0
                .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                .and_then(|pattern| pattern.Invoke())
                .map_err(|e| e.to_string())
        }
    }

    fn focus(&self, element: &Self::Element) -> Result<(), String> {
        unsafe { element.0.SetFocus().map_err(|e| e.to_string()) }
    }

    fn pointer(&self, action: Pointer) {
        unsafe {
            match action {
                Pointer::Click { at, right, count } => {
                    let _ = SetCursorPos(at.x.round() as i32, at.y.round() as i32);
                    let (down, up) = if right {
                        (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
                    } else {
                        (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
                    };
                    for _ in 0..count {
                        mouse_event(down, 0, 0, 0, 0);
                        mouse_event(up, 0, 0, 0, 0);
                    }
                }
                Pointer::Drag { from, to } => {
                    let _ = SetCursorPos(from.x.round() as i32, from.y.round() as i32);
                    mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
                    let _ = SetCursorPos(to.x.round() as i32, to.y.round() as i32);
                    mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
                }
                Pointer::Scroll { at, down, right } => {
                    let _ = SetCursorPos(at.x.round() as i32, at.y.round() as i32);
                    if down != 0 {
                        mouse_event(MOUSEEVENTF_WHEEL, 0, 0, -down * WHEEL_DELTA as i32, 0);
                    }
                    if right != 0 {
                        mouse_event(MOUSEEVENTF_HWHEEL, 0, 0, right * WHEEL_DELTA as i32, 0);
                    }
                }
            }
        }
    }

    fn type_char(&self, c: char) {
        let mut utf16 = [0u16; 2];
        for unit in c.encode_utf16(&mut utf16).iter().copied() {
            send_key_pair(0, unit, KEYEVENTF_UNICODE);
        }
    }

    fn key(&self, combo: Combo) {
        let modifiers = windows_modifiers(combo.flags);
        for vk in &modifiers {
            send_key_event(*vk, 0, KEYBD_EVENT_FLAGS(0));
        }
        if let Some(vk) = virtual_key(combo.code) {
            send_key_pair(vk, 0, KEYBD_EVENT_FLAGS(0));
        }
        for vk in modifiers.iter().rev() {
            send_key_event(*vk, 0, KEYEVENTF_KEYUP);
        }
    }

    fn pause(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    fn process_name(&self, pid: i32) -> Option<String> {
        self.apps()
            .into_iter()
            .find(|app| app.pid == pid)
            .map(|app| app.name)
    }
}

struct CaptureFlags {
    tx: mpsc::SyncSender<Result<Vec<u8>, String>>,
    width: u32,
    height: u32,
}

struct Screenshot(CaptureFlags);

fn capture_settings<T>(
    item: T,
    tx: mpsc::SyncSender<Result<Vec<u8>, String>>,
    width: usize,
    height: usize,
) -> Settings<CaptureFlags, T>
where
    T: windows_capture::settings::TryIntoCaptureItemWithType,
{
    Settings::new(
        item,
        CursorCaptureSettings::WithCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Include,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        CaptureFlags {
            tx,
            width: width as u32,
            height: height as u32,
        },
    )
}

impl GraphicsCaptureApiHandler for Screenshot {
    type Flags = CaptureFlags;
    type Error = String;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self(ctx.flags))
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut CaptureFrame,
        control: windows_capture::graphics_capture_api::InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let result = encode_frame(frame, self.0.width, self.0.height);
        let _ = self.0.tx.send(result);
        control.stop();
        Ok(())
    }
}

fn encode_frame(frame: &mut CaptureFrame, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let source_width = frame.width();
    let source_height = frame.height();
    let mut buffer = frame.buffer().map_err(|e| e.to_string())?;
    let bgra = buffer.as_nopadding_buffer().map_err(|e| e.to_string())?;
    let mut rgb = Vec::with_capacity(source_width as usize * source_height as usize * 3);
    for pixel in bgra.as_chunks::<4>().0 {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let image = RgbImage::from_raw(source_width, source_height, rgb)
        .ok_or("invalid Graphics Capture frame")?;
    let image = if source_width == width && source_height == height {
        DynamicImage::ImageRgb8(image)
    } else {
        DynamicImage::ImageRgb8(image).resize_exact(width, height, FilterType::Triangle)
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 82)
        .encode_image(&image)
        .map_err(|e| e.to_string())?;
    Ok(jpeg)
}

fn value_of(element: &IUIAutomationElement) -> Option<String> {
    if unsafe { element.CurrentIsPassword().is_ok_and(|v| v.as_bool()) } {
        return None;
    }
    unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()?
            .CurrentValue()
            .ok()
            .map(|v| v.to_string())
    }
}

fn frame_from_rect(rect: windows::Win32::Foundation::RECT) -> Option<Frame> {
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    (w > 0 && h > 0).then_some(Frame {
        x: rect.left.into(),
        y: rect.top.into(),
        w: w.into(),
        h: h.into(),
    })
}

fn id_from_hwnd(hwnd: HWND) -> u32 {
    hwnd.0 as usize as u32
}

fn hwnd_from_id(id: u32) -> HWND {
    HWND(id as usize as *mut _)
}

#[allow(non_upper_case_globals)]
fn role(control: Option<UIA_CONTROLTYPE_ID>, password: bool) -> &'static str {
    if password {
        return "AXSecureTextField";
    }
    match control {
        Some(UIA_ButtonControlTypeId) => "AXButton",
        Some(UIA_CheckBoxControlTypeId) => "AXCheckBox",
        Some(UIA_RadioButtonControlTypeId) => "AXRadioButton",
        Some(UIA_MenuItemControlTypeId) => "AXMenuItem",
        Some(UIA_ComboBoxControlTypeId) => "AXPopUpButton",
        Some(UIA_EditControlTypeId | UIA_DocumentControlTypeId) => "AXTextArea",
        Some(UIA_TextControlTypeId) => "AXStaticText",
        Some(UIA_WindowControlTypeId) => "AXWindow",
        Some(UIA_ListControlTypeId) => "AXList",
        Some(UIA_ListItemControlTypeId) => "AXRow",
        Some(UIA_TreeControlTypeId) => "AXOutline",
        Some(UIA_TreeItemControlTypeId) => "AXRow",
        Some(UIA_TabControlTypeId) => "AXTabGroup",
        Some(UIA_TabItemControlTypeId) => "AXRadioButton",
        Some(UIA_HyperlinkControlTypeId) => "AXLink",
        Some(UIA_ImageControlTypeId) => "AXImage",
        _ => "AXGroup",
    }
}

fn send_key_event(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) {
    let inputs = [INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }];
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

fn send_key_pair(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) {
    let inputs = [
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: scan,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        },
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: scan,
                    dwFlags: flags | KEYEVENTF_KEYUP,
                    ..Default::default()
                },
            },
        },
    ];
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

fn windows_modifiers(flags: u64) -> Vec<u16> {
    let mut keys_out = Vec::new();
    // `cmd` is the public cross-platform shortcut spelling inherited from the
    // Mac backend. On Windows its conventional equivalent is Ctrl.
    if flags & (keys::CONTROL | keys::COMMAND) != 0 {
        keys_out.push(VK_CONTROL.0);
    }
    if flags & keys::OPTION != 0 {
        keys_out.push(VK_MENU.0);
    }
    if flags & keys::SHIFT != 0 {
        keys_out.push(VK_SHIFT.0);
    }
    keys_out
}

/// Translate the public parser's stable ANSI key ids to Windows virtual keys.
fn virtual_key(code: u16) -> Option<u16> {
    Some(match code {
        0 => b'A',
        1 => b'S',
        2 => b'D',
        3 => b'F',
        4 => b'H',
        5 => b'G',
        6 => b'Z',
        7 => b'X',
        8 => b'C',
        9 => b'V',
        11 => b'B',
        12 => b'Q',
        13 => b'W',
        14 => b'E',
        15 => b'R',
        16 => b'Y',
        17 => b'T',
        18 => b'1',
        19 => b'2',
        20 => b'3',
        21 => b'4',
        22 => b'6',
        23 => b'5',
        25 => b'9',
        26 => b'7',
        28 => b'8',
        29 => b'0',
        31 => b'O',
        32 => b'U',
        34 => b'I',
        35 => b'P',
        37 => b'L',
        38 => b'J',
        40 => b'K',
        45 => b'N',
        46 => b'M',
        24 => VK_OEM_PLUS.0 as u8,
        27 => VK_OEM_MINUS.0 as u8,
        30 => VK_OEM_6.0 as u8,
        33 => VK_OEM_4.0 as u8,
        39 => VK_OEM_7.0 as u8,
        41 => VK_OEM_1.0 as u8,
        42 => VK_OEM_5.0 as u8,
        43 => VK_OEM_COMMA.0 as u8,
        44 => VK_OEM_2.0 as u8,
        47 => VK_OEM_PERIOD.0 as u8,
        50 => VK_OEM_3.0 as u8,
        36 => VK_RETURN.0 as u8,
        48 => VK_TAB.0 as u8,
        49 => VK_SPACE.0 as u8,
        51 => VK_BACK.0 as u8,
        53 => VK_ESCAPE.0 as u8,
        117 => VK_DELETE.0 as u8,
        115 => VK_HOME.0 as u8,
        119 => VK_END.0 as u8,
        116 => VK_PRIOR.0 as u8,
        121 => VK_NEXT.0 as u8,
        123 => VK_LEFT.0 as u8,
        124 => VK_RIGHT.0 as u8,
        125 => VK_DOWN.0 as u8,
        126 => VK_UP.0 as u8,
        122 => VK_F1.0 as u8,
        120 => VK_F2.0 as u8,
        99 => VK_F3.0 as u8,
        118 => VK_F4.0 as u8,
        96 => VK_F5.0 as u8,
        97 => VK_F6.0 as u8,
        98 => VK_F7.0 as u8,
        100 => VK_F8.0 as u8,
        101 => VK_F9.0 as u8,
        109 => VK_F10.0 as u8,
        103 => VK_F11.0 as u8,
        111 => VK_F12.0 as u8,
        _ => return None,
    } as u16)
}

pub fn serve(home: &Path) -> std::io::Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let desktop = WindowsDesktop::new().map_err(std::io::Error::other)?;
    let mut controller = Controller::new(desktop);
    crate::helper::serve(
        home,
        &mut controller,
        |desktop| {
            let display = desktop
                .display()
                .map(|d| json!({ "width": d.frame.w, "height": d.frame.h, "scale": d.backing }));
            json!({ "screen": true, "accessibility": true, "post_events": true, "display": display, "session": { "locked": false } })
        },
        |_, _| {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FakeNode {
        role: UIA_CONTROLTYPE_ID,
        password: bool,
        children: Vec<FakeNode>,
    }

    fn flatten(node: &FakeNode, depth: usize, max: usize, out: &mut Vec<(usize, &'static str)>) {
        if out.len() >= max {
            return;
        }
        out.push((depth, role(Some(node.role), node.password)));
        for child in &node.children {
            flatten(child, depth + 1, max, out);
        }
    }

    #[test]
    fn a_fake_uia_tree_keeps_depth_roles_passwords_and_limit() {
        let tree = FakeNode {
            role: UIA_WindowControlTypeId,
            password: false,
            children: vec![
                FakeNode {
                    role: UIA_ButtonControlTypeId,
                    password: false,
                    children: vec![],
                },
                FakeNode {
                    role: UIA_GroupControlTypeId,
                    password: false,
                    children: vec![FakeNode {
                        role: UIA_EditControlTypeId,
                        password: true,
                        children: vec![],
                    }],
                },
            ],
        };
        let mut found = Vec::new();
        flatten(&tree, 0, 4, &mut found);
        assert_eq!(
            found,
            [
                (0, "AXWindow"),
                (1, "AXButton"),
                (1, "AXGroup"),
                (2, "AXSecureTextField")
            ]
        );
        let mut limited = Vec::new();
        flatten(&tree, 0, 2, &mut limited);
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn korean_and_surrogate_text_are_valid_utf16_input_units() {
        let units: Vec<u16> = "한글🙂".encode_utf16().collect();
        assert_eq!(&units[..2], &[0xd55c, 0xae00]);
        assert_eq!(units.len(), 4, "the emoji is a surrogate pair");
    }

    #[test]
    fn public_shortcuts_translate_to_windows_virtual_keys() {
        assert_eq!(
            virtual_key(keys::parse("ctrl+a").unwrap().code),
            Some(b'A' as u16)
        );
        assert_eq!(
            virtual_key(keys::parse("return").unwrap().code),
            Some(VK_RETURN.0)
        );
        assert_eq!(
            windows_modifiers(keys::CONTROL | keys::SHIFT),
            [VK_CONTROL.0, VK_SHIFT.0]
        );
    }
}
