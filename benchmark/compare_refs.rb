#!/usr/bin/env ruby
# frozen_string_literal: true

require 'benchmark'
require 'fileutils'
require 'json'
require 'open3'
require 'optparse'
require 'tmpdir'

DEFAULT_DB_PATH = File.expand_path('../test/data/MaxMind-DB/test-data/GeoIP2-City-Test.mmdb', __dir__)
DEFAULT_CASES = %w[get get_path get_many get_many_path].freeze

BENCHMARK_RUNNER = <<~'RUBY'
  # frozen_string_literal: true

  require 'benchmark'
  require 'json'

  $LOAD_PATH.unshift(File.expand_path('lib', Dir.pwd))
  require 'maxmind/db/rust'

  db_path = ARGV.fetch(0)
  iterations = ARGV.fetch(1).to_i
  batch_size = ARGV.fetch(2).to_i
  cases = ARGV.fetch(3).split(',')
  warmup_iterations = ARGV.fetch(4).to_i
  samples = ARGV.fetch(5).to_i
  rng = Random.new(12_345)

  def random_ipv4(rng)
    "#{rng.rand(1..255)}.#{rng.rand(0..255)}.#{rng.rand(0..255)}.#{rng.rand(0..255)}"
  end

  def reader_mode
    if MaxMind::DB::Rust.const_defined?(:MODE_MMAP)
      MaxMind::DB::Rust::MODE_MMAP
    else
      MaxMind::DB::Rust::MODE_AUTO
    end
  end

  def case_supported?(reader, case_name)
    case case_name
    when 'get'
      reader.respond_to?(:get)
    when 'get_path'
      reader.respond_to?(:get_path)
    when 'get_many'
      reader.respond_to?(:get_many)
    when 'get_many_path'
      reader.respond_to?(:get_many_path)
    else
      false
    end
  end

  def run_case(reader, case_name, ips, batch_size)
    path = %w[country iso_code]
    case case_name
    when 'get'
      ips.each { |ip| reader.get(ip) }
    when 'get_path'
      ips.each { |ip| reader.get_path(ip, path) }
    when 'get_many'
      ips.each_slice(batch_size) { |batch| reader.get_many(batch) }
    when 'get_many_path'
      ips.each_slice(batch_size) { |batch| reader.get_many_path(batch, path) }
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

  def measure_case(reader, case_name, ips, warmup_ips, batch_size, samples)
    run_case(reader, case_name, warmup_ips, batch_size) unless warmup_ips.empty?

    measurements = Array.new(samples) do
      elapsed = Benchmark.realtime do
        run_case(reader, case_name, ips, batch_size)
      end

      {
        operations: ips.length,
        real_seconds: elapsed,
        operations_per_second: ips.length / elapsed,
      }
    end

    rates = measurements.map { |sample| sample.fetch(:operations_per_second) }
    {
      supported: true,
      operations: ips.length,
      sample_count: samples,
      samples: measurements,
      median_operations_per_second: median(rates),
      min_operations_per_second: rates.min,
      max_operations_per_second: rates.max,
    }
  end

  reader = MaxMind::DB::Rust::Reader.new(db_path, mode: reader_mode)
  ips = Array.new(iterations) { random_ipv4(rng) }
  warmup_ips = ips.first([warmup_iterations, ips.length].min)
  results = {}

  cases.each do |case_name|
    results[case_name] = if case_supported?(reader, case_name)
                           measure_case(reader, case_name, ips, warmup_ips, batch_size, samples)
                         else
                           { supported: false }
                         end
  end

  reader.close
  puts JSON.generate(results)
RUBY

class CommandFailure < StandardError
  attr_reader :stdout, :stderr

  def initialize(command, stdout, stderr)
    @stdout = stdout
    @stderr = stderr
    super("command failed: #{command.join(' ')}\n#{stderr}")
  end
end

def parse_options(argv)
  options = {
    baseline_ref: 'HEAD~1',
    candidate_ref: 'HEAD',
    db_path: DEFAULT_DB_PATH,
    iterations: 10_000,
    warmup_iterations: 1_000,
    samples: 5,
    batch_size: 100,
    cases: DEFAULT_CASES,
    json_output: nil,
    keep_worktrees: false,
    max_regression_pct: nil,
    skip_build: false,
  }

  parser = OptionParser.new do |opts|
    opts.banner = 'Usage: ruby benchmark/compare_refs.rb [options]'
    add_ref_options(opts, options)
    add_benchmark_options(opts, options)
    add_output_options(opts, options)
  end

  parser.parse!(argv)
  options
end

def add_ref_options(parser, options)
  parser.on('--baseline-ref REF', 'Git ref for the baseline build (default: HEAD~1)') do |value|
    options[:baseline_ref] = value
  end
  parser.on('--candidate-ref REF', 'Git ref for the candidate build (default: HEAD)') do |value|
    options[:candidate_ref] = value
  end
end

def add_benchmark_options(parser, options)
  parser.on('--database PATH', 'Database path to benchmark') do |value|
    options[:db_path] = File.expand_path(value)
  end
  parser.on('--iterations N', Integer, 'Number of lookup operations per case') do |value|
    options[:iterations] = value
  end
  parser.on('--warmup-iterations N', Integer, 'Warmup operations per case before sampling') do |value|
    options[:warmup_iterations] = value
  end
  parser.on('--samples N', Integer, 'Measured samples per case') do |value|
    options[:samples] = value
  end
  parser.on('--batch-size N', Integer, 'Batch size for get_many cases') do |value|
    options[:batch_size] = value
  end
  parser.on('--cases LIST', 'Comma-separated cases: get,get_path,get_many,get_many_path') do |value|
    options[:cases] = value.split(',').map(&:strip).reject(&:empty?)
  end
end

def add_output_options(parser, options)
  parser.on('--json-output PATH', 'Write raw benchmark results as JSON') do |value|
    options[:json_output] = value
  end
  parser.on('--keep-worktrees', 'Keep temporary git worktrees for inspection') do
    options[:keep_worktrees] = true
  end
  parser.on('--max-regression-pct N', Float, 'Exit non-zero if any supported case regresses by more than N percent') do |value|
    options[:max_regression_pct] = value
  end
  parser.on('--skip-build', 'Skip bundle/rake compile in each worktree') do
    options[:skip_build] = true
  end
end

def run_command(*command, chdir:)
  stdout, stderr, status = Open3.capture3(*command, chdir: chdir)
  raise CommandFailure.new(command, stdout, stderr) unless status.success?

  stdout
end

def prepare_worktree(repo_root, tmpdir, name, ref)
  path = File.join(tmpdir, name)
  run_command('git', 'worktree', 'add', '--detach', path, ref, chdir: repo_root)
  path
end

def remove_worktree(repo_root, path)
  return unless path && File.exist?(path)

  run_command('git', 'worktree', 'remove', '--force', path, chdir: repo_root)
rescue CommandFailure => e
  warn e.message
end

def build_worktree(path, skip_build:)
  return if skip_build

  begin
    run_command('bundle', 'check', chdir: path)
  rescue CommandFailure
    run_command('bundle', 'install', chdir: path)
  end

  run_command('bundle', 'exec', 'rake', 'compile', chdir: path)
end

def benchmark_worktree(path, options)
  stdout = run_command(
    'bundle',
    'exec',
    'ruby',
    '-e',
    BENCHMARK_RUNNER,
    '--',
    options[:db_path],
    options[:iterations].to_s,
    options[:batch_size].to_s,
    options[:cases].join(','),
    options[:warmup_iterations].to_s,
    options[:samples].to_s,
    chdir: path,
  )
  JSON.parse(stdout)
end

def compare_results(baseline, candidate, cases, max_regression_pct)
  regressions = []

  puts
  puts 'Benchmark Ref Comparison'
  puts '=' * 96
  puts 'Case              Base median/s  Cand median/s       Delta      Base min/s    Cand min/s'
  puts '-' * 96

  cases.each do |case_name|
    baseline_case = baseline.fetch(case_name, { 'supported' => false })
    candidate_case = candidate.fetch(case_name, { 'supported' => false })

    unless baseline_case['supported'] && candidate_case['supported']
      puts format('%-16s %14s %14s %11s %15s %13s', case_name, 'unsupported', 'unsupported', '-', '-', '-')
      next
    end

    baseline_rate = baseline_case.fetch('median_operations_per_second')
    candidate_rate = candidate_case.fetch('median_operations_per_second')
    delta_pct = ((candidate_rate / baseline_rate) - 1.0) * 100.0
    regressions << [case_name, delta_pct] if max_regression_pct && delta_pct < -max_regression_pct

    puts format(
      '%-16s %14.2f %14.2f %+10.2f%% %15.2f %13.2f',
      case_name,
      baseline_rate,
      candidate_rate,
      delta_pct,
      baseline_case.fetch('min_operations_per_second'),
      candidate_case.fetch('min_operations_per_second'),
    )
  end

  regressions
end

def write_json(path, payload)
  return unless path

  File.write(path, "#{JSON.pretty_generate(payload)}\n")
end

options = parse_options(ARGV)
repo_root = File.expand_path('..', __dir__)

abort "Database file not found: #{options[:db_path]}" unless File.exist?(options[:db_path])
abort 'Iterations must be positive' unless options[:iterations].positive?
abort 'Warmup iterations cannot be negative' if options[:warmup_iterations].negative?
abort 'Samples must be positive' unless options[:samples].positive?
abort 'Batch size must be positive' unless options[:batch_size].positive?
abort 'At least one benchmark case is required' if options[:cases].empty?

regressions = []
tmpdir = Dir.mktmpdir('maxminddb-rust-bench-')
baseline_path = nil
candidate_path = nil

begin
  baseline_path = prepare_worktree(repo_root, tmpdir, 'baseline', options[:baseline_ref])
  candidate_path = prepare_worktree(repo_root, tmpdir, 'candidate', options[:candidate_ref])

  warn "Building baseline #{options[:baseline_ref]}..."
  build_worktree(baseline_path, skip_build: options[:skip_build])
  warn "Building candidate #{options[:candidate_ref]}..."
  build_worktree(candidate_path, skip_build: options[:skip_build])

  warn "Benchmarking baseline #{options[:baseline_ref]}..."
  baseline = benchmark_worktree(baseline_path, options)
  warn "Benchmarking candidate #{options[:candidate_ref]}..."
  candidate = benchmark_worktree(candidate_path, options)

  payload = {
    baseline_ref: options[:baseline_ref],
    candidate_ref: options[:candidate_ref],
    database: options[:db_path],
    iterations: options[:iterations],
    warmup_iterations: options[:warmup_iterations],
    samples: options[:samples],
    batch_size: options[:batch_size],
    cases: options[:cases],
    baseline: baseline,
    candidate: candidate,
  }

  regressions = compare_results(baseline, candidate, options[:cases], options[:max_regression_pct])
  write_json(options[:json_output], payload)
ensure
  if options[:keep_worktrees]
    warn "Keeping worktrees under #{tmpdir}"
  else
    remove_worktree(repo_root, baseline_path)
    remove_worktree(repo_root, candidate_path)
    FileUtils.rm_rf(tmpdir)
  end
end

if regressions.any?
  warn
  warn 'Regressions exceeded threshold:'
  regressions.each do |case_name, delta_pct|
    warn format('  %s: %.2f%%', case_name, delta_pct)
  end
  exit 1
end
