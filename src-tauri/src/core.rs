use enigo::{Axis, Enigo, Mouse};
use serde::Serialize;
#[cfg(target_os = "linux")]
use std::process::Command;
use std::{
    env,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, ipc::Response};
use tauri::{Manager, command};
use tauri_plugin_clipboard;
use tokio::{sync::Mutex, time::Duration};

#[cfg(target_os = "windows")]
use crate::os::free_drag::set_window_proc;
use crate::{os, screenshot::get_target_monitor};

#[command]
pub async fn exit_app(window: tauri::Window, handle: tauri::AppHandle) {
    window.hide().unwrap();
    handle.exit(0);
}

#[command]
pub async fn get_selected_text() -> String {
    #[cfg(target_os = "linux")]
    if let Some(text) = get_linux_selected_text() {
        if !text.trim().is_empty() {
            return text;
        }
    }

    let text = match get_selected_text::get_selected_text() {
        Ok(text) => text,
        Err(_) => {
            return String::new();
        }
    };
    text
}

#[cfg(target_os = "linux")]
fn get_linux_selected_text() -> Option<String> {
    let commands: [(&str, &[&str]); 2] = [
        ("wl-paste", &["--primary", "--no-newline"]),
        ("xclip", &["-selection", "primary", "-o"]),
    ];

    for (program, args) in commands {
        let output = match Command::new(program).args(args).output() {
            Ok(output) => output,
            Err(_) => continue,
        };

        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }

    None
}

#[command]
pub async fn auto_start_hide_window(
    window: tauri::Window,
    auto_start_hide_window: tauri::State<'_, Mutex<bool>>,
) -> Result<(), ()> {
    let mut auto_start_hide_window = auto_start_hide_window.lock().await;

    if *auto_start_hide_window == false && env::args().any(|arg| arg == "--auto_start") {
        window.hide().unwrap();
        *auto_start_hide_window = true;
    }
    Ok(())
}

#[command]
pub async fn set_enable_proxy(enable: bool, host: String) -> Result<(), ()> {
    unsafe {
        if enable {
            std::env::set_var("NO_PROXY", "");
        } else {
            std::env::set_var("NO_PROXY", host);
        }
    }
    Ok(())
}

/// 鼠标滚轮穿透
#[command]
pub async fn scroll_through(
    window: tauri::Window,
    enigo: tauri::State<'_, Mutex<Enigo>>,
    length: i32,
) -> Result<(), ()> {
    let result = window.set_ignore_cursor_events(true);
    if result.is_err() {
        return Ok(());
    }

    tokio::time::sleep(Duration::from_millis(1)).await;

    {
        let mut enigo = enigo.lock().await;
        match enigo.scroll(length, Axis::Vertical) {
            Ok(_) => (),
            Err(e) => {
                log::error!("[scroll_through] scroll error: {}", e);
            }
        }
    }

    tokio::time::sleep(Duration::from_millis(128)).await;
    let _ = window.set_ignore_cursor_events(false);

    Ok(())
}

/// 鼠标滚轮穿透
#[command]
pub async fn click_through(window: tauri::Window) -> Result<(), ()> {
    let result = window.set_ignore_cursor_events(true);
    if result.is_err() {
        return Ok(());
    }

    tokio::time::sleep(Duration::from_millis(128)).await;
    match window.set_ignore_cursor_events(false) {
        Ok(_) => (),
        Err(_) => (),
    }

    Ok(())
}

/// 创建内容固定到屏幕的窗口
#[command]
pub async fn create_fixed_content_window(app: tauri::AppHandle, scroll_screenshot: bool) {
    let (_, _, monitor) = get_target_monitor();

    let monitor_x = monitor.x().unwrap() as f64;
    let monitor_y = monitor.y().unwrap() as f64;

    let window = tauri::WebviewWindowBuilder::new(
        &app,
        format!(
            "fixed-content-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ),
        tauri::WebviewUrl::App(PathBuf::from(format!(
            "/fixedContent?scroll_screenshot={}",
            scroll_screenshot
        ))),
    )
    .always_on_top(true)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .fullscreen(false)
    .title("Snow Shot - Fixed Content")
    .position(monitor_x, monitor_y)
    .decorations(false)
    .shadow(false)
    .transparent(false)
    .skip_taskbar(true)
    .resizable(false)
    .inner_size(0.0, 0.0)
    .build()
    .unwrap();

    window.hide().unwrap();
    window.center().unwrap();
}

#[command]
pub async fn read_image_from_clipboard(handle: tauri::AppHandle) -> Response {
    let clipboard = handle.state::<tauri_plugin_clipboard::Clipboard>();
    let image_data = match tauri_plugin_clipboard::Clipboard::read_image_binary(&clipboard) {
        Ok(image_data) => image_data,
        Err(_) => return Response::new(vec![]),
    };

    return Response::new(image_data);
}

/// 创建全屏绘制窗口
#[command]
pub async fn create_full_screen_draw_window(app: tauri::AppHandle) {
    let window_label = "full-screen-draw";

    let window = app.get_webview_window(window_label);

    if let Some(window) = window {
        // 发送改变鼠标穿透的消息
        window
            .emit("full-screen-draw-change-mouse-through", ())
            .unwrap();

        return;
    }

    // 首先先查询是否存在窗口

    let (_, _, monitor) = get_target_monitor();

    let monitor_x = monitor.x().unwrap() as f64;
    let monitor_y = monitor.y().unwrap() as f64;
    let monitor_width = monitor.width().unwrap() as f64;
    let monitor_height = monitor.height().unwrap() as f64;

    tauri::WebviewWindowBuilder::new(
        &app,
        window_label,
        tauri::WebviewUrl::App(PathBuf::from(format!("/fullScreenDraw"))),
    )
    .always_on_top(true)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .title("Snow Shot - Full Screen Draw")
    .position(monitor_x, monitor_y)
    .inner_size(monitor_width, monitor_height)
    .decorations(false)
    .shadow(false)
    .transparent(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .unwrap();

    tauri::WebviewWindowBuilder::new(
        &app,
        format!("{}_switch_mouse_through", window_label),
        tauri::WebviewUrl::App(PathBuf::from(format!(
            "/fullScreenDraw/switchMouseThrough?monitor_x={}&monitor_y={}&monitor_width={}&monitor_height={}",
            monitor_x,
            monitor_y,
            monitor_width,
            monitor_height
        ))),
    )
    .always_on_top(true)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .title("Snow Shot - Full Screen Draw - Switch Mouse Through")
    .position(monitor_x, monitor_y)
    .inner_size(0.0, 0.0)
    .decorations(false)
    .shadow(false)
    .transparent(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .unwrap();
}

#[derive(Serialize, Clone, Copy)]
pub struct MonitorInfo {
    mouse_x: i32,
    mouse_y: i32,
    monitor_x: i32,
    monitor_y: i32,
    monitor_width: u32,
    monitor_height: u32,
    monitor_scale_factor: f32,
}

#[command]
pub async fn get_current_monitor_info() -> Result<MonitorInfo, ()> {
    let (mouse_x, mouse_y, monitor) = get_target_monitor();

    let monitor_x = monitor.x().unwrap();
    let monitor_y = monitor.y().unwrap();
    let monitor_width = monitor.width().unwrap();
    let monitor_height = monitor.height().unwrap();
    let monitor_scale_factor = monitor.scale_factor().unwrap();

    let monitor_info = MonitorInfo {
        mouse_x: mouse_x - monitor_x,
        mouse_y: mouse_y - monitor_y,
        monitor_x: monitor_x,
        monitor_y: monitor_y,
        monitor_width: monitor_width,
        monitor_height: monitor_height,
        monitor_scale_factor: monitor_scale_factor,
    };
    Ok(monitor_info)
}

#[cfg(target_os = "windows")]
#[command]
pub async fn enable_free_drag(window: tauri::Window) {
    if let Ok(hwnd) = window.hwnd() {
        let _ = set_window_proc(hwnd);
    }
}

#[cfg(target_os = "linux")]
#[command]
pub async fn enable_free_drag(window: tauri::Window) {
    let _ = window.start_dragging();
}

#[command]
pub async fn send_new_version_notification(title: String, body: String) {
    os::notification::send_new_version_notification(title, body);
}

#[derive(Serialize, Clone, Copy)]
struct VideoRecordWindowInfo {
    monitor_x: f64,
    monitor_y: f64,
    monitor_width: f64,
    monitor_height: f64,
    monitor_scale_factor: f64,
    select_rect_min_x: i32,
    select_rect_min_y: i32,
    select_rect_max_x: i32,
    select_rect_max_y: i32,
}

/// 创建屏幕录制窗口
#[command]
pub async fn create_video_record_window(
    app: tauri::AppHandle,
    monitor_x: f64,
    monitor_y: f64,
    monitor_width: f64,
    monitor_height: f64,
    monitor_scale_factor: f64,
    select_rect_min_x: i32,
    select_rect_min_y: i32,
    select_rect_max_x: i32,
    select_rect_max_y: i32,
) {
    let window_label = "video-recording";

    let window = app.get_webview_window(window_label);

    if let Some(window) = window {
        window
            .emit(
                "reload-video-record",
                VideoRecordWindowInfo {
                    monitor_x,
                    monitor_y,
                    monitor_width,
                    monitor_height,
                    monitor_scale_factor,
                    select_rect_min_x,
                    select_rect_min_y,
                    select_rect_max_x,
                    select_rect_max_y,
                },
            )
            .unwrap();

        return;
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        window_label,
        tauri::WebviewUrl::App(PathBuf::from(format!(
            "/videoRecord?monitor_x={}&monitor_y={}&monitor_width={}&monitor_height={}&monitor_scale_factor={}&select_rect_min_x={}&select_rect_min_y={}&select_rect_max_x={}&select_rect_max_y={}",
            monitor_x,
            monitor_y,
            monitor_width,
            monitor_height,
            monitor_scale_factor,
            select_rect_min_x,
            select_rect_min_y,
            select_rect_max_x,
            select_rect_max_y
        ))),
    )
    .always_on_top(true)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .title("Snow Shot - Video Record")
    .position(monitor_x, monitor_y)
    .inner_size(monitor_width, monitor_height)
    .decorations(false)
    .shadow(false)
    .transparent(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .unwrap();

    let window_label = "video-recording-toolbar";

    let window = app.get_webview_window(window_label);

    if let Some(window) = window {
        window.destroy().unwrap();
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        window_label,
        tauri::WebviewUrl::App(PathBuf::from(format!(
            "/videoRecord/toolbar?monitor_x={}&monitor_y={}&monitor_width={}&monitor_height={}&monitor_scale_factor={}&select_rect_min_x={}&select_rect_min_y={}&select_rect_max_x={}&select_rect_max_y={}",
            monitor_x,
            monitor_y,
            monitor_width,
            monitor_height,
            monitor_scale_factor,
            select_rect_min_x,
            select_rect_min_y,
            select_rect_max_x,
            select_rect_max_y
        ))),
    )
    .always_on_top(true)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .title("Snow Shot - Video Record - Toolbar")
    .position(monitor_x, monitor_y)
    .inner_size(0.0, 0.0)
    .decorations(false)
    .shadow(false)
    .transparent(true)
    .skip_taskbar(true)
    .resizable(false)
    .build()
    .unwrap();
}
