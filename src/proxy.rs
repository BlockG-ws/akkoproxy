use crate::cache::{CacheKey, CachedResponse, ResponseCache};
use crate::config::Config;
use crate::image::{
    format_from_content_type, format_satisfies, is_image_content_type, parse_accept_header,
    ImageConverter, OutputFormat,
};
use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use ipnetwork::IpNetwork;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Custom header name for cache status
const X_CACHE_STATUS: &str = "x-cache-status";

/// Headers that should not be copied from upstream responses
/// These are either automatically set by the proxy or should not be forwarded
/// Note: ACCESS_CONTROL_ALLOW_ORIGIN is NOT excluded - it will be preserved from upstream
/// if present, otherwise the proxy will set it to "*"
const EXCLUDED_HEADERS: &[header::HeaderName] = &[
    header::CONTENT_LENGTH,
    header::CONTENT_TYPE,
    header::TRANSFER_ENCODING,
    header::CONNECTION,
    header::VIA,
    header::CACHE_CONTROL,
];

/// Check if a header should be excluded from upstream response
fn should_exclude_header(key: &header::HeaderName) -> bool {
    EXCLUDED_HEADERS.contains(key) || key.as_str() == X_CACHE_STATUS
}

/// Build Vary header value, prepending "Accept" to upstream value if present
fn build_vary_header(upstream_vary: Option<&str>) -> String {
    if let Some(upstream_value) = upstream_vary {
        // Check if "Accept" is already in the upstream Vary header (case-insensitive)
        let has_accept = upstream_value
            .split(',')
            .any(|v| v.trim().eq_ignore_ascii_case("accept"));

        if has_accept {
            // If upstream already has "Accept", just use upstream value
            upstream_value.to_string()
        } else {
            // Prepend "Accept" to upstream value
            format!("Accept, {}", upstream_value)
        }
    } else {
        // No upstream Vary header, just use "Accept"
        "Accept".to_string()
    }
}

/// Check if an IP address is in the trusted proxy list
fn is_trusted_proxy(ip: &IpAddr, trusted_proxies: &[String]) -> bool {
    if trusted_proxies.is_empty() {
        return false;
    }

    for proxy in trusted_proxies {
        // Try parsing as a CIDR range
        if let Ok(network) = proxy.parse::<IpNetwork>() {
            if network.contains(*ip) {
                return true;
            }
        }
        // Try parsing as a single IP address
        else if let Ok(proxy_ip) = proxy.parse::<IpAddr>() {
            if proxy_ip == *ip {
                return true;
            }
        }
    }

    false
}

/// Apply X-Forwarded headers to the upstream request builder
/// Only honors headers from trusted proxies, otherwise derives from actual connection
fn apply_forwarded_headers(
    request_builder: reqwest::RequestBuilder,
    headers: &HeaderMap,
    client_ip: IpAddr,
    config: &Config,
) -> reqwest::RequestBuilder {
    let mut builder = request_builder;

    // Check if header forwarding is enabled
    if !config.server.forward_headers_enabled {
        debug!("X-Forwarded header forwarding is disabled");
        return builder;
    }

    // Check if the client IP is trusted
    let is_trusted = is_trusted_proxy(&client_ip, &config.server.trusted_proxies);

    if is_trusted {
        debug!("Client IP {} is trusted, forwarding X-Forwarded-* headers", client_ip);
        
        // Forward X-Forwarded-Proto if present
        if let Some(forwarded_proto) = headers.get("x-forwarded-proto") {
            builder = builder.header("X-Forwarded-Proto", forwarded_proto);
        }
        
        // Forward X-Forwarded-For if present
        if let Some(forwarded_for) = headers.get("x-forwarded-for") {
            builder = builder.header("X-Forwarded-For", forwarded_for);
        }
        
        // Forward X-Forwarded-Host if present
        if let Some(forwarded_host) = headers.get("x-forwarded-host") {
            builder = builder.header("X-Forwarded-Host", forwarded_host);
        }
    } else {
        debug!("Client IP {} is not trusted, setting X-Forwarded-For from actual connection", client_ip);
        
        // Set X-Forwarded-For to the actual client IP
        // If there's an existing X-Forwarded-For from an untrusted source, append to it
        // (this preserves the chain but treats the untrusted part as the original client)
        if let Some(existing_xff) = headers.get("x-forwarded-for") {
            if let Ok(existing_value) = existing_xff.to_str() {
                let new_value = format!("{}, {}", existing_value, client_ip);
                builder = builder.header("X-Forwarded-For", new_value);
            } else {
                builder = builder.header("X-Forwarded-For", client_ip.to_string());
            }
        } else {
            builder = builder.header("X-Forwarded-For", client_ip.to_string());
        }

        // Do not forward X-Forwarded-Proto or X-Forwarded-Host from untrusted sources
        // Let the upstream derive these from the actual connection if needed
    }

    builder
}

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub cache: ResponseCache,
    pub client: reqwest::Client,
    pub image_converter: Arc<ImageConverter>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        debug!(
            "Initializing AppState with config: bind={}, upstream={}",
            config.server.bind, config.upstream.url
        );

        let cache = ResponseCache::new(
            config.cache.max_capacity,
            Duration::from_secs(config.cache.ttl),
            config.cache.max_item_size,
        );
        debug!(
            "Cache initialized: max_capacity={}, ttl={}s, max_item_size={} bytes",
            config.cache.max_capacity, config.cache.ttl, config.cache.max_item_size
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.upstream.timeout))
            .user_agent(format!("akkoproxy/{}", env!("CARGO_PKG_VERSION")))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("Failed to create HTTP client");
        debug!(
            "HTTP client configured: timeout={}s, user_agent=akkoproxy/{}, redirect_policy=none",
            config.upstream.timeout,
            env!("CARGO_PKG_VERSION")
        );

        let image_converter = Arc::new(ImageConverter::new(
            config.image.quality,
            config.image.max_dimension,
            config.image.enable_avif,
            config.image.enable_webp,
        ));
        debug!(
            "Image converter initialized: quality={}, max_dimension={}, avif={}, webp={}",
            config.image.quality,
            config.image.max_dimension,
            config.image.enable_avif,
            config.image.enable_webp
        );

        Self {
            config: Arc::new(config),
            cache,
            client,
            image_converter,
        }
    }
}

/// Main proxy handler
pub async fn proxy_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    uri: Uri,
    headers: HeaderMap,
    _request: Request,
) -> Result<Response, ProxyError> {
    let path = uri.path();
    let query = uri.query().unwrap_or("");

    debug!("Proxying request: {} {} from {}", path, query, addr);

    // Handle root path with redirect
    if path == "/" {
        return Ok(Response::builder()
            .status(StatusCode::MOVED_PERMANENTLY)
            .header(header::LOCATION, "https://github.com/BlockG-ws/akkoproxy")
            .body(Body::empty())
            .expect("Failed to build root redirect response"));
    }

    // Only handle /media and /proxy paths
    if !path.starts_with("/media") && !path.starts_with("/proxy") {
        warn!("Path not allowed: {}", path);
        return Err(ProxyError::PathNotAllowed);
    }

    // Parse query parameters if behind_cloudflare_free is enabled
    let (format_from_query, upstream_query) =
        if state.config.server.behind_cloudflare_free && !query.is_empty() {
            parse_query_for_format(query)
        } else {
            (None, query.to_string())
        };

    // Build upstream URL (without format query if it was present)
    let upstream_url = if upstream_query.is_empty() {
        format!("{}{}", state.config.upstream.url, path)
    } else {
        format!("{}{}?{}", state.config.upstream.url, path, upstream_query)
    };

    // Determine desired format
    let desired_format = if let Some(fmt) = format_from_query {
        // Use format from query parameter if available
        fmt
    } else {
        // Get Accept header to determine desired format
        let accept = headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("*/*");

        parse_accept_header(
            accept,
            state.config.image.enable_avif,
            state.config.image.enable_webp,
        )
    };

    // Generate cache key
    let cache_key = CacheKey::new(
        format!(
            "{}{}",
            path,
            if query.is_empty() {
                String::new()
            } else {
                format!("?{}", query)
            }
        ),
        format!("{:?}", desired_format),
    );

    // Check cache first
    if let Some(cached) = state.cache.get(&cache_key).await {
        debug!("Cache hit for {}", path);
        return Ok(build_response(
            cached.data.clone(),
            &cached.content_type,
            &state.config.server.via_header,
            cached.upstream_headers.as_ref(),
            true, // is_cache_hit
        ));
    }

    debug!(
        "Cache miss for {}, fetching from upstream: {}",
        path, upstream_url
    );

    // Build request with forwarded headers
    let request_builder = state.client.get(&upstream_url);
    
    // Apply X-Forwarded-* headers based on configuration and trust policy
    let request_builder = apply_forwarded_headers(
        request_builder,
        &headers,
        addr.ip(),
        &state.config,
    );

    // Fetch from upstream
    let response = request_builder.send().await.map_err(|e| {
        error!("Failed to fetch from upstream: {}", e);
        ProxyError::UpstreamError(e)
    })?;

    let status = response.status();

    // Handle non-success responses (redirects, errors, etc.)
    // For non-2xx responses, preserve and forward the response with its status code
    if !status.is_success() {
        debug!("Upstream returned non-success status: {}", status);

        // Preserve upstream headers
        let upstream_headers = if state.config.server.preserve_upstream_headers {
            Some(response.headers().clone())
        } else {
            None
        };

        let body_bytes = response.bytes().await.map_err(|e| {
            error!("Failed to read response body: {}", e);
            ProxyError::UpstreamError(e)
        })?;

        // Build response with the actual status code from upstream
        return Ok(build_response_with_status(
            body_bytes,
            status,
            &state.config.server.via_header,
            upstream_headers.as_ref(),
        ));
    }

    // Preserve upstream headers if configured (for success responses)
    let upstream_headers = if state.config.server.preserve_upstream_headers {
        Some(response.headers().clone())
    } else {
        None
    };

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let body_bytes = response.bytes().await.map_err(|e| {
        error!("Failed to read response body: {}", e);
        ProxyError::UpstreamError(e)
    })?;

    // Check if this is an image and conversion is requested
    // Skip conversion if upstream format already satisfies the desired format
    let upstream_format = format_from_content_type(&content_type);
    let needs_conversion = should_convert_image(
        &content_type,
        upstream_format,
        desired_format,
        body_bytes.len(),
        state.config.cache.max_item_size as usize,
    );

    let (final_data, final_content_type) = if needs_conversion {
        debug!("Converting image to {:?}", desired_format);

        match state.image_converter.convert(&body_bytes, desired_format) {
            Ok((converted, mime_type)) => {
                info!(
                    "Successfully converted image: {} bytes -> {} bytes",
                    body_bytes.len(),
                    converted.len()
                );
                (converted, mime_type.to_string())
            }
            Err(e) => {
                warn!("Failed to convert image: {}, returning original", e);
                (body_bytes, content_type)
            }
        }
    } else {
        if is_image_content_type(&content_type) && upstream_format.is_some() {
            debug!(
                "Skipping conversion: upstream format {:?} already satisfies desired format {:?}",
                upstream_format, desired_format
            );
        } else {
            debug!(
                "Not converting: is_image={}, format={:?}, size={}",
                is_image_content_type(&content_type),
                desired_format,
                body_bytes.len()
            );
        }
        (body_bytes, content_type)
    };

    // Cache the response
    if final_data.len() <= state.config.cache.max_item_size as usize {
        let cached_response = CachedResponse {
            data: final_data.clone(),
            content_type: final_content_type.clone(),
            upstream_headers: upstream_headers.clone(),
        };
        state.cache.put(cache_key, cached_response).await;
        debug!("Cached response for {}", path);
    } else {
        debug!("Response too large to cache: {} bytes", final_data.len());
    }

    Ok(build_response(
        final_data,
        &final_content_type,
        &state.config.server.via_header,
        upstream_headers.as_ref(),
        false, // is_cache_hit
    ))
}

/// Parse query string to extract format parameter and return modified query
/// Returns (format_option, remaining_query_string)
///
/// This parser is intentionally simple and only handles basic ASCII format values
/// ("avif", "webp") with case-insensitive matching. It handles '+' as space
/// (common in query strings) but does not perform full URL decoding.
///
/// Cloudflare Transform Rules generate clean query parameters like "format=avif"
/// so complex URL decoding is not necessary for this use case.
fn parse_query_for_format(query: &str) -> (Option<OutputFormat>, String) {
    let mut format_value = None;
    let mut remaining_params = Vec::new();

    for param in query.split('&') {
        if let Some((key, value)) = param.split_once('=') {
            if key == "format" {
                // Parse the format value directly (case-insensitive, trimmed)
                // We expect simple ASCII values like "avif" or "webp"
                // Strip common whitespace encodings like +
                let normalized = value.replace('+', " ");
                format_value = match normalized.trim().to_lowercase().as_str() {
                    "avif" => Some(OutputFormat::Avif),
                    "webp" => Some(OutputFormat::WebP),
                    _ => None, // Invalid or unsupported format values are ignored
                };
            } else {
                remaining_params.push(param);
            }
        } else {
            // Keep parameters without values (e.g., "debug" in "?debug&other=value")
            remaining_params.push(param);
        }
    }

    (format_value, remaining_params.join("&"))
}

/// Determine if image conversion is needed
fn should_convert_image(
    content_type: &str,
    upstream_format: Option<OutputFormat>,
    desired_format: OutputFormat,
    content_size: usize,
    max_size: usize,
) -> bool {
    // Must be an image
    if !is_image_content_type(content_type) {
        return false;
    }

    // Must not be requesting original format
    if desired_format == OutputFormat::Original {
        return false;
    }

    // Must be within size limits
    if content_size > max_size {
        return false;
    }

    // Skip conversion if upstream format already satisfies desired format
    !matches!(upstream_format, Some(fmt) if format_satisfies(fmt, desired_format))
}

/// Build HTTP response with appropriate headers
fn build_response(
    data: Bytes,
    content_type: &str,
    via_header: &str,
    upstream_headers: Option<&HeaderMap>,
    is_cache_hit: bool,
) -> Response {
    let mut builder = Response::builder().status(StatusCode::OK);

    // Check if upstream has CORS header
    let upstream_has_cors = upstream_headers
        .map(|h| h.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN))
        .unwrap_or(false);

    // Check if upstream has Vary header
    let upstream_vary = upstream_headers
        .and_then(|h| h.get(header::VARY))
        .and_then(|v| v.to_str().ok());

    // Add upstream headers if configured
    if let Some(headers) = upstream_headers {
        for (key, value) in headers.iter() {
            // Skip headers that shouldn't be copied (those set by the proxy)
            // Also skip Vary header as we'll handle it specially
            if !should_exclude_header(key) && key != header::VARY {
                builder = builder.header(key, value);
            }
        }
    }

    // Always set/override these headers
    builder = builder
        .header(header::CONTENT_TYPE, content_type)
        .header(header::VIA, via_header)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header("X-Cache-Status", if is_cache_hit { "HIT" } else { "MISS" });

    // Always add Vary header with Accept
    // If upstream has Vary header, prepend "Accept" to it
    builder = builder.header(header::VARY, build_vary_header(upstream_vary));

    // Only set CORS header if upstream didn't provide one
    if !upstream_has_cors {
        builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    }

    builder
        .body(Body::from(data))
        .expect("Failed to build response")
}

/// Build HTTP response with custom status code and headers
fn build_response_with_status(
    data: Bytes,
    status: StatusCode,
    via_header: &str,
    upstream_headers: Option<&HeaderMap>,
) -> Response {
    let mut builder = Response::builder().status(status);

    // Check if upstream has CORS header
    let upstream_has_cors = upstream_headers
        .map(|h| h.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN))
        .unwrap_or(false);

    // Check if upstream has Vary header
    let upstream_vary = upstream_headers
        .and_then(|h| h.get(header::VARY))
        .and_then(|v| v.to_str().ok());

    // Add upstream headers if configured
    if let Some(headers) = upstream_headers {
        for (key, value) in headers.iter() {
            // Skip headers that shouldn't be copied (those set by the proxy)
            // Also skip Vary header as we'll handle it specially
            if !should_exclude_header(key) && key != header::VARY {
                builder = builder.header(key, value);
            }
        }
    }

    // Always add Via header
    builder = builder.header(header::VIA, via_header);

    // Always add Vary header with Accept
    // If upstream has Vary header, prepend "Accept" to it
    builder = builder.header(header::VARY, build_vary_header(upstream_vary));

    // Only set CORS header if upstream didn't provide one
    if !upstream_has_cors {
        builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
    }

    builder
        .body(Body::from(data))
        .expect("Failed to build response with status")
}

/// Health check handler
pub async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Metrics handler
pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let stats = state.cache.stats();
    let body = format!(
        "# Cache Statistics\ncache_entries {}\ncache_size_bytes {}\n",
        stats.entry_count, stats.weighted_size
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

/// Proxy error types
#[derive(Debug)]
pub enum ProxyError {
    PathNotAllowed,
    UpstreamError(reqwest::Error),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ProxyError::PathNotAllowed => (StatusCode::FORBIDDEN, "Path not allowed".to_string()),
            ProxyError::UpstreamError(e) => {
                (StatusCode::BAD_GATEWAY, format!("Upstream error: {}", e))
            }
        };

        (status, message).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderName, HeaderValue};

    #[test]
    fn test_build_response_no_duplicate_headers() {
        // Create upstream headers that include content-type and via
        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
        upstream_headers.insert(header::VIA, HeaderValue::from_static("upstream-proxy"));
        upstream_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        upstream_headers.insert(
            HeaderName::from_static("x-cache-status"),
            HeaderValue::from_static("upstream-hit"),
        );
        upstream_headers.insert(
            HeaderName::from_static("x-custom-header"),
            HeaderValue::from_static("custom-value"),
        );

        // Build response with different content-type
        let response = build_response(
            Bytes::from("test data"),
            "image/avif",
            "akkoproxy/1.0",
            Some(&upstream_headers),
            true,
        );

        let headers = response.headers();

        // Content-Type should only have the proxy's value (image/avif), not upstream's (image/jpeg)
        let content_types: Vec<_> = headers.get_all(header::CONTENT_TYPE).iter().collect();
        assert_eq!(
            content_types.len(),
            1,
            "Content-Type should not be duplicated"
        );
        assert_eq!(content_types[0], "image/avif");

        // Via should only have the proxy's value
        let via_values: Vec<_> = headers.get_all(header::VIA).iter().collect();
        assert_eq!(via_values.len(), 1, "Via should not be duplicated");
        assert_eq!(via_values[0], "akkoproxy/1.0");

        // Cache-Control should only have the proxy's value
        let cache_control_values: Vec<_> = headers.get_all(header::CACHE_CONTROL).iter().collect();
        assert_eq!(
            cache_control_values.len(),
            1,
            "Cache-Control should not be duplicated"
        );
        assert_eq!(
            cache_control_values[0],
            "public, max-age=31536000, immutable"
        );

        // X-Cache-Status should only have the proxy's value
        let x_cache_status_values: Vec<_> = headers.get_all("x-cache-status").iter().collect();
        assert_eq!(
            x_cache_status_values.len(),
            1,
            "X-Cache-Status should not be duplicated"
        );
        assert_eq!(x_cache_status_values[0], "HIT");

        // Custom header should be preserved
        assert_eq!(headers.get("x-custom-header").unwrap(), "custom-value");
    }

    #[test]
    fn test_build_response_with_status_no_duplicate_headers() {
        // Create upstream headers
        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert(header::VIA, HeaderValue::from_static("upstream-proxy"));
        upstream_headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("https://example.com"),
        );
        upstream_headers.insert(
            HeaderName::from_static("x-custom-header"),
            HeaderValue::from_static("custom-value"),
        );

        // Build response
        let response = build_response_with_status(
            Bytes::from("redirect"),
            StatusCode::MOVED_PERMANENTLY,
            "akkoproxy/1.0",
            Some(&upstream_headers),
        );

        let headers = response.headers();

        // Via should only have the proxy's value
        let via_values: Vec<_> = headers.get_all(header::VIA).iter().collect();
        assert_eq!(via_values.len(), 1, "Via should not be duplicated");
        assert_eq!(via_values[0], "akkoproxy/1.0");

        // Access-Control-Allow-Origin should have upstream's value (not replaced)
        let acao_values: Vec<_> = headers
            .get_all(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .iter()
            .collect();
        assert_eq!(
            acao_values.len(),
            1,
            "Access-Control-Allow-Origin should not be duplicated"
        );
        assert_eq!(acao_values[0], "https://example.com");

        // Custom header should be preserved
        assert_eq!(headers.get("x-custom-header").unwrap(), "custom-value");
    }

    #[test]
    fn test_parse_query_for_format() {
        // Test format=avif
        let (format, remaining) = parse_query_for_format("format=avif&other=value");
        assert_eq!(format, Some(OutputFormat::Avif));
        assert_eq!(remaining, "other=value");

        // Test format=webp
        let (format, remaining) = parse_query_for_format("format=webp");
        assert_eq!(format, Some(OutputFormat::WebP));
        assert_eq!(remaining, "");

        // Test no format parameter
        let (format, remaining) = parse_query_for_format("other=value&another=test");
        assert_eq!(format, None);
        assert_eq!(remaining, "other=value&another=test");

        // Test format with unknown value
        let (format, remaining) = parse_query_for_format("format=jpeg&other=value");
        assert_eq!(format, None);
        assert_eq!(remaining, "other=value");

        // Test format in middle
        let (format, remaining) = parse_query_for_format("a=1&format=avif&b=2");
        assert_eq!(format, Some(OutputFormat::Avif));
        assert_eq!(remaining, "a=1&b=2");

        // Test format with URL encoding (spaces as +)
        let (format, remaining) = parse_query_for_format("format=webp+test&other=value");
        assert_eq!(format, None); // Should not match due to extra text
        assert_eq!(remaining, "other=value");

        // Test format with case insensitivity
        let (format, remaining) = parse_query_for_format("format=AVIF");
        assert_eq!(format, Some(OutputFormat::Avif));
        assert_eq!(remaining, "");

        let (format, remaining) = parse_query_for_format("format=WebP");
        assert_eq!(format, Some(OutputFormat::WebP));
        assert_eq!(remaining, "");

        // Test format with whitespace (+ is space in query strings)
        let (format, remaining) = parse_query_for_format("format=+avif+&other=value");
        assert_eq!(format, Some(OutputFormat::Avif));
        assert_eq!(remaining, "other=value");
    }

    #[test]
    fn test_cors_header_follows_upstream() {
        // Test when upstream provides CORS header
        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("https://example.com"),
        );

        let response = build_response(
            Bytes::from("test"),
            "text/plain",
            "akkoproxy/1.0",
            Some(&upstream_headers),
            false,
        );

        // Should use upstream CORS value
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "https://example.com"
        );

        // Test when upstream doesn't provide CORS header
        let response = build_response(
            Bytes::from("test"),
            "text/plain",
            "akkoproxy/1.0",
            None,
            false,
        );

        // Should use default "*"
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .unwrap(),
            "*"
        );
    }

    #[test]
    fn test_vary_header_always_present() {
        // Test with no upstream headers - should have Vary: Accept
        let response = build_response(
            Bytes::from("test"),
            "text/plain",
            "akkoproxy/1.0",
            None,
            false,
        );

        assert_eq!(response.headers().get(header::VARY).unwrap(), "Accept");

        // Test with upstream headers (no Vary) - should still have Vary: Accept
        let upstream_headers = HeaderMap::new();
        let response = build_response(
            Bytes::from("test"),
            "text/plain",
            "akkoproxy/1.0",
            Some(&upstream_headers),
            false,
        );

        assert_eq!(response.headers().get(header::VARY).unwrap(), "Accept");
    }

    #[test]
    fn test_vary_header_prepends_accept_to_upstream() {
        // Test when upstream has Vary header without Accept
        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert(header::VARY, HeaderValue::from_static("Origin, User-Agent"));

        let response = build_response(
            Bytes::from("test"),
            "text/plain",
            "akkoproxy/1.0",
            Some(&upstream_headers),
            false,
        );

        assert_eq!(
            response.headers().get(header::VARY).unwrap(),
            "Accept, Origin, User-Agent"
        );

        // Test when upstream has Vary header with Accept already present
        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert(header::VARY, HeaderValue::from_static("Accept, Origin"));

        let response = build_response(
            Bytes::from("test"),
            "text/plain",
            "akkoproxy/1.0",
            Some(&upstream_headers),
            false,
        );

        // Should not duplicate Accept
        assert_eq!(
            response.headers().get(header::VARY).unwrap(),
            "Accept, Origin"
        );

        // Test when upstream has Vary header with accept in different case
        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert(header::VARY, HeaderValue::from_static("ACCEPT, Origin"));

        let response = build_response(
            Bytes::from("test"),
            "text/plain",
            "akkoproxy/1.0",
            Some(&upstream_headers),
            false,
        );

        // Should recognize case-insensitive match and not duplicate
        assert_eq!(
            response.headers().get(header::VARY).unwrap(),
            "ACCEPT, Origin"
        );
    }

    #[test]
    fn test_vary_header_in_response_with_status() {
        // Test without upstream Vary header
        let response = build_response_with_status(
            Bytes::from("test"),
            StatusCode::NOT_FOUND,
            "akkoproxy/1.0",
            None,
        );

        assert_eq!(response.headers().get(header::VARY).unwrap(), "Accept");

        // Test with upstream Vary header
        let mut upstream_headers = HeaderMap::new();
        upstream_headers.insert(header::VARY, HeaderValue::from_static("Origin"));

        let response = build_response_with_status(
            Bytes::from("test"),
            StatusCode::MOVED_PERMANENTLY,
            "akkoproxy/1.0",
            Some(&upstream_headers),
        );

        assert_eq!(
            response.headers().get(header::VARY).unwrap(),
            "Accept, Origin"
        );
    }

    #[test]
    fn test_is_trusted_proxy() {
        // Test with empty trusted proxies list
        let trusted_proxies: Vec<String> = vec![];
        let ip = "192.168.1.1".parse::<IpAddr>().unwrap();
        assert!(!is_trusted_proxy(&ip, &trusted_proxies));

        // Test with single IP match
        let trusted_proxies = vec!["192.168.1.1".to_string()];
        let ip = "192.168.1.1".parse::<IpAddr>().unwrap();
        assert!(is_trusted_proxy(&ip, &trusted_proxies));

        // Test with single IP no match
        let ip = "192.168.1.2".parse::<IpAddr>().unwrap();
        assert!(!is_trusted_proxy(&ip, &trusted_proxies));

        // Test with CIDR range match
        let trusted_proxies = vec!["192.168.1.0/24".to_string()];
        let ip = "192.168.1.100".parse::<IpAddr>().unwrap();
        assert!(is_trusted_proxy(&ip, &trusted_proxies));

        // Test with CIDR range no match
        let ip = "192.168.2.100".parse::<IpAddr>().unwrap();
        assert!(!is_trusted_proxy(&ip, &trusted_proxies));

        // Test with multiple entries
        let trusted_proxies = vec![
            "10.0.0.0/8".to_string(),
            "172.16.0.0/12".to_string(),
            "192.168.1.1".to_string(),
        ];
        assert!(is_trusted_proxy(&"10.5.5.5".parse::<IpAddr>().unwrap(), &trusted_proxies));
        assert!(is_trusted_proxy(&"172.20.1.1".parse::<IpAddr>().unwrap(), &trusted_proxies));
        assert!(is_trusted_proxy(&"192.168.1.1".parse::<IpAddr>().unwrap(), &trusted_proxies));
        assert!(!is_trusted_proxy(&"8.8.8.8".parse::<IpAddr>().unwrap(), &trusted_proxies));

        // Test with IPv6
        let trusted_proxies = vec!["::1".to_string(), "fe80::/10".to_string()];
        assert!(is_trusted_proxy(&"::1".parse::<IpAddr>().unwrap(), &trusted_proxies));
        assert!(is_trusted_proxy(&"fe80::1".parse::<IpAddr>().unwrap(), &trusted_proxies));
        assert!(!is_trusted_proxy(&"2001:db8::1".parse::<IpAddr>().unwrap(), &trusted_proxies));
    }

    #[test]
    fn test_apply_forwarded_headers_disabled() {
        use reqwest::Client;
        
        let config = Config::with_upstream("https://example.com".to_string());
        assert!(!config.server.forward_headers_enabled);

        let client = Client::new();
        let builder = client.get("https://example.com/test");
        
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));
        headers.insert("x-forwarded-host", HeaderValue::from_static("example.org"));
        
        let client_ip = "10.0.0.1".parse::<IpAddr>().unwrap();
        
        let result_builder = apply_forwarded_headers(builder, &headers, client_ip, &config);
        
        // Headers should not be forwarded when disabled
        let request = result_builder.build().unwrap();
        assert!(request.headers().get("x-forwarded-proto").is_none());
        assert!(request.headers().get("x-forwarded-for").is_none());
        assert!(request.headers().get("x-forwarded-host").is_none());
    }

    #[test]
    fn test_apply_forwarded_headers_from_trusted_proxy() {
        use reqwest::Client;
        
        let mut config = Config::with_upstream("https://example.com".to_string());
        config.server.forward_headers_enabled = true;
        config.server.trusted_proxies = vec!["10.0.0.0/8".to_string()];

        let client = Client::new();
        let builder = client.get("https://example.com/test");
        
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));
        headers.insert("x-forwarded-host", HeaderValue::from_static("example.org"));
        
        let client_ip = "10.0.0.1".parse::<IpAddr>().unwrap(); // Trusted
        
        let result_builder = apply_forwarded_headers(builder, &headers, client_ip, &config);
        
        // Headers should be forwarded from trusted proxy
        let request = result_builder.build().unwrap();
        assert_eq!(request.headers().get("x-forwarded-proto").unwrap(), "https");
        assert_eq!(request.headers().get("x-forwarded-for").unwrap(), "1.2.3.4");
        assert_eq!(request.headers().get("x-forwarded-host").unwrap(), "example.org");
    }

    #[test]
    fn test_apply_forwarded_headers_from_untrusted_proxy() {
        use reqwest::Client;
        
        let mut config = Config::with_upstream("https://example.com".to_string());
        config.server.forward_headers_enabled = true;
        config.server.trusted_proxies = vec!["10.0.0.0/8".to_string()];

        let client = Client::new();
        let builder = client.get("https://example.com/test");
        
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));
        headers.insert("x-forwarded-host", HeaderValue::from_static("example.org"));
        
        let client_ip = "8.8.8.8".parse::<IpAddr>().unwrap(); // Untrusted
        
        let result_builder = apply_forwarded_headers(builder, &headers, client_ip, &config);
        
        // Proto and Host should not be forwarded from untrusted source
        let request = result_builder.build().unwrap();
        assert!(request.headers().get("x-forwarded-proto").is_none());
        assert!(request.headers().get("x-forwarded-host").is_none());
        
        // X-Forwarded-For should be set to actual client IP (appended to existing)
        let xff = request.headers().get("x-forwarded-for").unwrap().to_str().unwrap();
        assert_eq!(xff, "1.2.3.4, 8.8.8.8");
    }

    #[test]
    fn test_apply_forwarded_headers_no_existing_xff() {
        use reqwest::Client;
        
        let mut config = Config::with_upstream("https://example.com".to_string());
        config.server.forward_headers_enabled = true;
        config.server.trusted_proxies = vec!["10.0.0.0/8".to_string()];

        let client = Client::new();
        let builder = client.get("https://example.com/test");
        
        let headers = HeaderMap::new(); // No X-Forwarded headers
        
        let client_ip = "8.8.8.8".parse::<IpAddr>().unwrap(); // Untrusted
        
        let result_builder = apply_forwarded_headers(builder, &headers, client_ip, &config);
        
        // X-Forwarded-For should be set to actual client IP
        let request = result_builder.build().unwrap();
        assert_eq!(request.headers().get("x-forwarded-for").unwrap(), "8.8.8.8");
    }
}
