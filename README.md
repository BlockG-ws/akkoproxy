# Akkoma Media Proxy

A fast caching and optimization media proxy for Akkoma/Pleroma, built in Rust.

## Features

- **Caching Reverse Proxy**: Caches media and proxy requests to reduce load on upstream servers
- **Image Format Conversion**: Automatically converts images to modern formats (AVIF, WebP) based on client `Accept` headers
- **Path Filtering**: Only handles `/media` and `/proxy` endpoints for security
- **Performance**: Built with Tokio async runtime for high concurrency
- **Easy Configuration**: TOML-based configuration with sensible defaults
- **Out-of-the-box**: Works with just an upstream URL, no complex setup needed

## Quick Start

### Using Environment Variable

The simplest way to start:

```bash
UPSTREAM_URL=https://your-akkoma-instance.com ./akkoma-media-proxy
```

### Using Configuration File

Create a `config.toml` file:

```toml
[upstream]
url = "https://your-akkoma-instance.com"
```

Then run:

```bash
./akkoma-media-proxy
```

See `config.example.toml` for all available options.

## Installation

### From Binary

Download the latest release from the [releases page](https://github.com/BlockG-ws/fantastic-computing-machine/releases).

### Using Docker

```bash
docker run -p 3000:3000 \
  -e UPSTREAM_URL=https://your-akkoma-instance.com \
  ghcr.io/blockg-ws/akkoma-media-proxy:latest
```

With configuration file:

```bash
docker run -p 3000:3000 \
  -v $(pwd)/config.toml:/app/config.toml \
  ghcr.io/blockg-ws/akkoma-media-proxy:latest
```

### From Source

Requirements:
- Rust 1.70 or later

```bash
cargo build --release
./target/release/akkoma-media-proxy
```

## Configuration

### Upstream Configuration

```toml
[upstream]
url = "https://your-akkoma-instance.com"  # Required: Your Akkoma/Pleroma instance
timeout = 30                              # Request timeout in seconds
```

### Server Configuration

```toml
[server]
bind = "0.0.0.0:3000"                     # Bind address
via_header = "akkoma-media-proxy/0.1.0"   # Via header value
```

### Cache Configuration

```toml
[cache]
max_capacity = 10000      # Maximum number of cached items
ttl = 3600               # Cache TTL in seconds (1 hour)
max_item_size = 10485760  # Maximum cacheable item size (10MB)
```

### Image Processing Configuration

```toml
[image]
enable_avif = true        # Enable AVIF conversion
enable_webp = true        # Enable WebP conversion
quality = 85             # JPEG quality (1-100)
max_dimension = 4096     # Maximum image dimension
```

## How It Works

1. **Request Filtering**: Only `/media` and `/proxy` paths are allowed
2. **Cache Check**: Looks for cached response with the requested format
3. **Upstream Fetch**: If not cached, fetches from upstream server
4. **Image Conversion**: For images, converts to the best format based on `Accept` header:
   - Prefers AVIF if `image/avif` is accepted
   - Falls back to WebP if `image/webp` is accepted
   - Otherwise returns original or JPEG
5. **Caching**: Stores the converted response for future requests
6. **Response**: Returns the optimized content with appropriate headers

## Format Negotiation

The proxy respects the `Accept` header for image format selection:

```
Accept: image/avif,image/webp,image/*;q=0.8
```

Priority order:
1. AVIF (if enabled and accepted)
2. WebP (if enabled and accepted)
3. JPEG (fallback)

## Endpoints

- `GET /media/*` - Proxied media requests with caching and conversion
- `GET /proxy/*` - Proxied proxy requests with caching and conversion
- `GET /health` - Health check endpoint
- `GET /metrics` - Cache metrics (Prometheus-compatible)

## Security

- **Path Restriction**: Only `/media` and `/proxy` paths are allowed
- **No Directory Traversal**: Path validation prevents directory traversal attacks
- **Timeout Protection**: Upstream requests have configurable timeouts
- **Size Limits**: Configurable maximum cache item size
- **TLS**: Uses rustls for secure HTTPS connections to upstream

## Performance

- **Async I/O**: Built on Tokio for efficient concurrent request handling
- **Smart Caching**: LRU cache with TTL and size-based eviction
- **Connection Pooling**: Reuses HTTP connections to upstream
- **Efficient Image Processing**: Uses optimized image libraries

## Development

### Building

```bash
cargo build
```

### Testing

```bash
cargo test
```

### Running in Development

```bash
UPSTREAM_URL=https://example.com cargo run
```

## Environment Variables

- `CONFIG_PATH`: Path to configuration file (default: `config.toml`)
- `UPSTREAM_URL`: Upstream server URL (used if config file not found)
- `RUST_LOG`: Logging level (e.g., `debug`, `info`, `warn`, `error`)

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

