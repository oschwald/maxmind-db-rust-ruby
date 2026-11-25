# frozen_string_literal: true

require 'test_helper'

# These tests run in a child process because a regression deadlocks the
# whole interpreter, which would otherwise hang the test suite.
class PathCacheLockTest < Minitest::Test
  include ChildProcessHelper

  TEST_DATA_DIR = File.join(__dir__, 'data', 'MaxMind-DB', 'test-data')

  def test_get_path_element_reentering_reader_with_warm_cache
    script = <<~RUBY
      reader = MaxMind::DB::Rust::Reader.new(DB_PATH)
      raise 'warm-up failed' unless reader.get_path('1.1.1.1', ['array', 0]) == 1

      element = Object.new
      element.define_singleton_method(:to_int) do
        reader.get_path('1.1.1.1', ['array', 1])
        0
      end

      result = reader.get_path('1.1.1.1', ['array', element])
      raise "unexpected result: \#{result.inspect}" unless result == 1

      puts 'ok'
    RUBY

    assert_child_completes(script)
  end

  def test_get_many_path_element_reentering_reader_with_warm_cache
    script = <<~RUBY
      reader = MaxMind::DB::Rust::Reader.new(DB_PATH)
      raise 'warm-up failed' unless reader.get_many_path(['1.1.1.1'], ['array', 0]) == [1]

      element = Object.new
      element.define_singleton_method(:to_int) do
        reader.get_many_path(['1.1.1.1'], ['array', 1])
        0
      end

      result = reader.get_many_path(['1.1.1.1'], ['array', element])
      raise "unexpected result: \#{result.inspect}" unless result == [1]

      puts 'ok'
    RUBY

    assert_child_completes(script)
  end

  def test_get_path_element_releasing_gvl_with_warm_cache
    script = <<~RUBY
      reader = MaxMind::DB::Rust::Reader.new(DB_PATH)
      raise 'warm-up failed' unless reader.get_path('1.1.1.1', ['array', 0]) == 1

      queue = Queue.new
      element = Object.new
      element.define_singleton_method(:to_int) do
        queue << :converting
        sleep 0.3
        0
      end

      slow = Thread.new { reader.get_path('1.1.1.1', ['array', element]) }
      fast = Thread.new do
        queue.pop
        # Keep looking up a plain path for as long as the slow lookup is
        # inside a to_int call that has released the GVL.
        while slow.alive?
          raise 'unexpected fast result' unless reader.get_path('1.1.1.1', ['array', 0]) == 1

          sleep 0.01
        end
      end

      raise 'unexpected slow result' unless slow.value == 1

      fast.join

      puts 'ok'
    RUBY

    assert_child_completes(script)
  end

  private

  def assert_child_completes(body)
    skip 'Test database not found' unless File.exist?(decoder_db_path)

    lib_dir = File.expand_path('../lib', __dir__)
    script = <<~RUBY
      $LOAD_PATH.unshift(#{lib_dir.dump})
      require 'maxmind/db/rust'
      DB_PATH = #{decoder_db_path.dump}
      #{body}
    RUBY

    status, out, err = run_child(script)

    assert_predicate status, :success?, "stdout:\n#{out}\nstderr:\n#{err}"
    # stderr is not checked because some Ruby and Bundler versions print
    # unrelated warnings when the child starts.
    assert_equal "ok\n", out, err
  end

  def decoder_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-test-decoder.mmdb')
  end
end
