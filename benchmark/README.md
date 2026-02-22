# Benchmarks

This directory contains benchmark scripts for `maxmind-db-rust`.

## compare_lookups.rb

Compares random IP lookup throughput between the official MaxMind Ruby gem and this Rust implementation.

### Usage

```bash
ruby benchmark/compare_lookups.rb path/to/database.mmdb [iterations]
```

**Arguments:**

- `path/to/database.mmdb` - Required. Path to the MaxMind DB file to benchmark
- `iterations` - Optional. Number of random IP lookups to perform (default: 10,000)

### Examples

```bash
# Quick test with 1,000 lookups
ruby benchmark/compare_lookups.rb test/data/MaxMind-DB/test-data/GeoIP2-City-Test.mmdb 1000

# Standard benchmark with 10,000 lookups (default)
ruby benchmark/compare_lookups.rb GeoLite2-City.mmdb

# Intensive benchmark with 100,000 lookups
ruby benchmark/compare_lookups.rb GeoIP2-City.mmdb 100000
```

### Comparing with Official Gem

To compare against the official MaxMind gem, install it first:

```bash
gem install maxmind-db
```

Then run the benchmark. It will automatically detect and compare both implementations.

### What it Measures

The benchmark:

- Generates random IP addresses (IPv4 or IPv6 based on database)
- Performs lookups using both implementations (if available)
- Measures total time and calculates lookups per second
- Compares performance between:
  - Official MaxMind gem (FILE mode, which uses mmap internally)
  - Rust implementation (MMAP mode)
  - Rust implementation (Memory mode)

### Sample Output

```
MaxMind DB Benchmark: Official vs Rust Implementation
======================================================================
Database: GeoIP2-City.mmdb
Iterations: 10000

Database IP version: IPv6

Generating 10000 random IPv6 addresses...
Done.

Benchmarking official MaxMind::DB::Reader...
    0.269740   0.020994   0.290734 (  0.290761)
  Lookups/sec: 171962.59

Benchmarking MaxMind::DB::Rust::Reader (MMAP mode)...
    0.008790   0.000000   0.008790 (  0.008790)
  Lookups/sec: 5688042.06

Benchmarking MaxMind::DB::Rust::Reader (Memory mode)...
    0.008713   0.000037   0.008750 (  0.008750)
  Lookups/sec: 5714401.33

======================================================================
SUMMARY
======================================================================
Official (FILE):         171962.59 lookups/sec
Rust (MMAP):            5688042.06 lookups/sec (33.08x)
Rust (Memory):          5714401.33 lookups/sec (33.23x)
```

### Notes

- The benchmark uses random IP addresses, so your results may vary with your real query mix.
- Memory mode loads the whole database into RAM.
- MMAP mode uses memory-mapped I/O and usually has similar lookup throughput.
- The official gem is benchmarked in `MODE_FILE` (which uses mmap internally).
- In this environment, using `/var/lib/GeoIP/GeoIP2-City.mmdb` with 50k random lookups, Rust measured about `47x` higher throughput than the official gem.
- Use this script on your production-like database to get realistic numbers for your environment.
