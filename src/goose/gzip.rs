//! Gzip response bodies, decoded only as the load test reads them.
//!
//! Goose builds its clients with reqwest's gzip decoding off, asks for gzip itself
//! (see [`GooseUser::request`](super::GooseUser::request)), and decodes a gzip body
//! in [`GooseHttpResponse`](super::GooseHttpResponse) when the test reads it with
//! `chunk`, `bytes`, `text` or `json`. A body read only to be thrown away, by
//! [`GooseResponse::discard_body`](super::GooseResponse::discard_body) or because the
//! test dropped it unread, is read as it came over the wire and never decoded.
//!
//! The decoding matches what reqwest's did, byte for byte: a body is decoded when
//! its first `Content-Encoding` header is exactly `gzip`, only its first gzip member
//! is decoded and anything after that member is ignored (reqwest 0.13 through
//! tower-http 0.6 decodes gzip without `multiple_members`), a body cut short or
//! failing its CRC is an `is_decode()` error, an empty body decodes to nothing, and
//! `Content-Encoding` and `Content-Length` are removed from the headers the response
//! exposes. Unlike reqwest's, the bytes after the first member are still read, so
//! the connection can be reused.

use bytes::Bytes;
use flate2::write::GzDecoder;
use reqwest::header::{HeaderValue, CONTENT_ENCODING, CONTENT_LENGTH};
use reqwest::Response;
use std::io::{self, Write};
use std::pin::Pin;
use std::task::{Context, Poll};

#[cfg(test)]
thread_local! {
    /// How many times this thread fed body bytes to a gzip decoder, so unit tests can
    /// check that a body read only to be thrown away is never decoded. Thread local:
    /// each `#[tokio::test]` runs on its own thread, so tests running at the same time
    /// do not count each other's.
    pub(crate) static GZIP_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The state of a gzip body Goose decodes as it is read: the headers taken out of
/// the response, and the decoder, made when the first byte is decoded.
#[derive(Debug)]
pub(super) struct GzipBody {
    /// The `Content-Encoding` values taken out of the response's headers.
    content_encoding: Vec<HeaderValue>,
    /// The `Content-Length` values taken out of the response's headers.
    content_length: Vec<HeaderValue>,
    /// Decodes the body into its `Vec`; `None` until the first chunk is decoded.
    decoder: Option<GzDecoder<Vec<u8>>>,
    /// The first gzip member ended: what follows is read and ignored.
    member_ended: bool,
    /// The body was read to its end, or an error ended it.
    ended: bool,
}

impl GzipBody {
    /// When `response` has a gzip body, takes `Content-Encoding` and `Content-Length`
    /// out of its headers, as reqwest did when it decoded the body, and returns the
    /// state to decode it with.
    pub(super) fn take_from(response: &mut Response) -> Option<Box<GzipBody>> {
        let headers = response.headers_mut();
        if headers.get(CONTENT_ENCODING).map(HeaderValue::as_bytes) != Some(b"gzip") {
            return None;
        }
        let content_encoding = headers.get_all(CONTENT_ENCODING).iter().cloned().collect();
        let content_length = headers.get_all(CONTENT_LENGTH).iter().cloned().collect();
        headers.remove(CONTENT_ENCODING);
        headers.remove(CONTENT_LENGTH);
        Some(Box::new(GzipBody {
            content_encoding,
            content_length,
            decoder: None,
            member_ended: false,
            ended: false,
        }))
    }

    /// Puts the headers [`take_from`](Self::take_from) took back into `response`,
    /// which then describe the body as it came over the wire.
    pub(super) fn restore(self, response: &mut Response) {
        let headers = response.headers_mut();
        for value in self.content_encoding {
            headers.append(CONTENT_ENCODING, value);
        }
        for value in self.content_length {
            headers.append(CONTENT_LENGTH, value);
        }
    }

    /// Reads the next chunk of `response`'s body and returns what it decodes to,
    /// reading on until a chunk decodes to at least one byte. `None` at the end of
    /// the body. A body that does not decode returns an error, as reqwest's
    /// decoding did, and nothing more after it.
    pub(super) async fn chunk(
        &mut self,
        response: &mut Response,
    ) -> reqwest::Result<Option<Bytes>> {
        while !self.ended {
            let raw = match response.chunk().await {
                Ok(raw) => raw,
                Err(error) => {
                    self.ended = true;
                    return Err(error);
                }
            };
            let decoder = match raw {
                // Past the first member: read on, so the connection can be reused.
                Some(_) if self.member_ended => continue,
                Some(raw) => {
                    #[cfg(test)]
                    GZIP_DECODES.with(|decodes| decodes.set(decodes.get() + 1));
                    let decoder = self
                        .decoder
                        .get_or_insert_with(|| GzDecoder::new(Vec::new()));
                    if let Err(error) = decode(decoder, &raw, &mut self.member_ended) {
                        self.ended = true;
                        return Err(decode_error(error).await);
                    }
                    decoder
                }
                None => {
                    self.ended = true;
                    // An empty body, such as a HEAD response's, decodes to nothing.
                    let Some(decoder) = self.decoder.as_mut() else {
                        return Ok(None);
                    };
                    if let Err(error) = decoder.try_finish() {
                        return Err(decode_error(error).await);
                    }
                    decoder
                }
            };
            let decoded = std::mem::take(decoder.get_mut());
            if !decoded.is_empty() {
                return Ok(Some(Bytes::from(decoded)));
            }
        }
        Ok(None)
    }

    /// Reads the rest of `response`'s body and returns all it decodes to.
    pub(super) async fn read_to_end(&mut self, response: &mut Response) -> reqwest::Result<Bytes> {
        let mut body = Vec::new();
        while let Some(chunk) = self.chunk(response).await? {
            if body.is_empty() {
                body = Vec::from(chunk);
            } else {
                body.extend_from_slice(&chunk);
            }
        }
        Ok(Bytes::from(body))
    }
}

/// Writes `raw` into `decoder`, and flushes what it decodes to into the decoder's
/// `Vec`. Sets `member_ended` when the gzip member ends, and ignores the rest of
/// `raw`.
fn decode(
    decoder: &mut GzDecoder<Vec<u8>>,
    mut raw: &[u8],
    member_ended: &mut bool,
) -> io::Result<()> {
    while !raw.is_empty() {
        match decoder.write(raw)? {
            // The member and its trailer are complete: nothing more is written.
            0 => {
                *member_ended = true;
                break;
            }
            written => raw = &raw[written..],
        }
    }
    decoder.flush()
}

/// A response with `response`'s status, version, URL and headers, and `body`, so
/// reqwest's own `text` and `json` read the decoded body: the same character set
/// handling, the same errors.
pub(super) fn with_body(response: &Response, body: Bytes) -> Response {
    use reqwest::ResponseBuilderExt;

    let mut builder = http::Response::builder()
        .status(response.status())
        .version(response.version())
        .url(response.url().clone());
    if let Some(headers) = builder.headers_mut() {
        *headers = response.headers().clone();
    }
    match builder.body(body) {
        Ok(decoded) => Response::from(decoded),
        // The status, version and headers came from a valid response.
        Err(_) => unreachable!("rebuilding a valid response failed"),
    }
}

/// The error reqwest returns for a body that fails to decode: the same kind,
/// `is_decode()`, that its own gzip decoding returned, with `error` as its source.
async fn decode_error(error: io::Error) -> reqwest::Error {
    let body = reqwest::Body::wrap(ErrorBody(Some(error)));
    match Response::from(http::Response::new(body)).chunk().await {
        Err(error) => error,
        Ok(_) => unreachable!("a body that only fails read without failing"),
    }
}

/// A body whose first frame is `error`.
struct ErrorBody(Option<io::Error>);

impl http_body::Body for ErrorBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, io::Error>>> {
        Poll::Ready(self.0.take().map(Err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseConfiguration;
    use crate::goose::{get_base_url, GooseHttpResponse, GooseUser};
    use httpmock::{Method::GET, MockServer};

    const EMPTY_ARGS: Vec<&str> = vec![];

    /// A body big enough to come in more than one chunk, gzipped.
    fn original() -> Vec<u8> {
        (0..20_000)
            .map(|i| format!("line {i} of a body Goose decodes only when it is read\n"))
            .collect::<String>()
            .into_bytes()
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// A user of a load test started with `args`, against `server`.
    fn user(server: &MockServer, args: &[&str]) -> GooseUser {
        let configuration = GooseConfiguration::parse_args_default(args).unwrap();
        let base_url = get_base_url(Some(server.url("/")), None, None).unwrap();
        GooseUser::single(base_url, &configuration).unwrap()
    }

    /// The bodies a server can send with `Content-Encoding: gzip`, or without it,
    /// that Goose must read as reqwest's own gzip decoding read them.
    fn cases() -> Vec<(&'static str, Option<&'static str>, Vec<u8>)> {
        let body = original();
        let gzipped = gzip(&body);
        let mut multi = gzip(b"first member, ");
        multi.extend(gzip(b"second member"));
        let mut truncated = gzipped.clone();
        truncated.truncate(gzipped.len() - 10);
        let mut trailing = gzipped.clone();
        trailing.extend_from_slice(b"garbage");
        let mut bad_crc = gzipped.clone();
        let crc = bad_crc.len() - 8;
        bad_crc[crc] ^= 0xff;
        let latin1 = gzip(b"caf\xe9 au lait");
        vec![
            ("text", Some("gzip"), gzipped.clone()),
            ("multi", Some("gzip"), multi),
            ("empty", Some("gzip"), Vec::new()),
            ("truncated", Some("gzip"), truncated),
            ("trailing", Some("gzip"), trailing),
            ("bad-crc", Some("gzip"), bad_crc),
            ("not-gzip", Some("gzip"), b"not gzip at all".to_vec()),
            ("upper-case", Some("GZIP"), gzipped.clone()),
            ("identity", None, body),
            ("json", Some("gzip"), gzip(br#"{"a": [1, 2, 3], "b": "c"}"#)),
            ("latin1", Some("gzip"), latin1),
        ]
    }

    /// What reading a body gave: the bytes, or that it failed as a decode error.
    fn outcome(result: reqwest::Result<Vec<u8>>) -> Result<Vec<u8>, bool> {
        result.map_err(|error| error.is_decode())
    }

    /// Every way the test can read a body gives what it gave when reqwest decoded
    /// gzip itself, byte for byte, and the headers and length it exposes match too.
    #[tokio::test]
    async fn gzip_bodies_read_as_reqwest_read_them() {
        let server = MockServer::start();
        for (name, encoding, body) in cases() {
            let path = format!("/{name}");
            server.mock(|when, then| {
                when.method(GET).path(path.as_str());
                let then = then.status(200).body(body.clone());
                let then = match encoding {
                    Some(encoding) => then.header("content-encoding", encoding),
                    None => then,
                };
                if name == "latin1" {
                    then.header("content-type", "text/plain; charset=iso-8859-1");
                }
            });
        }
        let reference = reqwest::Client::builder().gzip(true).build().unwrap();
        let mut user = user(&server, &EMPTY_ARGS);
        for (name, _, _) in cases() {
            let path = format!("/{name}");
            let url = server.url(path.as_str());
            // Headers and length.
            let want = reference.get(&url).send().await.unwrap();
            let got = user.get(&path).await.unwrap().response.unwrap();
            for header in [CONTENT_ENCODING, CONTENT_LENGTH] {
                assert_eq!(
                    got.headers().get_all(&header).iter().collect::<Vec<_>>(),
                    want.headers().get_all(&header).iter().collect::<Vec<_>>(),
                    "{name}: {header}"
                );
            }
            assert_eq!(got.content_length(), want.content_length(), "{name}");

            // bytes()
            let want = outcome(want.bytes().await.map(Vec::from));
            let got = outcome(got.bytes().await.map(Vec::from));
            assert_eq!(got, want, "{name}: bytes");

            // text()
            let want = outcome(
                reference
                    .get(&url)
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .map(String::into_bytes),
            );
            let got = outcome(
                user.get(&path)
                    .await
                    .unwrap()
                    .response
                    .unwrap()
                    .text()
                    .await
                    .map(String::into_bytes),
            );
            assert_eq!(got, want, "{name}: text");

            // json()
            let want = reference
                .get(&url)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await;
            let got = user
                .get(&path)
                .await
                .unwrap()
                .response
                .unwrap()
                .json::<serde_json::Value>()
                .await;
            match (got, want) {
                (Ok(got), Ok(want)) => assert_eq!(got, want, "{name}: json"),
                (Err(got), Err(want)) => {
                    assert_eq!(got.is_decode(), want.is_decode(), "{name}: json")
                }
                (got, want) => panic!(
                    "{name}: json: {:?} where reqwest gave {:?}",
                    got.err(),
                    want.err()
                ),
            }

            // chunk(), to the end.
            let mut want = reference.get(&url).send().await.unwrap();
            let mut want_body = Vec::new();
            let want = loop {
                match want.chunk().await {
                    Ok(Some(chunk)) => want_body.extend_from_slice(&chunk),
                    Ok(None) => break Ok(want_body),
                    Err(error) => break Err(error.is_decode()),
                }
            };
            let mut got = user.get(&path).await.unwrap().response.unwrap();
            let mut got_body = Vec::new();
            let got = loop {
                match got.chunk().await {
                    Ok(Some(chunk)) => got_body.extend_from_slice(&chunk),
                    Ok(None) => break Ok(got_body),
                    Err(error) => break Err(error.is_decode()),
                }
            };
            assert_eq!(got, want, "{name}: chunk");
        }
    }

    /// A gzip body read as text is the body the server compressed.
    #[tokio::test]
    async fn a_gzip_body_read_as_text_is_the_original() {
        let server = MockServer::start();
        let body = original();
        let compressed = gzip(&body);
        server.mock(|when, then| {
            when.method(GET).path("/").header("accept-encoding", "gzip");
            then.status(200)
                .header("content-encoding", "gzip")
                .body(&compressed);
        });
        let mut user = user(&server, &EMPTY_ARGS);
        let goose = user.get("/").await.unwrap();
        assert_eq!(
            goose.response.unwrap().text().await.unwrap().as_bytes(),
            body
        );
    }

    /// `discard_body` reads a gzip body without decoding a byte of it, and returns
    /// its size on the wire; so does Goose reading a body the test dropped unread.
    /// Reading the same body as text decodes it.
    #[tokio::test]
    async fn a_discarded_gzip_body_is_never_decoded() {
        let server = MockServer::start();
        let compressed = gzip(&original());
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200)
                .header("content-encoding", "gzip")
                .body(&compressed);
        });
        let mut user = user(&server, &EMPTY_ARGS);
        let decodes = || GZIP_DECODES.with(|decodes| decodes.get());

        let before = decodes();
        let read = user.get("/").await.unwrap().discard_body().await.unwrap();
        assert_eq!(decodes(), before, "discard_body decoded the body");
        assert_eq!(read, compressed.len() as u64);

        let dropped = user.get("/").await.unwrap();
        drop(dropped);
        user.read_unread_bodies().await;
        assert_eq!(decodes(), before, "a body dropped unread was decoded");

        let goose = user.get("/").await.unwrap();
        goose.response.unwrap().text().await.unwrap();
        assert!(decodes() > before, "text() did not decode the body");
    }

    /// `into_inner` returns the body as it came over the wire, with the headers
    /// that say it is gzip.
    #[tokio::test]
    async fn into_inner_returns_the_wire_body() {
        let server = MockServer::start();
        let compressed = gzip(&original());
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200)
                .header("content-encoding", "gzip")
                .body(&compressed);
        });
        let mut user = user(&server, &EMPTY_ARGS);
        let goose = user.get("/").await.unwrap();
        let response = goose.response.unwrap();
        assert!(response.headers().get(CONTENT_ENCODING).is_none());
        let response = response.into_inner();
        assert_eq!(response.headers()[CONTENT_ENCODING], "gzip");
        assert_eq!(
            response.headers()[CONTENT_LENGTH],
            compressed.len().to_string().as_str()
        );
        assert_eq!(response.content_length(), Some(compressed.len() as u64));
        assert_eq!(response.bytes().await.unwrap(), compressed);
    }

    /// Goose asks for gzip unless the test set its own Accept-Encoding, which it
    /// leaves alone, and with `--no-gzip` asks for no compression.
    #[tokio::test]
    async fn goose_sets_accept_encoding_unless_the_test_did() {
        let server = MockServer::start();
        let gzip_mock = server.mock(|when, then| {
            when.method(GET).path("/").header("accept-encoding", "gzip");
            then.status(200).body("gzip");
        });
        let own_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/own")
                .header("accept-encoding", "identity");
            then.status(200).body("own");
        });
        let none_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/none")
                .header_missing("accept-encoding");
            then.status(200).body("none");
        });

        let mut user = user(&server, &EMPTY_ARGS);
        let goose = user.get("/").await.unwrap();
        assert_eq!(goose.response.unwrap().text().await.unwrap(), "gzip");
        gzip_mock.assert_calls(1);

        let request = user
            .get_request_builder(&crate::goose::GooseMethod::Get, "/own")
            .unwrap()
            .header("Accept-Encoding", "identity");
        let goose = user
            .request(
                crate::goose::GooseRequest::builder()
                    .set_request_builder(request)
                    .build(),
            )
            .await
            .unwrap();
        assert_eq!(goose.response.unwrap().text().await.unwrap(), "own");
        own_mock.assert_calls(1);

        let mut user = super::tests::user(&server, &["--no-gzip"]);
        let goose = user.get("/none").await.unwrap();
        assert_eq!(goose.response.unwrap().text().await.unwrap(), "none");
        none_mock.assert_calls(1);
    }

    /// With `--no-gzip` a gzip body is returned as it came, as before: Goose asked
    /// for no compression, so it decompresses none.
    #[tokio::test]
    async fn no_gzip_returns_a_gzip_body_as_it_came() {
        let server = MockServer::start();
        let compressed = gzip(b"compressed");
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.status(200)
                .header("content-encoding", "gzip")
                .body(&compressed);
        });
        let mut user = user(&server, &["--no-gzip"]);
        let response = user.get("/").await.unwrap().response.unwrap();
        assert_eq!(response.headers()[CONTENT_ENCODING], "gzip");
        assert_eq!(response.bytes().await.unwrap(), compressed);
    }

    /// A response built outside Goose's requests is read as it is.
    #[tokio::test]
    async fn a_response_from_elsewhere_is_not_decoded() {
        let compressed = gzip(b"elsewhere");
        let mut response = http::Response::new(compressed.clone());
        response
            .headers_mut()
            .insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        let response = GooseHttpResponse::from(Response::from(response));
        assert_eq!(response.bytes().await.unwrap(), compressed);
    }
}
