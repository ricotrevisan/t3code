//! Real HTTP requests against a loopback fixture; no invented WS diff method.
use super::http::fetch;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
};
pub(crate) fn endpoint(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let status = status.to_owned();
    let body = body.to_owned();
    let task = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut data = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            data.extend_from_slice(&chunk[..n]);
            if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&data[..end]).to_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if data.len() >= end + 4 + length {
                    break;
                }
            }
        }
        write!(stream,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        String::from_utf8(data).unwrap()
    });
    (origin, task)
}
#[tokio::test]
async fn r14_p_diff_http_carries_ref_and_bearer_to_contract_path() {
    let (origin, task) = endpoint(
        "200 OK",
        r#"{"patch":"diff --git a/a b/a\n","truncated":false,"nextCursor":null}"#,
    );
    let r = json!({"projectId":"p","host":"github.com","repository":"acme/repo","number":7,"cursor":"next-slice"});
    let response = fetch(&origin, "fixture-token", None, r.clone())
        .await
        .unwrap();
    assert_eq!(response["truncated"], false);
    let request = task.join().unwrap();
    assert!(request.starts_with("POST /api/pull-requests/diff HTTP/1.1\r\n"));
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer fixture-token")
    );
    let payload: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(payload, r);
}
#[tokio::test]
async fn r14_p_diff_http_uses_dpop_and_denial_has_no_sensitive_body() {
    let (origin, task) = endpoint("403 Forbidden", r#"{"message":"sensitive-server-detail"}"#);
    let key = t3code_desktop_gpui::dpop::Key::generate();
    let error = fetch(
        &origin,
        "fixture-token",
        Some(&key),
        json!({"projectId":"p","repository":"acme/repo","number":7}),
    )
    .await
    .unwrap_err();
    assert_eq!(error, "PR diff request refused (HTTP 403)");
    let request = task.join().unwrap().to_lowercase();
    assert!(request.contains("authorization: dpop fixture-token"));
    assert!(request.contains("\r\ndpop: "));
}
