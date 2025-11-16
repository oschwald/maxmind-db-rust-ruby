# frozen_string_literal: true

require 'bundler/gem_tasks'
require 'rake/testtask'

# Set up cross-compilation tasks for rb-sys-dock (used by CI)
# Check for ARGV containing native: tasks or rb-sys-dock environment
in_cross_compile_mode = ARGV.any? { |arg| arg.start_with?('native:') } || ENV.fetch('RB_SYS_DOCK_UID', nil)

if in_cross_compile_mode
  begin
    require 'rb_sys/extensiontask'

    GEMSPEC = Gem::Specification.load('maxmind-db-rust.gemspec')

    RbSys::ExtensionTask.new('maxmind_db_rust', GEMSPEC) do |ext|
      ext.lib_dir = 'lib/maxmind/db'
      ext.cross_compile = true
      ext.cross_platform = %w[
        x86_64-linux
        aarch64-linux
        x86_64-darwin
        arm64-darwin
        x64-mingw-ucrt
        x86_64-linux-musl
      ]
    end
  rescue LoadError
    # rb_sys not available - cross-compilation tasks won't be available
  end
end

# Local development compile task (only if not in cross-compile mode)
unless in_cross_compile_mode
  desc 'Compile the Rust extension'
  task :compile do
    sh 'bash build.sh'
  end
end

desc 'Clean build artifacts'
task :clean do
  # With workspace Cargo.toml, clean from the workspace root
  sh 'cargo clean' if Dir.exist?('target')
  rm_f 'lib/maxmind/db/maxmind_db_rust.so'
  rm_f 'lib/maxmind/db/maxmind_db_rust.bundle'
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
