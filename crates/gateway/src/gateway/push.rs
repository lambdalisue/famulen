//! Web Push: a message encrypted for one browser of the owner's and handed to
//! the push service that browser named (RFC 8030, 8188, 8291 and 8292). The
//! key that signs for this gateway is kept in the vault.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aes_gcm::{
    Aes128Gcm, KeyInit,
    aead::{Aead, Payload},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use hkdf::Hkdf;
use p256::{
    PublicKey, SecretKey,
    ecdsa::{Signature, SigningKey, signature::Signer},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;

use super::Gateway;
use crate::lock::lock;
use crate::log::{LogEntry, digest};
use crate::store::now;

/// What the log names the push services by.
const WEB_PUSH: &str = "web-push";
/// The vault entry of the key that signs.
const KEY: &str = "web-push:key";
/// Push services take 4096 bytes; the header, the tag and the delimiter are
/// part of them (RFC 8291, section 4).
const RECORD: u32 = 4096;
const MAX_PLAIN: usize = RECORD as usize - 86 - 16 - 1;
/// RFC 8292 allows a day; Apple asks not to sign more than once an hour.
const TOKEN_LIVES: i64 = 12 * 60 * 60;
const TOKEN_REUSED: i64 = 6 * 60 * 60;
/// Who a push service may contact about the messages. Apple refuses a token
/// without one; nothing of the owner's is told.
const CONTACT: &str = "https://github.com/lambdalisue/famulen";

/// Whether a vault entry is the gateway's own for Web Push, or would take its
/// file: the vault writes a colon as a hyphen.
pub(super) fn is_push_entry(name: &str) -> bool {
    name.replace(':', "-") == KEY.replace(':', "-")
}

/// Where one browser takes messages, as it gave it on subscribing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// What became of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pushed {
    Taken,
    /// The browser takes no more messages there.
    Gone,
}

#[derive(Clone)]
pub(super) struct Push {
    http: reqwest::Client,
    /// The token signed for each push service, and when.
    tokens: Arc<Mutex<HashMap<String, (i64, String)>>>,
    /// Held while the key is read or made, so that only one is made.
    key: Arc<Mutex<()>>,
    /// Set by tests alone, whose push service is on this machine.
    anywhere: bool,
}

impl Default for Push {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                // A redirect would lead past the check of the address.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
                .build()
                .expect("a plain HTTP client builds"),
            tokens: Arc::default(),
            key: Arc::default(),
            anywhere: false,
        }
    }
}

impl Subscription {
    /// Refuse an address no push service would have given, such as one of the
    /// owner's own network.
    pub fn check(&self) -> anyhow::Result<()> {
        let url = url::Url::parse(&self.endpoint)?;
        anyhow::ensure!(
            url.scheme() == "https",
            "a push service is reached by https"
        );
        let public = match url.host() {
            Some(url::Host::Domain(name)) => {
                let name = name.trim_end_matches('.').to_ascii_lowercase();
                name.contains('.')
                    && ![".local", ".localhost", ".internal", ".ts.net", ".home.arpa"]
                        .iter()
                        .any(|end| name.ends_with(end))
            }
            _ => false,
        };
        anyhow::ensure!(public, "a push service has a public name");
        self.keys().map(drop)
    }

    fn keys(&self) -> anyhow::Result<(PublicKey, Vec<u8>)> {
        let public = PublicKey::from_sec1_bytes(&decode(&self.p256dh)?)
            .map_err(|_| anyhow::anyhow!("not a subscription's key"))?;
        let auth = decode(&self.auth)?;
        anyhow::ensure!(auth.len() == 16, "not a subscription's secret");
        Ok((public, auth))
    }
}

fn decode(text: &str) -> anyhow::Result<Vec<u8>> {
    Ok(B64.decode(text.trim_end_matches('='))?)
}

fn new_key() -> SecretKey {
    loop {
        // Not every 32 bytes are a key of the curve, though nearly all are.
        if let Ok(key) = SecretKey::from_slice(&rand::random::<[u8; 32]>()) {
            return key;
        }
    }
}

fn expand<const N: usize>(salt: &[u8], secret: &[u8], info: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    Hkdf::<Sha256>::new(Some(salt), secret)
        .expand(info, &mut out)
        .expect("the lengths asked for are short");
    out
}

fn encrypt(
    plain: &[u8],
    to: &Subscription,
    from: &SecretKey,
    salt: [u8; 16],
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(plain.len() <= MAX_PLAIN, "too long for a push message");
    let (public, auth) = to.keys()?;
    let own = from.public_key().to_sec1_bytes();
    let shared = p256::ecdh::diffie_hellman(from.to_nonzero_scalar(), public.as_affine());
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(&public.to_sec1_bytes());
    info.extend_from_slice(&own);
    let ikm: [u8; 32] = expand(&auth, shared.raw_secret_bytes(), &info);
    let key: [u8; 16] = expand(&salt, &ikm, b"Content-Encoding: aes128gcm\0");
    let nonce: [u8; 12] = expand(&salt, &ikm, b"Content-Encoding: nonce\0");
    let mut padded = plain.to_vec();
    padded.push(2);
    let sealed = Aes128Gcm::new(&key.into())
        .encrypt(&nonce.into(), Payload::from(padded.as_slice()))
        .map_err(|_| anyhow::anyhow!("could not encrypt a push message"))?;
    let mut body = salt.to_vec();
    body.extend_from_slice(&RECORD.to_be_bytes());
    body.push(own.len() as u8);
    body.extend_from_slice(&own);
    body.extend_from_slice(&sealed);
    Ok(body)
}

fn token(key: &SecretKey, audience: &str, at: i64) -> String {
    let header = json!({ "typ": "JWT", "alg": "ES256" }).to_string();
    let claims = json!({ "aud": audience, "exp": at + TOKEN_LIVES, "sub": CONTACT }).to_string();
    let signed = format!("{}.{}", B64.encode(header), B64.encode(claims));
    let signature: Signature = SigningKey::from(key).sign(signed.as_bytes());
    format!("{signed}.{}", B64.encode(signature.to_bytes()))
}

impl Gateway {
    /// The public key browsers subscribe with, made on first use.
    pub fn push_key(&self) -> anyhow::Result<String> {
        Ok(B64.encode(self.push_secret()?.public_key().to_sec1_bytes()))
    }

    fn push_secret(&self) -> anyhow::Result<SecretKey> {
        let vault = self
            .vault
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("there is no vault: set {}", super::KEY_VAR))?;
        let _one = lock(&self.push.key);
        if let Some(kept) = vault.reveal(KEY)? {
            return SecretKey::from_slice(&decode(&kept)?)
                .map_err(|_| anyhow::anyhow!("the key of Web Push does not read"));
        }
        let key = new_key();
        vault.put(KEY, B64.encode(key.to_bytes()), "web push", super::GATEWAY)?;
        Ok(key)
    }

    /// Send `message` to the browser of `to`, which waits for it at most
    /// `ttl` while it is away.
    pub async fn push(
        &self,
        actor: impl AsRef<str>,
        to: &Subscription,
        message: &[u8],
        ttl: Duration,
    ) -> anyhow::Result<Pushed> {
        if !self.push.anywhere {
            to.check()?;
        }
        let secret = self.push_secret()?;
        let audience = url::Url::parse(&to.endpoint)?
            .origin()
            .ascii_serialization();
        let public = B64.encode(secret.public_key().to_sec1_bytes());
        let body = encrypt(message, to, &new_key(), rand::random())?;
        let sent = digest(&body);
        let started = Instant::now();
        let response = self
            .push
            .http
            .post(&to.endpoint)
            .header("ttl", ttl.as_secs())
            .header("urgency", "high")
            .header("content-encoding", "aes128gcm")
            .header("content-type", "application/octet-stream")
            .header(
                "authorization",
                format!(
                    "vapid t={}, k={public}",
                    self.push_token(&secret, &audience)
                ),
            )
            .body(body)
            .send()
            .await
            // The address is what lets anyone send to the browser.
            .map_err(reqwest::Error::without_url);
        let outcome = match &response {
            Ok(r) => r.status().as_u16().to_string(),
            Err(e) => format!("error: {e}"),
        };
        self.log().write(&LogEntry {
            actor: actor.as_ref().to_owned(),
            connection: WEB_PUSH.into(),
            tool: "push.send".into(),
            decision: "allow".into(),
            secret: KEY.into(),
            outcome,
            digest: sent,
            ms: started.elapsed().as_millis() as i64,
            ..LogEntry::default()
        })?;
        let status = response?.status();
        match status.as_u16() {
            404 | 410 => Ok(Pushed::Gone),
            _ if status.is_success() => Ok(Pushed::Taken),
            _ => anyhow::bail!("the push service answered {status}"),
        }
    }

    fn push_token(&self, key: &SecretKey, audience: &str) -> String {
        let at = now();
        let mut tokens = lock(&self.push.tokens);
        if let Some((signed, token)) = tokens.get(audience)
            && at - signed < TOKEN_REUSED
        {
            return token.clone();
        }
        let token = token(key, audience, at);
        tokens.insert(audience.to_owned(), (at, token.clone()));
        token
    }

    /// Send to a push service on this machine too.
    #[cfg(test)]
    fn pushing_anywhere(mut self) -> Self {
        self.push.anywhere = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Bytes, extract::State, http::HeaderMap, http::StatusCode};
    use p256::ecdsa::{VerifyingKey, signature::Verifier};

    use super::super::vault::{Vault, VaultKey};
    use super::*;
    use crate::connections::Connections;
    use crate::store::Store;

    const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";

    fn subscription(endpoint: &str) -> Subscription {
        Subscription {
            endpoint: endpoint.into(),
            p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".into(),
            auth: "BTBZMqHH6r4Tts7J_aSIgg".into(),
        }
    }

    fn gateway() -> (Gateway, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("famulen-push-{:x}", rand::random::<u64>()));
        let key = VaultKey::parse(VaultKey::generate()).unwrap();
        let store = Store::open_in_memory().unwrap();
        let gateway = Gateway::new(
            store.clone(),
            Connections::new(store),
            reqwest::Client::new(),
            Some(Vault::open(&dir, &key).unwrap()),
        );
        (gateway, dir)
    }

    type Seen = Arc<Mutex<Vec<(HeaderMap, Vec<u8>)>>>;

    async fn service(status: StatusCode) -> (String, Seen) {
        let seen = Seen::default();
        let app = Router::new()
            .fallback(
                move |State(seen): State<Seen>, headers: HeaderMap, body: Bytes| async move {
                    lock(&seen).push((headers, body.to_vec()));
                    (status, [("location", "http://127.0.0.1:1/")])
                },
            )
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = format!("http://{}/to", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (at, seen)
    }

    fn opened(body: &[u8]) -> Vec<u8> {
        let to = subscription("");
        let (salt, sender, sealed) = (&body[..16], &body[21..86], &body[86..]);
        let secret = SecretKey::from_slice(&decode(UA_PRIVATE).unwrap()).unwrap();
        let sender = PublicKey::from_sec1_bytes(sender).unwrap();
        let shared = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), sender.as_affine());
        let info = [
            b"WebPush: info\0".as_slice(),
            &secret.public_key().to_sec1_bytes(),
            &sender.to_sec1_bytes(),
        ]
        .concat();
        let ikm: [u8; 32] = expand(&to.keys().unwrap().1, shared.raw_secret_bytes(), &info);
        let key: [u8; 16] = expand(salt, &ikm, b"Content-Encoding: aes128gcm\0");
        let nonce: [u8; 12] = expand(salt, &ikm, b"Content-Encoding: nonce\0");
        let mut plain = Aes128Gcm::new(&key.into())
            .decrypt(&nonce.into(), sealed)
            .unwrap();
        assert_eq!(plain.pop(), Some(2));
        plain
    }

    #[test]
    fn a_message_is_encrypted_as_the_example_of_rfc_8291() {
        let from =
            SecretKey::from_slice(&decode("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw").unwrap())
                .unwrap();
        let salt = decode("DGv6ra1nlYgDCS1FRnbzlw").unwrap();
        let body = encrypt(
            b"When I grow up, I want to be a watermelon",
            &subscription(""),
            &from,
            salt.try_into().unwrap(),
        )
        .unwrap();
        assert_eq!(
            B64.encode(body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    #[test]
    fn a_message_too_long_for_push_services_is_refused() {
        let at = |len| encrypt(&vec![b'a'; len], &subscription(""), &new_key(), [0; 16]);
        assert_eq!(at(MAX_PLAIN).unwrap().len(), 4096);
        assert!(at(MAX_PLAIN + 1).is_err());
    }

    #[tokio::test]
    async fn a_message_reaches_the_push_service_encrypted_and_signed() {
        let (g, dir) = gateway();
        let g = g.pushing_anywhere();
        let (at, seen) = service(StatusCode::CREATED).await;
        let pushed = g
            .push(
                "secretary",
                &subscription(&at),
                b"hello",
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        assert_eq!(pushed, Pushed::Taken);
        let (headers, body) = lock(&seen)[0].clone();
        assert_eq!(headers["ttl"], "60");
        assert_eq!(headers["urgency"], "high");
        assert_eq!(headers["content-encoding"], "aes128gcm");
        assert_eq!(opened(&body), b"hello");

        let authorization = headers["authorization"].to_str().unwrap();
        let (token, key) = authorization
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        assert_eq!(key, g.push_key().unwrap());
        let (signed, signature) = token.rsplit_once('.').unwrap();
        VerifyingKey::from_sec1_bytes(&decode(key).unwrap())
            .unwrap()
            .verify(
                signed.as_bytes(),
                &Signature::from_slice(&decode(signature).unwrap()).unwrap(),
            )
            .unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&decode(signed.split_once('.').unwrap().1).unwrap()).unwrap();
        assert_eq!(claims["aud"], at.trim_end_matches("/to"));
        let left = claims["exp"].as_i64().unwrap() - now();
        assert!((1..=24 * 60 * 60).contains(&left), "{left}");
        assert!(claims["sub"].as_str().unwrap().starts_with("https://"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_message_is_logged_without_its_words_or_where_it_went() {
        let (g, dir) = gateway();
        let g = g.pushing_anywhere();
        let (at, _) = service(StatusCode::CREATED).await;
        let ttl = Duration::from_secs(60);
        g.push("secretary", &subscription(&at), b"hello", ttl)
            .await
            .unwrap();
        let nowhere = subscription("http://127.0.0.1:1/to");
        let error = g
            .push("secretary", &nowhere, b"hello", ttl)
            .await
            .unwrap_err();
        assert!(!format!("{error:#}").contains("/to"), "{error:#}");
        let logged = g.log_entries(10).unwrap();
        assert_eq!(logged[1].actor, "secretary");
        assert_eq!(logged[1].connection, WEB_PUSH);
        assert_eq!(logged[1].outcome, "201");
        let written = serde_json::to_string(&logged).unwrap();
        assert!(
            !written.contains("hello") && !written.contains("/to"),
            "{written}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_browser_that_takes_no_more_is_told_from_a_service_that_fails() {
        let (g, dir) = gateway();
        let g = g.pushing_anywhere();
        let ttl = Duration::from_secs(60);
        for (status, gone) in [
            (StatusCode::GONE, true),
            (StatusCode::NOT_FOUND, true),
            (StatusCode::FORBIDDEN, false),
            (StatusCode::SERVICE_UNAVAILABLE, false),
            (StatusCode::TEMPORARY_REDIRECT, false),
        ] {
            let (at, _) = service(status).await;
            let pushed = g.push("owner", &subscription(&at), b"x", ttl).await;
            assert_eq!(matches!(pushed, Ok(Pushed::Gone)), gone, "{status}");
            assert_eq!(pushed.is_err(), !gone, "{status}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_a_public_https_address_is_a_push_service() {
        for ok in [
            "https://web.push.apple.com/Qabc",
            "https://fcm.googleapis.com/fcm/send/abc",
            "https://updates.push.services.mozilla.com/wpush/v2/abc",
        ] {
            subscription(ok).check().unwrap();
        }
        for bad in [
            "http://web.push.apple.com/Qabc",
            "https://127.0.0.1/x",
            "https://[::1]/x",
            "https://192.168.1.1/x",
            "https://2130706433/x",
            "https://localhost/x",
            "https://printer.local/x",
            "https://machine.tail1234.ts.net/x",
            "https://nas/x",
            "nonsense",
        ] {
            assert!(subscription(bad).check().is_err(), "{bad}");
        }
        let mut broken = subscription("https://web.push.apple.com/Qabc");
        broken.auth = "AAAA".into();
        assert!(broken.check().is_err());
    }

    #[test]
    fn the_key_stays_the_same_and_out_of_everyones_reach() {
        let (g, dir) = gateway();
        let key = g.push_key().unwrap();
        assert_eq!(g.push_key().unwrap(), key);
        assert!(g.secrets().unwrap().is_empty());
        assert!(g.put_secret("secretary", KEY, "x", "").is_err());
        assert!(g.put_secret("secretary", "web-push-key", "x", "").is_err());
        assert!(g.describe_secret("secretary", KEY, "x").is_err());
        g.set_vault_permission(
            "owner",
            super::super::DELETE_SECRET,
            Some(crate::permissions::Permission::Allow),
        )
        .unwrap();
        assert!(g.delete_secret("secretary", KEY).is_err());
        assert_eq!(g.push_key().unwrap(), key);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
