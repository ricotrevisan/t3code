//! Effect RPC JSON envelopes. Keep framing distinct from domain projection.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    String(String),
    Number(u64),
}

#[derive(Debug, Serialize)]
#[serde(tag = "_tag")]
pub enum ClientMessage {
    Request {
        id: RequestId,
        tag: String,
        payload: Value,
        headers: Vec<(String, String)>,
    },
    Ack {
        #[serde(rename = "requestId")]
        request_id: RequestId,
    },
    Interrupt {
        #[serde(rename = "requestId")]
        request_id: RequestId,
    },
    Ping,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "_tag")]
pub enum ServerMessage {
    Chunk {
        #[serde(rename = "requestId")]
        request_id: RequestId,
        values: Vec<Value>,
    },
    Exit {
        #[serde(rename = "requestId")]
        request_id: RequestId,
        exit: RpcExit,
    },
    Pong,
    Defect {
        defect: Value,
    },
    ClientProtocolError {
        error: Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "_tag")]
pub enum RpcExit {
    Success { value: Value },
    Failure { cause: Value },
}

pub fn decode_frame(frame: &[u8]) -> Result<Vec<ServerMessage>, serde_json::Error> {
    let value: Value = serde_json::from_slice(frame)?;
    let messages = match value {
        Value::Array(batch) => batch,
        message => vec![message],
    };
    messages.into_iter().map(serde_json::from_value).collect()
}

pub fn request(id: u64, tag: &str, payload: Value) -> ClientMessage {
    ClientMessage::Request {
        id: RequestId::String(id.to_string()),
        tag: tag.to_owned(),
        payload,
        headers: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_matches_effect_wire_envelope() {
        assert_eq!(
            serde_json::to_value(request(1, "orchestration.subscribeShell", json!({}))).unwrap(),
            json!({"_tag":"Request","id":"1","tag":"orchestration.subscribeShell","payload":{},"headers":[]})
        );
    }

    #[test]
    fn decodes_single_and_batched_frames_preserving_id_type() {
        let batch = decode_frame(br#"[{"_tag":"Chunk","requestId":"1","values":[{"kind":"synchronized"}]},{"_tag":"Pong"},{"_tag":"Exit","requestId":2,"exit":{"_tag":"Failure","cause":[{"_tag":"Fail","error":"denied"}]}}]"#).unwrap();
        assert_eq!(batch.len(), 3);
        assert!(
            matches!(&batch[0], ServerMessage::Chunk { request_id: RequestId::String(id), .. } if id == "1")
        );
        assert!(matches!(
            &batch[2],
            ServerMessage::Exit {
                request_id: RequestId::Number(2),
                exit: RpcExit::Failure { .. }
            }
        ));
        assert!(matches!(
            decode_frame(br#"{"_tag":"Pong"}"#).unwrap()[0],
            ServerMessage::Pong
        ));
    }

    #[test]
    fn ack_uses_request_id_not_id() {
        let ack = ClientMessage::Ack {
            request_id: RequestId::String("1".into()),
        };
        assert_eq!(
            serde_json::to_value(ack).unwrap(),
            json!({"_tag":"Ack","requestId":"1"})
        );
    }

    #[test]
    fn rejects_unknown_or_malformed_protocol_instead_of_ignoring_it() {
        assert!(decode_frame(br#"{"_tag":"FutureProtocol"}"#).is_err());
        assert!(decode_frame(br#"{"_tag":"Chunk","values":[]}"#).is_err());
    }
}
