//! The access log: one line for each answered request.

use std::time::Duration;

use axum::{extract::Request, response::Response};
use tower_http::{
    classify::{ServerErrorsAsFailures, SharedClassifier},
    trace::{DefaultOnBodyChunk, DefaultOnEos, DefaultOnRequest, TraceLayer},
};
use tracing::Span;

use super::redact_token;

pub(super) type AccessLayer = TraceLayer<
    SharedClassifier<ServerErrorsAsFailures>,
    fn(&Request) -> Span,
    DefaultOnRequest,
    fn(&Response, Duration, &Span),
    DefaultOnBodyChunk,
    DefaultOnEos,
    (),
>;

pub(super) fn layer() -> AccessLayer {
    TraceLayer::new_for_http()
        .make_span_with(span as fn(&Request) -> Span)
        .on_response(answered as fn(&Response, Duration, &Span))
        // By default every 5xx is logged a second time as an error, which
        // turned each expected 503 for an unreachable library into an ERROR
        // line. With this classifier a 5xx status is the only failure there
        // is, and `answered` logs it already.
        .on_failure(())
}

/// Every line logged while a request is handled carries its method and URI.
/// The span keeps the target it had before the access log moved here, so
/// quieting the access lines does not strip this context from the others.
fn span(req: &Request) -> Span {
    tracing::info_span!(
        target: "neo_server::http",
        "request",
        method = %req.method(),
        uri = %redact_token(req.uri()),
    )
}

/// Marks the response of a route that sends files: audio streams, HLS
/// segments, covers and the frontend. A player asks for an audio file one
/// byte range at a time, so these made up most of the access log.
#[derive(Clone, Copy)]
pub(super) struct Routine;

/// Middleware for the routes that send files, see [`Routine`].
pub(super) async fn routine(mut response: Response) -> Response {
    response.extensions_mut().insert(Routine);
    response
}

/// A 5xx is a warning, so `journalctl -p warning` lists it. Its cause is
/// logged where it happened, at the level that cause deserves: a library
/// that cannot be reached is a warning, a failed query an error. A file
/// that was sent is only a debug line, while a refused one stays in.
fn answered(response: &Response, latency: Duration, _: &Span) {
    let status = response.status();
    let ms = latency.as_millis();
    let routine = response.extensions().get::<Routine>().is_some() && status.as_u16() < 400;
    if status.is_server_error() {
        tracing::warn!(
            latency = %format_args!("{ms} ms"),
            status = status.as_u16(),
            "finished processing request"
        );
    } else if routine {
        tracing::debug!(
            latency = %format_args!("{ms} ms"),
            status = status.as_u16(),
            "finished processing request"
        );
    } else {
        tracing::info!(
            latency = %format_args!("{ms} ms"),
            status = status.as_u16(),
            "finished processing request"
        );
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Body, http::StatusCode, middleware::map_response, routing::get};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::logging::captured;

    /// The log lines of one request through the access layer.
    async fn log_of(uri: &str) -> Vec<String> {
        log_filtered(uri, "info").await
    }

    async fn log_filtered(uri: &str, directives: &str) -> Vec<String> {
        let log = captured::start_filtered(directives);
        let app = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .route(
                "/file",
                get(|| async { "bytes" }).layer(map_response(routine)),
            )
            .route(
                "/gone",
                get(|| async { StatusCode::NOT_FOUND }).layer(map_response(routine)),
            )
            .route(
                "/unavailable",
                get(|| async {
                    tracing::warn!(target: "neo_server::http::media", "storage unavailable");
                    StatusCode::SERVICE_UNAVAILABLE
                }),
            )
            .layer(layer());
        let response = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let _ = response.into_body().collect().await;
        log.lines()
    }

    #[tokio::test]
    async fn success_is_one_info_line_without_the_token() {
        let lines = log_of("/ok?token=secret&a=1").await;
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with(" INFO"), "{lines:?}");
        assert!(lines[0].contains("uri=/ok?a=1"), "{lines:?}");
        assert!(lines[0].contains("status=200"), "{lines:?}");
        assert!(!lines[0].contains("secret"), "{lines:?}");
    }

    #[tokio::test]
    async fn server_error_status_is_one_warning_not_an_error() {
        let lines = log_of("/unavailable").await;
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("storage unavailable"), "{lines:?}");
        assert!(lines[1].starts_with(" WARN"), "{lines:?}");
        assert!(lines[1].contains("status=503"), "{lines:?}");
        assert!(
            !lines.iter().any(|line| line.contains("ERROR")),
            "{lines:?}"
        );
    }

    #[tokio::test]
    async fn quieting_access_lines_keeps_the_request_context() {
        let lines = log_filtered("/unavailable", "info,neo_server::http::access=warn").await;
        assert_eq!(lines.len(), 2, "{lines:?}");
        for line in &lines {
            assert!(
                line.contains("request{method=GET uri=/unavailable}"),
                "{lines:?}"
            );
        }
        let ok = log_filtered("/ok", "info,neo_server::http::access=warn").await;
        assert!(ok.is_empty(), "{ok:?}");
    }

    #[tokio::test]
    async fn sent_files_are_debug_lines_and_refused_ones_stay() {
        assert!(log_of("/file").await.is_empty());
        let lines = log_filtered("/file", "info,neo_server::http::access=debug").await;
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("DEBUG"), "{lines:?}");
        let lines = log_of("/gone").await;
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("status=404"), "{lines:?}");
    }
}
