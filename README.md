# Akkoma Media Proxy

A fast caching and optimization media proxy for Akkoma/Pleroma, built in Rust.

## Features

- **Caching Reverse Proxy**: Caches media and proxy requests to reduce load on upstream servers
- **Header Preservation**: Preserves all upstream headers by default, including redirects (302) with Location headers
- **Image Format Conversion**: Automatically converts images to modern formats (AVIF, WebP) based on client `Accept` headers
- **Path Filtering**: Only handles `/media` and `/proxy` endpoints for security
- **Performance**: Built with Tokio async runtime for high concurrency
- **Flexible Configuration**: TOML-based configuration with environment variable and CLI overrides
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
- Rust 1.85 or later

```bash
cargo build --release
./target/release/akkoma-media-proxy
```

## Configuration

Configuration is loaded with the following priority (highest to lowest):
1. **Environment Variables** (highest priority)
2. **Command-line Options**
3. **Configuration File** (lowest priority)

This means environment variables will override command-line options, which will override settings in the config file.

### Upstream Configuration

```toml
[upstream]
url = "https://your-akkoma-instance.com"  # Required: Your Akkoma/Pleroma instance
timeout = 30                              # Request timeout in seconds
```

### Server Configuration

```toml
[server]
bind = "0.0.0.0:3000"                          # Bind address
via_header = "akkoma-media-proxy/0.1.0"        # Via header value
preserve_upstream_headers = true               # Preserve all headers from upstream (default: true)
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
4. **Header Preservation**: All upstream headers (including Location for redirects) are preserved by default
5. **Image Conversion**: For images, converts to the best format based on `Accept` header:
   - Prefers AVIF if `image/avif` is accepted
   - Falls back to WebP if `image/webp` is accepted
   - Otherwise returns original or JPEG
6. **Caching**: Stores the converted response for future requests
7. **Response**: Returns the optimized content with appropriate headers

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

Environment variables have the **highest priority** and will override both command-line options and config file settings:

- `UPSTREAM_URL`: Upstream server URL (overrides config file and CLI option)
- `BIND_ADDRESS`: Server bind address (e.g., `0.0.0.0:3000`)
- `PRESERVE_HEADERS`: Preserve upstream headers (`true` or `false`)
- `RUST_LOG`: Logging level (e.g., `debug`, `info`, `warn`, `error`)

### Command-line Options

Command-line options have **medium priority** and will override config file settings:

```bash
akkoproxy [OPTIONS]

Options:
  -c, --config <FILE>          Path to configuration file
  -u, --upstream <URL>         Upstream server URL
  -b, --bind <ADDR>            Address to bind the server to
  --enable-avif                Enable AVIF conversion
  --disable-avif               Disable AVIF conversion
  --enable-webp                Enable WebP conversion
  --disable-webp               Disable WebP conversion
  --preserve-headers           Preserve all headers from upstream
  -h, --help                   Print help
  -V, --version                Print version
```

### Configuration Precedence Example

```bash
# Config file: config.toml
[upstream]
url = "https://config-url.com"

# Command line
./akkoproxy --upstream https://cli-url.com

# Environment variable
UPSTREAM_URL=https://env-url.com ./akkoproxy --upstream https://cli-url.com

# Result: https://env-url.com (environment has highest priority)
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

