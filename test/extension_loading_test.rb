# frozen_string_literal: true

require 'fileutils'
require 'open3'
require 'rbconfig'
require 'test_helper'
require 'tmpdir'

# Precompiled gems store the extension under a Ruby version directory. Source
# builds store it directly in lib/maxmind/db. These tests load each layout in a
# new process because this process has already loaded the extension.
class ExtensionLoadingTest < Minitest::Test
  LIB_DIR = File.expand_path('../lib/maxmind/db', __dir__)
  EXTENSION = "maxmind_db_rust.#{RbConfig::CONFIG['DLEXT']}".freeze
  RUBY_MINOR = RUBY_VERSION[/\d+\.\d+/]

  def test_loads_from_versioned_path
    with_lib do |lib|
      install_extension(lib, RUBY_MINOR)

      assert_loads File.join(lib, 'maxmind/db', RUBY_MINOR, EXTENSION), lib
    end
  end

  def test_loads_from_unversioned_path
    with_lib do |lib|
      install_extension(lib)

      assert_loads File.join(lib, 'maxmind/db', EXTENSION), lib
    end
  end

  def test_prefers_versioned_path
    with_lib do |lib|
      install_extension(lib)
      install_extension(lib, RUBY_MINOR)

      assert_loads File.join(lib, 'maxmind/db', RUBY_MINOR, EXTENSION), lib
    end
  end

  def test_reports_load_failure_of_versioned_extension
    with_lib do |lib|
      install_extension(lib)
      broken = File.join(lib, 'maxmind/db', RUBY_MINOR, EXTENSION)
      FileUtils.mkdir_p(File.dirname(broken))
      File.write(broken, 'not a shared library')

      _, stderr, status = run_require(lib)

      refute_predicate status, :success?
      assert_includes stderr, broken
    end
  end

  def test_ignores_versioned_extension_elsewhere_on_load_path
    with_lib do |lib|
      install_extension(lib)
      with_lib do |other|
        install_extension(other, RUBY_MINOR)

        assert_loads File.join(lib, 'maxmind/db', EXTENSION), lib, other
      end
    end
  end

  private

  def with_lib
    Dir.mktmpdir do |dir|
      lib = File.realpath(dir)
      FileUtils.mkdir_p(File.join(lib, 'maxmind/db'))
      FileUtils.cp(File.join(LIB_DIR, 'rust.rb'), File.join(lib, 'maxmind/db'))
      yield lib
    end
  end

  def install_extension(lib, version = '')
    target = File.join(lib, 'maxmind/db', version)
    FileUtils.mkdir_p(target)
    FileUtils.cp(built_extension, target)
  end

  # The built extension, from either layout.
  def built_extension
    [File.join(LIB_DIR, RUBY_MINOR, EXTENSION), File.join(LIB_DIR, EXTENSION)].find do |path|
      File.exist?(path)
    end || skip('The extension has not been built')
  end

  def assert_loads(expected, *load_path)
    stdout, stderr, status = run_require(*load_path)

    assert_predicate status, :success?, stderr
    assert_equal expected, stdout.chomp, stderr
  end

  def run_require(*load_path)
    script = <<~RUBY
      require 'maxmind/db/rust'
      MaxMind::DB::Rust::Reader
      puts $LOADED_FEATURES.grep(/maxmind_db_rust/)
    RUBY
    # Clear these variables so that Bundler does not add this repo's lib.
    env = { 'BUNDLER_SETUP' => nil, 'RUBYOPT' => nil, 'RUBYLIB' => nil }
    includes = load_path.flat_map { |path| ['-I', path] }
    Open3.capture3(env, RbConfig.ruby, *includes, '-e', script)
  end
end
