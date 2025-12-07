use crate::cache::{CacheKey, CachedResponse, ResponseCache};
use crate::config::Config;
use crate::image::{is_image_content_type, parse_accept_header, format_from_content_type, format_satisfies, ImageConverter, OutputFormat};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Headers that should not be copied from upstream responses
const EXCLUDED_HEADERS: &[header::HeaderName] = &[
    header::CONTENT_LENGTH,
    header::TRANSFER_ENCODING,
    header::CONNECTION,
];

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
        debug!("Initializing AppState with config: bind={}, upstream={}", 
               config.server.bind, config.upstream.url);
        
        let cache = ResponseCache::new(
            config.cache.max_capacity,
            Duration::from_secs(config.cache.ttl),
            config.cache.max_item_size,
        );
        debug!("Cache initialized: max_capacity={}, ttl={}s, max_item_size={} bytes",
               config.cache.max_capacity, config.cache.ttl, config.cache.max_item_size);
        
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.upstream.timeout))
            .user_agent(format!("akkoproxy/{}", env!("CARGO_PKG_VERSION")))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("Failed to create HTTP client");
        debug!("HTTP client configured: timeout={}s, user_agent=akkoproxy/{}, redirect_policy=none",
               config.upstream.timeout, env!("CARGO_PKG_VERSION"));
        
        let image_converter = Arc::new(ImageConverter::new(
            config.image.quality,
            config.image.max_dimension,
            config.image.enable_avif,
            config.image.enable_webp,
        ));
        debug!("Image converter initialized: quality={}, max_dimension={}, avif={}, webp={}",
               config.image.quality, config.image.max_dimension, 
               config.image.enable_avif, config.image.enable_webp);
        
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
    State(state): State<AppState>,
    uri: Uri,
    headers: HeaderMap,
    _request: Request,
) -> Result<Response, ProxyError> {
    let path = uri.path();
    let query = uri.query().unwrap_or("");
    
    debug!("Proxying request: {} {}", path, query);
    
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
    
    // Build upstream URL
    let upstream_url = if query.is_empty() {
        format!("{}{}", state.config.upstream.url, path)
    } else {
        format!("{}{}?{}", state.config.upstream.url, path, query)
    };
    
    // Get Accept header to determine desired format
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("*/*");
    
    let desired_format = parse_accept_header(
        accept,
        state.config.image.enable_avif,
        state.config.image.enable_webp,
    );
    
    // Generate cache key
    let cache_key = CacheKey::new(
        format!("{}{}", path, if query.is_empty() { String::new() } else { format!("?{}", query) }),
        format!("{:?}", desired_format),
    );
    
    // Check cache first
    if let Some(cached) = state.cache.get(&cache_key).await {
        debug!("Cache hit for {}", path);
        return Ok(build_response(
            cached.data.clone(), 
            &cached.content_type, 
            &state.config.server.via_header, 
            None,
        ));
    }
    
    debug!("Cache miss for {}, fetching from upstream: {}", path, upstream_url);
    
    // Fetch from upstream
    let response = state.client
        .get(&upstream_url)
        .send()
        .await
        .map_err(|e| {
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
                info!("Successfully converted image: {} bytes -> {} bytes", body_bytes.len(), converted.len());
                (converted, mime_type.to_string())
            }
            Err(e) => {
                warn!("Failed to convert image: {}, returning original", e);
                (body_bytes, content_type)
            }
        }
    } else {
        if is_image_content_type(&content_type) && upstream_format.is_some() {
            debug!("Skipping conversion: upstream format {:?} already satisfies desired format {:?}", 
                   upstream_format, desired_format);
        } else {
            debug!("Not converting: is_image={}, format={:?}, size={}", 
                   is_image_content_type(&content_type), desired_format, body_bytes.len());
        }
        (body_bytes, content_type)
    };
    
    // Cache the response
    if final_data.len() <= state.config.cache.max_item_size as usize {
        let cached_response = CachedResponse {
            data: final_data.clone(),
            content_type: final_content_type.clone(),
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
    ))
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
) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::OK);
    
    // Add upstream headers if configured
    if let Some(headers) = upstream_headers {
        for (key, value) in headers.iter() {
            // Skip headers that shouldn't be copied
            if !EXCLUDED_HEADERS.contains(key) {
                builder = builder.header(key, value);
            }
        }
    }
    
    // Always set/override these headers
    builder
        .header(header::CONTENT_TYPE, content_type)
        .header(header::VIA, via_header)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
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
    let mut builder = Response::builder()
        .status(status);
    
    // Add upstream headers if configured
    if let Some(headers) = upstream_headers {
        for (key, value) in headers.iter() {
            // Skip headers that shouldn't be copied
            if !EXCLUDED_HEADERS.contains(key) {
                builder = builder.header(key, value);
            }
        }
    }
    
    // Always add Via header
    builder
        .header(header::VIA, via_header)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
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
        stats.entry_count,
        stats.weighted_size
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
            ProxyError::PathNotAllowed => {
                (StatusCode::FORBIDDEN, "Path not allowed".to_string())
            }
            ProxyError::UpstreamError(e) => {
                (StatusCode::BAD_GATEWAY, format!("Upstream error: {}", e))
            }
        };
        
        (status, message).into_response()
    }
}
