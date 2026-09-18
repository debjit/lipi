#[cfg(target_os = "windows")]
use std::time::Duration;

#[derive(Debug, Clone, Default)]
pub struct PasteTarget {
    #[cfg(target_os = "linux")]
    pub x11_window: Option<u32>,
    #[cfg(target_os = "windows")]
    pub hwnd: Option<isize>,
}

/// Snapshot the currently focused window, skipping Lipi's own window if focused.
pub fn capture_paste_target() -> PasteTarget {
    let mut target = PasteTarget::default();

    #[cfg(target_os = "linux")]
    {
        if let Ok(win) = capture_x11_target() {
            target.x11_window = Some(win);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(hwnd) = capture_windows_target() {
            target.hwnd = Some(hwnd);
        }
    }

    target
}

#[cfg(target_os = "linux")]
fn capture_x11_target() -> Result<u32, String> {
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
                    if win_pid.map(|pid| pid != current_pid).unwrap_or(true) {
                        return Ok(win);
                    }
                }
            }
        }
    }

    // 2. Active window is Lipi itself or 0. Inspect stacking list and client list from top backwards.
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
fn capture_windows_target() -> Option<isize> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindow, GetWindowThreadProcessId, GW_HWNDNEXT,
    };

    unsafe {
        let current_pid = GetCurrentProcessId();
        let mut foreground: HWND = GetForegroundWindow();

        while !foreground.is_null() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(foreground, &mut pid);
            if pid != current_pid {
                return Some(foreground as isize);
            }
            foreground = GetWindow(foreground, GW_HWNDNEXT);
        }
    }
    None
}

/// Restore the captured target window, wait briefly, and simulate Ctrl+V.
pub fn paste_into_previous_app(target: Option<PasteTarget>, text: &str) -> Result<(), String> {
    // Step 1: Always copy to system clipboard first
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("Clipboard error: {}", e))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| format!("Clipboard write error: {}", e))?;

    // Step 2 & 3 & 4: Restore target window and simulate Ctrl+V
    #[cfg(target_os = "linux")]
    {
        let win_opt = target.as_ref().and_then(|t| t.x11_window);
        if let Some(win) = win_opt {
            let _ = xdo_linux::activate_x11(win);
        }

        // Driver / daemon level simulation if installed
        if is_command_available("ydotool") {
            let _ = std::process::Command::new("ydotool")
                .args(["key", "29:1", "47:1", "47:0", "29:0"])
                .status();
            return Ok(());
        }
        if is_command_available("wtype") {
            let _ = std::process::Command::new("wtype")
                .args(["-M", "ctrl", "v", "-m", "ctrl"])
                .status();
            return Ok(());
        }

        // Standard simulation (X11 / Xwayland via libxdo with portal approval on GNOME Wayland)
        if let Some(win) = win_opt {
            return xdo_linux::paste_x11(win);
        }

        return Err("No target window found".into());
    }

    #[cfg(target_os = "windows")]
    {
        restore_windows_window(target.as_ref().and_then(|t| t.hwnd))?;
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
fn restore_windows_window(hwnd: Option<isize>) -> Result<(), String> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

    if let Some(h) = hwnd {
        unsafe {
            let success = SetForegroundWindow(h as HWND);
            if success != 0 {
                return Ok(());
            }
        }
    }
    Err("Failed to restore target window".into())
}

#[cfg(target_os = "windows")]
fn send_windows_ctrl_v() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| format!("Enigo init error: {:?}", e))?;
    enigo
        .key(Key::Control, Direction::Press)
        .map_err(|e| format!("Key press error: {:?}", e))?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
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
        let _ = capture_paste_target();
    }
}




