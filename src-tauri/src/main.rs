// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    {
        // Unset snap GTK environment variables that corrupt host glibc/GTK when launched from VS Code snap terminal
        std::env::remove_var("GTK_PATH");
        std::env::remove_var("GTK_EXE_PREFIX");
        std::env::remove_var("GIO_MODULE_DIR");
        std::env::remove_var("LOCPATH");
        std::env::remove_var("GTK_IM_MODULE_FILE");

        // If running inside VS Code snap terminal, restore real user data dir
        if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
            if data_home.contains("/snap/") {
                std::env::remove_var("XDG_DATA_HOME");
            }
        }

        // The snap ships an older GSettings schema. WebKitGTK 2.52 reads
        // org.gnome.settings-daemon.plugins.xsettings.antialiasing and GLib
        // aborts when that key is missing. GSETTINGS_SCHEMA_DIR replaces the
        // host schema search path, so it has to go before GTK starts.
        if let Ok(orig) = std::env::var("XDG_DATA_DIRS_VSCODE_SNAP_ORIG") {
            std::env::set_var("XDG_DATA_DIRS", orig);
        } else if let Ok(dirs) = std::env::var("XDG_DATA_DIRS") {
            let host: Vec<&str> = dirs.split(':').filter(|dir| !dir.contains("/snap/")).collect();
            if host.is_empty() {
                std::env::remove_var("XDG_DATA_DIRS");
            } else {
                std::env::set_var("XDG_DATA_DIRS", host.join(":"));
            }
        }
        let snap = std::env::var("SNAP").is_ok();
        if let Ok(schema_dir) = std::env::var("GSETTINGS_SCHEMA_DIR") {
            if snap || schema_dir.contains("/snap/") {
                std::env::remove_var("GSETTINGS_SCHEMA_DIR");
            }
        }

        // Silence noisy upstream deprecation warning from libayatana-appindicator
        extern "C" fn null_log_handler(
            _log_domain: *const std::ffi::c_char,
            _log_level: i32,
            _message: *const std::ffi::c_char,
            _user_data: *mut std::ffi::c_void,
        ) {}

        extern "C" {
            fn g_log_set_handler(
                log_domain: *const std::ffi::c_char,
                log_levels: i32,
                log_func: extern "C" fn(*const std::ffi::c_char, i32, *const std::ffi::c_char, *mut std::ffi::c_void),
                user_data: *mut std::ffi::c_void,
            ) -> u32;
        }

        unsafe {
            let domain = b"libayatana-appindicator\0".as_ptr() as *const std::ffi::c_char;
            // G_LOG_LEVEL_WARNING = 16 (1 << 4), G_LOG_LEVEL_MESSAGE = 32 (1 << 5), G_LOG_LEVEL_INFO = 64 (1 << 6)
            g_log_set_handler(domain, 16 | 32 | 64, null_log_handler, std::ptr::null_mut());
        }
    }

    lipi_lib::run()
}
