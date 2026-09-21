use crate::os::{ElementRect, TryGetElementByFocus};
use xcap::Window;

pub struct UIElements {
    rect_list: Vec<ElementRect>,
}

impl UIElements {
    pub fn new() -> Self {
        Self {
            rect_list: Vec::new(),
        }
    }

    pub fn init(&mut self) -> Result<(), ()> {
        Ok(())
    }

    pub fn init_cache(
        &mut self,
        monitor_rect: ElementRect,
        _try_get_element_by_focus: TryGetElementByFocus,
    ) -> Result<(), ()> {
        let mut rect_list = Vec::new();

        for window in Window::all().unwrap_or_default() {
            if window.is_minimized().unwrap_or(true) {
                continue;
            }

            if let Ok(title) = window.title() {
                if title == "app" || title.contains("Snow Shot") {
                    continue;
                }
            }

            let x = match window.x() {
                Ok(x) => x,
                Err(_) => continue,
            };
            let y = match window.y() {
                Ok(y) => y,
                Err(_) => continue,
            };
            let width = match window.width() {
                Ok(width) => width as i32,
                Err(_) => continue,
            };
            let height = match window.height() {
                Ok(height) => height as i32,
                Err(_) => continue,
            };

            let rect = ElementRect {
                min_x: x,
                min_y: y,
                max_x: x + width,
                max_y: y + height,
            }
            .clip_rect(&monitor_rect);

            if rect.max_x > rect.min_x && rect.max_y > rect.min_y {
                rect_list.push(rect);
            }
        }

        // The monitor itself is the largest selectable region.
        rect_list.push(monitor_rect);
        self.rect_list = rect_list;

        Ok(())
    }

    pub fn recovery_window_z_order(&self) {}

    pub fn get_element_from_point_walker(
        &mut self,
        mouse_x: i32,
        mouse_y: i32,
        _app_window: &tauri::Window,
    ) -> Result<Vec<ElementRect>, ()> {
        let mut result = self
            .rect_list
            .iter()
            .copied()
            .filter(|rect| {
                rect.min_x <= mouse_x
                    && rect.max_x >= mouse_x
                    && rect.min_y <= mouse_y
                    && rect.max_y >= mouse_y
            })
            .collect::<Vec<_>>();

        // Return the innermost (smallest) window-level region first,
        // followed by the monitor rectangle.
        result.sort_by_key(|rect| {
            (rect.max_x - rect.min_x).saturating_mul(rect.max_y - rect.min_y)
        });

        Ok(result)
    }
}
