//! The diff contract is HTTP-only (environmentHttp.ts), with the same DPoP binding as WS auth.
use serde_json::Value;
use std::time::Duration;
use t3code_desktop_gpui::{auth, dpop};

pub fn diff(
    executor: &gpui_kit::BackgroundExecutor,
    access: auth::Access,
    payload: Value,
) -> gpui_kit::Task<Result<Value, String>> {
    executor.spawn(async move {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "Could not start diff request".to_owned())
            .and_then(|rt| {
                rt.block_on(fetch(
                    &access.origin,
                    &access.token,
                    access.proof_key.as_ref(),
                    payload,
                ))
            })
    })
}
pub(super) async fn fetch(
    origin: &str,
    token: &str,
    key: Option<&dpop::Key>,
    payload: Value,
) -> Result<Value, String> {
    let endpoint = format!("{}/api/pull-requests/diff", origin.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not create diff client")?;
    let request = client.post(&endpoint).json(&payload);
    let request = if let Some(key) = key {
        request
            .header("Authorization", format!("DPoP {token}"))
            .header("DPoP", key.proof("POST", &endpoint, Some(token))?)
    } else {
        request.bearer_auth(token)
    };
    let mut response = request
        .send()
        .await
        .map_err(|_| "Could not read the PR diff")?;
    if !response.status().is_success() {
        return Err(format!(
            "PR diff request refused (HTTP {})",
            response.status().as_u16()
        ));
    }
    // A paged response is bounded even if a faulty host ignores its own limits.
    const MAX: usize = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "PR diff download interrupted")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX {
            return Err("PR diff slice exceeds 16 MiB; read it on the host".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid PR diff response".into())
}
