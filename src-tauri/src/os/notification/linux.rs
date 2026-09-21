pub fn send_new_version_notification(title: String, body: String) {
    if let Err(error) = notify_rust::Notification::new()
        .summary(&title)
        .body(&body)
        .show()
    {
        log::warn!("[notification::send_new_version_notification] {}", error);
    }
}
