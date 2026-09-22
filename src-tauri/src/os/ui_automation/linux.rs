use crate::os::{ElementRect, TryGetElementByFocus};
use atspi::{AccessibilityConnection, CoordType, ObjectRefOwned};
use xcap::Window;

const MAX_ATSPI_DEPTH: usize = 12;
const MAX_ATSPI_NODES: usize = 800;

pub struct UIElements {
    rect_list: Vec<ElementRect>,
    atspi_rect_list: Vec<ElementRect>,
}

impl UIElements {
    pub fn new() -> Self {
        Self {
            rect_list: Vec::new(),
            atspi_rect_list: Vec::new(),
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

        rect_list.push(monitor_rect);
        self.rect_list = rect_list;

        self.atspi_rect_list =
            pollster::block_on(collect_atspi_rects(monitor_rect)).unwrap_or_default();

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
            .chain(self.atspi_rect_list.iter())
            .copied()
            .filter(|rect| {
                rect.min_x <= mouse_x
                    && rect.max_x >= mouse_x
                    && rect.min_y <= mouse_y
                    && rect.max_y >= mouse_y
            })
            .collect::<Vec<_>>();

        result
            .sort_by_key(|rect| (rect.max_x - rect.min_x).saturating_mul(rect.max_y - rect.min_y));
        result.dedup();

        Ok(result)
    }
}

async fn collect_atspi_rects(monitor_rect: ElementRect) -> Option<Vec<ElementRect>> {
    let connection = AccessibilityConnection::new().await.ok()?;
    let root = connection.root_accessible_on_registry().await.ok()?;
    let applications = root.get_children().await.ok()?;
    let bus = connection.connection();

    let mut rects = Vec::new();
    let mut visited = 0usize;

    for application in applications {
        if application.is_null() {
            continue;
        }

        if let Some(name) = accessible_name(bus, &application).await {
            let lower_name = name.to_lowercase();
            if lower_name == "app" || lower_name.contains("snow shot") {
                continue;
            }
        }

        Box::pin(collect_object(
            bus,
            application,
            monitor_rect,
            0,
            &mut rects,
            &mut visited,
        ))
        .await;
    }

    Some(rects)
}

async fn collect_object(
    connection: &atspi::zbus::Connection,
    object: ObjectRefOwned,
    monitor_rect: ElementRect,
    depth: usize,
    rects: &mut Vec<ElementRect>,
    visited: &mut usize,
) {
    if depth > MAX_ATSPI_DEPTH || *visited >= MAX_ATSPI_NODES || object.is_null() {
        return;
    }

    *visited += 1;

    let Some(destination) = object.name_as_str() else {
        return;
    };
    let path = object.path_as_str();

    if let Ok(component) =
        atspi::zbus::Proxy::new(connection, destination, path, "org.a11y.atspi.Component").await
    {
        if let Ok((x, y, width, height)) = component
            .call::<_, (i32, i32, i32, i32)>("GetExtents", &(CoordType::Screen,))
            .await
        {
            if width > 0 && height > 0 {
                let rect = ElementRect {
                    min_x: x,
                    min_y: y,
                    max_x: x + width,
                    max_y: y + height,
                }
                .clip_rect(&monitor_rect);

                if rect.max_x > rect.min_x && rect.max_y > rect.min_y {
                    rects.push(rect);
                }
            }
        }
    }

    if let Ok(accessible) =
        atspi::zbus::Proxy::new(connection, destination, path, "org.a11y.atspi.Accessible").await
    {
        if let Ok(children) = accessible
            .call::<_, Vec<ObjectRefOwned>>("GetChildren", &())
            .await
        {
            for child in children {
                Box::pin(collect_object(
                    connection,
                    child,
                    monitor_rect,
                    depth + 1,
                    rects,
                    visited,
                ))
                .await;
            }
        }
    }
}

async fn accessible_name(
    connection: &atspi::zbus::Connection,
    object: &ObjectRefOwned,
) -> Option<String> {
    let destination = object.name_as_str()?;
    let path = object.path_as_str();
    let proxy = atspi::zbus::Proxy::new(connection, destination, path, "org.a11y.atspi.Accessible")
        .await
        .ok()?;

    proxy.get_property::<String>("Name").await.ok()
}
