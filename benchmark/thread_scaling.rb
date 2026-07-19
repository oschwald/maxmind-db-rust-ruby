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
    ip: '81.2.69.142',
    iterations: 100_000,
    samples: 5,
    thread_counts: [1, 2, 4, 8],
    json_output: nil,
  }

  OptionParser.new do |opts|
    opts.banner = 'Usage: ruby benchmark/thread_scaling.rb [options]'
    opts.on('--database PATH', 'Database path') { |value| options[:db_path] = File.expand_path(value) }
    opts.on('--ip IP', 'IP address used for lookups') { |value| options[:ip] = value }
    opts.on('--iterations N', Integer, 'Total lookups per sample') { |value| options[:iterations] = value }
    opts.on('--samples N', Integer, 'Measured samples per thread count') { |value| options[:samples] = value }
    opts.on('--threads LIST', 'Comma-separated thread counts') do |value|
      options[:thread_counts] = value.split(',').map(&:to_i)
    end
    opts.on('--json-output PATH', 'Write raw measurements as JSON') { |value| options[:json_output] = value }
  end.parse!(argv)

  options
end

def median(values)
  sorted = values.sort
  midpoint = sorted.length / 2
  return sorted.fetch(midpoint) if sorted.length.odd?

  (sorted.fetch(midpoint - 1) + sorted.fetch(midpoint)) / 2.0
end

def root_array_count
  ObjectSpace.each_object(Array).count { |array| array.length == 4096 }
end

def run_threads(reader, ip, iterations, thread_count)
  per_thread, remainder = iterations.divmod(thread_count)
  start_queue = Queue.new
  threads = Array.new(thread_count) do |index|
    count = per_thread + (index < remainder ? 1 : 0)
    Thread.new do # rubocop:disable ThreadSafety/NewThread -- thread creation is the behavior under test
      start_queue.pop
      count.times { reader.get(ip) }
    end
  end

  elapsed = Benchmark.realtime do
    thread_count.times { start_queue << true }
    threads.each(&:join)
  end
  iterations / elapsed
end

options = parse_options(ARGV)

abort "Database file not found: #{options[:db_path]}" unless File.exist?(options[:db_path])
abort 'Iterations must be positive' unless options[:iterations].positive?
abort 'Samples must be positive' unless options[:samples].positive?
abort 'Thread counts must be positive' unless options[:thread_counts].all?(&:positive?)

reader = MaxMind::DB::Rust::Reader.new(options[:db_path], mode: MaxMind::DB::Rust::MODE_MMAP)
reader.get(options[:ip])
GC.start
root_arrays_before = root_array_count

results = options[:thread_counts].to_h do |thread_count|
  rates = Array.new(options[:samples]) do
    run_threads(reader, options[:ip], options[:iterations], thread_count)
  end
  [
    thread_count,
    {
      samples: rates,
      median_operations_per_second: median(rates),
      min_operations_per_second: rates.min,
      max_operations_per_second: rates.max,
    },
  ]
end

GC.start
root_arrays_after = root_array_count
reader.close

puts
puts 'Shared Reader Thread Scaling'
puts '=' * 78
puts 'Threads        Median ops/s       Min ops/s       Max ops/s'
puts '-' * 78
results.each do |thread_count, result|
  puts format('%-10d %16.2f %15.2f %15.2f', thread_count,
              result.fetch(:median_operations_per_second),
              result.fetch(:min_operations_per_second),
              result.fetch(:max_operations_per_second))
end
puts
puts "4,096-slot root arrays before/after: #{root_arrays_before}/#{root_arrays_after}"

if options[:json_output]
  payload = {
    options: options,
    root_arrays_before: root_arrays_before,
    root_arrays_after: root_arrays_after,
    results: results,
  }
  File.write(options[:json_output], "#{JSON.pretty_generate(payload)}\n")
end
