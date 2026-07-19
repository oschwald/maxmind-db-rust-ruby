# frozen_string_literal: true

require 'test_helper'

class ReaderTest < Minitest::Test
  TEST_DATA_DIR = File.join(__dir__, 'data', 'MaxMind-DB', 'test-data')

  def test_reader_class_exists
    assert defined?(MaxMind::DB::Rust::Reader)
  end

  def test_mode_constants
    assert_equal :MODE_AUTO, MaxMind::DB::Rust::MODE_AUTO
    assert_equal :MODE_FILE, MaxMind::DB::Rust::MODE_FILE
    assert_equal :MODE_MEMORY, MaxMind::DB::Rust::MODE_MEMORY
    assert_equal :MODE_MMAP, MaxMind::DB::Rust::MODE_MMAP
    assert_equal :MODE_PARAM_IS_BUFFER, MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER
  end

  def test_invalid_database_error_exists
    assert defined?(MaxMind::DB::Rust::InvalidDatabaseError)
    assert_operator MaxMind::DB::Rust::InvalidDatabaseError, :<, RuntimeError
  end

  def test_metadata_class_exists
    assert defined?(MaxMind::DB::Rust::Metadata)
  end

  def test_open_database_with_mode_memory
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path, mode: MaxMind::DB::Rust::MODE_MEMORY)

    refute_nil reader
    refute reader.closed
    reader.close

    assert reader.closed
  end

  def test_open_database_with_mode_mmap
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path, mode: MaxMind::DB::Rust::MODE_MMAP)

    refute_nil reader
    refute reader.closed
    reader.close
  end

  def test_open_database_with_mode_file
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path, mode: MaxMind::DB::Rust::MODE_FILE)

    refute_nil reader
    refute reader.closed
    reader.close
  end

  def test_open_database_with_mode_auto
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path, mode: MaxMind::DB::Rust::MODE_AUTO)

    refute_nil reader
    reader.close
  end

  def test_open_database_with_mode_param_is_buffer
    skip 'Test database not found' unless File.exist?(test_db_path)

    buffer = File.binread(test_db_path)
    reader = MaxMind::DB::Rust::Reader.new(buffer, mode: MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER)
    path_reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    refute_nil reader
    assert_equal path_reader.get('81.2.69.142'), reader.get('81.2.69.142')

    path_reader.close
    reader.close
  end

  def test_buffer_reader_preserves_invalid_utf8_string_bytes
    buffer = File.binread(string_value_db_path)
    valid_value = '1.1.1.16/28'
    value_offset = buffer.index(valid_value)

    refute_nil value_offset
    assert_equal value_offset, buffer.rindex(valid_value)

    buffer.setbyte(value_offset, 0xff)

    reader = MaxMind::DB::Rust::Reader.new(buffer, mode: MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER)
    value = reader.get('1.1.1.16')

    assert_equal Encoding::UTF_8, value.encoding
    refute_predicate value, :valid_encoding?
    assert_equal [0xff, *valid_value.bytes.drop(1)], value.bytes

    reader.close
  end

  def test_buffer_reader_preserves_invalid_utf8_map_key_bytes
    buffer = File.binread(decoder_db_path)
    valid_key = 'utf8_stringX'
    key_offset = buffer.index(valid_key)

    refute_nil key_offset
    assert_equal key_offset, buffer.rindex(valid_key)

    buffer.setbyte(key_offset, 0xff)

    reader = MaxMind::DB::Rust::Reader.new(buffer, mode: MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER)
    nested_map = reader.get('1.1.1.1').dig('map', 'mapX')
    key = nested_map.keys.find { |candidate| !candidate.valid_encoding? }

    refute_nil key
    assert_equal Encoding::UTF_8, key.encoding
    assert_equal [0xff, *valid_key.bytes.drop(1)], key.bytes
    assert_equal 'hello', nested_map[key]

    reader.close
  end

  def test_invalid_buffer_database
    error = assert_raises(MaxMind::DB::Rust::InvalidDatabaseError) do
      MaxMind::DB::Rust::Reader.new(
        'not a database',
        mode: MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER,
      )
    end
    assert_match(/valid MaxMind DB file/, error.message)
  end

  def test_open_database_default_mode
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    refute_nil reader
    reader.close
  end

  def test_reader_inspect
    skip 'Test database not found' unless File.exist?(test_db_path)

    modes = [
      MaxMind::DB::Rust::MODE_MMAP,
      MaxMind::DB::Rust::MODE_MEMORY,
    ]

    modes.each do |mode|
      reader = MaxMind::DB::Rust::Reader.new(test_db_path, mode: mode)

      assert_match(/\A#<MaxMind::DB::Rust::Reader:0x[0-9a-f]+ @closed=false @ip_version=6>\z/, reader.inspect)

      reader.close

      assert_match(/@closed=true/, reader.inspect)
    end
  end

  def test_buffer_reader_inspect_does_not_include_buffer
    skip 'Test database not found' unless File.exist?(test_db_path)

    buffer = File.binread(test_db_path)
    reader = MaxMind::DB::Rust::Reader.new(buffer, mode: MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER)

    assert_match(/MaxMind::DB::Rust::Reader/, reader.inspect)
    refute_includes reader.inspect, buffer

    reader.close
  end

  def test_invalid_database_file
    error = assert_raises(MaxMind::DB::Rust::InvalidDatabaseError) do
      MaxMind::DB::Rust::Reader.new(__FILE__)
    end
    assert_match(/valid MaxMind DB file/, error.message)
  end

  def test_nonexistent_database_file
    assert_raises(Errno::ENOENT) do
      MaxMind::DB::Rust::Reader.new('/nonexistent/path/to/database.mmdb')
    end
  end

  def test_get_ipv4_address
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    record = reader.get('81.2.69.142')

    refute_nil record
    assert_equal 'GB', record.dig('country', 'iso_code')

    reader.close
  end

  def test_get_rejects_empty_ip_address
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    error = assert_raises(ArgumentError) do
      reader.get('')
    end
    assert_match(/does not appear to be/, error.message)

    reader.close
  end

  def test_get_ipv6_address
    skip 'Test database not found' unless File.exist?(ipv6_test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(ipv6_test_db_path)
    _record = reader.get('::1')

    reader.close
  end

  def test_get_with_prefix_length_ipv6_address
    skip 'Test database not found' unless File.exist?(ipv6_test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(ipv6_test_db_path)
    record, prefix_length = reader.get_with_prefix_length('::1:ffff:ffff')

    assert_equal({ 'ip' => '::1:ffff:ffff' }, record)
    assert_equal 128, prefix_length

    reader.close
  end

  def test_get_with_ipaddr_object
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    ip = IPAddr.new('1.1.1.1')
    _record = reader.get(ip)

    # Should not raise an error
    reader.close
  end

  def test_get_with_prefix_length
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    _record, prefix_len = reader.get_with_prefix_length('1.1.1.1')

    assert_kind_of Integer, prefix_len
    assert_operator prefix_len, :>=, 0
    assert_operator prefix_len, :<=, 32

    reader.close
  end

  def test_get_returns_hash
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    record = reader.get('1.1.1.1')

    # Record should be either nil or a Hash
    assert(record.nil? || record.is_a?(Hash))

    reader.close
  end

  def test_metadata
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    metadata = reader.metadata

    assert_kind_of MaxMind::DB::Rust::Metadata, metadata
    assert_kind_of Integer, metadata.node_count
    assert_kind_of Integer, metadata.record_size
    assert_kind_of Integer, metadata.ip_version
    assert_kind_of String, metadata.database_type
    assert_kind_of Array, metadata.languages
    assert_kind_of Integer, metadata.binary_format_major_version
    assert_kind_of Integer, metadata.binary_format_minor_version
    assert_kind_of Integer, metadata.build_epoch
    assert_kind_of Hash, metadata.description
    assert_kind_of Integer, metadata.node_byte_size
    assert_kind_of Integer, metadata.search_tree_size

    reader.close
  end

  def test_close_and_closed
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    refute reader.closed

    reader.close

    assert reader.closed

    # Closing again should be idempotent
    reader.close

    assert reader.closed
  end

  def test_get_after_close
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    reader.close

    error = assert_raises(RuntimeError) do
      reader.get('1.1.1.1')
    end
    assert_match(/closed/, error.message)
  end

  def test_metadata_after_close
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    reader.close

    assert_raises(RuntimeError) do
      reader.metadata
    end
  end

  def test_iterator_support
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Test that reader responds to each
    assert_respond_to reader, :each

    # Test that we can iterate (just get first item)
    count = 0
    reader.each do |network, data|
      assert_kind_of IPAddr, network
      assert(data.nil? || data.is_a?(Hash))
      count += 1
      break if count >= 5 # Just test first 5 entries
    end

    assert_predicate count, :positive?

    reader.close
  end

  def test_enumerable_interface
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Test Enumerable methods
    first_three = reader.take(3)

    assert_equal 3, first_three.length

    first_three.each do |network, _data|
      assert_kind_of IPAddr, network
    end

    reader.close
  end

  def test_each_returns_enumerator_without_block
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    enumerator = reader.each

    assert_kind_of Enumerator, enumerator

    first_three = enumerator.take(3)

    assert_equal 3, first_three.length
    first_three.each do |network, data|
      assert_kind_of IPAddr, network
      assert(data.nil? || data.is_a?(Hash))
    end

    reader.close
  end

  def test_each_with_network_returns_enumerator_without_block
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    enumerator = reader.each('214.0.0.0/8')

    assert_kind_of Enumerator, enumerator

    networks = enumerator.take(3).map { |network, _data| network.to_s }

    assert_predicate networks.length, :positive?
    networks.each do |network|
      assert network.start_with?('214.'), "Network #{network} should be in 214.0.0.0/8"
    end

    reader.close
  end

  def test_iterator_within_ipv4_network_string
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Iterate within a specific IPv4 subnet using string
    # Use 214.0.0.0/8 which has entries in the test database
    networks = []
    reader.each('214.0.0.0/8') do |network, data|
      networks << network.to_s

      assert_kind_of IPAddr, network
      assert(data.nil? || data.is_a?(Hash))

      # Verify network is within the specified range
      assert network.to_s.start_with?('214.'), "Network #{network} should be in 214.0.0.0/8"
    end

    # Should find some networks in this range
    assert_predicate networks.length, :positive?, 'Should find networks in 214.0.0.0/8'

    reader.close
  end

  def test_iterator_within_ipv4_network_ipaddr
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Iterate within a specific IPv4 subnet using IPAddr
    subnet = IPAddr.new('81.2.69.0/24')
    networks = []
    reader.each(subnet) do |network, _data|
      networks << network.to_s

      assert_kind_of IPAddr, network
    end

    # Should find at least one network in this specific subnet
    assert_predicate networks.length, :positive?, 'Should find networks in 81.2.69.0/24'

    reader.close
  end

  def test_iterator_within_ipv6_network
    skip 'Test database not found' unless File.exist?(ipv6_test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(ipv6_test_db_path)

    # Iterate within a specific IPv6 subnet
    networks = []
    reader.each('2001::/16') do |network, _data|
      networks << network.to_s

      assert_kind_of IPAddr, network
      # Verify network is IPv6 and within range
      assert_predicate network, :ipv6?, 'Network should be IPv6'
      assert network.to_s.start_with?('2001:'), "Network #{network} should be in 2001::/16"
    end

    reader.close
  end

  def test_iterator_within_invalid_cidr
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Invalid CIDR should raise ArgumentError
    assert_raises(ArgumentError) do
      reader.each('not-a-valid-cidr') do |_network, _data|
        # Should not get here
      end
    end

    reader.close
  end

  def test_iterator_within_ipv6_in_ipv4_database
    skip 'Test database not found' unless File.exist?(ipv4_only_db_path)

    reader = MaxMind::DB::Rust::Reader.new(ipv4_only_db_path)

    assert_equal 4, reader.metadata.ip_version

    # IPv6 network in IPv4 database should raise ArgumentError
    assert_raises(ArgumentError) do
      reader.each('2001::/16') do |_network, _data|
        # Should not get here
      end
    end

    reader.close
  end

  def test_iterator_within_empty_result
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Use a network that likely has no data
    count = 0
    reader.each('240.0.0.0/8') do |network, data|
      count += 1

      assert_kind_of IPAddr, network
      assert(data.nil? || data.is_a?(Hash))
    end

    # This subnet might have no entries, which is fine
    # Just testing that it doesn't error

    reader.close
  end

  def test_iterator_within_modes
    skip 'Test database not found' unless File.exist?(test_db_path)

    # Test network iteration works in both MMAP and MEMORY modes
    [MaxMind::DB::Rust::MODE_MMAP, MaxMind::DB::Rust::MODE_MEMORY].each do |mode|
      reader = MaxMind::DB::Rust::Reader.new(test_db_path, mode: mode)

      networks = []
      reader.each('81.2.69.0/24') do |network, _data|
        networks << network.to_s
        break if networks.length >= 3 # Just test first 3
      end

      assert_predicate networks.length, :positive?, "Should find networks in mode #{mode}"

      reader.close
    end
  end

  def test_iterator_within_subset_of_full
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Collect networks from specific subnet
    subset_networks = []
    reader.each('214.0.0.0/8') do |network, _data|
      subset_networks << network.to_s
    end

    # Collect all networks
    all_networks = reader.map do |network, _data|
      network.to_s
    end

    # Subset should be less than or equal to all
    assert_operator subset_networks.length, :<=, all_networks.length, 'Subset should have fewer or equal networks than full database'

    # All subset networks should be in the all networks list
    subset_networks.each do |net|
      assert_includes all_networks, net,
                      "Network #{net} from subset should be in full database"
    end

    reader.close
  end

  def test_get_path
    skip 'Test database not found' unless File.exist?(decoder_db_path)

    reader = MaxMind::DB::Rust::Reader.new(decoder_db_path)

    assert_equal 'hello', reader.get_path('1.1.1.1', %w[map mapX utf8_stringX])
    assert_equal 1, reader.get_path('1.1.1.1', ['array', 0])
    assert_equal 3, reader.get_path('1.1.1.1', ['array', -1])
    assert_nil reader.get_path('1.1.1.1', ['array', 3])
    assert_nil reader.get_path('1.1.1.1', ['missing'])

    reader.close
  end

  def test_get_path_empty_path_returns_record
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    assert_equal reader.get('81.2.69.142'), reader.get_path('81.2.69.142', [])

    reader.close
  end

  def test_get_path_cache_uses_path_contents
    skip 'Test database not found' unless File.exist?(decoder_db_path)

    reader = MaxMind::DB::Rust::Reader.new(decoder_db_path)
    path = ['array', 0]

    assert_equal 1, reader.get_path('1.1.1.1', path)
    assert_equal 1, reader.get_path('1.1.1.1', path)

    path[1] = 1

    assert_equal 2, reader.get_path('1.1.1.1', path)

    path[1] = -1

    assert_equal 3, reader.get_path('1.1.1.1', path)

    reader.close
  end

  def test_get_path_rejects_invalid_path
    skip 'Test database not found' unless File.exist?(decoder_db_path)

    reader = MaxMind::DB::Rust::Reader.new(decoder_db_path)

    error = assert_raises(ArgumentError) do
      reader.get_path('1.1.1.1', 'array')
    end
    assert_match(/Path must be an Array/, error.message)

    error = assert_raises(ArgumentError) do
      reader.get_path('1.1.1.1', ['array', true])
    end
    assert_match(/Path elements must be Strings or Integers/, error.message)

    reader.close
  end

  def test_get_many
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    ips = ['81.2.69.142', '2001:220::', '1.1.1.1', '81.2.69.142']

    assert_equal ips.map { |ip| reader.get(ip) }, reader.get_many(ips)
    assert_equal ips.map { |ip| reader.get(ip) }, reader.get_many(ips.each)

    reader.close
  end

  def test_get_many_empty_array
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    assert_equal [], reader.get_many([])

    reader.close
  end

  def test_get_many_batches_larger_than_the_native_buffer
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    source_ips = ['81.2.69.142', '2001:220::', '1.1.1.1']
    ips = Array.new(257) { |index| source_ips[index % source_ips.length] }
    path = %w[country iso_code]

    assert_equal ips.map { |ip| reader.get(ip) }, reader.get_many(ips)
    assert_equal ips.map { |ip| reader.get_path(ip, path) }, reader.get_many_path(ips, path)

    reader.close
  end

  def test_get_many_streams_enumerables_without_materializing
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    ips = ['81.2.69.142', '2001:220::', '1.1.1.1', '81.2.69.142']
    stream = Object.new
    stream.define_singleton_method(:each) do |&block|
      return enum_for(:each) unless block

      ips.each(&block)
    end
    stream.define_singleton_method(:to_a) do
      raise 'get_many should not materialize Enumerable inputs'
    end

    assert_equal ips.map { |ip| reader.get(ip) }, reader.get_many(stream)
    assert_equal ips.map { |ip| reader.get_path(ip, %w[country iso_code]) },
                 reader.get_many_path(stream, %w[country iso_code])

    reader.close
  end

  def test_get_many_path
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    ips = ['81.2.69.142', '2001:220::', '1.1.1.1', '81.2.69.142']
    path = %w[country iso_code]

    assert_equal ips.map { |ip| reader.get_path(ip, path) }, reader.get_many_path(ips, path)
    assert_equal ips.map { |ip| reader.get_path(ip, path) }, reader.get_many_path(ips.each, path)

    reader.close
  end

  def test_get_many_rejects_invalid_input
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    error = assert_raises(ArgumentError) do
      reader.get_many('81.2.69.142')
    end
    assert_match(/ips must be an Array or Enumerable/, error.message)

    reader.close
  end

  def test_shared_reader_concurrent_lookups_paths_batches_and_iteration
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    ips = ['81.2.69.142', '2001:220::', '1.1.1.1']
    path = %w[country iso_code]
    start_queue = Queue.new

    threads = Array.new(8) do |worker_id|
      Thread.new do
        start_queue.pop
        operations = 0
        100.times do |index|
          ip = ips[(worker_id + index) % ips.length]
          reader.get(ip)
          reader.get_path(ip, path)
          reader.get_many(ips)
          reader.get_many_path(ips, path)
          operations += 4

          operations += reader.each.take(3).length if (index % 25).zero?
        end
        operations
      end
    end

    threads.length.times { start_queue << true }
    operations = threads.map(&:value)

    assert_operator operations.sum, :>, 0

    reader.close
  end

  def test_string_cache_stays_process_bounded_across_threads
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    GC.start
    root_arrays_before = ObjectSpace.each_object(Array).count { |array| array.length == 4096 }

    threads = Array.new(16) do
      Thread.new { reader.get('81.2.69.142') }
    end
    threads.each(&:join)
    GC.start
    root_arrays_after = ObjectSpace.each_object(Array).count { |array| array.length == 4096 }

    assert_operator root_arrays_before, :>=, 1
    assert_equal root_arrays_before, root_arrays_after

    reader.close
  end

  def test_close_during_concurrent_lookups_reports_closed_reader
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    start_queue = Queue.new

    threads = Array.new(4) do
      Thread.new do
        start_queue.pop
        operations = 0
        loop do
          reader.get('81.2.69.142')
          reader.get_path('81.2.69.142', %w[country iso_code])
          operations += 2
        rescue RuntimeError => e
          raise unless e.message.include?('closed')

          break operations
        end
      end
    end

    threads.length.times { start_queue << true }
    sleep 0.01
    reader.close
    operations = threads.map(&:value)

    assert reader.closed
    assert_operator operations.sum, :>, 0
  end

  def test_each_after_close_reports_closed_reader
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    reader.close

    error = assert_raises(RuntimeError) do
      reader.each do |_network, _data|
        # Should not get here
      end
    end
    assert_match(/closed/, error.message)
  end

  private

  def test_db_path
    File.join(TEST_DATA_DIR, 'GeoIP2-City-Test.mmdb')
  end

  def decoder_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-test-decoder.mmdb')
  end

  def ipv6_test_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-test-ipv6-32.mmdb')
  end

  def ipv4_only_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-test-ipv4-24.mmdb')
  end

  def string_value_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-string-value-entries.mmdb')
  end
end
