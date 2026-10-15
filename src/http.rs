//! Blocking HTTP for the source modules: one shared agent, a browser-like user agent, Korean
//! `Accept-Language`, and a generous body limit. Non-2xx responses are errors.
//!
//! Timeouts, connection failures, 429 and 5xx are retried after [`BACKOFF`] (a numeric
//! `Retry-After` up to [`MAX_RETRY_AFTER`] is honoured); other 4xx fail at once.

use std::sync::LazyLock;
use std::thread::sleep;
use std::time::Duration;

use serde_json::Value;
use ureq::Agent;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

use crate::error::{Error, Result};

pub const UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 (KHTML, like Gecko) contest-harness";
const BODY_LIMIT: u64 = 64 * 1024 * 1024;
/// Waits before the retries of one request; its length is the number of retries.
pub const BACKOFF: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(6)];
/// A longer `Retry-After` is not waited for: the request fails instead.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

fn builder() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).user_agent(UA).http_status_as_error(false)
}

static SHARED: LazyLock<Agent> = LazyLock::new(|| builder().build().into());

/// 429 and 5xx are worth another try; every other non-2xx status is final.
pub fn retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// Timeouts and connection-level failures are worth another try; TLS, URL and protocol errors are not.
pub fn retryable_error(err: &ureq::Error) -> bool {
    matches!(
        err,
        ureq::Error::Timeout(_) | ureq::Error::Io(_) | ureq::Error::ConnectionFailed | ureq::Error::HostNotFound
    )
}

/// How long to wait before retry number `retry` (0-based), or `None` to give up. A numeric
/// `Retry-After` (seconds) longer than the backoff is waited for, up to [`MAX_RETRY_AFTER`].
pub fn retry_delay(retry: usize, retry_after: Option<&str>) -> Option<Duration> {
    let backoff = *BACKOFF.get(retry)?;
    match retry_after.and_then(|value| value.trim().parse::<u64>().ok()).map(Duration::from_secs) {
        Some(asked) if asked > MAX_RETRY_AFTER => None,
        Some(asked) => Some(asked.max(backoff)),
        None => Some(backoff),
    }
}

/// An agent that additionally trusts `pem` — for servers that omit an intermediate certificate.
/// Certificate verification stays on.
pub fn agent_trusting(pem: &str) -> Result<Agent> {
    let cert =
        Certificate::from_pem(pem.as_bytes()).map_err(|err| Error(format!("인증서 PEM을 읽지 못했다: {err}")))?;
    let tls = TlsConfig::builder().root_certs(RootCerts::new_with_certs(&[cert.to_owned()])).build();
    Ok(builder().tls_config(tls).build().into())
}

/// A request: GET without a body, POST with one.
pub struct Request<'a> {
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: Option<&'a [u8]>,
    pub agent: Option<&'a Agent>,
}

/// A non-2xx answer: its status and `Retry-After` header.
type Refused = (u16, Option<String>);

impl<'a> Request<'a> {
    pub fn get(url: &'a str) -> Self {
        Request { url, headers: &[], body: None, agent: None }
    }

    /// The body of a 2xx answer, retrying transient failures (see the module docs).
    pub fn send(&self) -> Result<String> {
        let agent = self.agent.unwrap_or(&SHARED);
        let mut retry = 0;
        loop {
            let (failure, retry_after) = match self.attempt(agent) {
                Ok(Ok(body)) => return Ok(body),
                Ok(Err((status, retry_after))) if retryable_status(status) => (format!("HTTP {status}"), retry_after),
                Ok(Err((status, _))) => return Err(Error(format!("{}: HTTP {status}", self.url))),
                Err(err) if retryable_error(&err) => (err.to_string(), None),
                Err(err) => return Err(Error(format!("{}: {err}", self.url))),
            };
            let Some(delay) = retry_delay(retry, retry_after.as_deref()) else {
                return Err(Error(format!("{}: {failure} ({}회 시도)", self.url, retry + 1)));
            };
            sleep(delay);
            retry += 1;
        }
    }

    /// One request: the body of a 2xx answer, else the status and its `Retry-After`.
    fn attempt(&self, agent: &Agent) -> Result<Result<String, Refused>, ureq::Error> {
        let mut response = match self.body {
            None => {
                let mut request = agent.get(self.url).header("Accept-Language", "ko-KR,ko;q=0.9");
                for (name, value) in self.headers {
                    request = request.header(*name, *value);
                }
                request.call()?
            }
            Some(body) => {
                let mut request = agent.post(self.url).header("Accept-Language", "ko-KR,ko;q=0.9");
                for (name, value) in self.headers {
                    request = request.header(*name, *value);
                }
                request.send(body)?
            }
        };
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let retry_after = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(String::from);
            return Ok(Err((status, retry_after)));
        }
        Ok(Ok(response.body_mut().with_config().limit(BODY_LIMIT).read_to_string()?))
    }

    pub fn json(&self) -> Result<Value> {
        serde_json::from_str(&self.send()?).map_err(|err| Error(format!("{}: JSON이 아니다 ({err})", self.url)))
    }
}

pub fn get(url: &str) -> Result<String> {
    Request::get(url).send()
}

pub fn get_json(url: &str) -> Result<Value> {
    Request::get(url).json()
}

/// POST a JSON body and parse the JSON reply.
pub fn post_json(url: &str, payload: &Value, headers: &[(&str, &str)]) -> Result<Value> {
    let body = serde_json::to_vec(payload)?;
    let mut all = vec![("Content-Type", "application/json"), ("Accept", "application/json")];
    all.extend_from_slice(headers);
    Request { url, headers: &all, body: Some(&body), agent: None }.json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_rate_limits_and_server_errors_are_retried() {
        for status in [429, 500, 502, 503, 504, 599] {
            assert!(retryable_status(status), "{status}");
        }
        for status in [400, 401, 403, 404, 410, 422, 301, 304] {
            assert!(!retryable_status(status), "{status}");
        }
    }

    #[test]
    fn transport_failures_are_retried_but_not_tls_or_url_errors() {
        assert!(retryable_error(&ureq::Error::Timeout(ureq::Timeout::Global)));
        assert!(retryable_error(&ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset))));
        assert!(retryable_error(&ureq::Error::ConnectionFailed));
        assert!(!retryable_error(&ureq::Error::BadUri("x".into())));
        assert!(!retryable_error(&ureq::Error::TooManyRedirects));
        assert!(!retryable_error(&ureq::Error::StatusCode(404)));
    }

    #[test]
    fn retry_delay_backs_off_honours_retry_after_and_gives_up() {
        assert_eq!(retry_delay(0, None), Some(BACKOFF[0]));
        assert_eq!(retry_delay(1, None), Some(BACKOFF[1]));
        assert_eq!(retry_delay(BACKOFF.len(), None), None);
        assert_eq!(retry_delay(0, Some("20")), Some(Duration::from_secs(20)));
        assert_eq!(retry_delay(0, Some("30")), Some(MAX_RETRY_AFTER));
        assert_eq!(retry_delay(0, Some("31")), None);
        assert_eq!(retry_delay(1, Some("1")), Some(BACKOFF[1]));
        // An HTTP-date Retry-After is not parsed: the backoff applies.
        assert_eq!(retry_delay(0, Some("Wed, 21 Oct 2026 07:28:00 GMT")), Some(BACKOFF[0]));
    }
}
