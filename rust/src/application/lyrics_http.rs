//! HTTP transport for lyrics providers.
//!
//! Production requests use reqwest 0.13 with rustls + rustls-platform-verifier
//! (system CA trust). Each request wraps client construction (or a cheap Client
//! clone), `send`, and the full body read in one `async { ... }.compat().await`
//! future, polled by `async_global_executor`. Response bodies are never logged.

#[cfg(test)]
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_compat::CompatExt;

const USER_AGENT: &str = "mofu/0.1.0 (https://github.com/Sherlock-Holo/mofu)";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const MAX_BODY_BYTES: u64 = 1_048_576;
const MAX_REDIRECTS: usize = 5;

/// A GET request issued by a lyrics provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricsHttpRequest {
    pub url: String,
    pub query: Vec<(String, String)>,
}

impl LyricsHttpRequest {
    pub(crate) fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            query: Vec::new(),
        }
    }

    pub(crate) fn with_query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.push((key.into(), value.into()));
        self
    }
}

/// HTTP response with a size-capped body. The body is not logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LyricsHttpResponse {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: Vec<u8>,
}

/// Transport and protocol failures that can be shown to the user (no lyric text).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LyricsHttpError {
    #[error("歌词请求超时")]
    Timeout,
    #[error("歌词响应过大（超过 {limit} 字节）")]
    TooLarge { limit: u64 },
    #[error("歌词来源请求过于频繁，请{suffix}")]
    RateLimited {
        suffix: String,
        retry_after: Option<u64>,
    },
    #[error("歌词来源返回 HTTP {status}")]
    Status { status: u16 },
    #[error("无法建立安全的歌词来源连接")]
    Tls,
    #[error("歌词来源暂时不可用")]
    Network,
    #[error("歌词来源响应无效")]
    InvalidResponse,
}

impl LyricsHttpError {
    fn from_reqwest(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            return Self::Timeout;
        }
        let debug = format!("{err:?}").to_ascii_lowercase();
        if debug.contains("tls")
            || debug.contains("certificate")
            || debug.contains("dnsname")
            || debug.contains("webpki")
        {
            Self::Tls
        } else if err.is_body() || err.is_decode() {
            Self::InvalidResponse
        } else {
            Self::Network
        }
    }

    pub(crate) fn user_message(&self) -> String {
        self.to_string()
    }

    pub(crate) fn rate_limited(retry_after: Option<u64>) -> Self {
        let suffix = match retry_after {
            Some(seconds) if seconds > 0 => format!("{seconds} 秒后重试"),
            _ => "稍后重试".to_owned(),
        };
        Self::RateLimited {
            suffix,
            retry_after,
        }
    }
}

/// Production or test HTTP backend used by remote lyrics providers.
#[derive(Debug)]
pub(crate) enum LyricsHttp {
    Reqwest(ReqwestTransport),
    #[cfg(test)]
    Mock(MockTransport),
}

impl LyricsHttp {
    pub(crate) fn production() -> Self {
        Self::Reqwest(ReqwestTransport::new())
    }

    pub(crate) async fn get(
        &self,
        request: LyricsHttpRequest,
    ) -> Result<LyricsHttpResponse, LyricsHttpError> {
        match self {
            Self::Reqwest(transport) => {
                async {
                    let client = transport.client()?;
                    fetch_reqwest(&client, request).await
                }
                .compat()
                .await
            }
            #[cfg(test)]
            Self::Mock(transport) => transport.fetch(request),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ReqwestTransport {
    client: Mutex<Option<reqwest::Client>>,
}

impl ReqwestTransport {
    fn new() -> Self {
        Self {
            client: Mutex::new(None),
        }
    }

    fn client(&self) -> Result<reqwest::Client, LyricsHttpError> {
        let mut slot = self.client.lock().expect("lyrics http client mutex");
        if slot.is_none() {
            *slot = Some(build_client()?);
        }
        Ok(slot.as_ref().expect("client just inserted").clone())
    }
}

fn build_client() -> Result<reqwest::Client, LyricsHttpError> {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .user_agent(USER_AGENT)
        .build()
        .map_err(LyricsHttpError::from_reqwest)
}

async fn fetch_reqwest(
    client: &reqwest::Client,
    request: LyricsHttpRequest,
) -> Result<LyricsHttpResponse, LyricsHttpError> {
    let mut builder = client.get(&request.url);
    if !request.query.is_empty() {
        builder = builder.query(&request.query);
    }
    let mut response = builder
        .send()
        .await
        .map_err(LyricsHttpError::from_reqwest)?;
    let status = response.status().as_u16();
    let retry_after = parse_retry_after(response.headers().get("retry-after"));
    if let Some(length) = response.content_length()
        && length > MAX_BODY_BYTES
    {
        return Err(LyricsHttpError::TooLarge {
            limit: MAX_BODY_BYTES,
        });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(LyricsHttpError::from_reqwest)?
    {
        if chunk.len() > MAX_BODY_BYTES as usize - body.len() {
            return Err(LyricsHttpError::TooLarge {
                limit: MAX_BODY_BYTES,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(LyricsHttpResponse {
        status,
        retry_after,
        body,
    })
}

fn parse_retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<u64> {
    let raw = value?.to_str().ok()?.trim();
    raw.parse::<u64>().ok()
}

/// Interpret a completed HTTP response: 429, other errors, or the body bytes.
pub(crate) fn require_success(response: LyricsHttpResponse) -> Result<Vec<u8>, LyricsHttpError> {
    match response.status {
        200 | 201 => Ok(response.body),
        204 => Ok(Vec::new()),
        429 => Err(LyricsHttpError::rate_limited(response.retry_after)),
        status => Err(LyricsHttpError::Status { status }),
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct MockTransport {
    routes: Arc<Mutex<Vec<MockRoute>>>,
    hits: Arc<Mutex<Vec<String>>>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct MockRoute {
    url_contains: String,
    query_contains: Vec<(String, String)>,
    result: Result<LyricsHttpResponse, LyricsHttpError>,
}

#[cfg(test)]
impl MockTransport {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn enqueue(
        &self,
        url_contains: impl Into<String>,
        query_contains: Vec<(String, String)>,
        result: Result<LyricsHttpResponse, LyricsHttpError>,
    ) {
        self.routes.lock().expect("mock routes").push(MockRoute {
            url_contains: url_contains.into(),
            query_contains,
            result,
        });
    }

    pub(crate) fn enqueue_ok(
        &self,
        url_contains: impl Into<String>,
        query_contains: Vec<(String, String)>,
        body: impl AsRef<[u8]>,
    ) {
        self.enqueue(
            url_contains,
            query_contains,
            Ok(LyricsHttpResponse {
                status: 200,
                retry_after: None,
                body: body.as_ref().to_vec(),
            }),
        );
    }

    pub(crate) fn hits(&self) -> Vec<String> {
        self.hits.lock().expect("mock hits").clone()
    }

    fn fetch(&self, request: LyricsHttpRequest) -> Result<LyricsHttpResponse, LyricsHttpError> {
        let rendered = render_request(&request);
        self.hits.lock().expect("mock hits").push(rendered);
        let mut routes = self.routes.lock().expect("mock routes");
        let index = routes.iter().position(|route| {
            request.url.contains(&route.url_contains)
                && route.query_contains.iter().all(|(key, value)| {
                    request
                        .query
                        .iter()
                        .any(|(query_key, query_value)| query_key == key && query_value == value)
                })
        });
        match index {
            Some(index) => routes.remove(index).result,
            None => Err(LyricsHttpError::Status { status: 404 }),
        }
    }
}

#[cfg(test)]
fn render_request(request: &LyricsHttpRequest) -> String {
    if request.query.is_empty() {
        return request.url.clone();
    }
    let query = request
        .query
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{query}", request.url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn too_large_and_rate_limit_messages_do_not_include_bodies() {
        let large = LyricsHttpError::TooLarge {
            limit: MAX_BODY_BYTES,
        };
        assert!(large.user_message().contains("过大"));
        assert!(!large.user_message().contains('['));
        let limited = LyricsHttpError::rate_limited(Some(30));
        assert!(limited.user_message().contains("30"));
        assert!(limited.user_message().contains("频繁"));
    }

    #[test]
    fn require_success_maps_429_without_exposing_body() {
        let err = require_success(LyricsHttpResponse {
            status: 429,
            retry_after: Some(12),
            body: b"too many".to_vec(),
        })
        .expect_err("429");
        assert!(matches!(
            err,
            LyricsHttpError::RateLimited {
                retry_after: Some(12),
                ..
            }
        ));
        assert!(!err.user_message().contains("too many"));
    }
    #[test]
    fn chunked_body_stops_at_limit_without_waiting_for_end() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::mpsc;
        use std::thread;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (finished, stop) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0, "request ended before headers");
                request.extend_from_slice(&buffer[..count]);
                assert!(request.len() <= 8192, "test request headers too large");
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let chunk = vec![b'x'; 16_384];
            for _ in 0..65 {
                if write!(socket, "{:x}\r\n", chunk.len())
                    .and_then(|_| socket.write_all(&chunk))
                    .and_then(|_| socket.write_all(b"\r\n"))
                    .is_err()
                {
                    break;
                }
            }
            // Never finish the response: a full-body reader would time out.
            let _ = stop.recv_timeout(Duration::from_secs(3));
        });
        let result = async_global_executor::block_on(
            async {
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(1))
                    .build()
                    .unwrap();
                fetch_reqwest(
                    &client,
                    LyricsHttpRequest::new(format!("http://{address}/chunked")),
                )
                .await
            }
            .compat(),
        );
        let _ = finished.send(());
        server.join().unwrap();
        assert_eq!(
            result,
            Err(LyricsHttpError::TooLarge {
                limit: MAX_BODY_BYTES
            })
        );
    }
}
