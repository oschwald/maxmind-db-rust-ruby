# frozen_string_literal: true

require 'test_helper'

class ReaderTest < Minitest::Test
  TEST_DATA_DIR = File.join(__dir__, 'data', 'MaxMind-DB', 'test-data')

  def test_reader_class_exists
    assert defined?(MaxMind::DB::Rust::Reader)
  end

  def test_mode_constants
    assert_equal :MODE_AUTO, MaxMind::DB::Rust::MODE_AUTO
    assert_equal :MODE_MEMORY, MaxMind::DB::Rust::MODE_MEMORY
    assert_equal :MODE_MMAP, MaxMind::DB::Rust::MODE_MMAP
  end

  def test_invalid_database_error_exists
    assert defined?(MaxMind::DB::Rust::InvalidDatabaseError)
    assert MaxMind::DB::Rust::InvalidDatabaseError < RuntimeError
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

  def test_open_database_with_mode_auto
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path, mode: MaxMind::DB::Rust::MODE_AUTO)
    refute_nil reader
    reader.close
  end

  def test_open_database_default_mode
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    refute_nil reader
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
    record = reader.get('1.1.1.1')

    # The test database should have data
    refute_nil record if reader.metadata.database_type.include?('Test')

    reader.close
  end

  def test_get_ipv6_address
    skip 'Test database not found' unless File.exist?(ipv6_test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(ipv6_test_db_path)
    _record = reader.get('::1')

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
    assert prefix_len >= 0
    assert prefix_len <= 32

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
      break if count >= 5  # Just test first 5 entries
    end

    assert count > 0

    reader.close
  end

  def test_enumerable_interface
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Test Enumerable methods
    first_three = reader.take(3)
    assert_equal 3, first_three.length

    first_three.each do |network, data|
      assert_kind_of IPAddr, network
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
    assert networks.length.positive?, 'Should find networks in 214.0.0.0/8'

    reader.close
  end

  def test_iterator_within_ipv4_network_ipaddr
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Iterate within a specific IPv4 subnet using IPAddr
    subnet = IPAddr.new('81.2.69.0/24')
    networks = []
    reader.each(subnet) do |network, data|
      networks << network.to_s
      assert_kind_of IPAddr, network
    end

    # Should find at least one network in this specific subnet
    assert networks.length.positive?, 'Should find networks in 81.2.69.0/24'

    reader.close
  end

  def test_iterator_within_ipv6_network
    skip 'Test database not found' unless File.exist?(ipv6_test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(ipv6_test_db_path)

    # Iterate within a specific IPv6 subnet
    networks = []
    reader.each('2001::/16') do |network, data|
      networks << network.to_s
      assert_kind_of IPAddr, network
      # Verify network is IPv6 and within range
      assert network.ipv6?, 'Network should be IPv6'
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
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    # Check if database is IPv4-only
    if reader.metadata.ip_version == 4
      # IPv6 network in IPv4 database should raise ArgumentError
      assert_raises(ArgumentError) do
        reader.each('2001::/16') do |_network, _data|
          # Should not get here
        end
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
      reader.each('81.2.69.0/24') do |network, data|
        networks << network.to_s
        break if networks.length >= 3  # Just test first 3
      end

      assert networks.length.positive?, "Should find networks in mode #{mode}"

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
    all_networks = []
    reader.each do |network, _data|
      all_networks << network.to_s
    end

    # Subset should be less than or equal to all
    assert subset_networks.length <= all_networks.length,
           'Subset should have fewer or equal networks than full database'

    # All subset networks should be in the all networks list
    subset_networks.each do |net|
      assert all_networks.include?(net),
             "Network #{net} from subset should be in full database"
    end

    reader.close
  end

  private

  def test_db_path
    File.join(TEST_DATA_DIR, 'GeoIP2-City-Test.mmdb')
  end

  def ipv6_test_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-test-ipv6-32.mmdb')
  end
end
