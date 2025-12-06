# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial release of Akkoma Media Proxy
- Caching reverse proxy for Akkoma/Pleroma media
- Automatic image format conversion (AVIF, WebP)
- Content negotiation based on Accept headers
- Path filtering for `/media` and `/proxy` endpoints
- TOML-based configuration with sensible defaults
- Environment variable configuration support
- Docker support with multi-platform builds
- GitHub Actions CI/CD pipeline
- Health check endpoint (`/health`)
- Metrics endpoint (`/metrics`)
- Comprehensive documentation
- Example configuration files
- Docker Compose example

### Features
- High-performance async I/O with Tokio
- Intelligent caching with TTL and size limits
- Image quality and dimension controls
- Configurable Via header
- Connection pooling for upstream requests
- CORS support
- Gzip/Brotli compression
- Security hardening (path restrictions, timeouts)

## [0.1.0] - 2024-12-06

### Added
- Initial implementation

[Unreleased]: https://github.com/BlockG-ws/fantastic-computing-machine/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/BlockG-ws/fantastic-computing-machine/releases/tag/v0.1.0
