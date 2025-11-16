# frozen_string_literal: true

require 'bundler/gem_tasks'
require 'rake/testtask'
require 'rb_sys/extensiontask'

# Configure the Rust extension task
RbSys::ExtensionTask.new('maxmind_db_rust') do |ext|
  ext.lib_dir = 'lib/maxmind/db'
end

# Custom compile task for Rust extension (for local development)
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

# Cross-compilation tasks
desc 'Build native gems for all platforms'
task 'gem:native' do
  require 'rake_compiler_dock'

  # Platforms to build for
  platforms = %w[
    x86_64-linux
    aarch64-linux
    x86_64-darwin
    arm64-darwin
    x64-mingw-ucrt
    x86_64-linux-musl
  ]

  platforms.each do |platform|
    RakeCompilerDock.sh "bundle && rake native:#{platform} gem", platform: platform
  end
end

namespace :gem do
  desc 'Build native gem for current platform'
  task :current do
    sh 'rake native gem'
  end
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
