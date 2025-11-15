# maxmind-db-rust

[![Test](https://github.com/oschwald/maxmind-db-rust-ruby/actions/workflows/test.yml/badge.svg)](https://github.com/oschwald/maxmind-db-rust-ruby/actions/workflows/test.yml)
[![Lint](https://github.com/oschwald/maxmind-db-rust-ruby/actions/workflows/lint.yml/badge.svg)](https://github.com/oschwald/maxmind-db-rust-ruby/actions/workflows/lint.yml)

A high-performance Rust-based Ruby gem for reading MaxMind DB files. Provides API compatibility with the official `maxmind-db` gem while leveraging Rust for superior performance.

> **Note:** This is an unofficial library and is not endorsed by MaxMind. For the official Ruby library, see [maxmind-db](https://github.com/maxmind/MaxMind-DB-Reader-ruby).

## Features

- **High Performance**: Rust-based implementation provides significantly faster lookups than pure Ruby
- **API Compatible**: Familiar API similar to the official MaxMind::DB gem
- **Thread-Safe**: Safe to use from multiple threads
- **Memory Modes**: Support for both memory-mapped (MMAP) and in-memory modes
- **Iterator Support**: Iterate over all networks in the database (extension feature)
- **Type Support**: Works with both String and IPAddr objects

## Installation

Add this line to your application's Gemfile:

```ruby
gem 'maxmind-db-rust'
```

And then execute:

```bash
bundle install
```

Or install it yourself as:

```bash
gem install maxmind-db-rust
```

## Requirements

- Ruby 3.2 or higher
- Rust toolchain (for building from source)

## Usage

### Basic Usage

```ruby
require 'maxmind/db/rust'

# Open database
reader = MaxMind::DB::Rust::Reader.new(
  'GeoIP2-City.mmdb',
  mode: MaxMind::DB::Rust::MODE_MEMORY
)

# Lookup an IP address
record = reader.get('8.8.8.8')
if record
  puts record['country']['iso_code']
  puts record['country']['names']['en']
  puts record['city']['names']['en']
end

# Close the database
reader.close
```

### Get with Prefix Length

```ruby
require 'maxmind/db/rust'

reader = MaxMind::DB::Rust::Reader.new('GeoIP2-City.mmdb')

record, prefix_length = reader.get_with_prefix_length('8.8.8.8')
puts "Record: #{record}"
puts "Prefix length: #{prefix_length}"

reader.close
```

### Using IPAddr Objects

```ruby
require 'maxmind/db/rust'
require 'ipaddr'

reader = MaxMind::DB::Rust::Reader.new('GeoIP2-City.mmdb')

ip = IPAddr.new('8.8.8.8')
record = reader.get(ip)

reader.close
```

### Database Modes

```ruby
require 'maxmind/db/rust'

# MODE_AUTO: Uses memory-mapped files (default, best performance)
reader = MaxMind::DB::Rust::Reader.new(
  'GeoIP2-City.mmdb',
  mode: MaxMind::DB::Rust::MODE_AUTO
)

# MODE_MMAP: Explicitly use memory-mapped files (recommended)
reader = MaxMind::DB::Rust::Reader.new(
  'GeoIP2-City.mmdb',
  mode: MaxMind::DB::Rust::MODE_MMAP
)

# MODE_MEMORY: Load entire database into memory
reader = MaxMind::DB::Rust::Reader.new(
  'GeoIP2-City.mmdb',
  mode: MaxMind::DB::Rust::MODE_MEMORY
)
```

### Accessing Metadata

```ruby
require 'maxmind/db/rust'

reader = MaxMind::DB::Rust::Reader.new('GeoIP2-City.mmdb')

metadata = reader.metadata
puts "Database type: #{metadata.database_type}"
puts "Node count: #{metadata.node_count}"
puts "Record size: #{metadata.record_size}"
puts "IP version: #{metadata.ip_version}"
puts "Build epoch: #{metadata.build_epoch}"
puts "Languages: #{metadata.languages.join(', ')}"
puts "Description: #{metadata.description}"

reader.close
```

### Iterator Support (Extension Feature)

Iterate over all networks in the database:

```ruby
require 'maxmind/db/rust'

reader = MaxMind::DB::Rust::Reader.new('GeoLite2-Country.mmdb')

# Iterate over all networks
reader.each do |network, data|
  puts "#{network}: #{data['country']['iso_code']}"
  break # Remove this to see all networks
end

# Use Enumerable methods
countries = reader.map { |network, data| data['country']['iso_code'] }.uniq
puts "Unique countries: #{countries.size}"

reader.close
```

## API Documentation

### `MaxMind::DB::Rust::Reader`

#### `new(database_path, options = {})`

Create a new Reader instance.

**Parameters:**
- `database_path` (String): Path to the MaxMind DB file
- `options` (Hash): Optional configuration
  - `:mode` (Symbol): One of `:MODE_AUTO`, `:MODE_MEMORY`, or `:MODE_MMAP`

**Returns:** Reader instance

**Raises:**
- `Errno::ENOENT`: If the database file does not exist
- `MaxMind::DB::Rust::InvalidDatabaseError`: If the file is not a valid MaxMind DB

#### `get(ip_address)`

Look up an IP address in the database.

**Parameters:**
- `ip_address` (String or IPAddr): The IP address to look up

**Returns:** Hash with the record data, or `nil` if not found

**Raises:**
- `ArgumentError`: If looking up IPv6 in an IPv4-only database
- `MaxMind::DB::Rust::InvalidDatabaseError`: If the database is corrupt

#### `get_with_prefix_length(ip_address)`

Look up an IP address and return the prefix length.

**Parameters:**
- `ip_address` (String or IPAddr): The IP address to look up

**Returns:** Array `[record, prefix_length]` where record is a Hash or `nil`

#### `metadata()`

Get metadata about the database.

**Returns:** `MaxMind::DB::Rust::Metadata` instance

#### `close()`

Close the database and release resources.

#### `closed()`

Check if the database has been closed.

**Returns:** Boolean

#### `each { |network, data| ... }`

Iterate over all networks in the database.

**Yields:** IPAddr network and Hash data for each entry

**Returns:** Enumerator if no block given

### `MaxMind::DB::Rust::Metadata`

Metadata attributes:
- `binary_format_major_version` - Major version of the binary format
- `binary_format_minor_version` - Minor version of the binary format
- `build_epoch` - Unix timestamp when the database was built
- `database_type` - Type of database (e.g., "GeoIP2-City")
- `description` - Hash of locale codes to descriptions
- `ip_version` - 4 for IPv4-only, 6 for IPv4/IPv6 support
- `languages` - Array of supported locale codes
- `node_count` - Number of nodes in the search tree
- `record_size` - Record size in bits (24, 28, or 32)
- `node_byte_size` - Size of a node in bytes
- `search_tree_size` - Size of the search tree in bytes

### Constants

- `MaxMind::DB::Rust::MODE_AUTO` - Automatically choose the best mode (uses MMAP)
- `MaxMind::DB::Rust::MODE_MEMORY` - Load entire database into memory
- `MaxMind::DB::Rust::MODE_MMAP` - Use memory-mapped file I/O (recommended)

### Exceptions

- `MaxMind::DB::Rust::InvalidDatabaseError` - Raised when the database file is corrupt or invalid

## Comparison with Official Gem

| Feature | maxmind-db (official) | maxmind-db-rust (this gem) |
|---------|----------------------|---------------------------|
| Implementation | Pure Ruby | Rust with Ruby bindings |
| Performance | Baseline | 10-50x faster |
| API | MaxMind::DB | MaxMind::DB::Rust |
| MODE_FILE | ✓ | ✗ |
| MODE_MEMORY | ✓ | ✓ |
| MODE_AUTO | ✓ | ✓ |
| MODE_MMAP | ✗ | ✓ |
| Iterator support | ✗ | ✓ |
| Thread-safe | ✓ | ✓ |

## Performance

Expected performance characteristics (will vary based on hardware):
- Single-threaded lookups: 300,000 - 500,000 lookups/second
- Significantly faster than pure Ruby implementations
- Memory-mapped mode (MMAP) provides best performance
- Fully thread-safe for concurrent lookups

## Development

Interested in contributing? See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed developer documentation, including:

- Development setup and prerequisites
- Building and testing the extension
- Code quality guidelines
- Project structure
- Submitting changes

### Quick Start

```bash
git clone https://github.com/oschwald/maxmind-db-rust-ruby.git
cd maxmind-db-rust-ruby
git submodule update --init --recursive
bundle install
bundle exec rake compile
bundle exec rake test
```

## Contributing

1. Fork it
2. Create your feature branch (`git checkout -b my-new-feature`)
3. Commit your changes (`git commit -am 'Add some feature'`)
4. Push to the branch (`git push origin my-new-feature`)
5. Create a new Pull Request

## License

This software is licensed under the ISC License. See the LICENSE file for details.

## Support

- **Issues**: https://github.com/oschwald/maxmind-db-rust-ruby/issues
- **Documentation**: https://www.rubydoc.info/gems/maxmind-db-rust

## Credits

This gem uses the [maxminddb](https://github.com/oschwald/maxminddb-rust) Rust crate for the core MaxMind DB reading functionality.

Built with [magnus](https://github.com/matsadler/magnus) and [rb-sys](https://github.com/oxidize-rb/rb-sys).
