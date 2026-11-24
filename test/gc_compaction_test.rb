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

  private

  def test_db_path
    File.join(TEST_DATA_DIR, 'GeoIP2-City-Test.mmdb')
  end
end
