//! Wayland has no X11 key grab. Alt+R is bound through the desktop portal instead.

#[cfg(not(target_os = "linux"))]
pub fn spawn_wayland_shortcuts(_app: tauri::AppHandle) {}

#[cfg(target_os = "linux")]
pub fn spawn_wayland_shortcuts(app: tauri::AppHandle) {
    if !crate::paste::is_wayland_session() {
        return;
    }
        std::thread::spawn(move || {
            let ctx = glib::MainContext::new();
            match ctx.with_thread_default(|| portal_loop(&app)) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("[GlobalShortcut] Wayland Alt+R unavailable: {e}"),
                Err(e) => eprintln!("[GlobalShortcut] Wayland Alt+R unavailable: {e}"),
            }
        });
}

#[cfg(target_os = "linux")]
fn portal_loop(app: &tauri::AppHandle) -> Result<(), String> {
    use gio::prelude::*;
    use tauri::Emitter;

    let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|e| e.to_string())?;

    let app_for_signal = app.clone();
    conn.signal_subscribe(
        Some("org.freedesktop.portal.Desktop"),
        Some("org.freedesktop.portal.GlobalShortcuts"),
        Some("Activated"),
        Some("/org/freedesktop/portal/desktop"),
        None,
        gio::DBusSignalFlags::NONE,
        move |_conn, _sender, _path, _iface, _member, params| {
            let id = params.child_value(1).get::<String>().unwrap_or_default();
            if id == "record" {
                let _ = app_for_signal.emit("tray_toggle_recording", ());
            }
        },
    );

    let created = portal_call(
        &conn,
        "CreateSession",
        &options_param(&[("handle_token", "lipi_create"), ("session_handle_token", "lipi")]),
        "lipi_create",
    )?;
    let session_handle: String = glib::VariantDict::new(Some(&created))
        .lookup("session_handle")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "portal session missing".to_string())?;
    let session = glib::variant::ObjectPath::try_from(session_handle.as_str()).map_err(|e| e.to_string())?;

    let listed = portal_call(
        &conn,
        "ListShortcuts",
        &glib::Variant::tuple_from_iter([
            session.to_variant(),
            options_dict("lipi_list"),
        ]),
        "lipi_list",
    );
    let already = listed.as_ref().map(|v| shortcut_bound(v)).unwrap_or(false);
    if !already {
        portal_call(
            &conn,
            "BindShortcuts",
            &glib::Variant::tuple_from_iter([
                session.to_variant(),
                record_shortcuts()?,
                "".to_variant(),
                options_dict("lipi_bind"),
            ]),
            "lipi_bind",
        )?;
        eprintln!("[GlobalShortcut] Wayland Alt+R bound. Confirm it if the system dialog is still open.");
    }

    let ctx = glib::MainContext::thread_default().ok_or_else(|| "no glib context".to_string())?;
    glib::MainLoop::new(Some(&ctx), false).run();
    Ok(())
}

#[cfg(target_os = "linux")]
fn record_shortcuts() -> Result<glib::Variant, String> {
    use glib::prelude::ToVariant;
    let ty = glib::VariantTy::new("(sa{sv})").map_err(|e| e.to_string())?;
    let props = glib::VariantDict::new(None);
    props.insert("description", "Toggle Lipi recording");
    props.insert("preferred_trigger", "ALT+r");
    let shortcut = glib::Variant::tuple_from_iter(["record".to_variant(), props.end()]);
    Ok(glib::Variant::array_from_iter_with_type(ty, [shortcut]))
}

#[cfg(target_os = "linux")]
fn options_dict(token: &str) -> glib::Variant {
    let dict = glib::VariantDict::new(None);
    dict.insert("handle_token", token);
    dict.end()
}

#[cfg(target_os = "linux")]
fn options_param(pairs: &[(&str, &str)]) -> glib::Variant {
    let dict = glib::VariantDict::new(None);
    for (k, v) in pairs {
        dict.insert(*k, *v);
    }
    glib::Variant::tuple_from_iter([dict.end()])
}

#[cfg(target_os = "linux")]
fn shortcut_bound(results: &glib::Variant) -> bool {
    let Some(list) = glib::VariantDict::new(Some(results)).lookup_value("shortcuts", None) else {
        return false;
    };
    for i in 0..list.n_children() {
        let id = list.child_value(i).child_value(0).get::<String>().unwrap_or_default();
        if id == "record" {
            return true;
        }
    }
    false
}

/// `/org/freedesktop/portal/desktop/request/SENDER/token`
#[cfg(target_os = "linux")]
fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
}

#[cfg(target_os = "linux")]
fn portal_call(
    conn: &gio::DBusConnection,
    method: &str,
    params: &glib::Variant,
    token: &str,
) -> Result<glib::Variant, String> {
    use std::cell::RefCell;
    use std::rc::Rc;

    let unique = conn
        .unique_name()
        .ok_or_else(|| "session bus has no unique name".to_string())?;
    let path = request_path(unique.as_str(), token);
    let got: Rc<RefCell<Option<(u32, glib::Variant)>>> = Rc::new(RefCell::new(None));
    let slot = got.clone();
    let token_suffix = format!("/{token}");
    let id = conn.signal_subscribe(
        Some("org.freedesktop.portal.Desktop"),
        Some("org.freedesktop.portal.Request"),
        Some("Response"),
        None,
        None,
        gio::DBusSignalFlags::NONE,
        move |_c, _s, obj_path, _i, _m, params| {
            if !obj_path.ends_with(&token_suffix) {
                return;
            }
            let code = params.child_value(0).get::<u32>().unwrap_or(2);
            *slot.borrow_mut() = Some((code, params.child_value(1)));
        },
    );
    let call = conn.call_sync(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.GlobalShortcuts",
        method,
        Some(params),
        None,
        gio::DBusCallFlags::NONE,
        30_000,
        gio::Cancellable::NONE,
    );
    let reply = call.map_err(|e| format!("{method}: {e}"))?;

    let ctx = glib::MainContext::thread_default()
        .ok_or_else(|| "no glib context".to_string())?;
    let start = std::time::Instant::now();
    while got.borrow().is_none() && start.elapsed() < std::time::Duration::from_secs(120) {
        if !ctx.iteration(false) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    conn.signal_unsubscribe(id);
    let (code, results) = got.borrow_mut().take().ok_or_else(|| {
        format!(
            "{method}: no portal response for {path} (reply {})",
            reply.print(false)
        )
    })?;
    if code != 0 {
        return Err(format!("{method}: cancelled ({code})"));
    }
    Ok(results)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn record_shortcut_uses_xdg_trigger() {
        let list = record_shortcuts().unwrap();
        assert_eq!(list.type_().as_str(), "a(sa{sv})");
        let trigger = glib::VariantDict::new(Some(&list.child_value(0).child_value(1)))
            .lookup::<String>("preferred_trigger")
            .unwrap()
            .unwrap();
        assert_eq!(trigger, "ALT+r");
    }

    #[test]
    fn portal_request_path_rewrites_unique_name() {
        assert_eq!(
            request_path(":1.215", "lipi_create"),
            "/org/freedesktop/portal/desktop/request/1_215/lipi_create"
        );
    }

    #[test]
    fn portal_create_session_lists_shortcuts() {
        if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
            return;
        }
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(|| -> Result<(), String> {
            use gio::prelude::*;
            let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
                .map_err(|e| e.to_string())?;
            let created = portal_call(
                &conn,
                "CreateSession",
                &options_param(&[
                    ("handle_token", "lipi_test_create"),
                    ("session_handle_token", "lipi_test"),
                ]),
                "lipi_test_create",
            )?;
            let session_handle: String = glib::VariantDict::new(Some(&created))
                .lookup("session_handle")
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "session_handle missing".to_string())?;
            let session = glib::variant::ObjectPath::try_from(session_handle.as_str())
                .map_err(|e| e.to_string())?;
            let listed = portal_call(
                &conn,
                "ListShortcuts",
                &glib::Variant::tuple_from_iter([
                    session.to_variant(),
                    options_dict("lipi_test_list"),
                ]),
                "lipi_test_list",
            )?;
            let _ = shortcut_bound(&listed);
            Ok(())
        })
        .expect("glib context")
        .expect("portal calls");
    }
}
