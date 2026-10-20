use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde_json::json;
use uuid::Uuid;

use super::*;

const TOKEN: &str = "APP/TOKEN+NOT_FOR_LOGS";
const SECRET: &str = "CLIENT_SECRET_NOT_FOR_LOGS";
const REQUEST_ID: &str = "AAAAAAAA-6666-4666-8666-666666666666";
const ORGANIZATION: &str = "33333333-3333-4333-8333-333333333333";
const USER: &str = "11111111-1111-4111-8111-111111111111";

#[tokio::test]
async fn combined_transport_is_bound_bounded_and_never_falls_back() {
    let body=json!({"contractVersion":"1.0","introspection":{"active":true},"context":{"appCode":"stores"}}).to_string();
    let peer = HttpPeer::start(reply(200, Some("application/json"), body.as_bytes()));
    let mut settings = peer.settings();
    settings.legacy_authorization = false;
    let client = PlatformClient::new(&settings).unwrap();
    for _ in 0..2 {
        client
            .authorization_pair(TOKEN, "store", "create", REQUEST_ID)
            .await
            .unwrap();
    }
    let calls = peer.captured();
    assert_eq!(calls.len(), 2);
    for call in calls {
        assert_eq!(call.method, "POST");
        assert_eq!(call.target, "/api/apps/stores/authorize");
        assert!(call.headers["authorization"].starts_with("Basic "));
        let form = url::form_urlencoded::parse(&call.body)
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(form.len(), 3);
        assert_eq!(form["token"], TOKEN);
        assert_eq!(form["resource"], "store");
        assert_eq!(form["action"], "create");
    }
    let peer = HttpPeer::start(reply(
        200,
        Some("application/json"),
        br#"{"contractVersion":"2.0","introspection":{},"context":{}}"#,
    ));
    settings.origin = url::Url::parse(&peer.origin).unwrap();
    assert!(PlatformClient::new(&settings)
        .unwrap()
        .authorization_pair(TOKEN, "store", "create", REQUEST_ID)
        .await
        .is_err());
    assert_eq!(peer.captured().len(), 1);
    for (upstream, expected) in [
        ("OPERATION_DENIED", "OPERATION_DENIED"),
        ("FORBIDDEN", "DATA_DENIED"),
    ] {
        let body = json!({"code":upstream}).to_string();
        let peer = HttpPeer::start(reply(403, Some("application/json"), body.as_bytes()));
        settings.origin = url::Url::parse(&peer.origin).unwrap();
        let error = PlatformClient::new(&settings)
            .unwrap()
            .authorization_pair(TOKEN, "store", "create", REQUEST_ID)
            .await
            .unwrap_err();
        assert_eq!(error.code(), expected);
    }
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// A real loopback HTTP peer. Raw replies let tests exercise body framing and
/// incomplete bodies without relying on the HTTP server to normalize them.
struct HttpPeer {
    origin: String,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl HttpPeer {
    fn start(reply: Vec<u8>) -> Self {
        Self::with_delayed_tail(reply, None)
    }

    fn with_delayed_tail(reply: Vec<u8>, tail: Option<(Duration, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let observed = requests.clone();
        let stopping = stopped.clone();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        socket
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        if let Some(request) = read_request(&mut socket) {
                            observed.lock().unwrap().push(request);
                            let _ = socket.write_all(&reply);
                            if let Some((delay, bytes)) = &tail {
                                thread::sleep(*delay);
                                let _ = socket.write_all(bytes);
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            origin,
            requests,
            stopped,
            worker: Some(worker),
        }
    }

    fn settings(&self) -> PlatformConfig {
        PlatformConfig {
            legacy_authorization: true,
            origin: url::Url::parse(&self.origin).unwrap(),
            client_id: "stores-client".into(),
            client_secret: SECRET.into(),
            timeout: Duration::from_secs(1),
            max_concurrent: 1,
            ca_cert_file: None,
        }
    }

    fn captured(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for HttpPeer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn read_request(socket: &mut TcpStream) -> Option<CapturedRequest> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let end = loop {
        let count = socket.read(&mut chunk).ok()?;
        if count == 0 || bytes.len() + count > 64 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(offset) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            break offset + 4;
        }
    };
    let head = std::str::from_utf8(&bytes[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers: BTreeMap<_, _> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .ok()?
        .unwrap_or(0);
    if length > 1024 * 1024 {
        return None;
    }
    while bytes.len() - end < length {
        let count = socket.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Some(CapturedRequest {
        method,
        target,
        headers,
        body: bytes[end..end + length].to_vec(),
    })
}

fn reply(status: u16, content_type: Option<&str>, body: &[u8]) -> Vec<u8> {
    let media = content_type
        .map(|value| format!("Content-Type: {value}\r\n"))
        .unwrap_or_default();
    let mut bytes = format!(
        "HTTP/1.1 {status} Test\r\n{media}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn query(request: &CapturedRequest) -> BTreeMap<String, String> {
    let target = url::Url::parse(&format!("http://loopback.test{}", request.target)).unwrap();
    target.query_pairs().into_owned().collect()
}

fn assert_safe(error: ApiError) {
    let text = format!("{error} {error:?} {}", error.message());
    for private in [TOKEN, SECRET, "PRIVATE_UPSTREAM_DETAIL", "private.invalid"] {
        assert!(!text.contains(private));
    }
}

#[derive(Clone, Copy)]
enum CallKind {
    Introspection,
    Context,
    Member,
    Members,
}

async fn call(
    client: &PlatformClient,
    kind: CallKind,
    request_id: &str,
) -> Result<Value, ApiError> {
    let organization = Uuid::parse_str(ORGANIZATION).unwrap();
    match kind {
        CallKind::Introspection => client.introspect(TOKEN, request_id).await,
        CallKind::Context => client.context(TOKEN, request_id).await,
        CallKind::Member => {
            client
                .member(
                    TOKEN,
                    Resource::Store,
                    Action::Update,
                    organization,
                    Uuid::parse_str(USER).unwrap(),
                    request_id,
                )
                .await
        }
        CallKind::Members => {
            client
                .members(
                    TOKEN,
                    Resource::Staff,
                    Action::Assign,
                    organization,
                    2,
                    50,
                    "李 四&scope=TENANT",
                    request_id,
                )
                .await
        }
    }
}

#[tokio::test]
async fn requests_use_fixed_stores_paths_and_the_correct_basic_or_bearer_identity() {
    let peer = HttpPeer::start(reply(200, Some("application/json"), b"{}"));
    let client = PlatformClient::new(&peer.settings()).unwrap();
    for kind in [
        CallKind::Introspection,
        CallKind::Context,
        CallKind::Member,
        CallKind::Members,
    ] {
        assert_eq!(call(&client, kind, REQUEST_ID).await.unwrap(), json!({}));
    }
    let requests = peer.captured();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].target, "/oauth/introspect");
    assert_eq!(
        requests[0].headers["authorization"],
        "Basic c3RvcmVzLWNsaWVudDpDTElFTlRfU0VDUkVUX05PVF9GT1JfTE9HUw=="
    );
    assert_eq!(
        requests[0].headers["content-type"],
        "application/x-www-form-urlencoded"
    );
    let form: BTreeMap<_, _> = url::form_urlencoded::parse(&requests[0].body)
        .into_owned()
        .collect();
    assert_eq!(form, BTreeMap::from([("token".into(), TOKEN.into())]));
    assert_eq!(requests[1].target, "/api/apps/stores/context");
    assert_eq!(
        requests[2].target.split('?').next().unwrap(),
        format!("/api/apps/stores/members/{USER}")
    );
    assert_eq!(
        query(&requests[2]),
        BTreeMap::from([
            ("resource".into(), "store".into()),
            ("action".into(), "update".into()),
            ("organizationId".into(), ORGANIZATION.into()),
        ])
    );
    assert_eq!(
        requests[3].target.split('?').next().unwrap(),
        "/api/apps/stores/members"
    );
    assert_eq!(
        query(&requests[3]),
        BTreeMap::from([
            ("resource".into(), "store-staff".into()),
            ("action".into(), "assign".into()),
            ("organizationId".into(), ORGANIZATION.into()),
            ("page".into(), "2".into()),
            ("size".into(), "50".into()),
            ("q".into(), "李 四&scope=TENANT".into()),
        ])
    );
    for request in &requests {
        assert_eq!(request.headers["accept"], "application/json");
        assert_eq!(request.headers["x-request-id"], REQUEST_ID.to_lowercase());
        assert!(!request.target.contains("work-orders"));
        assert!(!request.target.contains(TOKEN));
    }
    for request in &requests[1..] {
        assert_eq!(request.method, "GET");
        assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
        assert!(request.body.is_empty());
        assert!(!format!("{:?}", request.headers).contains(SECRET));
    }
}

#[tokio::test]
async fn arbitrary_request_ids_are_omitted_instead_of_reflected_in_headers() {
    let peer = HttpPeer::start(reply(200, Some("application/json"), b"{}"));
    let client = PlatformClient::new(&peer.settings()).unwrap();
    for request_id in [
        "not-a-uuid",
        "\r\nx-private: APP_TOKEN_NOT_FOR_LOGS",
        "aaaaaaaa666646668666666666666666",
    ] {
        call(&client, CallKind::Context, request_id).await.unwrap();
    }
    let requests = peer.captured();
    assert_eq!(requests.len(), 3);
    assert!(requests
        .iter()
        .all(|request| !request.headers.contains_key("x-request-id")));
}

#[tokio::test]
async fn upstream_statuses_map_to_fixed_errors_without_echoing_private_bodies() {
    let body = format!("{TOKEN} {SECRET} PRIVATE_UPSTREAM_DETAIL https://private.invalid");
    for kind in [
        CallKind::Introspection,
        CallKind::Context,
        CallKind::Member,
        CallKind::Members,
    ] {
        for (status, expected_status, expected_code) in [
            (400, 400, "VALIDATION_ERROR"),
            (401, 401, "INVALID_APP_TOKEN"),
            (403, 403, "DATA_DENIED"),
            (
                404,
                if matches!(kind, CallKind::Member) {
                    404
                } else {
                    503
                },
                if matches!(kind, CallKind::Member) {
                    "MEMBER_NOT_FOUND"
                } else {
                    "PLATFORM_UNAVAILABLE"
                },
            ),
            (201, 503, "PLATFORM_UNAVAILABLE"),
            (204, 503, "PLATFORM_UNAVAILABLE"),
            (429, 503, "PLATFORM_UNAVAILABLE"),
            (500, 503, "PLATFORM_UNAVAILABLE"),
        ] {
            let peer = HttpPeer::start(reply(status, Some("application/json"), body.as_bytes()));
            let client = PlatformClient::new(&peer.settings()).unwrap();
            let error = call(&client, kind, REQUEST_ID).await.unwrap_err();
            assert_eq!(error.status(), expected_status);
            assert_eq!(error.code(), expected_code);
            assert_safe(error);
        }
    }
}

#[tokio::test]
async fn successful_http_requires_json_media_type_and_well_formed_json() {
    for (media, body) in [
        (None, b"{}".as_slice()),
        (Some("text/html"), b"{}".as_slice()),
        (Some("application/problem+json"), b"{}".as_slice()),
        (Some("application/jsonx"), b"{}".as_slice()),
        (
            Some("application/json"),
            b"PRIVATE_UPSTREAM_DETAIL".as_slice(),
        ),
        (Some("application/json"), b"".as_slice()),
    ] {
        let peer = HttpPeer::start(reply(200, media, body));
        let client = PlatformClient::new(&peer.settings()).unwrap();
        let error = call(&client, CallKind::Context, REQUEST_ID)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "PLATFORM_UNAVAILABLE");
        assert_safe(error);
    }
    let peer = HttpPeer::start(reply(200, Some("Application/JSON; charset=utf-8"), b"{}"));
    let client = PlatformClient::new(&peer.settings()).unwrap();
    assert_eq!(
        call(&client, CallKind::Context, REQUEST_ID).await.unwrap(),
        json!({})
    );
}

fn json_of_size(length: usize) -> Vec<u8> {
    let mut body = Vec::with_capacity(length);
    body.push(b'"');
    body.resize(length - 1, b'a');
    body.push(b'"');
    body
}

fn chunked(body: &[u8]) -> Vec<u8> {
    let mut bytes = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    // Separate chunks ensure cumulative size is checked as well as chunk size.
    for part in body.chunks(4096) {
        bytes.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
        bytes.extend_from_slice(part);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(b"0\r\n\r\n");
    bytes
}

#[tokio::test]
async fn body_limit_checks_declared_lengths_and_cumulative_chunked_bytes() {
    let oversized_head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_BODY + 1).into_bytes();
    for bytes in [oversized_head, chunked(&json_of_size(MAX_BODY + 1))] {
        let peer = HttpPeer::start(bytes);
        let client = PlatformClient::new(&peer.settings()).unwrap();
        assert_eq!(
            call(&client, CallKind::Context, REQUEST_ID)
                .await
                .unwrap_err()
                .code(),
            "PLATFORM_UNAVAILABLE"
        );
    }
    for bytes in [
        reply(200, Some("application/json"), &json_of_size(MAX_BODY)),
        chunked(&json_of_size(MAX_BODY)),
    ] {
        let peer = HttpPeer::start(bytes);
        let client = PlatformClient::new(&peer.settings()).unwrap();
        let value = call(&client, CallKind::Context, REQUEST_ID).await.unwrap();
        assert_eq!(value.as_str().unwrap().len(), MAX_BODY - 2);
    }
}

#[tokio::test]
async fn redirects_cannot_forward_basic_credentials_or_bearer_tokens() {
    let target = HttpPeer::start(reply(200, Some("application/json"), b"{}"));
    for (kind, status) in [(CallKind::Introspection, 307), (CallKind::Context, 302)] {
        let wire = format!("HTTP/1.1 {status} Redirect\r\nLocation: {}/escaped\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", target.origin).into_bytes();
        let peer = HttpPeer::start(wire);
        let client = PlatformClient::new(&peer.settings()).unwrap();
        let error = call(&client, kind, REQUEST_ID).await.unwrap_err();
        assert_eq!(error.code(), "PLATFORM_UNAVAILABLE");
        assert_safe(error);
        assert_eq!(peer.captured().len(), 1);
    }
    assert!(target.captured().is_empty());
}

#[tokio::test]
async fn connection_failure_is_not_retried_and_body_reads_use_the_call_timeout() {
    let peer = HttpPeer::start(Vec::new());
    let client = PlatformClient::new(&peer.settings()).unwrap();
    let error = call(&client, CallKind::Introspection, REQUEST_ID)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "PLATFORM_UNAVAILABLE");
    assert_safe(error);
    assert_eq!(peer.captured().len(), 1);

    let slow = HttpPeer::with_delayed_tail(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{".to_vec(),
        Some((Duration::from_millis(250), b"}".to_vec())),
    );
    let mut settings = slow.settings();
    settings.timeout = Duration::from_millis(50);
    let client = PlatformClient::new(&settings).unwrap();
    let error = call(&client, CallKind::Context, REQUEST_ID)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "PLATFORM_UNAVAILABLE");
    assert_safe(error);
}

#[test]
fn additional_ca_failures_are_bounded_and_sanitized() {
    let peer = HttpPeer::start(reply(200, Some("application/json"), b"{}"));
    let mut settings = peer.settings();
    let path = std::env::temp_dir().join(format!("stores-private-ca-{}.pem", Uuid::new_v4()));
    settings.ca_cert_file = Some(path.clone());
    for bytes in [
        None,
        Some(b"PRIVATE_UPSTREAM_DETAIL".to_vec()),
        Some(vec![b'a'; MAX_CA as usize + 1]),
    ] {
        if let Some(bytes) = bytes {
            std::fs::write(&path, bytes).unwrap();
        }
        let error = PlatformClient::new(&settings).err().unwrap();
        let source = std::error::Error::source(&error).unwrap();
        let rendered = format!("{error} {error:?} {source} {source:?}");
        for private in [path.to_str().unwrap(), "PRIVATE_UPSTREAM_DETAIL", SECRET] {
            assert!(!rendered.contains(private));
        }
    }
    std::fs::remove_file(path).unwrap();
}
