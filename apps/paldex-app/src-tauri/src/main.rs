// Release builds must not spawn a console window alongside the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    paldex_app_lib::run();
}
