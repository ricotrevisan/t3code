//! In-memory draft shelves hold uploaded references, never file bodies.
use serde_json::Value;
use std::{collections::HashMap, io::Read, path::Path};
use t3code_desktop_gpui::connection;

#[derive(Default)]
pub(crate) struct AttachmentDrafts {
    pub(crate) shelves: HashMap<(String, String), Vec<Value>>,
    // One body at a time, including across disconnect/reconnect.
    pub(crate) uploading: bool,
    pub(crate) stashes: HashMap<String, Vec<t3code_desktop_gpui::drafts::StashEntry>>,
    pub(crate) stash_deleting: bool,
    pub(crate) stash_writing: bool,
    pub(crate) transfers: Vec<AttachmentTransfer>,
    pub(crate) missing_stash_resources: HashMap<(String, String), Vec<Value>>,
}

#[derive(Clone)]
pub(crate) enum AttachmentInput {
    Path(std::path::PathBuf),
    Bytes {
        name: String,
        bytes: Vec<u8>,
        source: Option<Value>,
    },
}

pub(crate) struct AttachmentTransfer {
    pub(crate) id: String,
    pub(crate) owner: (String, String),
    pub(crate) name: String,
    pub(crate) phase: &'static str,
    pub(crate) cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) retry: Option<AttachmentInput>,
}

/// Expand a native folder drop without following symlinks. Reject the entire
/// batch on limits: no silently truncated tree or unexpectedly partial upload.
pub(crate) fn expand_paths(
    paths: Vec<std::path::PathBuf>,
    limit: usize,
) -> Result<Vec<std::path::PathBuf>, String> {
    let mut stack: Vec<_> = paths.into_iter().map(|p| (p, 0)).collect();
    let mut files = Vec::new();
    let mut entries = 0usize;
    let mut total = 0u64;
    while let Some((path, depth)) = stack.pop() {
        entries += 1;
        if entries > 2000 || depth > 16 {
            return Err("Folder exceeds 2000 entries or 16 directory levels".into());
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| "Could not open attachment")?;
        if metadata.file_type().is_symlink() {
            return Err("Folder drops cannot contain symbolic links".into());
        }
        if metadata.is_dir() {
            let children = std::fs::read_dir(path).map_err(|_| "Could not read dropped folder")?;
            for child in children {
                if stack.len() >= 2000 {
                    return Err("Folder exceeds 2000 entries".into());
                }
                stack.push((
                    child.map_err(|_| "Could not read folder entry")?.path(),
                    depth + 1,
                ));
            }
        } else if metadata.is_file() {
            total = total.saturating_add(metadata.len());
            if total > 100 * 1024 * 1024 {
                return Err("Folder drop exceeds 100 MiB total".into());
            }
            files.push(path);
            if files.len() > limit {
                return Err("You can attach up to 100 files per message".into());
            }
        } else {
            return Err("Only regular files can be attached".into());
        }
    }
    files.sort();
    Ok(files)
}

impl AttachmentInput {
    pub(crate) fn name(&self) -> String {
        match self {
            Self::Path(path) => path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned(),
            Self::Bytes { name, .. } => name.clone(),
        }
    }
    pub(crate) fn source(&self) -> Option<Value> {
        match self {
            Self::Bytes { source, .. } => source.clone(),
            Self::Path(_) => None,
        }
    }
    pub(crate) fn read(self, image: bool) -> Result<Vec<u8>, String> {
        match self {
            Self::Path(path) => read_attachment(&path, image),
            Self::Bytes { bytes, .. } => {
                let limit = if image { 10 } else { 50 } * 1024 * 1024;
                if bytes.is_empty() || bytes.len() > limit {
                    Err("Clipboard attachment exceeds upload limits".into())
                } else {
                    Ok(bytes)
                }
            }
        }
    }
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
        || id.encode_utf16().count() > 256
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
    if relative.encode_utf16().count() > 4096
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
