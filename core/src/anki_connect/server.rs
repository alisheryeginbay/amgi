//! A small HTTP server for the AnkiConnect API on 127.0.0.1.
//!
//! Only this Mac can reach it, and only browser extensions and pages on localhost
//! may call it from a browser: a request from any other web page is turned away, as
//! is one whose `Host` isn't a loopback name, which a DNS rebinding attack would send.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::{AnkiConnectDelegate, VERSION, handle};
use crate::{AmgiError, CardCollection};

/// Requests carry media as base64, so allow large ones.
const MAX_BODY: usize = 256 << 20;
const MAX_HEADER_LINE: usize = 16 << 10;
const MAX_HEADERS: usize = 100;

/// Answers AnkiConnect requests for a collection until stopped or dropped.
#[derive(uniffi::Object)]
pub struct AnkiConnectServer {
    port: u16,
    stopped: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

#[uniffi::export]
impl AnkiConnectServer {
    /// Listen on 127.0.0.1 at `port`; 0 picks a free one.
    #[uniffi::constructor]
    pub fn start(
        collection: Arc<CardCollection>,
        port: u16,
        delegate: Arc<dyn AnkiConnectDelegate>,
    ) -> Result<Arc<Self>, AmgiError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|e| AmgiError::AnkiConnect {
            message: if e.kind() == std::io::ErrorKind::AddrInUse {
                format!("Port {port} is in use, perhaps by Anki with AnkiConnect. Quit it, then turn this on again.")
            } else {
                format!("Couldn't listen on port {port}: {e}.")
            },
        })?;
        let port = listener.local_addr()?.port();
        let stopped = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let stopped = stopped.clone();
            move || {
                for stream in listener.incoming() {
                    if stopped.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let (collection, delegate) = (collection.clone(), delegate.clone());
                    thread::spawn(move || serve(stream, &collection, delegate.as_ref()));
                }
            }
        });
        Ok(Arc::new(Self { port, stopped, thread: Mutex::new(Some(thread)) }))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stop listening. Requests already being answered finish.
    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        // Wake the accept loop so it sees it's stopped.
        let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, self.port));
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
}

impl Drop for AnkiConnectServer {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Request {
    method: String,
    origin: Option<String>,
    host: Option<String>,
    request_headers: Option<String>,
    body: Vec<u8>,
}

fn serve(stream: TcpStream, collection: &CardCollection, delegate: &dyn AnkiConnectDelegate) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut reader = BufReader::new(&stream);
    let response = match read_request(&mut reader) {
        Ok(request) => respond(&request, collection, delegate),
        Err(status) => Response::text(status, status),
    };
    let mut stream = &stream;
    let _ = stream.write_all(&response.bytes());
    let _ = stream.flush();
}

fn read_line(reader: &mut impl BufRead) -> Result<String, &'static str> {
    let mut line = Vec::new();
    reader
        .by_ref()
        .take(MAX_HEADER_LINE as u64)
        .read_until(b'\n', &mut line)
        .map_err(|_| "400 Bad Request")?;
    if !line.ends_with(b"\n") {
        return Err("400 Bad Request");
    }
    String::from_utf8(line).map(|l| l.trim_end().to_string()).map_err(|_| "400 Bad Request")
}

fn read_request(reader: &mut impl BufRead) -> Result<Request, &'static str> {
    let line = read_line(reader)?;
    let method = line.split(' ').next().unwrap_or_default().to_string();
    let mut request = Request { method, origin: None, host: None, request_headers: None, body: Vec::new() };
    let mut length = 0;
    for n in 0.. {
        let line = read_line(reader)?;
        if line.is_empty() {
            break;
        }
        if n == MAX_HEADERS {
            return Err("431 Request Header Fields Too Large");
        }
        let Some((name, value)) = line.split_once(':') else { return Err("400 Bad Request") };
        let value = value.trim().to_string();
        match name.trim().to_ascii_lowercase().as_str() {
            "origin" => request.origin = Some(value),
            "host" => request.host = Some(value),
            "access-control-request-headers" => request.request_headers = Some(value),
            "content-length" => length = value.parse().map_err(|_| "400 Bad Request")?,
            "transfer-encoding" => return Err("411 Length Required"),
            _ => {}
        }
    }
    if length > MAX_BODY {
        return Err("413 Content Too Large");
    }
    request.body = vec![0; length];
    reader.read_exact(&mut request.body).map_err(|_| "400 Bad Request")?;
    Ok(request)
}

fn respond(request: &Request, collection: &CardCollection, delegate: &dyn AnkiConnectDelegate) -> Response {
    if !request.host.as_deref().is_none_or(loopback_host) || !request.origin.as_deref().is_none_or(allowed_origin) {
        return Response::text("403 Forbidden", "Only browser extensions and pages on this Mac can use amgi's AnkiConnect.");
    }
    let mut response = match request.method.as_str() {
        "OPTIONS" => {
            let mut response = Response::text("204 No Content", "");
            response.headers.push(("Access-Control-Allow-Methods", "POST, GET, OPTIONS".into()));
            let headers = request.request_headers.clone().unwrap_or_else(|| "Content-Type".into());
            response.headers.push(("Access-Control-Allow-Headers", headers));
            response.headers.push(("Access-Control-Allow-Private-Network", "true".into()));
            response.headers.push(("Access-Control-Max-Age", "600".into()));
            response
        }
        "GET" => Response::text("200 OK", &format!("AnkiConnect v.{VERSION}")),
        "POST" => match std::str::from_utf8(&request.body) {
            Ok(body) => Response {
                status: "200 OK",
                content_type: "application/json",
                headers: Vec::new(),
                body: handle(collection, body, delegate).into_bytes(),
            },
            Err(_) => Response::text("400 Bad Request", "The request isn't UTF-8."),
        },
        _ => Response::text("405 Method Not Allowed", "405 Method Not Allowed"),
    };
    if let Some(origin) = &request.origin {
        response.headers.push(("Access-Control-Allow-Origin", origin.clone()));
        response.headers.push(("Vary", "Origin".into()));
    }
    response
}

/// `localhost`, `127.0.0.1` or `[::1]`, with or without a port.
fn loopback_host(host: &str) -> bool {
    let name = match host.rsplit_once(':') {
        Some((name, port)) if !name.is_empty() && port.chars().all(|c| c.is_ascii_digit()) && !host.ends_with(']') => name,
        _ => host,
    };
    matches!(name.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

/// A browser extension, or a page served from this Mac.
fn allowed_origin(origin: &str) -> bool {
    let Some((scheme, rest)) = origin.split_once("://") else { return false };
    match scheme.to_ascii_lowercase().as_str() {
        "chrome-extension" | "moz-extension" | "safari-web-extension" | "extension" => true,
        "http" | "https" => !rest.contains('/') && loopback_host(rest),
        _ => false,
    }
}

struct Response {
    status: &'static str,
    content_type: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Response {
    fn text(status: &'static str, body: &str) -> Self {
        Response { status, content_type: "text/plain; charset=utf-8", headers: Vec::new(), body: body.as_bytes().to_vec() }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut head = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            self.content_type,
            self.body.len()
        );
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_origins() {
        assert!(loopback_host("127.0.0.1:8765"));
        assert!(loopback_host("localhost"));
        assert!(loopback_host("[::1]:8765"));
        assert!(!loopback_host("evil.example:8765"));
        assert!(!loopback_host("127.0.0.1.evil.example"));

        assert!(allowed_origin("chrome-extension://abcdef"));
        assert!(allowed_origin("moz-extension://1234-5678"));
        assert!(allowed_origin("http://localhost:3000"));
        assert!(allowed_origin("http://127.0.0.1"));
        assert!(!allowed_origin("https://example.com"));
        assert!(!allowed_origin("http://localhost.example.com"));
        assert!(!allowed_origin("null"));
    }

    #[test]
    fn requests() {
        let read = |raw: &str| read_request(&mut raw.as_bytes()).map(|r| (r.method, r.origin, String::from_utf8(r.body).unwrap()));
        assert_eq!(
            read("POST / HTTP/1.1\r\nHost: 127.0.0.1:8765\r\nOrigin: chrome-extension://x\r\nContent-Length: 2\r\n\r\n{}"),
            Ok(("POST".into(), Some("chrome-extension://x".into()), "{}".into()))
        );
        assert_eq!(read("POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\n{}").map(|_| ()), Err("400 Bad Request"));
        assert_eq!(read("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").map(|_| ()), Err("411 Length Required"));
    }
}
