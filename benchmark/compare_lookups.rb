#!/usr/bin/env ruby
# frozen_string_literal: true

# Benchmark comparing MaxMind official Ruby gem vs Rust implementation
# Usage: ruby benchmark/compare_lookups.rb path/to/database.mmdb [iterations]
# Set RUST_ONLY=1 to skip requiring/benchmarking the official gem for faster runs.

require 'benchmark'
require 'ipaddr'

# Add lib directory to load path when running standalone
$LOAD_PATH.unshift(File.expand_path('../lib', __dir__))

# Try to require both implementations unless instructed otherwise
if ENV['RUST_ONLY'] == '1'
  OFFICIAL_AVAILABLE = false
  warn 'Skipping official maxmind-db gem benchmark (RUST_ONLY=1)'
else
  begin
    require 'maxmind/db'
    OFFICIAL_AVAILABLE = true
  rescue LoadError
    OFFICIAL_AVAILABLE = false
    warn 'Warning: Official maxmind-db gem not available. Install with: gem install maxmind-db'
  end
end

require 'maxmind/db/rust'

# Configuration
DB_PATH = ARGV[0]
ITERATIONS = (ARGV[1] || 10_000).to_i

unless DB_PATH
  warn 'Usage: ruby benchmark/compare_lookups.rb path/to/database.mmdb [iterations]'
  warn 'Example: ruby benchmark/compare_lookups.rb test/data/MaxMind-DB/test-data/GeoIP2-City-Test.mmdb 10000'
  exit 1
end

unless File.exist?(DB_PATH)
  warn "Error: Database file not found: #{DB_PATH}"
  exit 1
end

# Generate random IP addresses
def generate_random_ipv4
  "#{rand(1..255)}.#{rand(0..255)}.#{rand(0..255)}.#{rand(0..255)}"
end

def generate_random_ipv6
  parts = Array.new(8) { format('%x', rand(0..0xffff)) }
  parts.join(':')
end

def generate_test_ips(count, ipv6: false)
  Array.new(count) do
    ipv6 ? generate_random_ipv6 : generate_random_ipv4
  end
end

puts 'MaxMind DB Benchmark: Official vs Rust Implementation'
puts '=' * 70
puts "Database: #{DB_PATH}"
puts "Iterations: #{ITERATIONS}"
puts

# Detect IP version from database and check coverage
rust_reader = MaxMind::DB::Rust::Reader.new(DB_PATH)
ip_version = rust_reader.metadata.ip_version

# For IPv6-capable databases, check if IPv4 has better coverage
ipv6 = false
if ip_version == 6
  # Sample 100 random IPs of each type to check coverage
  ipv4_hits = 100.times.count do
    result = begin
      rust_reader.get(generate_random_ipv4)
    rescue StandardError
      nil
    end
    !result.nil?
  end

  ipv6_hits = 100.times.count do
    result = begin
      rust_reader.get(generate_random_ipv6)
    rescue StandardError
      nil
    end
    !result.nil?
  end

  # Use IPv4 if it has significantly better coverage
  ipv6 = ipv6_hits > ipv4_hits

  puts "Database IP version: IPv#{ip_version} (dual-stack)"
  puts "IPv4 sample hit rate: #{ipv4_hits}%"
  puts "IPv6 sample hit rate: #{ipv6_hits}%"
  puts "Using #{ipv6 ? 'IPv6' : 'IPv4'} addresses for benchmark"
else
  ipv6 = false
  puts "Database IP version: IPv#{ip_version}"
end

rust_reader.close
puts

# Generate test IPs
puts "Generating #{ITERATIONS} random IP#{ipv6 ? 'v6' : 'v4'} addresses..."
test_ips = generate_test_ips(ITERATIONS, ipv6: ipv6)
puts 'Done.'
puts

# Benchmark both implementations
results = {}

if OFFICIAL_AVAILABLE
  puts 'Benchmarking official MaxMind::DB::Reader...'

  # Test with MODE_FILE (official gem's default, uses mmap internally)
  official_reader = MaxMind::DB.new(DB_PATH, mode: MaxMind::DB::MODE_FILE)

  time = Benchmark.measure do
    test_ips.each do |ip|
      official_reader.get(ip)
    rescue StandardError
      # Ignore lookup errors for benchmark purposes
      nil
    end
  end

  official_reader.close
  results[:official] = time

  puts "  #{time}"
  puts "  Lookups/sec: #{(ITERATIONS / time.real).round(2)}"
  puts
end

puts 'Benchmarking MaxMind::DB::Rust::Reader (MMAP mode)...'

rust_reader_mmap = MaxMind::DB::Rust::Reader.new(DB_PATH, mode: MaxMind::DB::Rust::MODE_MMAP)

time = Benchmark.measure do
  test_ips.each do |ip|
    rust_reader_mmap.get(ip)
  rescue StandardError
    # Ignore lookup errors for benchmark purposes
    nil
  end
end

rust_reader_mmap.close
results[:rust_mmap] = time

puts "  #{time}"
puts "  Lookups/sec: #{(ITERATIONS / time.real).round(2)}"
puts

puts 'Benchmarking MaxMind::DB::Rust::Reader (Memory mode)...'

rust_reader_memory = MaxMind::DB::Rust::Reader.new(DB_PATH, mode: MaxMind::DB::Rust::MODE_MEMORY)

time = Benchmark.measure do
  test_ips.each do |ip|
    rust_reader_memory.get(ip)
  rescue StandardError
    # Ignore lookup errors for benchmark purposes
    nil
  end
end

rust_reader_memory.close
results[:rust_memory] = time

puts "  #{time}"
puts "  Lookups/sec: #{(ITERATIONS / time.real).round(2)}"
puts

# Summary comparison
puts '=' * 70
puts 'SUMMARY'
puts '=' * 70

if OFFICIAL_AVAILABLE
  official_rate = ITERATIONS / results[:official].real
  rust_mmap_rate = ITERATIONS / results[:rust_mmap].real
  rust_memory_rate = ITERATIONS / results[:rust_memory].real

  puts format('Official (FILE):      %10.2f lookups/sec', official_rate)
  puts format('Rust (MMAP):          %10.2f lookups/sec (%.2fx)',
              rust_mmap_rate, rust_mmap_rate / official_rate)
  puts format('Rust (Memory):        %10.2f lookups/sec (%.2fx)',
              rust_memory_rate, rust_memory_rate / official_rate)
  puts

  if rust_mmap_rate > official_rate
    improvement = (((rust_mmap_rate / official_rate) - 1) * 100).round(1)
    puts "🚀 Rust (MMAP) is #{improvement}% faster than official gem"
  end

  if rust_memory_rate > rust_mmap_rate
    improvement = (((rust_memory_rate / rust_mmap_rate) - 1) * 100).round(1)
    puts "💾 Rust Memory mode is #{improvement}% faster than MMAP mode"
  end
else
  rust_mmap_rate = ITERATIONS / results[:rust_mmap].real
  rust_memory_rate = ITERATIONS / results[:rust_memory].real

  puts format('Rust (MMAP):          %10.2f lookups/sec', rust_mmap_rate)
  puts format('Rust (Memory):        %10.2f lookups/sec (%.2fx)',
              rust_memory_rate, rust_memory_rate / rust_mmap_rate)
  puts
  puts 'Note: Install maxmind-db gem to compare with official implementation'
end
