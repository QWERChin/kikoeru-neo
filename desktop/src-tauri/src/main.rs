// No console window behind the app on Windows release builds. The e2e test
// build keeps its console, so the WebDriver job can see the app's log.
#![cfg_attr(
    all(not(debug_assertions), not(feature = "e2e")),
    windows_subsystem = "windows"
)]

fn main() {
    kikoeru_neo_desktop::run();
}
