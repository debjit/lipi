use std::sync::mpsc;
use std::thread;

#[cfg(target_os = "windows")]
use std::time::Duration;

pub struct ClipboardService {
    tx: mpsc::Sender<(String, mpsc::Sender<Result<(), String>>)>,
}

impl ClipboardService {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<(String, mpsc::Sender<Result<(), String>>)>();
        thread::spawn(move || {
            let mut clipboard = arboard::Clipboard::new().ok();
            while let Ok((text, done)) = rx.recv() {
                if clipboard.is_none() {
                    clipboard = arboard::Clipboard::new().ok();
                }
                let res = match clipboard.as_mut() {
                    Some(cb) => cb.set_text(&text).map_err(|e| e.to_string()),
                    None => Err("Clipboard unavailable".into()),
                };
                let _ = done.send(res);
            }
        });
        Self { tx }
    }

    pub fn set_text(&self, text: &str) -> Result<(), String> {
        let (done_tx, done_rx) = mpsc::channel();
        self.tx
            .send((text.to_string(), done_tx))
            .map_err(|e| format!("Clipboard thread closed: {e}"))?;
        done_rx
            .recv()
            .map_err(|e| format!("Clipboard thread stalled: {e}"))?
    }
}

#[derive(Debug, Clone, Default)]
pub struct PasteTarget {
    #[cfg(target_os = "linux")]
    pub x11_window: Option<u32>,
    #[cfg(target_os = "linux")]
    pub wayland_inject: bool,
    #[cfg(target_os = "windows")]
    pub hwnd: Option<isize>,
}

impl PasteTarget {
    pub fn has_target(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            if self.wayland_inject || self.x11_window.is_some() {
                return true;
            }
        }
        #[cfg(target_os = "windows")]
        if self.hwnd.is_some() {
            return true;
        }
        false
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn is_wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|s| s.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || std::env::var("WAYLAND_DISPLAY").map(|s| !s.is_empty()).unwrap_or(false)
}

/// Snapshot the currently focused window.
/// If Lipi's main window is active (allow_fallback = false), returns no target so dictation stays in Lipi editor.
/// If Lipi is in mini floating widget mode (allow_fallback = true), falls back to the window right underneath.
pub fn capture_paste_target(allow_fallback: bool, lipi_focused: bool) -> PasteTarget {
    let mut target = PasteTarget::default();

    #[cfg(target_os = "linux")]
    {
        if is_wayland_session() {
            target.wayland_inject = allow_fallback || !lipi_focused;
        } else if let Ok(win) = capture_x11_target(allow_fallback) {
            target.x11_window = Some(win);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let _ = lipi_focused;
        if let Some(hwnd) = capture_windows_target(allow_fallback) {
            target.hwnd = Some(hwnd);
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = (allow_fallback, lipi_focused);
    }

    target
}

#[cfg(target_os = "linux")]
fn capture_x11_target(allow_fallback: bool) -> Result<u32, String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let (conn, screen_num) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    let net_active_window = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .atom;

    let net_wm_pid = conn
        .intern_atom(false, b"_NET_WM_PID")
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .atom;

    let current_pid = std::process::id();

    // 1. Try active window
    if let Ok(prop) = conn.get_property(false, root, net_active_window, AtomEnum::WINDOW, 0, 1) {
        if let Ok(reply) = prop.reply() {
            if let Some(win) = reply.value32().and_then(|mut v| v.next()) {
                if win != 0 {
                    let win_pid = get_window_pid(&conn, win, net_wm_pid);
                    if let Some(pid) = win_pid {
                        if pid == current_pid {
                            if !allow_fallback {
                                return Err("Lipi is active window".into());
                            }
                        } else {
                            return Ok(win);
                        }
                    } else {
                        return Ok(win);
                    }
                }
            }
        }
    }

    if !allow_fallback {
        return Err("No external active window found".into());
    }

    // 2. Active window is Lipi itself in mini widget mode. Inspect stacking list and client list from top backwards.
    for atom_name in [b"_NET_CLIENT_LIST_STACKING" as &[u8], b"_NET_CLIENT_LIST"] {
        if let Ok(atom_reply) = conn.intern_atom(false, atom_name).map(|c| c.reply()) {
            if let Ok(atom) = atom_reply {
                if let Ok(prop) = conn.get_property(false, root, atom.atom, AtomEnum::WINDOW, 0, 1024) {
                    if let Ok(reply) = prop.reply() {
                        if let Some(windows) = reply.value32() {
                            let list: Vec<u32> = windows.collect();
                            for &win in list.iter().rev() {
                                if win == 0 {
                                    continue;
                                }
                                let win_pid = get_window_pid(&conn, win, net_wm_pid);
                                if let Some(pid) = win_pid {
                                    if pid != current_pid {
                                        return Ok(win);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Err("No external window found".into())
}

#[cfg(target_os = "linux")]
fn get_window_pid<C: x11rb::connection::Connection>(
    conn: &C,
    win: u32,
    net_wm_pid: u32,
) -> Option<u32> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let reply = conn
        .get_property(false, win, net_wm_pid, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let pid = {
        let mut iter = reply.value32()?;
        iter.next()
    };
    pid
}

#[cfg(target_os = "windows")]
fn capture_windows_target(allow_fallback: bool) -> Option<isize> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
        IsWindowVisible, GA_ROOTOWNER, GWL_EXSTYLE, GW_HWNDNEXT, WS_EX_TOOLWINDOW,
    };

    unsafe fn is_pasteable(hwnd: HWND) -> bool {
        if hwnd.is_null() || IsWindowVisible(hwnd) == 0 {
            return false;
        }
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW != 0 {
            return false;
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute;
            const DWMWA_CLOAKED: u32 = 14;
            let mut cloaked: u32 = 0;
            let _ = DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut _,
                std::mem::size_of::<u32>() as u32,
            );
            if cloaked != 0 {
                return false;
            }
        }
        true
    }

    unsafe {
        let current_pid = GetCurrentProcessId();
        let mut foreground: HWND = GetForegroundWindow();
        if foreground.is_null() {
            return None;
        }
        foreground = GetAncestor(foreground, GA_ROOTOWNER);
        if foreground.is_null() {
            foreground = GetForegroundWindow();
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(foreground, &mut pid);
        if pid != current_pid && is_pasteable(foreground) {
            return Some(foreground as isize);
        }

        if !allow_fallback {
            return None;
        }

        foreground = GetWindow(foreground, GW_HWNDNEXT);
        while !foreground.is_null() {
            let top = GetAncestor(foreground, GA_ROOTOWNER);
            let candidate = if top.is_null() { foreground } else { top };
            let mut next_pid = 0u32;
            GetWindowThreadProcessId(candidate, &mut next_pid);
            if next_pid != current_pid && is_pasteable(candidate) {
                return Some(candidate as isize);
            }
            foreground = GetWindow(foreground, GW_HWNDNEXT);
        }
    }
    None
}

/// Restore the captured target window, wait briefly, and simulate Ctrl+V.
pub fn paste_into_previous_app(
    clipboard: &ClipboardService,
    target: Option<PasteTarget>,
    text: &str,
) -> Result<(), String> {
    clipboard.set_text(text)?;

    #[cfg(target_os = "linux")]
    {
        let win_opt = target.as_ref().and_then(|t| t.x11_window);
        let wayland_inject = target.as_ref().map(|t| t.wayland_inject).unwrap_or(false);
        if let Some(win) = win_opt {
            let _ = xdo_linux::activate_x11(win);
        }

        if is_command_available("ydotool") {
            if let Ok(status) = std::process::Command::new("ydotool")
                .args(["key", "29:1", "47:1", "47:0", "29:0"])
                .status()
            {
                if status.success() {
                    return Ok(());
                }
            }
        }
        if is_wayland_session() && is_command_available("wtype") {
            if let Ok(status) = std::process::Command::new("wtype")
                .args(["-M", "ctrl", "v", "-m", "ctrl"])
                .status()
            {
                if status.success() {
                    return Ok(());
                }
            }
        }

        if let Some(win) = win_opt {
            return xdo_linux::paste_x11(win);
        }

        if wayland_inject {
            return Err("Could not inject Ctrl+V on Wayland. Install ydotool (with ydotoold) or wtype, or copy then paste manually.".into());
        }
        return Err("No target window found".into());
    }

    #[cfg(target_os = "windows")]
    {
        restore_windows_window(target.as_ref().and_then(|t| t.hwnd));
        std::thread::sleep(Duration::from_millis(150));
        send_windows_ctrl_v()?;
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    Err("Auto-paste not supported on this platform".into())
}

#[cfg(target_os = "linux")]
mod xdo_linux {
    use std::os::raw::{c_char, c_int, c_ulong, c_void};
    use std::ptr;
    use std::time::Duration;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};

    #[link(name = "xdo")]
    extern "C" {
        fn xdo_new(display: *const c_char) -> *mut c_void;
        fn xdo_free(xdo: *mut c_void);
        fn xdo_activate_window(xdo: *mut c_void, window: c_ulong) -> c_int;
        fn xdo_send_keysequence_window(
            xdo: *mut c_void,
            window: c_ulong,
            keysequence: *const c_char,
            delay: u32,
        ) -> c_int;
    }

    pub fn activate_x11(win: u32) -> Result<(), String> {
        // Send EWMH client message with source=2 (direct user interaction) for GNOME Mutter & KWin
        if let Ok((conn, screen_num)) = x11rb::connect(None) {
            let screen = &conn.setup().roots[screen_num];
            let root = screen.root;
            if let Ok(reply) = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW").map(|c| c.reply()) {
                if let Ok(net_active) = reply {
                    let event = ClientMessageEvent {
                        response_type: 33, // CLIENT_MESSAGE
                        format: 32,
                        sequence: 0,
                        window: win,
                        type_: net_active.atom,
                        data: [2, 0, 0, 0, 0].into(), // 2 = direct user interaction
                    };
                    let _ = conn.send_event(
                        false,
                        root,
                        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                        event,
                    );
                    let _ = conn.flush();
                }
            }
        }

        unsafe {
            let ctx = xdo_new(ptr::null());
            if !ctx.is_null() {
                let _ = xdo_activate_window(ctx, win as c_ulong);
                xdo_free(ctx);
            }
        }

        Ok(())
    }

    pub fn paste_x11(win: u32) -> Result<(), String> {
        activate_x11(win)?;
        std::thread::sleep(Duration::from_millis(200));
        unsafe {
            let ctx = xdo_new(ptr::null());
            if !ctx.is_null() {
                let seq = std::ffi::CString::new("ctrl+v").unwrap();
                let res = xdo_send_keysequence_window(ctx, 0, seq.as_ptr(), 20000);
                if res != 0 {
                    let _ = xdo_send_keysequence_window(ctx, win as c_ulong, seq.as_ptr(), 20000);
                }
                xdo_free(ctx);
                return Ok(());
            }
        }
        Err("Failed to send paste keystroke".into())
    }
}

#[cfg(target_os = "windows")]
fn restore_windows_window(hwnd: Option<isize>) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

    let Some(h) = hwnd else {
        return;
    };
    unsafe {
        let current = GetForegroundWindow();
        if current as isize == h {
            return;
        }
        let _ = SetForegroundWindow(h as HWND);
    }
}

#[cfg(target_os = "windows")]
fn send_windows_ctrl_v() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| format!("Enigo init error: {:?}", e))?;
    enigo
        .key(Key::Control, Direction::Press)
        .map_err(|e| format!("Key press error: {:?}", e))?;
    enigo
        .key(Key::Other(0x56), Direction::Click)
        .map_err(|e| format!("Key click error: {:?}", e))?;
    enigo
        .key(Key::Control, Direction::Release)
        .map_err(|e| format!("Key release error: {:?}", e))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn is_command_available(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_paste_target() {
        let _ = capture_paste_target(false, true);
        let _ = capture_paste_target(true, false);
    }
}




