//! Blocking HTTP for the source modules: one shared agent, a browser-like user agent, Korean
//! `Accept-Language`, and a generous body limit. Non-2xx responses are errors.

use std::sync::LazyLock;
use std::time::Duration;

use serde_json::Value;
use ureq::Agent;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

use crate::error::{Error, Result};

pub const UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 (KHTML, like Gecko) contest-harness";
const BODY_LIMIT: u64 = 64 * 1024 * 1024;

fn builder() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).user_agent(UA)
}

static SHARED: LazyLock<Agent> = LazyLock::new(|| builder().build().into());

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

impl<'a> Request<'a> {
    pub fn get(url: &'a str) -> Self {
        Request { url, headers: &[], body: None, agent: None }
    }

    pub fn send(&self) -> Result<String> {
        let agent = self.agent.unwrap_or(&SHARED);
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
        Ok(response.body_mut().with_config().limit(BODY_LIMIT).read_to_string()?)
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
