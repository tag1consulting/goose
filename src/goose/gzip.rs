//! Gzip response bodies, decompressed only when the load test reads them through
//! [`GooseHttpResponse`](super::GooseHttpResponse); every member, as RFC 1952 allows.

use super::TransactionError;
use bytes::Bytes;
use flate2::write::MultiGzDecoder;
use reqwest::{header::CONTENT_ENCODING, Response};
use std::io::{self, Read, Write};

#[cfg(test)]
thread_local! {
    /// How many times this thread fed body bytes to a gzip decoder.
    pub(crate) static GZIP_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Decompresses a gzip body read with `chunk`, into its `Vec`.
pub(super) type ChunkDecoder = MultiGzDecoder<Vec<u8>>;

/// Whether `response`'s body is gzip compressed.
pub(super) fn is_gzip(response: &Response) -> bool {
    let encoding = response.headers().get(CONTENT_ENCODING);
    encoding.is_some_and(|encoding| encoding.as_bytes().eq_ignore_ascii_case(b"gzip"))
}

fn error(error: io::Error) -> TransactionError {
    TransactionError::Custom(format!("gzip response body failed to decompress: {error}"))
}

fn count() {
    #[cfg(test)]
    GZIP_DECODES.with(|decodes| decodes.set(decodes.get() + 1));
}

/// Decompresses a whole gzip body; an empty one, as a HEAD's, is empty.
pub(super) fn decode(body: &[u8]) -> Result<Bytes, TransactionError> {
    let mut decoded = Vec::new();
    if !body.is_empty() {
        count();
        let mut decoder = flate2::read::MultiGzDecoder::new(body);
        decoder.read_to_end(&mut decoded).map_err(error)?;
    }
    Ok(decoded.into())
}

/// Decompresses `raw`, the next chunk of a gzip body, with `decoder`, made at the first
/// chunk; `None` at the end of the body, which must end whole. `None` if nothing decoded.
pub(super) fn decode_chunk(
    decoder: &mut Option<Box<ChunkDecoder>>,
    raw: Option<&[u8]>,
) -> Result<Option<Bytes>, TransactionError> {
    let decoder = match (raw, decoder) {
        (Some(raw), decoder) => {
            count();
            let decoder = decoder.get_or_insert_with(|| Box::new(MultiGzDecoder::new(Vec::new())));
            decoder
                .write_all(raw)
                .and_then(|_| decoder.flush())
                .map_err(error)?;
            decoder
        }
        (None, Some(decoder)) => {
            decoder.try_finish().map_err(error)?;
            decoder
        }
        (None, None) => return Ok(None),
    };
    let decoded = std::mem::take(decoder.get_mut());
    Ok((!decoded.is_empty()).then(|| decoded.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GooseConfiguration;
    use crate::goose::{get_base_url, GooseMethod, GooseRequest, GooseUser};
    use httpmock::{Method::GET, MockServer};

    /// A body big enough to come in more than one chunk.
    fn original() -> Vec<u8> {
        let lines = (0..20_000).map(|i| format!("line {i} of a body Goose decompresses\n"));
        lines.collect::<String>().into_bytes()
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), Default::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// A server whose `/` sends `body` as gzip, and a user of a test started with `args`.
    fn serve(body: &[u8], args: &[&str]) -> (MockServer, GooseUser) {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/");
            then.header("content-encoding", "gzip").body(body);
        });
        let configuration = GooseConfiguration::parse_args_default(args).unwrap();
        let base_url = get_base_url(Some(server.url("/")), None, None).unwrap();
        (server, GooseUser::single(base_url, &configuration).unwrap())
    }

    #[tokio::test]
    async fn a_gzip_body_read_as_text_is_the_original() {
        let (_server, mut user) = serve(&gzip(&original()), &[]);
        let response = user.get("/").await.unwrap().response.unwrap();
        assert_eq!(response.text().await.unwrap().as_bytes(), original());
    }

    /// Neither `discard_body` nor Goose reading a dropped body decompresses it.
    #[tokio::test]
    async fn a_discarded_gzip_body_is_never_decoded() {
        let compressed = gzip(&original());
        let (_server, mut user) = serve(&compressed, &[]);
        let decodes = || GZIP_DECODES.with(|decodes| decodes.get());
        let before = decodes();
        let read = user.get("/").await.unwrap().discard_body().await.unwrap();
        assert_eq!(decodes(), before, "discard_body decompressed the body");
        assert_eq!(read, compressed.len() as u64);
        drop(user.get("/").await.unwrap());
        user.read_unread_bodies().await;
        assert_eq!(decodes(), before, "a body dropped unread was decompressed");
        let response = user.get("/").await.unwrap().response.unwrap();
        response.text().await.unwrap();
        assert!(decodes() > before, "text() did not decompress the body");
    }

    /// With `--no-gzip`, no Accept-Encoding, and a gzip body comes back as it came.
    #[tokio::test]
    async fn no_gzip_returns_a_gzip_body_as_it_came() {
        let compressed = gzip(b"compressed");
        let (server, mut user) = serve(b"", &["--no-gzip"]);
        let mock = server.mock(|when, then| {
            when.path("/none").header_missing("accept-encoding");
            then.header("content-encoding", "gzip").body(&compressed);
        });
        let response = user.get("/none").await.unwrap().response.unwrap();
        assert_eq!(response.headers()[CONTENT_ENCODING], "gzip");
        assert_eq!(response.bytes().await.unwrap(), compressed);
        mock.assert_calls(1);
    }

    /// Goose asks for gzip, unless the test set its own Accept-Encoding.
    #[tokio::test]
    async fn a_tests_own_accept_encoding_is_left_alone() {
        let (server, mut user) = serve(b"", &[]);
        let gzip = server.mock(|when, then| {
            when.path("/gzip").header("accept-encoding", "gzip");
            then.body("gzip");
        });
        let own = server.mock(|when, then| {
            when.path("/own").header("accept-encoding", "identity");
            then.body("own");
        });
        user.get("/gzip").await.unwrap();
        let request = user.get_request_builder(&GooseMethod::Get, "/own").unwrap();
        let request = request.header("Accept-Encoding", "identity");
        let request = GooseRequest::builder().set_request_builder(request);
        user.request(request.build()).await.unwrap();
        gzip.assert_calls(1);
        own.assert_calls(1);
    }

    /// A body of two gzip members is decompressed whole, by `bytes` and by `chunk`.
    #[tokio::test]
    async fn every_member_of_a_gzip_body_is_decompressed() {
        let (_server, mut user) = serve(&[gzip(&original()), gzip(b"two")].concat(), &[]);
        let want = [original(), b"two".to_vec()].concat();
        let response = user.get("/").await.unwrap().response.unwrap();
        assert_eq!(response.bytes().await.unwrap(), want);
        let mut response = user.get("/").await.unwrap().response.unwrap();
        let mut got = Vec::new();
        while let Some(chunk) = response.chunk().await.unwrap() {
            got.extend_from_slice(&chunk);
        }
        assert_eq!(got, want);
    }
}
