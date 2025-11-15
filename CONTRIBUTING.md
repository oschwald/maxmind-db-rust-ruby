# Contributing to maxmind-db-rust

Thank you for your interest in contributing to maxmind-db-rust! This document provides guidelines and instructions for developers.

## Table of Contents

- [Development Setup](#development-setup)
- [Building the Extension](#building-the-extension)
- [Running Tests](#running-tests)
- [Code Quality](#code-quality)
- [Project Structure](#project-structure)
- [Making Changes](#making-changes)
- [Testing Guidelines](#testing-guidelines)
- [Submitting Changes](#submitting-changes)
- [Release Process](#release-process)

## Development Setup

### Prerequisites

- Ruby 3.2 or higher
- Rust toolchain (stable)
- Bundler
- Git

### Installing Rust

If you don't have Rust installed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### Clone and Setup

```bash
git clone https://github.com/oschwald/maxmind-db-rust-ruby.git
cd maxmind-db-rust-ruby

# Initialize git submodules (for test data)
git submodule update --init --recursive

# Install dependencies
bundle install

# Configure git to use the .githooks directory
git config core.hooksPath .githooks
```

This will enable the pre-commit hook that runs `precious lint` on staged files before each commit.

## Building the Extension

### First Time Build

```bash
# Compile the Rust extension
bundle exec rake compile
```

This will:

1. Run `extconf.rb` to generate the Makefile
2. Compile the Rust code using rb-sys
3. Place the compiled extension in `lib/maxmind/db/`

### Clean Build

```bash
# Clean build artifacts
bundle exec rake clean

# Clean and rebuild
bundle exec rake clobber compile
```

### Development Build (Debug)

The default rake task builds in release mode. For faster compile times during development:

```bash
# Set environment variable for debug builds
CARGO_PROFILE=dev bundle exec rake compile
```

Note: Debug builds are significantly slower at runtime but compile faster.

## Running Tests

### Test Organization

Tests are organized into two categories:

1. **Our Own Tests** (`test/*_test.rb`)
   - Tests specific to this implementation
   - License: ISC (same as project)

2. **MaxMind Upstream Tests** (`test/maxmind/test_*.rb`)
   - Adapted from official MaxMind-DB-Reader-ruby
   - License: Apache-2.0 or MIT (MaxMind, Inc.)
   - See `test/maxmind/README.md` for details

### Running Tests

```bash
# Run all tests (recommended before submitting PR)
bundle exec rake test

# Run only our own tests
bundle exec rake test_own

# Run only MaxMind upstream compatibility tests
bundle exec rake test_maxmind

# Run with verbose output
bundle exec rake test_verbose

# Run specific test file
bundle exec ruby test/reader_test.rb

# Run specific test method
bundle exec ruby test/reader_test.rb -n test_get_ipv4_address
```

### Test Data

Test databases are stored in `test/data/MaxMind-DB/` as a git submodule. If tests are failing with "file not found" errors:

```bash
git submodule update --init --recursive
```

## Code Quality

### Precious (Recommended)

The easiest way to run all linters and formatters is using [precious](https://github.com/houseabsolute/precious):

```bash
# Install precious (once)
cargo install precious

# Check all linters
precious lint --all

# Auto-fix all issues
precious tidy --all

# Check only staged files (useful before committing)
precious lint --staged

# Run specific linter
precious lint -c rubocop
```

The pre-commit hook automatically runs `precious lint --staged` before each commit.

### RuboCop (Ruby)

```bash
# Check Ruby code style
bundle exec rubocop

# Auto-fix issues
bundle exec rubocop -a

# Check specific files
bundle exec rubocop lib/maxmind/db/rust.rb
```

### Clippy (Rust)

```bash
# Run Rust linter
cd ext/maxmind_db_rust
cargo clippy -- -D warnings
```

### Formatting

```bash
# Format Rust code
cd ext/maxmind_db_rust
cargo fmt

# Check formatting without changing files
cargo fmt -- --check
```

## Project Structure

```
maxmind-db-rust-ruby/
├── ext/maxmind_db_rust/          # Rust extension code
│   ├── Cargo.toml                # Rust dependencies
│   ├── extconf.rb                # Ruby build configuration
│   └── src/
│       └── lib.rs                # Main Rust implementation
├── lib/                          # Ruby integration layer
│   └── maxmind/
│       └── db/
│           └── rust.rb           # Ruby API wrapper
├── test/                         # Test suite
│   ├── *_test.rb                 # Our own tests
│   ├── maxmind/                  # Upstream tests (MaxMind copyright)
│   │   ├── README.md             # Licensing info
│   │   └── test_*.rb             # Adapted tests
│   └── data/
│       └── MaxMind-DB/           # Test databases (submodule)
├── Rakefile                      # Build and test tasks
├── maxmind-db-rust.gemspec       # Gem specification
├── README.md                     # User documentation
├── CONTRIBUTING.md               # This file
├── CHANGELOG.md                  # Version history
└── LICENSE                       # ISC License
```

## Making Changes

### Workflow

1. **Create a branch** for your changes:

   ```bash
   git checkout -b feature/my-new-feature
   ```

2. **Make your changes**:
   - Rust code: `ext/maxmind_db_rust/src/lib.rs`
   - Ruby wrapper: `lib/maxmind/db/rust.rb`
   - Tests: Add tests in `test/` directory

3. **Compile and test**:

   ```bash
   bundle exec rake compile
   bundle exec rake test
   ```

4. **Check code quality**:

   ```bash
   bundle exec rubocop
   cd ext/maxmind_db_rust && cargo clippy && cargo fmt
   ```

5. **Commit your changes**:
   ```bash
   git add .
   git commit -m "Add feature: description"
   ```

### Commit Message Guidelines

- Use present tense ("Add feature" not "Added feature")
- Use imperative mood ("Move cursor to..." not "Moves cursor to...")
- First line should be 50 characters or less
- Reference issues and pull requests when relevant

Example:

```
Add support for custom metadata attributes

- Implement metadata attribute getter
- Add tests for custom attributes
- Update documentation

Fixes #123
```

## Testing Guidelines

### Writing Tests

When adding new features or fixing bugs:

1. **Add tests to our own test suite** (`test/*_test.rb`):

   ```ruby
   def test_new_feature
     reader = MaxMind::DB::Rust::Reader.new('path/to/test.mmdb')
     result = reader.new_method
     assert_equal expected_value, result
     reader.close
   end
   ```

2. **Ensure existing tests pass**:

   ```bash
   bundle exec rake test
   ```

3. **Test both modes** (MMAP and MEMORY) when relevant:
   ```ruby
   [MaxMind::DB::Rust::MODE_MMAP, MaxMind::DB::Rust::MODE_MEMORY].each do |mode|
     reader = MaxMind::DB::Rust::Reader.new(path, mode: mode)
     # Test logic here
     reader.close
   end
   ```

### Test Coverage

- Test normal operation (happy path)
- Test edge cases (empty results, boundary conditions)
- Test error conditions (invalid input, closed readers, etc.)
- Test thread safety for concurrent operations
- Test both IPv4 and IPv6 addresses

### Upstream Test Synchronization

When the official MaxMind-DB-Reader-ruby gem is updated:

1. Update the submodule:

   ```bash
   cd test/maxmind-db-reader-ruby
   git pull origin main
   cd ../..
   git add test/maxmind-db-reader-ruby
   ```

2. Review changes:

   ```bash
   cd test/maxmind-db-reader-ruby
   git log --oneline --since="3 months ago" -- test/
   git diff HEAD~10..HEAD -- test/test_reader.rb
   ```

3. Apply relevant changes to `test/maxmind/test_reader.rb`:
   - Maintain our namespace changes (MaxMind::DB::Rust)
   - Keep MODE_MMAP instead of MODE_FILE
   - Preserve our path adjustments
   - Update expected error messages if needed

4. Document changes in `test/maxmind/README.md`

## Submitting Changes

### Before Submitting a Pull Request

1. **Ensure all tests pass**:

   ```bash
   bundle exec rake test
   ```

2. **Run code quality checks**:

   ```bash
   bundle exec rubocop
   cd ext/maxmind_db_rust && cargo clippy && cargo fmt --check
   ```

3. **Update documentation** if needed:
   - README.md for user-facing changes
   - CHANGELOG.md for notable changes
   - Code comments for complex logic

4. **Rebase on main**:
   ```bash
   git fetch origin
   git rebase origin/main
   ```

### Pull Request Process

1. **Push your branch**:

   ```bash
   git push origin feature/my-new-feature
   ```

2. **Create a pull request** on GitHub with:
   - Clear description of changes
   - Reference to related issues
   - Screenshots/examples if applicable
   - Test results

3. **Address review feedback**:
   - Make requested changes
   - Push updates to the same branch
   - Respond to comments

4. **Wait for approval** and merge

## Release Process

### Version Numbering

We follow [Semantic Versioning](https://semver.org/):

- **MAJOR**: Incompatible API changes
- **MINOR**: Backward-compatible functionality additions
- **PATCH**: Backward-compatible bug fixes

### Creating a Release

1. **Update version** in `maxmind-db-rust.gemspec`:

   ```ruby
   s.version = '0.2.0'
   ```

2. **Update CHANGELOG.md**:

   ```markdown
   ## [0.2.0] - 2025-01-15

   ### Added

   - New feature X

   ### Fixed

   - Bug fix Y
   ```

3. **Commit version bump**:

   ```bash
   git add maxmind-db-rust.gemspec CHANGELOG.md
   git commit -m "Bump version to 0.2.0"
   ```

4. **Create and push tag**:

   ```bash
   git tag -a v0.2.0 -m "Release version 0.2.0"
   git push origin main
   git push origin v0.2.0
   ```

5. **Build and publish gem**:
   ```bash
   bundle exec rake build
   gem push pkg/maxmind-db-rust-0.2.0.gem
   ```

## Performance Considerations

### Benchmarking

When making performance-related changes:

1. **Create benchmark script**:

   ```ruby
   require 'benchmark'
   require 'maxmind/db/rust'

   reader = MaxMind::DB::Rust::Reader.new('path/to/db.mmdb')
   ips = File.readlines('test_ips.txt').map(&:strip)

   Benchmark.bm do |x|
     x.report("lookups:") do
       100_000.times { reader.get(ips.sample) }
     end
   end
   ```

2. **Compare before and after** your changes

3. **Document results** in PR description

### Optimization Tips

- Use MODE_MMAP for best performance (default)
- Release GIL during I/O operations (already implemented)
- Minimize Ruby object allocations in hot paths
- Use Arc for thread-safe sharing instead of cloning

## Getting Help

- **Issues**: https://github.com/oschwald/maxmind-db-rust-ruby/issues
- **Discussions**: Use GitHub Discussions for questions
- **MaxMind Docs**: https://maxmind.github.io/MaxMind-DB/

## Code of Conduct

- Be respectful and inclusive
- Provide constructive feedback
- Focus on the code, not the person
- Help others learn and grow

## License

By contributing, you agree that your contributions will be licensed under the ISC License.

Thank you for contributing! 🎉
