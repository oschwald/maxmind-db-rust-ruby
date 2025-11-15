# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
