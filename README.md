# maxmind-db-rust

[![Test](https://github.com/oschwald/maxmind-db-rust-ruby/actions/workflows/test.yml/badge.svg)](https://github.com/oschwald/maxmind-db-rust-ruby/actions/workflows/test.yml)
[![Lint](https://github.com/oschwald/maxmind-db-rust-ruby/actions/workflows/lint.yml/badge.svg)](https://github.com/oschwald/maxmind-db-rust-ruby/actions/workflows/lint.yml)

A Ruby gem for reading MaxMind DB files, implemented in Rust.
It keeps the API close to the official `maxmind-db` gem while adding Rust-backed performance.

> **Note:** This is an unofficial library and is not endorsed by MaxMind. For the official Ruby library, see [maxmind-db](https://github.com/maxmind/MaxMind-DB-Reader-ruby).

## Features

- Rust implementation focused on fast lookups
- API modeled after the official `maxmind-db` gem
- Thread-safe lookups
- Supports file-backed, MMAP, in-memory, and buffer-backed modes
- Includes network iteration support
- Accepts both `String` and `IPAddr` inputs
- Includes selective path lookup and batch lookup extensions

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

### Selective and Batch Lookups

```ruby
require 'maxmind/db/rust'

reader = MaxMind::DB::Rust::Reader.new('GeoIP2-City.mmdb')

# Decode one field without materializing the full record.
iso_code = reader.get_path('8.8.8.8', ['country', 'iso_code'])

# Batch full-record lookups.
ips = ['8.8.8.8', '1.1.1.1', '208.67.222.222']
records = reader.get_many(ips)

# Batch one-field lookups.
iso_codes = reader.get_many_path(ips, ['country', 'iso_code'])

reader.close
```

Decoded MMDB strings, including hash keys, are frozen. Call `dup` before
modifying a decoded string.

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

# MODE_FILE: Official-gem compatibility alias for path-backed MMAP
reader = MaxMind::DB::Rust::Reader.new(
  'GeoIP2-City.mmdb',
  mode: MaxMind::DB::Rust::MODE_FILE
)

# MODE_MEMORY: Load entire database into memory
reader = MaxMind::DB::Rust::Reader.new(
  'GeoIP2-City.mmdb',
  mode: MaxMind::DB::Rust::MODE_MEMORY
)

# MODE_PARAM_IS_BUFFER: Read from a String containing database bytes
buffer = File.binread('GeoIP2-City.mmdb')
reader = MaxMind::DB::Rust::Reader.new(
  buffer,
  mode: MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER
)
```

`MODE_AUTO`, `MODE_FILE`, and `MODE_MMAP` require the underlying file not to be
modified or truncated while the reader is alive. To refresh a database, write
the replacement to a new file and atomically rename it over the old path. Use
`MODE_MEMORY` if the file lifecycle cannot be controlled this way.

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

# Iterate over networks within a specific subnet (String CIDR notation)
reader.each('192.168.0.0/16') do |network, data|
  puts "#{network}: #{data['city']['names']['en']}"
end

# Iterate over networks within a specific subnet (IPAddr object)
require 'ipaddr'
subnet = IPAddr.new('10.0.0.0/8')
reader.each(subnet) do |network, data|
  puts "#{network}: #{data['country']['iso_code']}"
end

# Use Enumerable methods
countries = reader.map { |network, data| data['country']['iso_code'] }.uniq
puts "Unique countries: #{countries.size}"

reader.close
```

## API Documentation

### `MaxMind::DB::Rust::Reader`

#### `new(database, options = {})`

Create a new Reader instance.

**Parameters:**

- `database` (String): Path to the MaxMind DB file, or database bytes when using `:MODE_PARAM_IS_BUFFER`
- `options` (Hash): Optional configuration
  - `:mode` (Symbol): One of `:MODE_AUTO`, `:MODE_FILE`, `:MODE_MEMORY`, `:MODE_MMAP`, or `:MODE_PARAM_IS_BUFFER`

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

#### `get_path(ip_address, path)`

Look up an IP address and return only the value at `path`.

**Parameters:**

- `ip_address` (String or IPAddr): The IP address to look up
- `path` (Array): String map keys and Integer array indexes. Negative indexes count from the end.

**Returns:** The value at the path, or `nil` if the record or path is not found

#### `get_with_prefix_length(ip_address)`

Look up an IP address and return the prefix length.

**Parameters:**

- `ip_address` (String or IPAddr): The IP address to look up

**Returns:** Array `[record, prefix_length]` where record is a Hash or `nil`

#### `get_many(ip_addresses)`

Look up multiple IP addresses.

**Parameters:**

- `ip_addresses` (Array or Enumerable): IP address strings or IPAddr objects

**Returns:** Array of record values in input order

#### `get_many_path(ip_addresses, path)`

Look up one path for multiple IP addresses.

**Parameters:**

- `ip_addresses` (Array or Enumerable): IP address strings or IPAddr objects
- `path` (Array): String map keys and Integer array indexes

**Returns:** Array of path values in input order

#### `metadata()`

Get metadata about the database.

**Returns:** `MaxMind::DB::Rust::Metadata` instance

#### `verify()`

Perform a comprehensive validation of the database metadata, search tree, data
section separator, and referenced data records. Verification traverses the
entire database and may use memory proportional to the number of distinct
referenced values, so it is intended for explicit integrity checks rather than
the lookup hot path.

**Returns:** `true` when the database passes verification

**Raises:** `MaxMind::DB::Rust::InvalidDatabaseError` when verification fails

#### `close()`

Close the database and release resources.

#### `closed()`

Check if the database has been closed.

**Returns:** Boolean

#### `each(network = nil) { |network, data| ... }`

Iterate over networks in the database.

**Parameters:**

- `network` (String or IPAddr, optional): Network CIDR to iterate within (e.g., "192.168.0.0/16"). If omitted, iterates over all networks in the database.

**Yields:** IPAddr network and Hash data for each entry

**Returns:** Enumerator if no block given

**Raises:**

- `ArgumentError`: If network CIDR is invalid or IPv6 network specified for IPv4-only database

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
- `MaxMind::DB::Rust::MODE_FILE` - Official-gem compatibility alias for path-backed MMAP
- `MaxMind::DB::Rust::MODE_MEMORY` - Load entire database into memory
- `MaxMind::DB::Rust::MODE_MMAP` - Use memory-mapped file I/O (recommended)
- `MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER` - Read database bytes from a Ruby String

### Exceptions

- `MaxMind::DB::Rust::InvalidDatabaseError` - Raised when the database file is corrupt or invalid

## Comparison with Official Gem

| Feature              | maxmind-db (official) | maxmind-db-rust (this gem)                 |
| -------------------- | --------------------- | ------------------------------------------ |
| Implementation       | Pure Ruby             | Rust with Ruby bindings                    |
| Performance          | Baseline              | Faster lookup throughput in our benchmarks |
| API                  | MaxMind::DB           | MaxMind::DB::Rust                          |
| MODE_FILE            | ✓                     | ✓                                          |
| MODE_MEMORY          | ✓                     | ✓                                          |
| MODE_AUTO            | ✓                     | ✓                                          |
| MODE_PARAM_IS_BUFFER | ✓                     | ✓                                          |
| MODE_MMAP            | ✗                     | ✓                                          |
| Iterator support     | ✗                     | ✓                                          |
| Thread-safe          | ✓                     | ✓                                          |

## Performance

Lookup performance depends on hardware, Ruby version, database, and workload.

- In this project’s random-lookup benchmarks, this gem is consistently faster than the official Ruby implementation.
- `MODE_MMAP` and `MODE_MEMORY` both perform well; which is faster can vary by environment.
- Prefer `get_path` or `get_many_path` when only a small part of a record is
  needed. Selective decoding avoids constructing the rest of the Ruby object
  graph and can be substantially faster than a full-record lookup.
- Textual IP address strings use the fastest input path. `IPAddr` objects are
  supported but require Ruby method calls to extract their address family and
  integer value.
- For current, reproducible numbers on your own data and Ruby version, run `benchmark/compare_lookups.rb` against your database.
- Readers are safe to share across Ruby threads. Ruby-facing lookup and decode
  work currently runs under MRI's global VM lock, so additional Ruby threads do
  not make one process perform lookups in parallel.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for developer documentation, including:

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

1. Fork the repository
2. Create a feature branch (`git checkout -b my-new-feature`)
3. Commit your changes (`git commit -am 'Describe your change'`)
4. Push to the branch (`git push origin my-new-feature`)
5. Open a Pull Request

## License

This software is licensed under the ISC License. See the LICENSE file for details.

## Support

- **Issues**: https://github.com/oschwald/maxmind-db-rust-ruby/issues
- **Documentation**: https://www.rubydoc.info/gems/maxmind-db-rust

## Credits

This gem uses the [maxminddb](https://github.com/oschwald/maxminddb-rust) Rust crate for the core MaxMind DB reading functionality.

Built with [magnus](https://github.com/matsadler/magnus) and [rb-sys](https://github.com/oxidize-rb/rb-sys).
