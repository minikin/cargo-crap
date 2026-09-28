//! The HTTP client for the `TypeSafe` evaluation endpoint.
//!
//! Blocking, one request per pair. Transient failures — a connection that
//! failed or dropped, a 429, any 5xx (the API's own 529 "overloaded"
//! included) — are retried a bounded number of times with a doubling
//! back-off; any other 4xx is final, since the same request will be refused
//! the same way.

use crate::duplicates::triage::verdict::DecodeError;
use std::fmt;
use std::io;
use std::time::Duration;

/// Where the API key is read from — the only place it is read from.
pub const API_KEY_VAR: &str = "TYPESAFE_API_KEY";
/// Overrides the API's base URL (tests point it at a local stub).
pub const BASE_URL_VAR: &str = "TYPESAFE_BASE_URL";
/// The API's base URL, absent an override.
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";

const ENDPOINT_PATH: &str = "/v1/systemone";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_ATTEMPTS: u32 = 3;
const DEFAULT_BACKOFF: Duration = Duration::from_millis(500);
/// The longest `Retry-After` honoured; a longer ask waits this long, then
/// the attempt count decides.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

/// Everything a triage run needs to reach the API.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The model every request names.
    pub model: String,
    /// The API's base URL, without the endpoint path.
    pub base_url: String,
    /// The bearer token; `None` when `TYPESAFE_API_KEY` is unset or empty.
    pub api_key: Option<String>,
    /// Limit on each request, connection to last byte.
    pub timeout: Duration,
    /// Limit on establishing the connection alone, so a host that drops
    /// connection attempts fails in seconds rather than a full timeout.
    pub connect_timeout: Duration,
    /// How many times one request is tried before the run gives up.
    pub attempts: u32,
    /// The wait after the first failed attempt; it doubles after each one.
    pub backoff: Duration,
}

impl Settings {
    /// Settings for `model`, with the key and base URL read from the
    /// process environment.
    #[must_use]
    pub fn from_env(model: &str) -> Self {
        Self::from_lookup(model, |name| std::env::var(name).ok())
    }

    /// Settings for `model`, with the key and base URL read through
    /// `lookup` — so tests can supply an environment without mutating the
    /// process's. An empty value counts as unset.
    #[must_use]
    pub fn from_lookup(
        model: &str,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let set = |name| lookup(name).filter(|value: &String| !value.is_empty());
        Self {
            model: model.to_owned(),
            base_url: set(BASE_URL_VAR).unwrap_or_else(|| DEFAULT_BASE_URL.to_owned()),
            api_key: set(API_KEY_VAR),
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            attempts: DEFAULT_ATTEMPTS,
            backoff: DEFAULT_BACKOFF,
        }
    }

    /// The evaluation endpoint's full URL.
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("{}{ENDPOINT_PATH}", self.base_url.trim_end_matches('/'))
    }
}

/// Why a triage run produced no verdicts. Every variant names its cause, so
/// the one warning a failed run prints is actionable.
#[derive(Debug)]
pub enum TriageError {
    /// `TYPESAFE_API_KEY` is unset or empty.
    MissingKey,
    /// A side's source could not be read back from disk.
    Source(io::Error),
    /// The API answered with a failure status, after every allowed attempt.
    Status {
        status: u16,
        body: String,
        attempts: u32,
    },
    /// The API could not be reached, after every allowed attempt.
    Transport {
        endpoint: String,
        message: String,
        attempts: u32,
    },
    /// The API answered, but not with the three answers asked for.
    Decode(DecodeError),
    /// The threads the requests run on could not be started.
    Threads(String),
}

impl fmt::Display for TriageError {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::MissingKey => write!(f, "{API_KEY_VAR} is not set"),
            Self::Source(e) => write!(f, "could not read a duplicate's source: {e}"),
            Self::Status {
                status,
                body,
                attempts,
            } => write!(
                f,
                "the TypeSafe API answered {status} after {attempts} attempt(s): {}",
                body.chars().take(200).collect::<String>()
            ),
            Self::Transport {
                endpoint,
                message,
                attempts,
            } => write!(
                f,
                "could not reach the TypeSafe API at {endpoint} after {attempts} attempt(s): \
                 {message}"
            ),
            Self::Decode(e) => write!(f, "unexpected answer from the TypeSafe API: {e}"),
            Self::Threads(e) => write!(f, "could not start the triage request threads: {e}"),
        }
    }
}

impl std::error::Error for TriageError {}

/// Whether a failure status is worth retrying: rate limits and server-side
/// errors are transient; any other client error is not.
#[must_use]
pub fn is_retryable_status(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

/// How long to wait after failed attempt number `attempt` (1-based).
#[must_use]
pub fn backoff(
    base: Duration,
    attempt: u32,
) -> Duration {
    base.saturating_mul(1 << attempt.saturating_sub(1).min(16))
}

/// How long to wait after failed attempt number `attempt`: what the API
/// asked for in `Retry-After`, when it gave a number of seconds.
#[must_use]
pub fn retry_delay(
    retry_after: Option<&str>,
    base: Duration,
    attempt: u32,
) -> Duration {
    retry_after
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map_or_else(
            || backoff(base, attempt),
            |secs| Duration::from_secs(secs).min(MAX_RETRY_AFTER),
        )
}

/// A blocking client for one run: one connection pool shared by every
/// request, which may be sent from many threads at once.
pub struct Client {
    agent: ureq::Agent,
    endpoint: String,
    authorization: String,
    attempts: u32,
    backoff: Duration,
}

impl Client {
    /// A client for `settings`, or [`TriageError::MissingKey`] without a key.
    ///
    /// # Errors
    ///
    /// When `settings` carries no API key.
    pub fn new(settings: &Settings) -> Result<Self, TriageError> {
        let key = settings.api_key.as_deref().ok_or(TriageError::MissingKey)?;
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(settings.timeout))
            .timeout_connect(Some(settings.connect_timeout))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self {
            agent,
            endpoint: settings.endpoint(),
            authorization: format!("Bearer {key}"),
            attempts: settings.attempts.max(1),
            backoff: settings.backoff,
        })
    }

    /// Send `body` and return the answer's text, retrying transient failures.
    ///
    /// # Errors
    ///
    /// When every attempt failed transiently, or one failed finally.
    pub fn evaluate(
        &self,
        body: &str,
    ) -> Result<String, TriageError> {
        let mut attempt = 1;
        loop {
            match self.attempt(body, attempt) {
                Err(Failure::Transient { retry_after, .. }) if attempt < self.attempts => {
                    std::thread::sleep(retry_delay(retry_after.as_deref(), self.backoff, attempt));
                    attempt += 1;
                },
                Err(Failure::Transient { error, .. } | Failure::Final(error)) => return Err(error),
                Ok(answer) => return Ok(answer),
            }
        }
    }

    /// One try, classified.
    fn attempt(
        &self,
        body: &str,
        attempt: u32,
    ) -> Result<String, Failure> {
        let transport = |e: ureq::Error| Failure::Transient {
            error: TriageError::Transport {
                endpoint: self.endpoint.clone(),
                message: e.to_string(),
                attempts: attempt,
            },
            retry_after: None,
        };
        let mut response = self
            .agent
            .post(&self.endpoint)
            .header("Authorization", &self.authorization)
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(transport)?;
        let text = response.body_mut().read_to_string();
        if response.status().is_success() {
            // A success whose body was lost is worth another try.
            return text.map_err(transport);
        }
        // A failure is classified by its status, whether or not its body
        // survived: a 401 cut off mid-body is still a 401.
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        Err(failed_status(
            response.status().as_u16(),
            text.unwrap_or_else(|e| format!("(body unreadable: {e})")),
            attempt,
            retry_after,
        ))
    }
}

/// A failure status, as a failed attempt worth another try or not.
fn failed_status(
    status: u16,
    body: String,
    attempts: u32,
    retry_after: Option<String>,
) -> Failure {
    let error = TriageError::Status {
        status,
        body,
        attempts,
    };
    if is_retryable_status(status) {
        Failure::Transient { error, retry_after }
    } else {
        Failure::Final(error)
    }
}

/// A failed attempt: worth another try — after what the API asked for, if it
/// asked — or not.
enum Failure {
    Transient {
        error: TriageError,
        retry_after: Option<String>,
    },
    Final(TriageError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn server_errors_and_rate_limits_are_retried() {
        for status in [429, 500, 502, 503, 529, 599] {
            assert!(is_retryable_status(status), "{status} is transient");
        }
    }

    #[test]
    fn other_client_errors_are_not_retried() {
        for status in [400, 401, 403, 404, 413, 422] {
            assert!(
                !is_retryable_status(status),
                "{status} will not change on retry"
            );
        }
    }

    #[test]
    fn settings_read_the_key_and_base_url_from_the_environment() {
        let settings = Settings::from_lookup(
            "jev-1",
            lookup(&[
                (API_KEY_VAR, "secret"),
                (BASE_URL_VAR, "http://127.0.0.1:9"),
            ]),
        );
        assert_eq!(settings.model, "jev-1");
        assert_eq!(settings.api_key.as_deref(), Some("secret"));
        assert_eq!(settings.base_url, "http://127.0.0.1:9");
    }

    #[test]
    fn an_unset_or_empty_key_is_no_key() {
        assert_eq!(Settings::from_lookup("m", lookup(&[])).api_key, None);
        assert_eq!(
            Settings::from_lookup("m", lookup(&[(API_KEY_VAR, "")])).api_key,
            None
        );
    }

    #[test]
    fn the_base_url_defaults_to_the_public_api() {
        let settings = Settings::from_lookup("m", lookup(&[(BASE_URL_VAR, "")]));
        assert_eq!(settings.base_url, "https://api.typesafe.ai");
        assert_eq!(settings.endpoint(), "https://api.typesafe.ai/v1/systemone");
    }

    #[test]
    fn the_endpoint_joins_the_base_url_without_a_double_slash() {
        let settings = Settings::from_lookup("m", lookup(&[(BASE_URL_VAR, "http://stub:1/")]));
        assert_eq!(settings.endpoint(), "http://stub:1/v1/systemone");
    }

    #[test]
    fn the_defaults_bound_every_request() {
        let settings = Settings::from_lookup("m", lookup(&[]));
        assert_eq!(settings.timeout, std::time::Duration::from_secs(10));
        assert_eq!(settings.attempts, 3);
    }

    #[test]
    fn a_connection_attempt_is_bounded_separately() {
        // A host that drops SYNs fails in seconds, not a full request timeout.
        let settings = Settings::from_lookup("m", lookup(&[]));
        assert_eq!(settings.connect_timeout, std::time::Duration::from_secs(3));
    }

    #[test]
    fn retry_after_is_honoured_up_to_a_cap() {
        let base = std::time::Duration::from_millis(100);
        let secs = std::time::Duration::from_secs;
        assert_eq!(retry_delay(Some("2"), base, 1), secs(2));
        assert_eq!(retry_delay(Some(" 1 "), base, 1), secs(1));
        assert_eq!(retry_delay(Some("3600"), base, 1), secs(10), "capped");
        assert_eq!(retry_delay(None, base, 2), backoff(base, 2));
        assert_eq!(
            retry_delay(Some("Wed, 21 Oct 2026 07:28:00 GMT"), base, 1),
            backoff(base, 1),
            "an HTTP date falls back to the back-off"
        );
    }

    #[test]
    fn the_backoff_doubles_between_attempts() {
        let base = std::time::Duration::from_millis(100);
        assert_eq!(backoff(base, 1), std::time::Duration::from_millis(100));
        assert_eq!(backoff(base, 2), std::time::Duration::from_millis(200));
        assert_eq!(backoff(base, 3), std::time::Duration::from_millis(400));
    }

    #[test]
    fn every_error_names_its_cause() {
        let cases = [
            (TriageError::MissingKey, "TYPESAFE_API_KEY"),
            (
                TriageError::Status {
                    status: 503,
                    body: "overloaded".into(),
                    attempts: 3,
                },
                "503",
            ),
            (
                TriageError::Transport {
                    endpoint: "http://stub/v1/systemone".into(),
                    message: "connection refused".into(),
                    attempts: 3,
                },
                "http://stub/v1/systemone",
            ),
            (
                TriageError::Source(std::io::Error::other("src/a.rs: gone")),
                "src/a.rs",
            ),
            (
                TriageError::Threads("out of threads".into()),
                "out of threads",
            ),
        ];
        for (error, cause) in cases {
            assert!(error.to_string().contains(cause), "{error}");
        }
    }
}
