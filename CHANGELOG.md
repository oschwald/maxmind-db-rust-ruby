# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.4] - 2025-11-16

### Fixed

- Release workflow for publishing multiple platform-specific gems

## [0.1.3] - 2025-11-16

### Added

- Pre-compiled native gems for multiple platforms, eliminating the need to compile Rust during installation:
  - `x86_64-linux` (Linux x86_64)
  - `aarch64-linux` (Linux ARM64)
  - `x86_64-darwin` (macOS Intel)
  - `arm64-darwin` (macOS Apple Silicon)
  - `x64-mingw-ucrt` (Windows)
  - `x86_64-linux-musl` (Alpine Linux)
- Source gem as fallback for platforms without pre-compiled binaries

## [0.1.2] - 2025-11-15

### Added

- Automated release script (`dev-bin/release.sh`) that validates changelog dates, updates gemspec version, runs tests, and creates GitHub releases

### Changed

- Updated actions/checkout from v4 to v5 in GitHub workflows

### Fixed

- Release workflow no longer runs twice (removed redundant triggers)

### Removed

- Unused test/maxmind-db-reader-ruby git submodule (documentation now references upstream repository by URL)

## [0.1.1] - 2025-11-15

### Fixed

- Release workflow now has environment set.

## [0.1.0] - 2025-11-15

### Added

- Initial release
- Reader class with `get()`, `get_with_prefix_length()`, `metadata()`, `close()`, and `closed()` methods
- Metadata class with all standard MaxMind DB metadata attributes
- Support for MODE_AUTO, MODE_MEMORY, and MODE_MMAP modes
- Iterator support via `each` method (Enumerable interface)
  - Iterate over all networks in database
  - Network-scoped iteration with optional CIDR parameter (String or IPAddr)
- InvalidDatabaseError exception for corrupt databases
- Thread-safe implementation using Rust Arc and RwLock
- Support for both String and IPAddr IP address inputs
- High-performance Rust implementation using maxminddb crate
- Comprehensive API documentation

### Not Implemented

- MODE_FILE support (use MODE_MMAP instead)
- File descriptor support in constructor
