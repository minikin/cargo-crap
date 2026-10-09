//! The HTTP client every triage provider is reached through.
//!
//! Blocking, one request per pair, sent to the endpoint and with the key the
//! run's [`Provider`] names. A connection that failed or dropped, a
//! 429 and any 5xx (the API's own 529 "overloaded" included) count as
//! transient and are retried a bounded number of times with a doubling
//! back-off. Any other 4xx is final: the same request will be refused the
//! same way.

use crate::duplicates::triage::cache;
use crate::duplicates::triage::provider::Provider;
use crate::duplicates::triage::verdict::DecodeError;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Cargo's target directory, where the verdict cache lives.
pub const TARGET_DIR_VAR: &str = "CARGO_TARGET_DIR";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_ATTEMPTS: u32 = 3;
const DEFAULT_BACKOFF: Duration = Duration::from_millis(500);
/// The longest `Retry-After` honoured; a longer ask waits this long, then
/// the attempt count decides.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

/// Everything a triage run needs to reach the API.
#[derive(Clone)]
pub struct Settings {
    /// Whose API is asked: its endpoint, key variable and wire shape.
    pub provider: &'static dyn Provider,
    /// The model every request names.
    pub model: String,
    /// What the provider's endpoint path is joined to. How much of the path
    /// it holds is the provider's convention: `https://api.typesafe.ai` for
    /// `TypeSafe`, `https://api.openai.com/v1` for `OpenAI`.
    pub base_url: String,
    /// The bearer token; `None` when the provider's key variable is unset
    /// or empty.
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
    /// Where verdicts are cached; `None` asks about every pair every time.
    pub cache_dir: Option<PathBuf>,
}

/// Shows which provider and model, never the key: settings end up in panic
/// messages and logs, and CI logs are often public.
impl fmt::Debug for Settings {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        f.debug_struct("Settings")
            .field("provider", &self.provider.id())
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("attempts", &self.attempts)
            .field("backoff", &self.backoff)
            .field("cache_dir", &self.cache_dir)
            .finish()
    }
}

impl Settings {
    /// Settings for asking `provider`'s `model` in the project rooted at
    /// `project_root`, with the key, base URL and target directory read from
    /// the process environment.
    #[must_use]
    pub fn from_env(
        provider: &'static dyn Provider,
        model: &str,
        project_root: &Path,
    ) -> Self {
        Self::from_lookup(provider, model, project_root, |name| {
            std::env::var(name).ok()
        })
    }

    /// Settings for asking `provider`'s `model` in the project rooted at
    /// `project_root`, with the provider's key and base-URL variables and
    /// `CARGO_TARGET_DIR` read through `lookup`, so tests can supply an
    /// environment without mutating the process's. No other provider's
    /// variables are read. An empty value counts as unset. Verdicts are cached in
    /// the project's target directory (`CARGO_TARGET_DIR` when set, as for
    /// cargo, else `target/` beside the configuration), so `cargo clean`
    /// sweeps them wherever the command was run from.
    #[must_use]
    pub fn from_lookup(
        provider: &'static dyn Provider,
        model: &str,
        project_root: &Path,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let set = |name| lookup(name).filter(|value: &String| !value.is_empty());
        let target = set(TARGET_DIR_VAR).map_or_else(|| project_root.join("target"), PathBuf::from);
        Self {
            provider,
            model: model.to_owned(),
            base_url: set(provider.base_url_var())
                .unwrap_or_else(|| provider.default_base_url().to_owned()),
            api_key: set(provider.key_var()),
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            attempts: DEFAULT_ATTEMPTS,
            backoff: DEFAULT_BACKOFF,
            cache_dir: Some(cache::default_dir(&target)),
        }
    }

    /// The evaluation endpoint's full URL.
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            self.provider.endpoint_path()
        )
    }
}

/// Why a triage run produced no verdicts. Every variant names its cause, so
/// the one warning a failed run prints is actionable.
#[derive(Debug)]
pub enum TriageError {
    /// The provider's key variable, `var`, is unset or empty.
    MissingKey { var: &'static str },
    /// A side's source could not be read back from disk.
    Source(io::Error),
    /// The API answered with a failure status, after every allowed attempt.
    Status {
        /// How the provider names its API.
        api: &'static str,
        status: u16,
        body: String,
        attempts: u32,
    },
    /// The API could not be reached, after every allowed attempt.
    Transport {
        /// How the provider names its API.
        api: &'static str,
        endpoint: String,
        message: String,
        attempts: u32,
    },
    /// The API answered, but not with the three answers asked for.
    Decode {
        /// How the provider names its API.
        api: &'static str,
        error: DecodeError,
    },
    /// The threads the requests run on could not be started.
    Threads(String),
}

impl fmt::Display for TriageError {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::MissingKey { var } => write!(f, "{var} is not set"),
            Self::Source(e) => write!(f, "could not read a duplicate's source: {e}"),
            Self::Status {
                api,
                status,
                body,
                attempts,
            } => write!(
                f,
                "the {api} answered {status} after {attempts} attempt(s): {}",
                body.chars().take(200).collect::<String>()
            ),
            Self::Transport {
                api,
                endpoint,
                message,
                attempts,
            } => write!(
                f,
                "could not reach the {api} at {endpoint} after {attempts} attempt(s): {message}"
            ),
            Self::Decode { api, error } => write!(f, "unexpected answer from the {api}: {error}"),
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
    /// How the provider names its API, for errors.
    api: &'static str,
    endpoint: String,
    authorization: String,
    attempts: u32,
    backoff: Duration,
}

impl Client {
    /// A client for `settings`, or `None` when they carry no API key, which
    /// is the only thing a client can be missing.
    #[must_use]
    pub fn new(settings: &Settings) -> Option<Self> {
        let key = settings.api_key.as_deref()?;
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(settings.timeout))
            .timeout_connect(Some(settings.connect_timeout))
            .http_status_as_error(false)
            .build()
            .into();
        Some(Self {
            agent,
            api: settings.provider.display_name(),
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
                api: self.api,
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
            self.api,
            response.status().as_u16(),
            text.unwrap_or_else(|e| format!("(body unreadable: {e})")),
            attempt,
            retry_after,
        ))
    }
}

/// A failure status, as a failed attempt worth another try or not.
fn failed_status(
    api: &'static str,
    status: u16,
    body: String,
    attempts: u32,
    retry_after: Option<String>,
) -> Failure {
    let error = TriageError::Status {
        api,
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

/// A failed attempt, worth another try or not. A transient one carries what
/// the API asked to wait, when it asked.
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
    use crate::duplicates::triage::provider::PROVIDERS;
    use crate::duplicates::triage::provider::openai::OpenAi;
    use crate::duplicates::triage::provider::typesafe::{self, TypeSafe};
    use proptest::prelude::*;

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
            &TypeSafe,
            "jev-1",
            Path::new("/proj"),
            lookup(&[
                (typesafe::API_KEY_VAR, "secret"),
                (typesafe::BASE_URL_VAR, "http://127.0.0.1:9"),
            ]),
        );
        assert_eq!(settings.model, "jev-1");
        assert_eq!(settings.api_key.as_deref(), Some("secret"));
        assert_eq!(settings.base_url, "http://127.0.0.1:9");
    }

    #[test]
    fn an_unset_or_empty_key_is_no_key() {
        assert_eq!(
            Settings::from_lookup(&TypeSafe, "m", Path::new("/proj"), lookup(&[])).api_key,
            None
        );
        assert_eq!(
            Settings::from_lookup(
                &TypeSafe,
                "m",
                Path::new("/proj"),
                lookup(&[(typesafe::API_KEY_VAR, "")])
            )
            .api_key,
            None
        );
    }

    #[test]
    fn the_base_url_defaults_to_the_public_api() {
        let settings = Settings::from_lookup(
            &TypeSafe,
            "m",
            Path::new("/proj"),
            lookup(&[(typesafe::BASE_URL_VAR, "")]),
        );
        assert_eq!(settings.base_url, "https://api.typesafe.ai");
        assert_eq!(settings.endpoint(), "https://api.typesafe.ai/v1/systemone");
    }

    #[test]
    fn the_endpoint_joins_the_base_url_without_a_double_slash() {
        let settings = Settings::from_lookup(
            &TypeSafe,
            "m",
            Path::new("/proj"),
            lookup(&[(typesafe::BASE_URL_VAR, "http://stub:1/")]),
        );
        assert_eq!(settings.endpoint(), "http://stub:1/v1/systemone");
    }

    #[test]
    fn the_defaults_bound_every_request() {
        let settings = Settings::from_lookup(&TypeSafe, "m", Path::new("/proj"), lookup(&[]));
        assert_eq!(settings.timeout, std::time::Duration::from_secs(10));
        assert_eq!(settings.attempts, 3);
    }

    #[test]
    fn a_connection_attempt_is_bounded_separately() {
        // A host that drops SYNs fails in seconds, not a full request timeout.
        let settings = Settings::from_lookup(&TypeSafe, "m", Path::new("/proj"), lookup(&[]));
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
    fn verdicts_are_cached_under_the_projects_target_directory() {
        // Beside the configuration that enabled triage, not wherever the
        // command happened to run; CARGO_TARGET_DIR still wins, as for cargo.
        let default = Settings::from_lookup(&TypeSafe, "m", Path::new("/proj"), lookup(&[]));
        assert_eq!(
            default.cache_dir,
            Some(
                PathBuf::from("/proj/target")
                    .join("cargo-crap")
                    .join("triage")
            )
        );
        let custom = Settings::from_lookup(
            &TypeSafe,
            "m",
            Path::new("/proj"),
            lookup(&[(TARGET_DIR_VAR, "/tmp/t")]),
        );
        assert_eq!(
            custom.cache_dir,
            Some(PathBuf::from("/tmp/t").join("cargo-crap").join("triage"))
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
            (
                TriageError::MissingKey {
                    var: "TYPESAFE_API_KEY",
                },
                "TYPESAFE_API_KEY is not set",
            ),
            (
                TriageError::MissingKey {
                    var: "OPENAI_API_KEY",
                },
                "OPENAI_API_KEY is not set",
            ),
            (
                TriageError::Status {
                    api: "OpenAI API",
                    status: 503,
                    body: "overloaded".into(),
                    attempts: 3,
                },
                "the OpenAI API answered 503 after 3 attempt(s): overloaded",
            ),
            (
                TriageError::Transport {
                    api: "OpenAI API",
                    endpoint: "http://stub/v1/decisions".into(),
                    message: "connection refused".into(),
                    attempts: 3,
                },
                "could not reach the OpenAI API at http://stub/v1/decisions after 3 \
                 attempt(s): connection refused",
            ),
            (
                TriageError::Decode {
                    api: "OpenAI API",
                    error: crate::duplicates::triage::verdict::DecodeError::new("x: no answer"),
                },
                "unexpected answer from the OpenAI API: x: no answer",
            ),
            (
                TriageError::Source(std::io::Error::other("src/a.rs: gone")),
                "could not read a duplicate's source: src/a.rs: gone",
            ),
            (
                TriageError::Threads("out of threads".into()),
                "could not start the triage request threads: out of threads",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn openai_settings_read_only_openai_variables() {
        let settings = Settings::from_lookup(
            &OpenAi,
            "gpt-6-luna",
            Path::new("/proj"),
            lookup(&[
                (typesafe::API_KEY_VAR, "typesafe-secret"),
                (typesafe::BASE_URL_VAR, "http://typesafe:1"),
            ]),
        );
        assert_eq!(
            settings.api_key, None,
            "a TypeSafe key is never an OpenAI key"
        );
        assert_eq!(settings.base_url, "https://api.openai.com/v1");
        assert_eq!(settings.endpoint(), "https://api.openai.com/v1/decisions");
        let settings = Settings::from_lookup(
            &OpenAi,
            "gpt-6-luna",
            Path::new("/proj"),
            lookup(&[
                ("OPENAI_API_KEY", "openai-secret"),
                ("OPENAI_BASE_URL", "http://127.0.0.1:9/v1/"),
            ]),
        );
        assert_eq!(settings.api_key.as_deref(), Some("openai-secret"));
        assert_eq!(settings.endpoint(), "http://127.0.0.1:9/v1/decisions");
        assert_eq!(settings.provider.id(), "openai");
    }

    /// Every variable any registered provider reads.
    fn provider_variables() -> Vec<&'static str> {
        PROVIDERS
            .iter()
            .flat_map(|p| [p.key_var(), p.base_url_var()])
            .collect()
    }

    proptest! {
        /// Whatever is set, a provider's settings carry only values read
        /// from its own variables.
        #[test]
        fn settings_never_carry_another_providers_values(
            set in proptest::collection::vec(any::<bool>(), 2 * PROVIDERS.len()),
            which in 0..PROVIDERS.len(),
        ) {
            let vars = provider_variables();
            prop_assert_eq!(vars.len(), set.len());
            let provider = PROVIDERS[which];
            let present: Vec<&str> = vars
                .iter()
                .zip(&set)
                .filter(|(_, on)| **on)
                .map(|(var, _)| *var)
                .collect();
            let settings = Settings::from_lookup(provider, "m", Path::new("/proj"), |name| {
                present.contains(&name).then(|| format!("{name}-value"))
            });
            if let Some(key) = &settings.api_key {
                prop_assert_eq!(key, &format!("{}-value", provider.key_var()));
            }
            let own_url = format!("{}-value", provider.base_url_var());
            prop_assert!(
                settings.base_url == own_url || settings.base_url == provider.default_base_url(),
                "{}", settings.base_url
            );
        }
    }

    #[test]
    fn debug_output_names_the_provider_and_never_shows_the_key() {
        let settings = Settings::from_lookup(
            &OpenAi,
            "gpt-6-luna",
            Path::new("/proj"),
            lookup(&[("OPENAI_API_KEY", "sk-very-secret")]),
        );
        let shown = format!("{settings:?}");
        assert!(!shown.contains("sk-very-secret"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
        assert!(shown.contains("provider: \"openai\""), "{shown}");
        assert!(shown.contains("gpt-6-luna"), "{shown}");
        let keyless = Settings::from_lookup(&OpenAi, "m", Path::new("/proj"), lookup(&[]));
        assert!(
            format!("{keyless:?}").contains("api_key: None"),
            "{keyless:?}"
        );
    }

    #[test]
    fn a_client_needs_a_key() {
        let keyless = Settings::from_lookup(&TypeSafe, "m", Path::new("/proj"), lookup(&[]));
        assert!(Client::new(&keyless).is_none());
        let keyed = Settings {
            api_key: Some("k".into()),
            ..keyless
        };
        assert!(Client::new(&keyed).is_some());
    }
}
