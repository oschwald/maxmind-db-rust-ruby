# frozen_string_literal: true

require 'bigdecimal'
require 'open3'
require 'rbconfig'
require 'test_helper'

# Arguments are converted with Ruby methods such as `to_str`. Whatever those
# methods raise or throw must reach the caller. Some of these tests run in a
# child process because a regression crashes the interpreter.
class ConversionErrorsTest < Minitest::Test
  TEST_DATA_DIR = File.join(__dir__, 'data', 'MaxMind-DB', 'test-data')

  def test_throw_from_conversion_methods_passes_through
    assert_child_succeeds(<<~RUBY)
      def thrower(method)
        Object.new.tap { |object| object.define_singleton_method(method) { throw :done, :thrown } }
      end

      calls = {
        'path to_ary' => -> { reader.get_path('1.1.1.1', thrower(:to_ary)) },
        'path element to_str' => -> { reader.get_path('1.1.1.1', [thrower(:to_str)]) },
        'path element to_int' => -> { reader.get_path('1.1.1.1', [thrower(:to_int)]) },
        'get_many to_ary' => -> { reader.get_many(thrower(:to_ary)) },
        'get_many_path to_ary' => -> { reader.get_many_path(thrower(:to_ary), ['ip']) },
        'each to_str' => -> { reader.each(thrower(:to_str)) { |*| nil } },
      }
      calls.each do |name, call|
        result = catch(:done) { call.call }
        raise "\#{name}: \#{result.inspect}" unless result == :thrown
      end
    RUBY
  end

  def test_throw_comparing_a_cached_path_passes_through
    assert_child_succeeds(<<~RUBY)
      # Hashing the path converts each element once. The next conversion is
      # the comparison with the cached path.
      { 'ip' => :to_str, 0 => :to_int }.each do |value, method|
        reader.get_path('1.1.1.1', [value])
        calls = 0
        element = Object.new
        element.define_singleton_method(method) do
          calls += 1
          calls > 1 ? throw(:done, :thrown) : value
        end
        result = catch(:done) { reader.get_path('1.1.1.1', [element]) }
        raise "\#{method}: \#{result.inspect}" unless result == :thrown
      end
    RUBY
  end

  def test_throw_from_the_class_name_of_a_wrong_type_passes_through
    assert_child_succeeds(<<~RUBY)
      class Unprintable
        def self.to_s
          $to_s_called = true
          throw(:done, :thrown)
        end

        def each = yield('bad ip')
      end

      calls = {
        'get_many' => -> { reader.get_many(Unprintable.new) },
        'get_many_path' => -> { reader.get_many_path(Unprintable.new, ['ip']) },
        'get_path' => -> { reader.get_path('1.1.1.1', Unprintable.new) },
        'options' => -> { MaxMind::DB::Rust::Reader.new('x', Unprintable.new) },
        'options.dup' => lambda do
          options = Class.new(Hash) { def dup = Unprintable.new }.new
          MaxMind::DB::Rust::Reader.new('x', options)
        end,
      }
      # Building an error message that names the class must not lose the throw.
      # Not calling `to_s` at all is fine too, so accept an ordinary error then.
      calls.each do |name, call|
        $to_s_called = false
        result = catch(:done) do
          call.call
        rescue ArgumentError, TypeError
          :wrong_type
        end
        expected = $to_s_called ? :thrown : :wrong_type
        raise "\#{name}: \#{result.inspect}, not \#{expected}" unless result == expected

        begin
          raise IOError, name
        rescue IOError
          nil
        end
      end
    RUBY
  end

  def test_thread_exit_in_a_conversion_method_ends_the_thread
    assert_child_succeeds(<<~RUBY)
      element = Object.new
      element.define_singleton_method(:to_str) { Thread.exit }
      thread = Thread.new do
        reader.get_path('1.1.1.1', [element])
        :not_reached
      end
      thread.report_on_exception = false
      raise 'thread kept running' unless thread.value.nil?
    RUBY
  end

  def test_exceptions_from_conversion_methods_propagate
    reader = MaxMind::DB::Rust::Reader.new(db_path)

    [IOError, Interrupt].each do |error|
      raiser = lambda do |method|
        Object.new.tap { |object| object.define_singleton_method(method) { raise error, 'conversion failed' } }
      end

      [
        -> { reader.get_path('1.1.1.1', raiser.call(:to_ary)) },
        -> { reader.get_path('1.1.1.1', [raiser.call(:to_str)]) },
        -> { reader.get_path('1.1.1.1', [raiser.call(:to_int)]) },
        -> { reader.get_many(raiser.call(:to_ary)) },
        -> { reader.each(raiser.call(:to_str)) },
      ].each do |call|
        assert_raises(error) { call.call }
      end
    end

    reader.close
  end

  def test_range_errors_from_to_int_propagate
    reader = MaxMind::DB::Rust::Reader.new(db_path)

    [RangeError, FloatDomainError].each do |error_class|
      element = Object.new
      element.define_singleton_method(:to_int) { raise error_class, 'conversion failed' }

      error = assert_raises(error_class) { reader.get_path('1.1.1.1', [element]) }
      assert_equal 'conversion failed', error.message
    end

    reader.close
  end

  def test_no_method_errors_inside_numeric_conversions_propagate
    reader = MaxMind::DB::Rust::Reader.new(db_path)
    elements = [
      Class.new(Numeric) { def to_i = undefined_helper }.new,
      Class.new(Numeric) { def to_int = undefined_helper }.new,
      Class.new(Numeric) { def to_int = Object.new.undefined_helper }.new,
    ]

    elements.each do |element|
      error = assert_raises(NameError) { reader.get_path('1.1.1.1', [element]) }
      assert_equal :undefined_helper, error.name
    end

    other_receiver = Class.new(Numeric) { def to_int = Object.new.to_i }.new
    assert_raises(NoMethodError) { reader.get_path('1.1.1.1', [other_receiver]) }
    no_receiver = Class.new(Numeric) { def to_int = raise(NoMethodError.new('mine', :to_i)) }.new
    error = assert_raises(NoMethodError) { reader.get_path('1.1.1.1', [no_receiver]) }
    assert_equal 'mine', error.message

    reader.close
  end

  def test_range_error_from_a_custom_numeric_propagates
    reader = MaxMind::DB::Rust::Reader.new(db_path)
    element = Class.new(Numeric) { def to_int = raise(RangeError, 'conversion failed') }.new

    error = assert_raises(RangeError) { reader.get_path('1.1.1.1', [element]) }
    assert_equal 'conversion failed', error.message

    reader.close
  end

  def test_bigdecimal_without_an_integer_value_is_an_argument_error
    reader = MaxMind::DB::Rust::Reader.new(db_path)

    %w[NaN Infinity].each do |value|
      assert_raises(ArgumentError) { reader.get_path('1.1.1.1', [BigDecimal(value)]) }
    end
    reader.close
  end

  def test_values_of_the_wrong_type_are_still_argument_errors
    reader = MaxMind::DB::Rust::Reader.new(db_path)

    wrong_types = [
      nil, :ip, Object.new, 2**70, 1e30, Float::INFINITY, -Float::INFINITY, Float::NAN, Complex(1, 1),
      Class.new(Numeric).new
    ]
    wrong_types.each do |element|
      error = assert_raises(ArgumentError, element.inspect) { reader.get_path('1.1.1.1', [element]) }
      assert_equal 'Path elements must be Strings or Integers', error.message
    end
    assert_raises(ArgumentError) { reader.get_path('1.1.1.1', 'ip') }
    assert_raises(ArgumentError) { reader.get_many(1) }

    reader.close
  end

  private

  def assert_child_succeeds(body)
    lib_dir = File.expand_path('../lib', __dir__)
    script = <<~RUBY
      $LOAD_PATH.unshift(#{lib_dir.dump})
      require 'maxmind/db/rust'
      reader = MaxMind::DB::Rust::Reader.new(#{db_path.dump})
      #{body}
    RUBY

    _stdout, stderr, status = Open3.capture3(RbConfig.ruby, '-e', script)

    assert_predicate status, :success?, stderr
  end

  def db_path
    path = File.join(TEST_DATA_DIR, 'MaxMind-DB-test-ipv6-24.mmdb')
    skip 'Test database not found' unless File.exist?(path)
    path
  end
end
