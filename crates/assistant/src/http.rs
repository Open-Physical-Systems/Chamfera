//! The HTTPS transport: a blocking POST to the Messages API, read as a stream of lines.
//! It runs on the assistant's worker thread, never on the UI thread.
//!
//! `ureq` (MIT or Apache-2.0) on `rustls` with the `ring` provider; certificates are
//! checked against the operating system's root certificates (`rustls-native-certs`). The
//! key goes into the `x-api-key` header and nowhere else: not into errors, logs or the
//! request body.

use crate::agent::{Transport, TransportError};
use crate::api::{API_URL, API_VERSION};
use crate::key::ApiKey;
use serde_json::Value;
use std::io::{BufRead, Read};
use std::sync::Arc;
use std::time::Duration;

/// Most bytes read from one streamed reply.
const MAX_STREAM_BYTES: u64 = 256 * 1024 * 1024;
/// Most bytes read from an error reply.
const MAX_ERROR_BYTES: u64 = 64 * 1024;
/// Longest line of a stream (bytes).
const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

pub struct HttpTransport {
    agent: ureq::Agent,
    key: ApiKey,
    url: String,
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("url", &self.url)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// The operating system's trusted root certificates. Certificates it cannot parse are
/// skipped; with none at all, every connection fails its certificate check.
fn system_roots() -> ureq::tls::RootCerts {
    let found = rustls_native_certs::load_native_certs();
    let certs: Vec<ureq::tls::Certificate<'static>> = found
        .certs
        .iter()
        .map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned())
        .collect();
    ureq::tls::RootCerts::new_with_certs(&certs)
}

impl HttpTransport {
    pub fn new(key: ApiKey) -> HttpTransport {
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::Rustls)
            .root_certs(system_roots())
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(300)))
            .user_agent(concat!("chamfera/", env!("CARGO_PKG_VERSION")))
            .build();
        HttpTransport {
            agent: config.into(),
            key,
            url: API_URL.to_owned(),
        }
    }
}

/// A transport error's text, with anything that could hold the key removed.
fn network(e: impl std::fmt::Display, key: &ApiKey) -> TransportError {
    let text = e.to_string().replace(key.expose(), "[key]");
    TransportError::Network(format!("could not reach the Claude API: {text}"))
}

impl Transport for HttpTransport {
    fn send(
        &mut self,
        body: &Value,
        line: &mut dyn FnMut(&str) -> bool,
    ) -> Result<(), TransportError> {
        let response = self
            .agent
            .post(&self.url)
            .header("x-api-key", self.key.expose())
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .send(body.to_string())
            .map_err(|e| network(e, &self.key))?;
        let status = response.status().as_u16();
        let mut body = response.into_body();
        if status != 200 {
            let text = body
                .with_config()
                .limit(MAX_ERROR_BYTES)
                .lossy_utf8(true)
                .read_to_string()
                .unwrap_or_default();
            return Err(TransportError::Http { status, body: text });
        }
        let mut reader =
            std::io::BufReader::new(body.with_config().limit(MAX_STREAM_BYTES).reader());
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = (&mut reader)
                .take(MAX_LINE_BYTES as u64 + 1)
                .read_until(b'\n', &mut buf)
                .map_err(|e| network(e, &self.key))?;
            if n == 0 {
                return Ok(());
            }
            if buf.len() > MAX_LINE_BYTES {
                return Err(TransportError::Network(
                    "the stream sent a line that is too long".into(),
                ));
            }
            let text = String::from_utf8_lossy(&buf);
            if !line(&text) {
                return Ok(());
            }
        }
    }
}
