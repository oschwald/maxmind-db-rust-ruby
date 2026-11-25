# frozen_string_literal: true

require 'open3'
require 'rbconfig'
require 'tmpdir'
require 'test_helper'

# Loading the extension must not be affected by constants outside its own
# namespace, or hide errors. Each test needs a fresh process.
class LoadTest < Minitest::Test
  REQUIRE = "require 'maxmind/db/rust'\nraise 'no Reader' unless MaxMind::DB::Rust.const_defined?(:Reader, false)\n"

  def test_top_level_constants_with_the_same_names_are_ignored
    assert_loads("DB = Object.new\n#{REQUIRE}raise 'wrong DB' if MaxMind::DB.equal?(DB)")
    assert_loads("class Reader; end\n#{REQUIRE}")
    assert_loads("module Rust; end\n#{REQUIRE}")
    assert_loads("module MaxMind; class DB; end; end\nmodule Rust; end\n#{REQUIRE}")
  end

  def test_const_missing_hooks_are_not_called
    assert_loads(<<~RUBY)
      module MaxMind
        def self.const_missing(name) = raise("const_missing \#{name}")
      end
      #{REQUIRE}
    RUBY
  end

  def test_errors_from_autoloads_propagate
    Dir.mktmpdir do |dir|
      file = File.join(dir, 'broken.rb')
      File.write(file, "raise IOError, 'autoload failed'")

      assert_loads(<<~RUBY)
        module MaxMind
          autoload :DB, #{file.dump}
        end
        begin
          require 'maxmind/db/maxmind_db_rust'
          raise 'no error'
        rescue IOError => e
          raise unless e.message == 'autoload failed'
        end
      RUBY
    end
  end

  def test_autoload_that_defines_nothing_is_ignored
    Dir.mktmpdir do |dir|
      file = File.join(dir, 'empty.rb')
      File.write(file, '')

      assert_loads("module MaxMind; autoload :DB, #{file.dump}; end\n#{REQUIRE}")
    end
  end

  def test_const_missing_is_not_called_after_an_empty_autoload
    Dir.mktmpdir do |dir|
      file = File.join(dir, 'empty.rb')
      File.write(file, '')

      assert_loads(<<~RUBY)
        module MaxMind
          autoload :DB, #{file.dump}
          def self.const_missing(name) = raise("const_missing \#{name}")
        end
        #{REQUIRE}
      RUBY
    end
  end

  def test_autoload_of_this_gem_works
    assert_loads("module MaxMind; autoload :DB, 'maxmind/db/rust'; end\nMaxMind::DB::Rust::Reader")
  end

  def test_name_errors_from_autoloads_propagate
    Dir.mktmpdir do |dir|
      file = File.join(dir, 'broken.rb')
      File.write(file, 'UndefinedHelper')

      assert_loads(<<~RUBY)
        module MaxMind
          autoload :DB, #{file.dump}
        end
        begin
          require 'maxmind/db/maxmind_db_rust'
          raise 'no error'
        rescue NameError => e
          raise unless e.name == :UndefinedHelper
        end
      RUBY
    end
  end

  private

  def assert_loads(script)
    lib_dir = File.expand_path('../lib', __dir__)
    _stdout, stderr, status = Open3.capture3(RbConfig.ruby, "-I#{lib_dir}", '-e', script)

    assert_predicate status, :success?, "#{script}\n#{stderr}"
  end
end
