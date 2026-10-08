//! Owned context payloads behind canonical composer references. Source panels capture;
//! this module validates and transports snapshots, never recaptures live sources.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DraftContextRecord(Value);

impl DraftContextRecord {
    /// Intake for contract records. IDs are assigned here, never supplied by a panel.
    pub fn capture(kind: &str, label: &str, fields: Value) -> Result<Self, &'static str> {
        let mut value = fields
            .as_object()
            .cloned()
            .ok_or("Context fields must be an object")?;
        value.insert("version".into(), json!(1));
        value.insert("kind".into(), json!(kind));
        value.insert("label".into(), json!(label));
        value.insert(
            "contextId".into(),
            json!(format!("ctx_{}", uuid::Uuid::new_v4().simple())),
        );
        let record = Self(Value::Object(value));
        record.validate()?;
        Ok(record)
    }
    pub fn terminal(
        label: &str,
        terminal_id: &str,
        terminal_label: &str,
        start: u64,
        end: u64,
        text: &str,
    ) -> Result<Self, &'static str> {
        Self::capture(
            "terminal",
            label,
            json!({"terminalId":terminal_id,"terminalLabel":terminal_label,"lineStart":start,"lineEnd":end,"text":text}),
        )
    }
    /// Citation is a forward-compatible contract kind: its payload uses AssistantCitation.
    pub fn citation(label: &str, citation: Value) -> Result<Self, &'static str> {
        let s = |key: &str, max: usize| {
            citation[key]
                .as_str()
                .is_some_and(|v| v.encode_utf16().count() <= max)
        };
        if citation["version"] != 1
            || !["environmentId", "threadId", "messageId"]
                .iter()
                .all(|k| s(k, 512) && !citation[k].as_str().unwrap_or_default().trim().is_empty())
            || !s("text", 8000)
            || citation["text"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .is_empty()
            || !s("prefix", 32)
            || !s("suffix", 32)
            || citation["start"]
                .as_u64()
                .zip(citation["end"].as_u64())
                .is_none_or(|(a, b)| a >= b || b > 9_007_199_254_740_991)
            || citation.get("comment").is_some_and(|_| !s("comment", 8000))
        {
            return Err("Invalid assistant citation");
        }
        Self::capture("assistant-citation", label, json!({"payload":citation}))
    }
    pub fn value(&self) -> &Value {
        &self.0
    }
    pub fn id(&self) -> &str {
        self.0["contextId"].as_str().unwrap_or_default()
    }
    pub fn kind(&self) -> &str {
        self.0["kind"].as_str().unwrap_or_default()
    }
    pub fn label(&self) -> &str {
        self.0["label"].as_str().unwrap_or_default()
    }
    pub fn reference(&self) -> String {
        let label = self
            .label()
            .replace('\\', "\\\\")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace(['\n', '\r'], " ");
        format!("[{label}](t3-context://v1/{}/{})", self.kind(), self.id())
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let string = |key: &str, max: usize, nonempty: bool| {
            self.0[key].as_str().is_some_and(|s| {
                s.encode_utf16().count() <= max && (!nonempty || !s.trim().is_empty())
            })
        };
        let range = |a: &str, b: &str| {
            self.0[a]
                .as_u64()
                .zip(self.0[b].as_u64())
                .is_some_and(|(a, b)| b >= a)
        };
        if self.0["version"] != 1
            || !string("contextId", 128, true)
            || !self
                .id()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || !string("label", 200, false)
        {
            return Err("Invalid context identity or label");
        }
        let valid = match self.kind() {
            "terminal" => {
                string("terminalId", 255, true)
                    && string("terminalLabel", 255, true)
                    && range("lineStart", "lineEnd")
                    && string("text", 64000, false)
            }
            "mention" => string("path", 2048, true),
            "skill" => string("name", 255, true),
            "file" | "image" => {
                string("attachmentId", 128, true)
                    && self.0["attachmentId"]
                        .as_str()
                        .unwrap_or_default()
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                    && string("name", 255, true)
                    && string("mimeType", 100, true)
                    && self.0["sizeBytes"].as_u64().is_some()
            }
            "review-comment" => {
                string("sectionId", 255, true)
                    && string("sectionTitle", 2048, false)
                    && string("filePath", 2048, true)
                    && range("startIndex", "endIndex")
                    && string("rangeLabel", 2048, false)
                    && string("text", 16000, false)
                    && string("diff", 32000, false)
                    && self
                        .0
                        .get("fenceLanguage")
                        .is_none_or(|_| string("fenceLanguage", 64, false))
                    && self.0.get("pullRequest").is_none_or(|pr| {
                        pr.is_object()
                            && pr["number"].as_u64().is_some_and(|n| n > 0 && n <= 9_007_199_254_740_991)
                            && ["title", "url", "headBranch", "baseBranch"].iter().all(|key| {
                                pr[key].as_str().is_some_and(|s| s.encode_utf16().count() <= 2048)
                            })
                            && matches!(pr["state"].as_str(), Some("open" | "closed" | "merged"))
                            && pr["isDraft"].is_boolean()
                    })
            }
            "element" | "preview-annotation" => false, // producers need a typed validator before intake
            kind => {
                kind.len() <= 40
                    && kind.starts_with(|c: char| c.is_ascii_lowercase())
                    && kind
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                    && self.0.get("payload").is_some_and(|v| {
                        serde_json::to_string(v).is_ok_and(|s| s.encode_utf16().count() <= 64000)
                    })
            }
        };
        if valid {
            Ok(())
        } else {
            Err("Invalid or unsupported context payload")
        }
    }
}

/// Only referenced payloads ride the message. Retain unreferenced records in the
/// draft so native editor undo can recover a deleted chip without losing its data.
pub fn message_context(
    text: &str,
    owned: &[DraftContextRecord],
) -> Result<(String, Option<Value>), &'static str> {
    let (text, derived) = crate::mentions::message_context(text);
    let mut records = derived
        .and_then(|v| v["records"].as_array().cloned())
        .unwrap_or_default();
    for record in owned {
        if text.contains(&format!(
            "](t3-context://v1/{}/{})",
            record.kind(),
            record.id()
        )) {
            record.validate()?;
            if !records
                .iter()
                .any(|v| v["contextId"] == record.0["contextId"])
            {
                records.push(record.0.clone());
            }
        }
    }
    if records.len() > 200 {
        return Err("Too many context records (maximum 200)");
    }
    Ok((
        text,
        (!records.is_empty()).then(|| json!({"version":1,"records":records})),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r14_int_review_metadata_validates_and_survives_message_transport() {
        let fields = json!({"sectionId":"pr:17","sectionTitle":"Review","filePath":"src/main.rs","startIndex":0,"endIndex":1,"rangeLabel":"1","text":"Review","diff":"","pullRequest":{"number":17,"title":"Fix","url":"https://host.test/pull/17","headBranch":"fix","baseBranch":"main","state":"open","isDraft":false}});
        let record = DraftContextRecord::capture("review-comment", "PR #17", fields.clone()).unwrap();
        let (_, context) = message_context(&record.reference(), &[record]).unwrap();
        assert_eq!(context.unwrap()["records"][0]["pullRequest"], fields["pullRequest"]);
        for (key, value) in [("number", json!(0)), ("state", json!("unknown")), ("isDraft", Value::Null), ("title", json!("😀".repeat(1025)))] {
            let mut malformed = fields.clone();
            malformed["pullRequest"][key] = value;
            assert!(DraftContextRecord::capture("review-comment", "PR", malformed).is_err(), "{key}");
        }
        let mut missing = fields;
        missing["pullRequest"].as_object_mut().unwrap().remove("url");
        assert!(DraftContextRecord::capture("review-comment", "PR", missing).is_err());
    }

    #[test]
    fn r13_c_context_snapshot_identity_bounds_and_reference_transport() {
        let r =
            DraftContextRecord::terminal("Build", "t1", "Build", 1, 2, "ok\n</context>").unwrap();
        let text = format!(
            "{} {}",
            r.reference(),
            crate::mentions::file_link("src/main.rs")
        );
        let (sent, context) = message_context(&text, std::slice::from_ref(&r)).unwrap();
        assert!(sent.contains(r.id()));
        assert_eq!(context.unwrap()["records"].as_array().unwrap().len(), 2);
        assert!(message_context("deleted", &[r]).unwrap().1.is_none());
        assert!(DraftContextRecord::terminal("x", "t", "t", 2, 1, "x").is_err());
        assert!(DraftContextRecord::terminal("x", "t", "t", 1, 1, &"😀".repeat(32001)).is_err());
    }
}
