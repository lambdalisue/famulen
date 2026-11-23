//! The HTTP receivers: one address a service sends to for each receiver, and
//! the checks a definition names for what comes there.

use std::collections::BTreeMap;

use hmac::{KeyInit, Mac};
use jsonwebtoken::jwk::JwkSet;
use serde_json::Value;
use sha2::Sha256;

use super::super::Gateway;
use super::Arrival;
use crate::connections::Connection;
use crate::definitions::{Definition, Hmac as HmacVerify, HttpReceive, JwtVerify, Verify, seconds};
use crate::store::now;
use crate::template::Context;

/// How long an issuer is not asked again for a key it did not publish.
const KEYS_AGAIN_SECS: i64 = 60;

impl Gateway {
    /// The HTTP receiver `receiver` of the connection `id`, when its
    /// definition opens one and it is sent somewhere.
    fn http_receiver(
        &self,
        id: &str,
        receiver: &str,
    ) -> anyhow::Result<Option<(Connection, Definition, HttpReceive)>> {
        let Some(connection) = self.sealed().get(id)? else {
            return Ok(None);
        };
        if self.destination_of(id, receiver)?.is_none() {
            return Ok(None);
        }
        let Some(Ok(def)) = self.definition_of(&connection) else {
            return Ok(None);
        };
        let Some(http) = def.receive.get(receiver).and_then(|r| r.http.clone()) else {
            return Ok(None);
        };
        Ok(Some((connection, def, http)))
    }

    /// Take one request to the HTTP receiver `receiver` of `connection`:
    /// the status and body to answer with.
    pub(in crate::gateway) async fn receive_http(
        &self,
        connection: &str,
        receiver: &str,
        headers: BTreeMap<String, String>,
        body: &str,
    ) -> anyhow::Result<(u16, String)> {
        let Some((c, def, http)) = self.http_receiver(connection, receiver)? else {
            return Ok((404, String::new()));
        };
        let payload: Value =
            serde_json::from_str(body).unwrap_or_else(|_| Value::String(body.to_owned()));
        let mut ctx = self.context(&c, &def, Value::Null, true)?;
        ctx.headers = headers.clone();
        ctx.body = Some(body.to_owned());
        ctx.doc = payload.clone();
        if !self.verify(&http.verify, &ctx).await? {
            self.log_receipt(connection, receiver, &payload, false)?;
            return Ok((401, String::new()));
        }
        if let Some(challenge) = &http.challenge
            && let Some(answer) = ctx.render_text(challenge)
        {
            return Ok((200, answer));
        }
        let shared = http
            .verify
            .token
            .as_ref()
            .and_then(|t| ctx.render_text(&t.equals))
            .filter(|s| !s.is_empty());
        let headers = headers
            .into_iter()
            .filter(|(_, v)| shared.as_deref().is_none_or(|s| !v.contains(s)))
            .collect();
        self.deliver(Arrival {
            headers,
            ..Arrival::new(connection, receiver, payload)
        })?;
        Ok((200, String::new()))
    }

    /// Serve the HTTP receivers at `listen` until stopped.
    pub fn serve_receivers(
        &self,
        listen: std::net::SocketAddr,
    ) -> futures_util::future::BoxFuture<'static, anyhow::Result<()>> {
        let gateway = self.clone();
        Box::pin(async move {
            async fn take(
                axum::extract::State(gateway): axum::extract::State<Gateway>,
                axum::extract::Path((connection, receiver)): axum::extract::Path<(String, String)>,
                headers: axum::http::HeaderMap,
                body: String,
            ) -> (axum::http::StatusCode, String) {
                let headers = headers
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
                    .collect();
                match gateway
                    .receive_http(&connection, &receiver, headers, &body)
                    .await
                {
                    Ok((status, text)) => (
                        axum::http::StatusCode::from_u16(status)
                            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
                        text,
                    ),
                    Err(error) => {
                        tracing::debug!(%connection, %receiver, %error, "could not take what arrived");
                        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, String::new())
                    }
                }
            }
            let app = axum::Router::new()
                .route(
                    "/receive/{connection}/{receiver}",
                    axum::routing::post(take),
                )
                .with_state(gateway);
            let listener = tokio::net::TcpListener::bind(listen).await?;
            axum::serve(listener, app).await?;
            Ok(())
        })
    }

    /// Whether what came to an HTTP receiver passes its definition's check.
    async fn verify(&self, verify: &Verify, ctx: &Context) -> anyhow::Result<bool> {
        if let Some(hmac) = &verify.hmac_sha256 {
            return Ok(hmac_ok(hmac, ctx));
        }
        if let Some(token) = &verify.token {
            return Ok(
                match (
                    ctx.render_text(&token.value),
                    ctx.render_text(&token.equals),
                ) {
                    (Some(got), Some(want)) => same(got.as_bytes(), want.as_bytes()),
                    _ => false,
                },
            );
        }
        if let Some(jwt) = &verify.jwt {
            return Ok(self.jwt_ok(jwt, ctx).await);
        }
        Ok(false)
    }

    /// Whether a JWT is signed by one of its issuer's keys, for the issuer
    /// and the audience a definition names.
    async fn jwt_ok(&self, jwt: &JwtVerify, ctx: &Context) -> bool {
        let (Some(value), Some(issuer), Some(audience)) = (
            ctx.render_text(&jwt.value),
            ctx.render_text(&jwt.issuer),
            ctx.render_text(&jwt.audience),
        ) else {
            return false;
        };
        let token = value.strip_prefix("Bearer ").unwrap_or(&value);
        let Ok(header) = jsonwebtoken::decode_header(token) else {
            return false;
        };
        let Some(kid) = header.kid.as_deref() else {
            return false;
        };
        let Ok(keys) = self.issuer_keys(&issuer, kid).await else {
            return false;
        };
        let Some(Ok(key)) = keys.find(kid).map(jsonwebtoken::DecodingKey::from_jwk) else {
            return false;
        };
        let mut validation = jsonwebtoken::Validation::new(header.alg);
        validation.set_issuer(&[issuer]);
        validation.set_audience(&[audience]);
        jsonwebtoken::decode::<Value>(token, &key, &validation).is_ok()
    }

    /// The keys an issuer publishes (its OpenID configuration's `jwks_uri`),
    /// kept once fetched. A key that is not among them has them fetched
    /// again, since issuers change their keys, but not for every token.
    pub(super) async fn issuer_keys(&self, issuer: &str, kid: &str) -> anyhow::Result<JwkSet> {
        if let Some((at, keys)) = crate::lock::read(&self.jwks).get(issuer)
            && (keys.find(kid).is_some() || now() - at < KEYS_AGAIN_SECS)
        {
            return Ok(keys.clone());
        }
        let config: Value = self
            .http
            .get(format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            ))
            .send()
            .await?
            .json()
            .await?;
        let uri = config["jwks_uri"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("{issuer} publishes no jwks_uri"))?;
        let keys: JwkSet = self.http.get(uri).send().await?.json().await?;
        crate::lock::write(&self.jwks).insert(issuer.to_owned(), (now(), keys.clone()));
        Ok(keys)
    }

    /// Keep `keys` as what `issuer` published long ago.
    #[cfg(test)]
    pub(super) fn set_issuer_keys(&self, issuer: &str, keys: JwkSet) {
        crate::lock::write(&self.jwks).insert(issuer.to_owned(), (0, keys));
    }
}

/// Whether the signature is the HMAC a definition describes, and not too old.
fn hmac_ok(check: &HmacVerify, ctx: &Context) -> bool {
    let (Some(secret), Some(base), Some(signature)) = (
        ctx.render_text(&check.secret),
        ctx.render_text(&check.base),
        ctx.render_text(&check.signature),
    ) else {
        return false;
    };
    if let (Some(at), Some(age)) = (&check.timestamp, &check.max_age) {
        let at: i64 = ctx
            .render_text(at)
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        if now().abs_diff(at) > seconds(age).unwrap_or(0) {
            return false;
        }
    }
    let Some(expected) = signature
        .strip_prefix(check.prefix.as_str())
        .and_then(decode_hex)
    else {
        return false;
    };
    let Ok(mut mac) = hmac::Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(base.as_bytes());
    mac.verify_slice(&expected).is_ok()
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Compare without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
