# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-02-10

### Added
- **Human-readable size formats**: Configuration options `max_item_size` and `disk_cache_max_size` now support human-readable formats like "10M", "1G", "10MiB", "1GiB", etc.
  - Supports both decimal (MB, GB) and binary (MiB, GiB) units
  - Backward compatible with numeric byte values
- **Command-line options for cache settings**:
  - `--forward-headers` / `--no-forward-headers`: Enable/disable X-Forwarded-* header forwarding
  - `--disk-cache` / `--no-disk-cache`: Enable/disable disk-based cache
  - `--disk-cache-path <PATH>`: Specify disk cache directory
  - `--disk-cache-max-size <SIZE>`: Set maximum disk cache size with human-readable format support
- **Environment variables for cache settings**:
  - `FORWARD_HEADERS_ENABLED`: Enable/disable X-Forwarded-* header forwarding (true/false)
  - `DISK_CACHE_ENABLED`: Enable/disable disk-based cache (true/false)
  - `DISK_CACHE_PATH`: Specify disk cache directory path
  - `DISK_CACHE_MAX_SIZE`: Set maximum disk cache size with human-readable format support

### Changed
- Bumped version from 0.1.2 to 0.2.0

## [0.1.2] - 2026-02-10

### Added
- **Optional nginx-like disk cache**: Persistent disk-based cache for media files
  - New `disk_cache_enabled` configuration option (default: false)
  - New `disk_cache_path` configuration option to specify cache directory (default: ./cache)
  - New `disk_cache_max_size` configuration option to limit disk space usage (default: 1GB)
  - Automatic LRU eviction when disk cache size limit is reached
  - TTL-based expiration using file metadata
  - Atomic file writes to prevent cache corruption
  - Fallback chain: memory cache → disk cache → upstream fetch
  - Cache statistics in `/metrics` endpoint now include disk cache metrics

### Changed
- Bumped version from 0.1.1 to 0.1.2

## [0.1.1] - 2026-02-09

### Fixed
- Fixed infinite redirect loop when Akkoma has `force_ssl: [rewrite_on: [:x_forwarded_proto]]` enabled

### Added
- **Secure X-Forwarded headers support**: Opt-in forwarding of `X-Forwarded-Proto`, `X-Forwarded-For`, and `X-Forwarded-Host` headers with trusted proxy validation
- Configuration options `forward_headers_enabled` and `trusted_proxies` for controlling header forwarding behavior
- IP address and CIDR range matching for trusted proxy verification
- Automatic header derivation from actual connection for untrusted sources
- Comprehensive test suite for header forwarding and trusted proxy functionality

### Security
- X-Forwarded headers are now only honored from explicitly trusted proxy sources
- Prevents header spoofing attacks by validating client IP against configured trusted proxies
- Headers from untrusted sources are ignored or overwritten with actual connection information

## [0.1.0] - 2024-12-06

### Added
- Initial implementation

[Unreleased]: https://github.com/BlockG-ws/akkoproxy/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/BlockG-ws/akkoproxy/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/BlockG-ws/akkoproxy/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/BlockG-ws/akkoproxy/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/BlockG-ws/akkoproxy/releases/tag/v0.1.0
