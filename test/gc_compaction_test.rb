# frozen_string_literal: true

require 'open3'
require 'rbconfig'
require 'test_helper'

class GcCompactionTest < Minitest::Test
  TEST_DATA_DIR = File.join(__dir__, 'data', 'MaxMind-DB', 'test-data')

  # The string cache array's handle is kept in a Rust thread-local, so the
  # array must stay at the same address across compaction. This runs in a
  # child process because a stale handle crashes the VM instead of failing.
  def test_string_cache_survives_compaction
    skip 'GC compaction is not supported' unless GC.respond_to?(:compact)
    skip 'Test database not found' unless File.exist?(test_db_path)

    lib_dir = File.expand_path('../lib', __dir__)
    script = <<~RUBY
      require 'json'
      require 'objspace'
      $LOAD_PATH.unshift(#{lib_dir.dump})
      require 'maxmind/db/rust'

      # The cache constant's name is not a valid Ruby constant name, so find
      # the array through the module's references instead of const_get.
      def cache_address
        roots = ObjectSpace.reachable_objects_from(MaxMind::DB::Rust).select do |object|
          object.is_a?(Array) && object.length == 4096
        end
        raise "expected one string cache array, found \#{roots.length}" unless roots.length == 1

        JSON.parse(ObjectSpace.dump(roots.first)).fetch('address')
      end

      reader = MaxMind::DB::Rust::Reader.new(#{test_db_path.dump})
      expected = reader.get('81.2.69.142')
      address_before = cache_address

      GC.verify_compaction_references(toward: :empty, expand_heap: true)

      raise 'string cache array moved during compaction' unless cache_address == address_before

      1_000.times do
        raise 'lookup changed after compaction' unless reader.get('81.2.69.142') == expected
      end
      reader.close
    RUBY

    _stdout, stderr, status = Open3.capture3(RbConfig.ruby, '-e', script)

    assert_predicate status, :success?, stderr
  end

  # The lookup error waits while `each` unwinds, so it must survive compaction
  # there. This runs in a child process because a stale handle can crash the VM.
  def test_batch_lookup_error_survives_compaction
    skip 'GC compaction is not supported' unless GC.respond_to?(:compact)
    skip 'Test database not found' unless File.exist?(test_db_path)

    lib_dir = File.expand_path('../lib', __dir__)
    script = <<~RUBY
      $LOAD_PATH.unshift(#{lib_dir.dump})
      require 'maxmind/db/rust'

      reader = MaxMind::DB::Rust::Reader.new(#{test_db_path.dump})
      stream = Object.new
      stream.define_singleton_method(:each) do |&block|
        block.call('not an ip')
      ensure
        GC.verify_compaction_references(toward: :empty, expand_heap: true)
        Array.new(20_000) { +'reuse the freed slots' }
      end

      5.times do
        reader.get_many(stream)
        raise 'no error'
      rescue StandardError => e
        raise "wrong error: \#{e.class}: \#{e.message}" unless e.instance_of?(IPAddr::InvalidAddressError)
      end
      reader.close
    RUBY

    _stdout, stderr, status = Open3.capture3(RbConfig.ruby, '-e', script)

    assert_predicate status, :success?, stderr
  end

  # Defining a method from the block copies it. The copy must keep what the
  # block uses alive. This runs in a child process because it can crash the VM.
  def test_method_defined_from_a_batch_block_survives_gc
    skip 'Test database not found' unless File.exist?(test_db_path)

    lib_dir = File.expand_path('../lib', __dir__)
    script = <<~RUBY
      $LOAD_PATH.unshift(#{lib_dir.dump})
      require 'maxmind/db/rust'

      def stream
        Object.new.tap do |stream|
          stream.define_singleton_method(:each) do |&block|
            define_singleton_method(:emit, &block)
            emit('81.2.69.142')
          end
        end
      end

      path = %w[country iso_code]
      streams = Array.new(100) do
        reader = MaxMind::DB::Rust::Reader.new(#{test_db_path.dump})
        input = stream
        raise 'wrong result' unless reader.get_many_path(input, path) == ['GB']

        input
      end
      3.times do
        GC.start
        GC.verify_compaction_references(toward: :empty, expand_heap: true) if GC.respond_to?(:compact)
        Array.new(20_000) { +'reuse the freed slots' }
      end
      streams.each do |input|
        input.emit('1.1.1.1')
        raise 'no error'
      rescue RuntimeError => e
        raise unless e.message.include?('after it returned')
      end
    RUBY

    _stdout, stderr, status = Open3.capture3(RbConfig.ruby, '-e', script)

    assert_predicate status, :success?, stderr
  end

  private

  def test_db_path
    File.join(TEST_DATA_DIR, 'GeoIP2-City-Test.mmdb')
  end
end
