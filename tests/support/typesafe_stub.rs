//! A recording stand-in for the `TypeSafe` API, so no test needs a network or
//! a key.
//!
//! A std-only HTTP/1.1 server on `127.0.0.1:0`. Every request is recorded —
//! method, path, headers, body — before it is answered, so a test can read
//! back exactly what was sent even when the reply was a failure. Bodies are
//! read by `Content-Length`; a chunked upload or an unreadable length is
//! refused with a 400 naming the problem, never recorded with a guessed body.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// How often the accept loop looks at the stop flag, and the base of its
/// back-off when accepting keeps failing.
const POLL: Duration = Duration::from_millis(5);

/// Read and write timeout on every connection: a client that goes quiet is
/// hung up on instead of parking a thread for the rest of the run.
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// What the stub does with one request.
#[derive(Debug, Clone)]
pub enum Reply {
    /// Answer with this status, JSON body and any extra headers.
    Answer {
        status: u16,
        body: String,
        headers: Vec<(String, String)>,
    },
    /// Wait, then do `then`. The wait happens after the request is recorded
    /// and outside the lock, so other requests are not held up.
    Delay { after: Duration, then: Box<Reply> },
    /// Close the connection without answering.
    Drop,
    /// Send these bytes verbatim, then close — for responses no well-behaved
    /// server sends, such as a body cut short.
    Raw(String),
}

impl Reply {
    /// A 200 carrying `body`.
    pub fn json(body: &str) -> Self {
        Self::Answer {
            status: 200,
            body: body.to_owned(),
            headers: Vec::new(),
        }
    }

    /// A failure with `status` and a small JSON error body.
    pub fn status(status: u16) -> Self {
        Self::Answer {
            status,
            body: format!(r#"{{"error":"stub status {status}"}}"#),
            headers: Vec::new(),
        }
    }

    /// `then`, after waiting `after`.
    pub fn delayed(
        after: Duration,
        then: Reply,
    ) -> Self {
        Self::Delay {
            after,
            then: Box::new(then),
        }
    }

    /// Add a response header, e.g. `Retry-After` on a 429. Panics on
    /// [`Reply::Drop`], which sends no response to carry it.
    pub fn with_header(
        self,
        name: &str,
        value: &str,
    ) -> Self {
        match self {
            Self::Answer {
                status,
                body,
                mut headers,
            } => {
                headers.push((name.to_owned(), value.to_owned()));
                Self::Answer {
                    status,
                    body,
                    headers,
                }
            },
            Self::Delay { after, then } => Self::delayed(after, then.with_header(name, value)),
            Self::Drop => panic!("a dropped connection sends no headers"),
            Self::Raw(_) => panic!("a raw reply carries its own headers"),
        }
    }
}

/// One request as the stub received it.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl RecordedRequest {
    /// The first header named `name`, compared case-insensitively.
    pub fn header(
        &self,
        name: &str,
    ) -> Option<&str> {
        header(&self.headers, name)
    }

    /// The body parsed as JSON; panics when it is not JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("the request body is JSON")
    }
}

type Responder = dyn Fn(&RecordedRequest) -> Reply + Send + Sync;

/// A running stub. Dropping it stops the server.
pub struct TypesafeStub {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    /// The accept loop; it hands back every connection thread it started.
    server: Option<JoinHandle<Vec<JoinHandle<()>>>>,
}

impl TypesafeStub {
    /// Answer every request with whatever `responder` returns for it. Each
    /// connection is served on its own thread, so parallel clients work; the
    /// responder runs under the same lock that records the request, so
    /// [`requests`](Self::requests) lists requests in the order they were
    /// answered.
    pub fn respond_with(
        responder: impl Fn(&RecordedRequest) -> Reply + Send + Sync + 'static
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the stub");
        let addr = listener.local_addr().expect("the stub's address");
        // Polled rather than blocking, so stopping needs no wake-up
        // connection that could fail and leave `drop` joining forever.
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let responder: Arc<Responder> = Arc::new(responder);
        let server = {
            let requests = Arc::clone(&requests);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                accept_until_stopped(&listener, &stop, &requests, &responder)
            })
        };
        Self {
            addr,
            requests,
            stop,
            server: Some(server),
        }
    }

    /// Answer the n-th request to be answered with `replies[n]`; once the
    /// script runs out, its last reply repeats — so `[Reply::status(503)]`
    /// is an API that always fails.
    ///
    /// Positions follow the order requests reach the responder. With
    /// parallel clients that order is not the order they were issued in, so
    /// a test that needs a particular request to get a particular reply must
    /// use [`respond_with`](Self::respond_with) and choose by content.
    pub fn scripted(replies: Vec<Reply>) -> Self {
        assert!(!replies.is_empty(), "a script needs at least one reply");
        let next = AtomicUsize::new(0);
        Self::respond_with(move |_| {
            let n = next.fetch_add(1, Ordering::SeqCst);
            replies[n.min(replies.len() - 1)].clone()
        })
    }

    /// `http://127.0.0.1:<port>`, for `TYPESAFE_BASE_URL`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Every request answered so far, in the order it was answered.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("stub lock").clone()
    }

    /// How many requests have been answered so far.
    pub fn request_count(&self) -> usize {
        self.requests.lock().expect("stub lock").len()
    }
}

impl Drop for TypesafeStub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Join the connection threads too, not only the accept loop: a test
        // that ends with one still running leaves it to the process teardown.
        // The socket timeouts bound how long any of them can take.
        if let Some(Ok(connections)) = self.server.take().map(JoinHandle::join) {
            for connection in connections {
                let _ = connection.join();
            }
        }
    }
}

/// Accept connections until `stop` is set, serving each on its own thread,
/// and return those threads for the caller to join. Every accepted
/// connection is served, even one accepted just before the stop; persistent
/// accept errors (fd exhaustion) back off instead of spinning a core.
fn accept_until_stopped(
    listener: &TcpListener,
    stop: &AtomicBool,
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
    responder: &Arc<Responder>,
) -> Vec<JoinHandle<()>> {
    let mut connections = Vec::new();
    let mut failures: u32 = 0;
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                failures = 0;
                let requests = Arc::clone(requests);
                let responder = Arc::clone(responder);
                connections.push(std::thread::spawn(move || {
                    serve(&stream, &requests, &*responder);
                }));
            },
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(POLL),
            Err(_) => {
                failures = failures.saturating_add(1);
                std::thread::sleep(POLL * failures.min(50));
            },
        }
    }
    connections
}

/// Read one request, record it and pick its reply under one lock, then
/// deliver the reply. Recording comes first, so a client that has its
/// response can already read the request back.
fn serve(
    stream: &TcpStream,
    requests: &Mutex<Vec<RecordedRequest>>,
    responder: &Responder,
) {
    // Accepted sockets may inherit the listener's non-blocking mode (they
    // do on macOS); this connection is served with blocking I/O and timeouts.
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(IO_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(IO_TIMEOUT)).is_err()
    {
        return;
    }
    let request = match read_request(stream) {
        Ok(Some(request)) => request,
        Ok(None) => return,
        Err(problem) => {
            refuse(stream, &problem);
            return;
        },
    };
    let reply = {
        let mut recorded = requests.lock().expect("stub lock");
        let reply = responder(&request);
        recorded.push(request);
        reply
    };
    deliver(stream, reply);
}

/// Send `reply` down `stream`; for [`Reply::Drop`], send nothing — the
/// stream closes when the caller lets go of it.
fn deliver(
    stream: &TcpStream,
    reply: Reply,
) {
    match reply {
        Reply::Answer {
            status,
            body,
            headers,
        } => write_response(stream, status, &body, &headers),
        Reply::Delay { after, then } => {
            std::thread::sleep(after);
            deliver(stream, *then);
        },
        Reply::Drop => {},
        Reply::Raw(bytes) => {
            let mut stream = stream;
            let _ = stream.write_all(bytes.as_bytes());
        },
    }
}

/// Answer a request the stub cannot read faithfully with a 400 naming the
/// problem, and say so on stderr, where a test's output will show it.
fn refuse(
    stream: &TcpStream,
    problem: &str,
) {
    eprintln!("typesafe stub: refusing a request: {problem}");
    let body = serde_json::json!({ "error": format!("typesafe stub: {problem}") }).to_string();
    write_response(stream, 400, &body, &[]);
    drain(stream);
}

/// Read whatever the client already sent, so closing the socket sends FIN
/// rather than a reset that could destroy the response before it is read.
fn drain(mut stream: &TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut sink = [0u8; 4096];
    while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
}

/// Parse a request line, headers and a `Content-Length` body.
///
/// `Ok(None)` for a connection that closed or went quiet before sending a
/// request line; `Err` naming the problem for a body the stub cannot read
/// faithfully.
fn read_request(stream: &TcpStream) -> Result<Option<RecordedRequest>, String> {
    let mut reader = BufReader::new(stream);
    let Some((method, path, headers)) = read_head(&mut reader) else {
        return Ok(None);
    };
    let length = body_length(&headers)?;
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return Ok(None);
    }
    Ok(Some(RecordedRequest {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }))
}

type Head = (String, String, Vec<(String, String)>);

/// The request line and headers, or `None` when the connection ended first.
fn read_head(reader: &mut impl BufRead) -> Option<Head> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            return Some((method, path, headers));
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_owned(), value.trim().to_owned()));
    }
}

/// The body's length from `Content-Length` (0 when absent), or the reason the
/// body cannot be read by length.
fn body_length(headers: &[(String, String)]) -> Result<usize, String> {
    if header(headers, "transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
    {
        return Err("chunked request bodies are not supported; send Content-Length".to_owned());
    }
    match header(headers, "content-length") {
        None => Ok(0),
        Some(value) => value
            .parse()
            .map_err(|_| format!("unparsable Content-Length {value:?}")),
    }
}

fn header<'h>(
    headers: &'h [(String, String)],
    name: &str,
) -> Option<&'h str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn write_response(
    mut stream: &TcpStream,
    status: u16,
    body: &str,
    headers: &[(String, String)],
) {
    let reason = if status < 400 { "OK" } else { "Error" };
    let extra: String = headers
        .iter()
        .flat_map(|(name, value)| [name.as_str(), ": ", value.as_str(), "\r\n"])
        .collect();
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    /// Send one HTTP/1.1 request and return the raw response ("" when the
    /// server closed the connection without answering).
    fn send(
        base_url: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> String {
        let host = base_url.trim_start_matches("http://");
        let header_lines = headers
            .iter()
            .fold(String::new(), |mut lines, (name, value)| {
                let _ = write!(lines, "{name}: {value}\r\n");
                lines
            });
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\n{header_lines}\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        send_raw(base_url, &request)
    }

    /// Write `request` verbatim and return the raw response.
    fn send_raw(
        base_url: &str,
        request: &str,
    ) -> String {
        let mut stream = TcpStream::connect(base_url.trim_start_matches("http://"))
            .expect("connect to the stub");
        stream.write_all(request.as_bytes()).expect("send");
        let mut raw = String::new();
        // A dropped connection may surface as a reset rather than EOF.
        let _ = stream.read_to_string(&mut raw);
        raw
    }

    fn status_of(raw: &str) -> u16 {
        raw.split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .expect("a status line")
    }

    fn body_of(raw: &str) -> &str {
        raw.split_once("\r\n\r\n").map_or("", |(_, body)| body)
    }

    #[test]
    fn answers_with_the_scripted_json_and_records_the_request() {
        let stub = TypesafeStub::scripted(vec![Reply::json(r#"{"ok":true}"#)]);
        let raw = send(
            &stub.base_url(),
            "/v1/systemone",
            &[
                ("Authorization", "Bearer k"),
                ("Content-Type", "application/json"),
            ],
            r#"{"state":1}"#,
        );
        assert_eq!(status_of(&raw), 200, "{raw}");
        assert_eq!(body_of(&raw), r#"{"ok":true}"#);

        let requests = stub.requests();
        assert_eq!(requests.len(), 1);
        let recorded = &requests[0];
        assert_eq!(recorded.method, "POST");
        assert_eq!(recorded.path, "/v1/systemone");
        assert_eq!(recorded.body, r#"{"state":1}"#);
        assert_eq!(recorded.json()["state"], 1);
        assert_eq!(
            recorded.header("authorization"),
            Some("Bearer k"),
            "case-insensitive"
        );
        assert_eq!(recorded.header("x-missing"), None);
    }

    #[test]
    fn fails_with_the_scripted_status() {
        let stub = TypesafeStub::scripted(vec![Reply::status(503)]);
        let raw = send(&stub.base_url(), "/v1/systemone", &[], "{}");
        assert_eq!(status_of(&raw), 503, "{raw}");
        assert_eq!(
            stub.request_count(),
            1,
            "a failed request is still recorded"
        );
    }

    #[test]
    fn drops_the_connection_without_answering() {
        let stub = TypesafeStub::scripted(vec![Reply::Drop]);
        let raw = send(&stub.base_url(), "/v1/systemone", &[], "{}");
        assert!(raw.is_empty(), "no response bytes: {raw}");
        assert_eq!(
            stub.request_count(),
            1,
            "a dropped request is still recorded"
        );
    }

    #[test]
    fn scripted_replies_are_served_in_order_and_the_last_one_repeats() {
        let stub = TypesafeStub::scripted(vec![Reply::status(500), Reply::json("{}")]);
        let codes: Vec<u16> = (0..4)
            .map(|_| status_of(&send(&stub.base_url(), "/v1/systemone", &[], "{}")))
            .collect();
        assert_eq!(codes, [500, 200, 200, 200]);
    }

    #[test]
    fn a_responder_answers_from_the_request_it_sees() {
        let stub = TypesafeStub::respond_with(|request| {
            if request.body.contains("fail") {
                Reply::status(502)
            } else {
                Reply::json(r#"{"echo":true}"#)
            }
        });
        let ok = send(&stub.base_url(), "/v1/systemone", &[], r#"{"pair":"a"}"#);
        let failed = send(&stub.base_url(), "/v1/systemone", &[], r#"{"pair":"fail"}"#);
        assert_eq!(status_of(&ok), 200);
        assert_eq!(status_of(&failed), 502);
    }

    #[test]
    fn nothing_is_recorded_until_a_request_arrives() {
        let stub = TypesafeStub::scripted(vec![Reply::json("{}")]);
        assert_eq!(stub.request_count(), 0);
        assert!(stub.requests().is_empty());
    }

    #[test]
    fn concurrent_requests_are_all_answered_and_recorded() {
        let stub = TypesafeStub::scripted(vec![Reply::json("{}")]);
        let base = stub.base_url();
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let base = base.clone();
                std::thread::spawn(move || {
                    status_of(&send(
                        &base,
                        "/v1/systemone",
                        &[],
                        &format!(r#"{{"n":{i}}}"#),
                    ))
                })
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.join().expect("client thread"), 200);
        }
        let mut seen: Vec<i64> = stub
            .requests()
            .iter()
            .map(|r| r.json()["n"].as_i64().expect("n"))
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn requests_are_recorded_in_the_order_they_were_answered() {
        // The slow request takes the responder first; the fast one arrives
        // while it is still inside. Recording must follow the answering
        // order, or a parallel test that reads requests() positionally lies.
        let stub = TypesafeStub::respond_with(|request| {
            if request.body.contains("slow") {
                std::thread::sleep(Duration::from_millis(300));
            }
            Reply::json("{}")
        });
        let base = stub.base_url();
        let slow = {
            let base = base.clone();
            std::thread::spawn(move || send(&base, "/v1/systemone", &[], r#"{"n":"slow"}"#))
        };
        std::thread::sleep(Duration::from_millis(100));
        send(&base, "/v1/systemone", &[], r#"{"n":"fast"}"#);
        slow.join().expect("slow client");
        let order: Vec<String> = stub
            .requests()
            .iter()
            .map(|r| r.json()["n"].as_str().expect("n").to_owned())
            .collect();
        assert_eq!(order, ["slow", "fast"]);
    }

    #[test]
    fn dropping_the_stub_returns_promptly() {
        let stub = TypesafeStub::scripted(vec![Reply::json("{}")]);
        send(&stub.base_url(), "/v1/systemone", &[], "{}");
        let started = Instant::now();
        drop(stub);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_client_that_sends_nothing_is_disconnected() {
        let stub = TypesafeStub::scripted(vec![Reply::json("{}")]);
        let mut stream =
            TcpStream::connect(stub.base_url().trim_start_matches("http://")).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("client timeout");
        let mut byte = [0u8; 1];
        let read = stream.read(&mut byte);
        assert!(
            matches!(read, Ok(0)),
            "the stub hangs up on a silent client instead of parking a thread: {read:?}"
        );
        assert_eq!(stub.request_count(), 0);
    }

    #[test]
    fn a_chunked_request_is_refused_loudly() {
        let stub = TypesafeStub::scripted(vec![Reply::json("{}")]);
        let raw = send_raw(
            &stub.base_url(),
            "POST /v1/systemone HTTP/1.1\r\nHost: stub\r\nTransfer-Encoding: chunked\r\n\
             Connection: close\r\n\r\n2\r\n{}\r\n0\r\n\r\n",
        );
        assert_eq!(status_of(&raw), 400, "{raw}");
        assert!(body_of(&raw).contains("chunked"), "{raw}");
        assert_eq!(
            stub.request_count(),
            0,
            "never recorded with a made-up body"
        );
    }

    #[test]
    fn an_unparsable_content_length_is_refused_loudly() {
        let stub = TypesafeStub::scripted(vec![Reply::json("{}")]);
        let raw = send_raw(
            &stub.base_url(),
            "POST /v1/systemone HTTP/1.1\r\nHost: stub\r\nContent-Length: two\r\n\
             Connection: close\r\n\r\n{}",
        );
        assert_eq!(status_of(&raw), 400, "{raw}");
        assert!(body_of(&raw).contains("Content-Length"), "{raw}");
        assert_eq!(stub.request_count(), 0);
    }

    #[test]
    fn a_reply_can_carry_extra_headers() {
        let stub = TypesafeStub::scripted(vec![Reply::status(429).with_header("Retry-After", "0")]);
        let raw = send(&stub.base_url(), "/v1/systemone", &[], "{}");
        assert_eq!(status_of(&raw), 429, "{raw}");
        assert!(raw.contains("\r\nRetry-After: 0\r\n"), "{raw}");
    }

    #[test]
    fn a_raw_reply_is_sent_byte_for_byte() {
        // For responses no well-behaved server sends: a body cut short, a
        // lying Content-Length.
        let response = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 99\r\n\r\n{\"err";
        let stub = TypesafeStub::scripted(vec![Reply::Raw(response.to_owned())]);
        let raw = send(&stub.base_url(), "/v1/systemone", &[], "{}");
        assert_eq!(raw, response);
        assert_eq!(stub.request_count(), 1);
    }

    #[test]
    fn a_delayed_reply_arrives_after_the_delay() {
        let stub = TypesafeStub::scripted(vec![Reply::delayed(
            Duration::from_millis(200),
            Reply::json(r#"{"late":true}"#).with_header("X-Stub", "delayed"),
        )]);
        let started = Instant::now();
        let raw = send(&stub.base_url(), "/v1/systemone", &[], "{}");
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(body_of(&raw), r#"{"late":true}"#);
        assert!(
            raw.contains("\r\nX-Stub: delayed\r\n"),
            "headers reach the delayed reply: {raw}"
        );
        assert_eq!(stub.request_count(), 1);
    }
}
