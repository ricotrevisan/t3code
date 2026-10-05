//! OAuth token exchange uses the fork's form-encoded bootstrap grant, not a bearer pairing token.
//! Errors intentionally omit URLs, tokens, and server response bodies.
use crate::connection::ConnectionOptions;
use serde::Deserialize;
use std::time::Duration;
use url::Url;

pub struct Access {
    pub origin: String,
    pub token: String,
    pub proof_key: Option<crate::dpop::Key>,
}

impl Access {
    pub fn keychain_value(&self) -> String {
        match &self.proof_key {
            Some(key) => {
                serde_json::json!({"version":1,"token":self.token,"proof_key":key.keychain_value()})
                    .to_string()
            }
            None => self.token.clone(),
        }
    }

    pub fn from_keychain(origin: String, value: String) -> Result<Self, &'static str> {
        if value.starts_with('{') {
            let record: serde_json::Value =
                serde_json::from_str(&value).map_err(|_| "Invalid stored credential")?;
            if record["version"] != 1 {
                return Err("Unsupported stored credential");
            }
            let token = record["token"]
                .as_str()
                .filter(|token| !token.trim().is_empty())
                .ok_or("Invalid stored credential")?
                .to_owned();
            let proof_key = crate::dpop::Key::from_keychain(
                record["proof_key"]
                    .as_str()
                    .ok_or("Invalid stored proof key")?,
            )?;
            Ok(Self {
                origin,
                token,
                proof_key: Some(proof_key),
            })
        } else if !value.trim().is_empty() {
            Ok(Self {
                origin,
                token: value,
                proof_key: None,
            })
        } else {
            Err("Invalid stored credential")
        }
    }
}

pub fn origin(value: &str) -> Result<String, &'static str> {
    let options = ConnectionOptions::new(value, "validation-only".into())?;
    Ok(options.origin())
}

fn bootstrap(server: &str, credential: &str) -> Result<(String, String), &'static str> {
    let server = origin(server)?;
    let credential = credential.trim();
    if credential.is_empty() {
        return Err("Enter a pairing token or pairing URL");
    }
    if credential.contains("://") {
        let link = Url::parse(credential).map_err(|_| "Invalid pairing URL")?;
        if !link.username().is_empty()
            || link.password().is_some()
            || link.path() != "/pair"
            || link.query().is_some()
            || link.origin().ascii_serialization() != server
        {
            return Err("Pairing URL must belong to the selected server");
        }
        let fragment = link.fragment().ok_or("Pairing URL has no token")?;
        let values: Vec<_> = url::form_urlencoded::parse(fragment.as_bytes())
            .filter(|(key, _)| key == "token")
            .collect();
        if values.len() != 1 || values[0].1.trim().is_empty() {
            return Err("Pairing URL has no unique token");
        }
        return Ok((server, values[0].1.to_string()));
    }
    Ok((server, credential.into()))
}

#[derive(Deserialize)]
struct TokenResult {
    access_token: String,
    token_type: String,
    issued_token_type: String,
    expires_in: f64,
}

pub async fn exchange(server: &str, credential: &str) -> Result<Access, &'static str> {
    exchange_with_key(server, credential, None).await
}

pub async fn exchange_with_key(
    server: &str,
    credential: &str,
    proof_key: Option<crate::dpop::Key>,
) -> Result<Access, &'static str> {
    let (origin, token) = bootstrap(server, credential)?;
    let url = Url::parse(&origin)
        .map_err(|_| "Invalid server origin")?
        .join("oauth/token")
        .map_err(|_| "Invalid token endpoint")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not create authorization client")?;
    let mut request = client.post(url.clone()).form(&[
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:token-exchange",
        ),
        ("subject_token", token.as_str()),
        (
            "subject_token_type",
            "urn:t3:params:oauth:token-type:environment-bootstrap",
        ),
        (
            "requested_token_type",
            "urn:ietf:params:oauth:token-type:access_token",
        ),
        ("client_label", "T3 Code Native"),
        ("client_device_type", "desktop"),
        ("client_os", "macOS"),
    ]);
    if let Some(key) = &proof_key {
        request = request.header("DPoP", key.proof("POST", url.as_str(), None)?);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "Could not reach pairing endpoint")?;
    if !response.status().is_success() {
        return Err("Pairing rejected; token may be expired, consumed, or unauthorized");
    }
    let result: TokenResult = response
        .json()
        .await
        .map_err(|_| "Invalid pairing response")?;
    if result.token_type
        != if proof_key.is_some() {
            "DPoP"
        } else {
            "Bearer"
        }
        || result.issued_token_type != "urn:ietf:params:oauth:token-type:access_token"
        || result.access_token.trim().is_empty()
        || !result.expires_in.is_finite()
        || result.expires_in <= 0.
    {
        return Err("Unsupported or invalid access credential");
    }
    Ok(Access {
        origin,
        token: result.access_token,
        proof_key,
    })
}

pub fn pair(
    server: String,
    credential: String,
) -> async_channel::Receiver<Result<Access, &'static str>> {
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => {
                let _ = sender.send_blocking(Err("Could not start pairing runtime"));
                return;
            }
        };
        runtime.block_on(async move {
            tokio::select! {
                result=exchange_with_key(&server,&credential,Some(crate::dpop::Key::generate()))=>{ let _=sender.send(result).await; }
                _=sender.closed()=>{}
            }
        });
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keychain_restore_preserves_proof_binding_and_legacy_bearer_credentials() {
        let access = Access {
            origin: "https://owned.invalid".into(),
            token: "owned-access".into(),
            proof_key: Some(crate::dpop::Key::generate()),
        };
        let restored =
            Access::from_keychain(access.origin.clone(), access.keychain_value()).unwrap();
        assert_eq!(access.token, restored.token);
        assert_eq!(
            access.proof_key.unwrap().public_jwk(),
            restored.proof_key.unwrap().public_jwk()
        );
        let legacy =
            Access::from_keychain("https://owned.invalid".into(), "owned-bearer".into()).unwrap();
        assert!(legacy.proof_key.is_none());
        assert!(
            Access::from_keychain(
                "https://owned.invalid".into(),
                r#"{"version":2,"token":"owned"}"#.into()
            )
            .is_err()
        );
        assert!(
            Access::from_keychain(
                "https://owned.invalid".into(),
                r#"{"version":1,"token":"owned","proof_key":"invalid"}"#.into()
            )
            .is_err()
        );
    }
    #[test]
    fn pairing_link_cannot_send_token_to_a_different_server() {
        assert!(bootstrap("https://host", "https://other/pair#token=secret").is_err());
        assert!(bootstrap("https://host", "https://host/pair#token=a&token=b").is_err());
        assert!(bootstrap("https://host", "https://user:password@host/pair#token=a").is_err());
        assert_eq!(
            bootstrap("https://host", "https://host/pair#token=encoded%20token").unwrap(),
            ("https://host".into(), "encoded token".into())
        );
        assert_eq!(
            bootstrap("http://localhost:3777", " bare-token ").unwrap(),
            ("http://localhost:3777".into(), "bare-token".into())
        );
    }
    #[tokio::test]
    async fn actual_exchange_is_form_encoded_and_returns_bearer_access_not_bootstrap() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).await.unwrap();
                header.push(byte[0]);
                assert!(header.len() < 8192);
            }
            let header = String::from_utf8(header).unwrap().to_lowercase();
            assert!(header.starts_with("post /oauth/token "));
            assert!(header.contains("content-type: application/x-www-form-urlencoded"));
            assert!(!header.contains("authorization:"));
            let length: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            let fields: std::collections::HashMap<_, _> =
                url::form_urlencoded::parse(&body).into_owned().collect();
            assert_eq!(fields["subject_token"], "one-time-bootstrap");
            assert_eq!(
                fields["subject_token_type"],
                "urn:t3:params:oauth:token-type:environment-bootstrap"
            );
            assert_eq!(fields["client_os"], "macOS");
            let response = r#"{"access_token":"access-only","token_type":"Bearer","issued_token_type":"urn:ietf:params:oauth:token-type:access_token","expires_in":3600,"scope":"test"}"#;
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
        });
        let result = exchange(&format!("http://{address}"), "one-time-bootstrap")
            .await
            .unwrap();
        assert_eq!(result.token, "access-only");
        server.await.unwrap();
    }
}
