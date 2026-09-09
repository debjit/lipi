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
    }

    lipi_lib::run()
}
