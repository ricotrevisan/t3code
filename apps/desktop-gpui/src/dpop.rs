//! Proofs bind a credential to this client's key and each HTTP request.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::elliptic_curve::rand_core::OsRng;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct Key(SigningKey);

impl Key {
    pub fn generate() -> Self {
        Self(SigningKey::random(&mut OsRng))
    }

    // This representation may only be stored with the credential in Keychain.
    pub fn keychain_value(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0.to_bytes())
    }

    pub fn from_keychain(value: &str) -> Result<Self, &'static str> {
        let bytes = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| "Invalid proof key")?;
        SigningKey::from_slice(&bytes)
            .map(Self)
            .map_err(|_| "Invalid proof key")
    }

    pub fn public_jwk(&self) -> Value {
        let point = self.0.verifying_key().to_encoded_point(false);
        json!({"kty":"EC","crv":"P-256","x":URL_SAFE_NO_PAD.encode(point.x().expect("uncompressed key has x")),"y":URL_SAFE_NO_PAD.encode(point.y().expect("uncompressed key has y"))})
    }

    pub fn proof(
        &self,
        method: &str,
        url: &str,
        token: Option<&str>,
    ) -> Result<String, &'static str> {
        let mut url = url::Url::parse(url).map_err(|_| "Invalid proof endpoint")?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err("Invalid proof endpoint");
        }
        url.set_query(None);
        url.set_fragment(None);
        let header = json!({"typ":"dpop+jwt","alg":"ES256","jwk":self.public_jwk()});
        let mut claims = json!({"htm":method.to_uppercase(),"htu":url.as_str(),"jti":uuid::Uuid::new_v4().to_string(),"iat":chrono::Utc::now().timestamp()});
        if let Some(token) = token {
            claims["ath"] = json!(URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes())));
        }
        let input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        let signature: Signature = self.0.sign(input.as_bytes());
        Ok(format!(
            "{input}.{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier;
    #[test]
    fn restored_key_signs_fresh_request_bound_proofs_without_exposing_private_key() {
        let key = Key::generate();
        let restored = Key::from_keychain(&key.keychain_value()).unwrap();
        assert_eq!(key.public_jwk(), restored.public_jwk());
        let first = key
            .proof(
                "post",
                "https://example.invalid/api/auth/websocket-ticket?ignored=1#ignored",
                Some("owned-token"),
            )
            .unwrap();
        let second = restored
            .proof(
                "POST",
                "https://example.invalid/api/auth/websocket-ticket",
                Some("owned-token"),
            )
            .unwrap();
        let parts: Vec<_> = first.split('.').collect();
        let header: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        let claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert!(header["jwk"].get("d").is_none());
        assert_eq!(claims["htm"], "POST");
        assert_eq!(
            claims["htu"],
            "https://example.invalid/api/auth/websocket-ticket"
        );
        assert_eq!(
            claims["ath"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"owned-token"))
        );
        assert_ne!(first, second);
        let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
        restored
            .0
            .verifying_key()
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
        assert!(
            restored
                .0
                .verifying_key()
                .verify(b"changed request", &signature)
                .is_err()
        );
    }
}
