//! The doorcode server's JSON API, as an administrator's token reaches it.

use std::time::Duration;

use serde_json::Value;
use ureq::http::{Request, Response};
use ureq::{Agent, Body};

/// How long any one API call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Server {
    agent: Agent,
    api: String,
    token: String,
}

impl Server {
    pub(crate) fn new(url: &str, token: &str) -> Self {
        let agent: Agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(CALL_TIMEOUT))
            .build()
            .into();
        Self {
            agent,
            api: format!("{}/api/v1", url.trim_end_matches('/')),
            token: token.to_owned(),
        }
    }

    pub(crate) fn get(&self, path: &str) -> Result<Value, String> {
        self.call("GET", path, None)
    }

    pub(crate) fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.call("POST", path, Some(body))
    }

    pub(crate) fn put(&self, path: &str, body: Option<&Value>) -> Result<Value, String> {
        self.call("PUT", path, body)
    }

    pub(crate) fn delete(&self, path: &str) -> Result<Value, String> {
        self.call("DELETE", path, None)
    }

    /// Calls `method` on `path` with `body` as JSON, and returns the JSON
    /// answer, null when it is empty; a status other than 2xx is an error
    /// that quotes the answer.
    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let answer = self.answer(method, path, body);
        answer.map_err(|error| format!("{method} {path}: {error}"))
    }

    fn answer(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let request = self.request(method, path, body);
        let request = request.map_err(|error| error.to_string())?;
        let response = self.agent.run(request);
        read(response.map_err(|error| error.to_string())?)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Request<String>, ureq::http::Error> {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.api))
            .header("Authorization", format!("Bearer {}", self.token));
        if body.is_some() {
            request = request.header("Content-Type", "application/json");
        }
        request.body(body.map(Value::to_string).unwrap_or_default())
    }
}

/// The JSON a response carries.
fn read(mut response: Response<Body>) -> Result<Value, String> {
    let status = response.status();
    let text = response.body_mut().read_to_string();
    let text = text.map_err(|error| error.to_string())?;
    answer(status.is_success(), &text).map_err(|why| format!("{status} {why}"))
}

/// The JSON a successful call answered with, or the answer as the reason a
/// call failed.
fn answer(success: bool, text: &str) -> Result<Value, String> {
    if !success {
        return Err(text.chars().take(300).collect());
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(text).map_err(|error| format!("answered no JSON: {error}"))
}
