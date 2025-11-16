# frozen_string_literal: true

require 'bundler/gem_tasks'
require 'rake/testtask'

# Custom compile task for Rust extension
desc 'Compile the Rust extension'
task :compile do
  sh 'bash build.sh'
end

desc 'Clean build artifacts'
task :clean do
  sh 'cargo clean --manifest-path ext/maxmind_db_rust/Cargo.toml' if Dir.exist?('ext/maxmind_db_rust/target')
  rm_f 'lib/maxmind/db/maxmind_db_rust.so'
  rm_rf 'pkg'
  rm_rf 'tmp'
end

# Our own tests
Rake::TestTask.new(:test_own) do |t|
  t.libs << 'test'
  t.libs << 'lib'
  t.test_files = FileList['test/*_test.rb']
  t.description = 'Run our own tests'
end

# MaxMind upstream tests (adapted from official gem)
Rake::TestTask.new(:test_maxmind) do |t|
  t.libs << 'test'
  t.libs << 'lib'
  t.test_files = FileList['test/maxmind/test_*.rb']
  t.description = 'Run MaxMind upstream compatibility tests'
end

# Run all tests
Rake::TestTask.new(:test) do |t|
  t.libs << 'test'
  t.libs << 'lib'
  t.test_files = FileList['test/*_test.rb', 'test/maxmind/test_*.rb']
  t.description = 'Run all tests (own + MaxMind upstream)'
end

desc 'Run all tests with verbose output'
task :test_verbose do
  ENV['TESTOPTS'] = '-v'
  Rake::Task[:test].invoke
end

task default: %i[compile test]
