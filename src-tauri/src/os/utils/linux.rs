use std::process::Command;

pub fn get_focused_window() -> Option<u32> {
    let output = Command::new("xdotool")
        .args(["getactivewindow"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok()
}

pub fn switch_always_on_top(window_id: u32) {
    let window_id = format!("{:#x}", window_id);
    let _ = Command::new("wmctrl")
        .args(["-i", "-r", &window_id, "-b", "toggle,above"])
        .status();
}

pub fn set_draw_window_style(window: tauri::Window) {
    let _ = window.set_decorations(false);
    let _ = window.set_skip_taskbar(true);
    let _ = window.set_always_on_top(true);
}
