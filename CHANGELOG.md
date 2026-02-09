# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] - 2026-02-09

### Fixed
- Fixed infinite redirect loop when Akkoma has `force_ssl: [rewrite_on: [:x_forwarded_proto]]` enabled by forwarding `X-Forwarded-Proto`, `X-Forwarded-For`, and `X-Forwarded-Host` headers to upstream

### Added
- X-Forwarded headers support for proper SSL/TLS detection

## [0.1.0] - 2024-12-06

### Added
- Initial implementation

[Unreleased]: https://github.com/BlockG-ws/fantastic-computing-machine/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/BlockG-ws/fantastic-computing-machine/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/BlockG-ws/fantastic-computing-machine/releases/tag/v0.1.0
