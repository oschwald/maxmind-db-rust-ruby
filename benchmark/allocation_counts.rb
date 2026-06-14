#!/usr/bin/env ruby
# frozen_string_literal: true

require 'benchmark'
require 'json'
require 'optparse'

$LOAD_PATH.unshift(File.expand_path('../lib', __dir__))
require 'maxmind/db/rust'

DEFAULT_DB_PATH = File.expand_path('../test/data/MaxMind-DB/test-data/GeoIP2-City-Test.mmdb', __dir__)
DEFAULT_CASES = %w[get get_path get_many].freeze

def parse_options(argv)
  options = {
    db_path: DEFAULT_DB_PATH,
    iterations: 1_000,
    warmup_iterations: 100,
    samples: 5,
    batch_size: 100,
    cases: DEFAULT_CASES,
    ip: '81.2.69.142',
    path: %w[country iso_code],
    json_output: nil,
  }

  parser = OptionParser.new do |opts|
    opts.banner = 'Usage: ruby benchmark/allocation_counts.rb [options]'
    opts.on('--database PATH', 'Database path to benchmark') { |value| options[:db_path] = File.expand_path(value) }
    opts.on('--iterations N', Integer, 'Lookup operations per sample') { |value| options[:iterations] = value }
    opts.on('--warmup-iterations N', Integer, 'Warmup operations before measuring') { |value| options[:warmup_iterations] = value }
    opts.on('--samples N', Integer, 'Measured samples per case') { |value| options[:samples] = value }
    opts.on('--batch-size N', Integer, 'Batch size for get_many') { |value| options[:batch_size] = value }
    opts.on('--cases LIST', 'Comma-separated cases: get,get_path,get_many') do |value|
      options[:cases] = value.split(',').map(&:strip).reject(&:empty?)
    end
    opts.on('--ip IP', 'IP address used for lookups') { |value| options[:ip] = value }
    opts.on('--path PATH', 'Dot-separated lookup path for get_path') { |value| options[:path] = value.split('.') }
    opts.on('--json-output PATH', 'Write raw measurements as JSON') { |value| options[:json_output] = value }
  end

  parser.parse!(argv)
  options
end

def run_case(reader, case_name, ips, path, batch_size)
  case case_name
  when 'get'
    ips.each { |ip| reader.get(ip) }
  when 'get_path'
    ips.each { |ip| reader.get_path(ip, path) }
  when 'get_many'
    ips.each_slice(batch_size) { |batch| reader.get_many(batch) }
  else
    raise ArgumentError, "unknown case: #{case_name}"
  end
end

def median(values)
  sorted = values.sort
  midpoint = sorted.length / 2
  return sorted.fetch(midpoint) if sorted.length.odd?

  (sorted.fetch(midpoint - 1) + sorted.fetch(midpoint)) / 2.0
end

def measure_allocations(&block)
  GC.start(full_mark: true, immediate_sweep: true)
  GC.disable
  before = GC.stat
  elapsed = Benchmark.realtime { block.yield }
  after = GC.stat

  {
    allocated_objects: after.fetch(:total_allocated_objects) - before.fetch(:total_allocated_objects),
    elapsed_seconds: elapsed,
  }
ensure
  GC.enable
end

def measure_case(reader, case_name, config)
  ips = config.fetch(:ips)
  warmup_ips = config.fetch(:warmup_ips)
  path = config.fetch(:path)
  batch_size = config.fetch(:batch_size)
  samples = config.fetch(:samples)

  run_case(reader, case_name, warmup_ips, path, batch_size) unless warmup_ips.empty?

  measurements = Array.new(samples) do
    measured = measure_allocations do
      run_case(reader, case_name, ips, path, batch_size)
    end

    allocated_objects = measured.fetch(:allocated_objects)
    {
      operations: ips.length,
      allocated_objects: allocated_objects,
      allocated_objects_per_operation: allocated_objects.to_f / ips.length,
      elapsed_seconds: measured.fetch(:elapsed_seconds),
    }
  end

  rates = measurements.map { |sample| sample.fetch(:allocated_objects_per_operation) }
  {
    operations: ips.length,
    sample_count: samples,
    samples: measurements,
    median_allocated_objects_per_operation: median(rates),
    min_allocated_objects_per_operation: rates.min,
    max_allocated_objects_per_operation: rates.max,
  }
end

def print_results(results)
  puts
  puts 'Allocation Counts'
  puts '=' * 86
  puts 'Case              Median objects/op   Min objects/op   Max objects/op'
  puts '-' * 86

  results.each do |case_name, result|
    puts format(
      '%-16s %17.4f %16.4f %16.4f',
      case_name,
      result.fetch(:median_allocated_objects_per_operation),
      result.fetch(:min_allocated_objects_per_operation),
      result.fetch(:max_allocated_objects_per_operation),
    )
  end
end

options = parse_options(ARGV)

abort "Database file not found: #{options[:db_path]}" unless File.exist?(options[:db_path])
abort 'Iterations must be positive' unless options[:iterations].positive?
abort 'Warmup iterations cannot be negative' if options[:warmup_iterations].negative?
abort 'Samples must be positive' unless options[:samples].positive?
abort 'Batch size must be positive' unless options[:batch_size].positive?
abort 'At least one benchmark case is required' if options[:cases].empty?

reader = MaxMind::DB::Rust::Reader.new(options[:db_path], mode: MaxMind::DB::Rust::MODE_MMAP)
ips = Array.new(options[:iterations], options[:ip])
warmup_ips = ips.first([options[:warmup_iterations], ips.length].min)
case_config = {
  ips: ips,
  warmup_ips: warmup_ips,
  path: options[:path],
  batch_size: options[:batch_size],
  samples: options[:samples],
}

results = options[:cases].to_h do |case_name|
  [
    case_name,
    measure_case(reader, case_name, case_config),
  ]
end

reader.close
print_results(results)

if options[:json_output]
  File.write(
    options[:json_output],
    "#{JSON.pretty_generate({ options: options, results: results })}\n",
  )
end
