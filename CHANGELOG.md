# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added `Reader#verify` for opt-in comprehensive database integrity checks
  using the verifier introduced by `maxminddb` 0.30.0.
- Added iteration and shared-reader thread-scaling benchmarks, including a
  cache-root retention check across thread churn.

### Performance

- Batched Ruby array and hash insertion while decoding MMDB records to reduce
  protected Ruby C API calls during full-record lookups.
- Replaced per-thread string caches with one fixed-size process cache that uses
  its Ruby root array directly, improving reuse while bounding retained memory.
- Batched `get_many` and `get_many_path` result construction to reduce Ruby
  array insertion overhead for large input collections.
- Improved `IPAddr` lookup performance by converting its public integer
  representation directly instead of allocating packed address strings.
- Avoided allocating a temporary candidate vector on parsed-path cache hits.

### Changed

- Upgraded the `maxminddb` crate to 0.30.0, improving selective path decoding
  and hardening iteration and verification of corrupt databases.
- Documented selective lookup and input-type performance guidance, and
  clarified that thread-safe lookups remain serialized by MRI's global VM lock.
- Documented the file-lifecycle safety contract for memory-mapped readers and
  the requirement to atomically replace database paths during updates.
- Simplified reader ownership by removing redundant outer `Arc` wrappers and a
  duplicate closed flag while preserving atomic close semantics.
- Removed the redundant extension-level Cargo lockfile; workspace builds use
  the root lockfile as the single dependency lock.
- Moved MMDB-to-Ruby decoding and string-cache logic into a focused Rust module.
- Alternated baseline and candidate subprocesses between benchmark samples to
  reduce order and thermal bias in git-ref comparisons.
- Added fixed-record and prebuilt-`IPAddr` benchmark cases for cache-hot and
  input-conversion performance comparisons.
- Extended allocation benchmarking to cover selective batch lookups.

### Fixed

- Decoded MMDB UTF-8 strings and map keys directly into Ruby strings from raw
  bytes, avoiding unchecked Rust strings while preserving Ruby's behavior for
  invalid UTF-8 in corrupt databases.
- Propagated Ruby hash insertion errors from `Metadata#description` instead of
  silently returning a partial description.

## [0.5.0] - 2026-06-14

### Added

- Added `MODE_FILE` as an official-gem compatibility alias for path-backed
  memory-mapped readers.
- Added `MODE_PARAM_IS_BUFFER` for constructing readers from Ruby strings
  containing database bytes.
- Added `Reader#get_path` and `Reader#get_many_path` for selective path lookups.
- Added `Reader#get_many` for batch lookups over arrays and enumerables.
- Added Enumerator return support for `Reader#each` when called without a block.
- Added `Reader#inspect` with closed state and database IP version.
- Added benchmark tooling for comparing git refs and measuring Ruby object
  allocations.
- Added official gem parity tests, compatibility audit tests, bad data corpus
  tests, and reader concurrency stress tests.

### Changed

- Upgraded the `maxminddb` crate to 0.28.1.
- Improved IPv4 string lookup performance with a strict fast-path parser.
- Streamed non-array `get_many` and `get_many_path` inputs instead of
  materializing enumerables.
- Cached parsed lookup paths per reader to reduce repeated path parsing.
- Simplified reader open mode parsing and centralized lookup error handling.
- Documented Rust extension safety invariants and `Send` requirements.

### Fixed

- Fixed source gem native extension installation so `require "maxmind/db/rust"`
  works after installing the source gem.
- Removed an unsafe reader iterator transmute.
- Improved invalid database error construction consistency across reader open
  and iteration paths.
- Fixed dead test assertions and made adapted MaxMind tests independent of the
  current working directory.

### Security

- Pinned GitHub Actions to commit SHAs.
- Restricted workflow permissions and disabled persisted checkout credentials.
- Added a zizmor workflow and source-gem install smoke test to release checks.

## [0.4.0] - 2026-04-25

### Performance

- Restored lookup performance with a generic bounded cache of frozen Ruby strings reused across decoded keys and scalar values.
- Removed hardcoded interned string tables in favor of the generic string cache.
- Simplified decoding so lookups and iteration use the same `maxminddb` decode path again.
- Reduced repeated cache-root lookup overhead with a thread-local `OnceCell` for the Ruby-owned string cache roots.
- Borrowed decoded map keys directly during deserialization to avoid `Cow` overhead in the hot decode path.
- Upgraded `maxminddb` crate to 0.28.0, which includes several performance
  improvements.

## [0.3.0] - 2026-02-22

### Changed

- Improved lookup performance by using a generic bounded key cache for decoded map keys.
- Improved `IPAddr` lookup performance by decoding packed bytes from `IPAddr#hton` directly.
- Switched map-key cache hashing to `FxHashMap` for faster key-cache access.
- Switched map-key cache roots to a Ruby-owned cache array with Rust key-to-index lookups.
- Refactored duplicated prefix and `within` decode paths in the Rust reader for simpler maintenance.
- Refactored duplicate database file-open error handling shared by MMAP and MEMORY modes.
- Updated Rust and Ruby dependencies.
- Added Ruby 4.0 coverage to CI workflows.

### Fixed

- Made extension initialization idempotent across `MaxMind::DB` class/module loading modes to avoid typed-data incompatibility when the extension is loaded more than once.
- When loaded with the official `MaxMind::DB` class, `MaxMind::DB::Rust` now uses anonymous module creation to preserve canonical module naming.
- Scoped Rust dependency cache per Ruby version in CI tests and stopped caching `target/` in the test workflow to avoid cross-version artifact contamination.

## [0.2.1] - 2025-12-18

### Changed

- Upgraded to the `0.27.1` release of the `maxminddb` crate.

## [0.2.0] - 2025-11-28

### Changed

- Upgraded to the `0.27.0` release of the `maxminddb` crate.
- Expanded String interning.

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
