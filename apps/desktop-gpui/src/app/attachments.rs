//! In-memory draft shelves hold uploaded references, never file bodies.
use serde_json::Value;
use std::{collections::HashMap, io::Read, path::Path};
use t3code_desktop_gpui::connection;

#[derive(Default)]
pub(crate) struct AttachmentDrafts {
    pub(crate) shelves: HashMap<(String, String), Vec<Value>>,
    // One body at a time, including across disconnect/reconnect.
    pub(crate) uploading: bool,
}

#[derive(Clone)]
pub(crate) struct AttachmentScope {
    pub(crate) origin: String,
    pub(crate) thread: String,
    pub(crate) generation: u64,
    pub(crate) sender: async_channel::Sender<connection::Command>,
}

impl AttachmentScope {
    pub(crate) fn key(&self) -> (String, String) {
        (self.origin.clone(), self.thread.clone())
    }
}

// Public contracts: images <=10 MiB, files <=50 MiB. A bounded read also
// catches a file growing after metadata, without allocating its entire body.
pub(crate) fn read_attachment(path: &Path, image: bool) -> Result<Vec<u8>, String> {
    let limit = if image { 10 } else { 50 } * 1024 * 1024;
    let file = std::fs::File::open(path).map_err(|_| "Could not open attachment")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Could not inspect attachment")?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        return Err(
            "Attachment must be a nonempty regular file (images ≤10 MiB, files ≤50 MiB)".into(),
        );
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read attachment")?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err("Attachment size changed or exceeds the upload limit".into());
    }
    Ok(bytes)
}

/// Accept only the captured origin's signed upload route. Never include the
/// returned URL in diagnostics: its last segment is a bearer capability.
pub(crate) fn upload_target(origin: &str, value: &Value) -> Result<(String, String), String> {
    let invalid = || "Invalid attachment upload response".to_owned();
    let id = value["attachmentId"].as_str().ok_or_else(invalid)?;
    if id.trim().is_empty()
        || id.trim() != id
        || id.chars().count() > 256
        || id.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    // The signed service issues pending-<uuid>[-<extension>] resources only.
    let resource = id.strip_prefix("pending-").ok_or_else(invalid)?;
    let uuid_text = resource.get(..36).ok_or_else(invalid)?;
    let parsed = uuid::Uuid::parse_str(uuid_text).map_err(|_| invalid())?;
    if parsed.hyphenated().to_string() != uuid_text.to_ascii_lowercase() {
        return Err(invalid());
    }
    let suffix = &resource[36..];
    if !suffix.is_empty()
        && !suffix.strip_prefix('-').is_some_and(|extension| {
            !extension.is_empty()
                && extension.len() <= 10
                && extension.bytes().all(|b| b.is_ascii_alphanumeric())
        })
    {
        return Err(invalid());
    }
    let relative = value["relativeUrl"].as_str().ok_or_else(invalid)?;
    let token = relative
        .strip_prefix("/api/attachments/upload/")
        .ok_or_else(invalid)?;
    let parts: Vec<_> = token.split('.').collect();
    if relative.len() > 4096
        || parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    {
        return Err(invalid());
    }
    let base = url::Url::parse(origin).map_err(|_| invalid())?;
    if !matches!(base.scheme(), "http" | "https") || base.origin().ascii_serialization() != origin {
        return Err(invalid());
    }
    Ok((id.to_owned(), format!("{origin}{relative}")))
}
