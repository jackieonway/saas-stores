//! Optional loopback listener for hosts that block inbound Rust test traffic.
//!
//! Set `STORES_TEST_NODE` to an existing Node executable. Only the listener moves
//! to that child process; requests still use the production Rust HTTP client and
//! the original Rust fixture handlers and assertions.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
pub(crate) struct Request {
    pub method: String,
    pub target: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

type Reply = (Vec<u8>, Option<(Duration, Vec<u8>)>);

pub(crate) struct NodePeer {
    pub origin: String,
    child: Child,
    reader: Option<thread::JoinHandle<()>>,
}

impl NodePeer {
    pub fn enabled() -> bool {
        std::env::var_os("STORES_TEST_NODE").is_some()
    }

    pub fn start(mut respond: impl FnMut(Request) -> Reply + Send + 'static) -> Self {
        let node = std::env::var_os("STORES_TEST_NODE").expect("STORES_TEST_NODE is configured");
        let mut command = Command::new(node);
        command
            .arg("-e")
            .arg(include_str!("test_http_node.cjs"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().expect("start optional Node HTTP fixture");
        let output = child.stdout.take().expect("fixture output pipe");
        let mut input = child.stdin.take().expect("fixture input pipe");
        let (ready, startup) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                let event: Value = serde_json::from_str(&line).expect("fixture event JSON");
                if let Some(origin) = event["origin"].as_str() {
                    ready
                        .send(origin.to_owned())
                        .expect("fixture startup receiver");
                    continue;
                }
                let request = serde_json::from_value(event["request"].clone())
                    .expect("fixture request shape");
                // Record and handle the request before acknowledging it, so
                // immediate request-count and header assertions remain exact.
                let (bytes, tail) = respond(request);
                let tail = tail.map(
                    |(delay, bytes)| json!({ "delayMs": delay.as_millis() as u64, "bytes": bytes }),
                );
                // Buffer the command so large-body fixtures do not perform a
                // pipe syscall for every JSON byte-array element.
                let mut command =
                    serde_json::to_vec(&json!({ "id": event["id"], "bytes": bytes, "tail": tail }))
                        .unwrap();
                command.push(b'\n');
                if input
                    .write_all(&command)
                    .and_then(|_| input.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut fixture = Self {
            origin: String::new(),
            child,
            reader: Some(reader),
        };
        fixture.origin = startup
            .recv_timeout(Duration::from_secs(10))
            .expect("Node HTTP fixture must report its listening origin");
        assert!(fixture.origin.starts_with("http://127.0.0.1:"));
        fixture
    }

    pub fn router(app: axum::Router) -> Self {
        use axum::{body::Body, http::Request as HttpRequest};
        use tower::ServiceExt;

        // Own the timer driver on the bridge thread. A timed-out client may
        // drop the fixture while its handler is still awaiting a test delay.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        Self::start(move |request| {
            let mut builder = HttpRequest::builder()
                .method(request.method.as_str())
                .uri(request.target);
            for (name, value) in request.headers {
                builder = builder.header(name, value);
            }
            let request = builder.body(Body::from(request.body)).unwrap();
            let response = runtime.block_on(async {
                let response = app.clone().oneshot(request).await.unwrap();
                let (parts, body) = response.into_parts();
                let body = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
                let mut wire = format!("HTTP/1.1 {} Test\r\n", parts.status.as_u16());
                for (name, value) in &parts.headers {
                    if !matches!(
                        name.as_str(),
                        "content-length" | "connection" | "transfer-encoding"
                    ) {
                        wire.push_str(name.as_str());
                        wire.push_str(": ");
                        wire.push_str(value.to_str().unwrap());
                        wire.push_str("\r\n");
                    }
                }
                wire.push_str(&format!(
                    "Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                ));
                let mut wire = wire.into_bytes();
                wire.extend_from_slice(&body);
                wire
            });
            (response, None)
        })
    }
}

impl Drop for NodePeer {
    fn drop(&mut self) {
        // This child is exclusively owned by this fixture, with no persistent
        // files. Terminate it before joining the pipe reader, including on panic.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            reader.join().expect("Node HTTP fixture reader");
        }
    }
}
