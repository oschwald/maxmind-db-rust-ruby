#!/usr/bin/env ruby
# frozen_string_literal: true

require 'benchmark'
require 'json'
require 'optparse'

$LOAD_PATH.unshift(File.expand_path('../lib', __dir__))
require 'maxmind/db/rust'

DEFAULT_DB_PATH = File.expand_path('../test/data/MaxMind-DB/test-data/GeoIP2-City-Test.mmdb', __dir__)

def parse_options(argv)
  options = {
    db_path: DEFAULT_DB_PATH,
    network: nil,
    max_records: nil,
    warmup_runs: 1,
    samples: 5,
    json_output: nil,
  }

  OptionParser.new do |opts|
    opts.banner = 'Usage: ruby benchmark/iteration.rb [options]'
    opts.on('--database PATH', 'Database path') { |value| options[:db_path] = File.expand_path(value) }
    opts.on('--network CIDR', 'Only iterate within this network') { |value| options[:network] = value }
    opts.on('--max-records N', Integer, 'Stop each sample after N yielded records') do |value|
      options[:max_records] = value
    end
    opts.on('--warmup-runs N', Integer, 'Full iteration runs before measuring') { |value| options[:warmup_runs] = value }
    opts.on('--samples N', Integer, 'Measured iteration runs') { |value| options[:samples] = value }
    opts.on('--json-output PATH', 'Write raw measurements as JSON') { |value| options[:json_output] = value }
  end.parse!(argv)

  options
end

def iterate(reader, network, max_records)
  count = 0
  catch(:iteration_limit) do
    records = network ? reader.each(network) : reader.each
    records.each do |_record_network, _data|
      count += 1
      throw :iteration_limit if max_records && count >= max_records
    end
  end
  count
end

def median(values)
  sorted = values.sort
  midpoint = sorted.length / 2
  return sorted.fetch(midpoint) if sorted.length.odd?

  (sorted.fetch(midpoint - 1) + sorted.fetch(midpoint)) / 2.0
end

options = parse_options(ARGV)

abort "Database file not found: #{options[:db_path]}" unless File.exist?(options[:db_path])
abort 'Samples must be positive' unless options[:samples].positive?
abort 'Warmup runs cannot be negative' if options[:warmup_runs].negative?
abort 'Max records must be positive' if options[:max_records] && !options[:max_records].positive?

reader = MaxMind::DB::Rust::Reader.new(options[:db_path], mode: MaxMind::DB::Rust::MODE_MMAP)
options[:warmup_runs].times { iterate(reader, options[:network], options[:max_records]) }

measurements = Array.new(options[:samples]) do
  count = 0
  elapsed = Benchmark.realtime do
    count = iterate(reader, options[:network], options[:max_records])
  end
  {
    records: count,
    elapsed_seconds: elapsed,
    records_per_second: count / elapsed,
  }
end
reader.close

rates = measurements.map { |measurement| measurement.fetch(:records_per_second) }
results = {
  options: options,
  measurements: measurements,
  median_records_per_second: median(rates),
  min_records_per_second: rates.min,
  max_records_per_second: rates.max,
}

puts
puts 'Iteration Throughput'
puts '=' * 72
puts format('Median: %12.2f records/s', results.fetch(:median_records_per_second))
puts format('Range:  %12.2f - %.2f records/s', results.fetch(:min_records_per_second),
            results.fetch(:max_records_per_second))

File.write(options[:json_output], "#{JSON.pretty_generate(results)}\n") if options[:json_output]
