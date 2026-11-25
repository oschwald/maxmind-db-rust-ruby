# frozen_string_literal: true

$LOAD_PATH.unshift File.expand_path('../lib', __dir__)

require 'minitest/autorun'
require 'pathname'
require 'tmpdir'

begin
  require 'maxmind/db'
  OFFICIAL_MAXMIND_DB_AVAILABLE = true
rescue LoadError
  OFFICIAL_MAXMIND_DB_AVAILABLE = false
end

require 'maxmind/db/rust' if OFFICIAL_MAXMIND_DB_AVAILABLE

class OfficialParityTest < Minitest::Test
  TEST_DATA_DIR = File.join(__dir__, 'data', 'MaxMind-DB', 'test-data')
  PATH_MODES = %i[MODE_AUTO MODE_FILE MODE_MEMORY].freeze
  LOOKUP_IPS = ['81.2.69.142', '1.1.1.1', '2001:220::'].freeze

  def setup
    skip 'official maxmind-db gem is not installed' unless OFFICIAL_MAXMIND_DB_AVAILABLE
  end

  def test_official_mode_constants_match
    %i[MODE_AUTO MODE_FILE MODE_MEMORY MODE_PARAM_IS_BUFFER].each do |name|
      assert_equal MaxMind::DB.const_get(name), MaxMind::DB::Rust.const_get(name)
    end
  end

  def test_path_mode_lookup_results_match_official_gem
    PATH_MODES.each do |mode_name|
      with_readers(city_db_path, mode_name) do |official, rust|
        LOOKUP_IPS.each do |ip|
          assert_same_record official.get(ip), rust.get(ip), "#{mode_name} #{ip}"
        end
      end
    end
  end

  def test_buffer_mode_lookup_results_match_official_gem
    buffer = File.binread(city_db_path)
    with_readers(buffer, :MODE_PARAM_IS_BUFFER) do |official, rust|
      LOOKUP_IPS.each do |ip|
        assert_same_record official.get(ip), rust.get(ip), "MODE_PARAM_IS_BUFFER #{ip}"
      end
    end
  end

  def test_get_with_prefix_length_matches_official_gem
    cases = [
      ['1.1.1.1', ipv4_db_path],
      ['1.1.1.3', ipv4_db_path],
      ['::2:0:1', ipv6_db_path],
      ['1.1.1.3', decoder_db_path],
    ]

    cases.each do |ip, path|
      with_readers(path, :MODE_FILE) do |official, rust|
        assert_equal official.get_with_prefix_length(ip), rust.get_with_prefix_length(ip), ip
      end
    end
  end

  def test_ipv6_in_ipv4_database_error_matches_official_gem
    with_readers(ipv4_db_path, :MODE_FILE) do |official, rust|
      official_error = assert_raises(ArgumentError) { official.get('2001::') }
      rust_error = assert_raises(ArgumentError) { rust.get('2001::') }

      assert_equal official_error.message, rust_error.message
    end
  end

  # The tests below cover the API gaps from RG-95: each one runs the same call
  # against both gems.

  def test_pathname_path_matches_official_gem
    PATH_MODES.each do |mode_name|
      with_readers(Pathname.new(city_db_path), mode_name) do |official, rust|
        assert_same_record official.get('81.2.69.142'), rust.get('81.2.69.142'), mode_name
      end
    end
  end

  def test_positional_options_hash_matches_official_gem
    official = MaxMind::DB.new(city_db_path, { mode: MaxMind::DB::MODE_MEMORY })
    rust = MaxMind::DB::Rust::Reader.new(city_db_path, { mode: MaxMind::DB::Rust::MODE_MEMORY })

    assert_same_record official.get('81.2.69.142'), rust.get('81.2.69.142'), 'positional Hash'
  ensure
    official&.close
    rust&.close
  end

  def test_subclassing_matches_official_gem
    [MaxMind::DB, MaxMind::DB::Rust::Reader].each do |base|
      subclass = Class.new(base) do
        attr_reader :initialized

        def initialize(*)
          @initialized = :subclass
          super
        end
      end

      reader = subclass.new(city_db_path)

      assert_instance_of subclass, reader, base.name
      assert_equal :subclass, reader.initialized, base.name
      reader.close
    end
  end

  def test_unreadable_file_error_matches_official_gem
    skip 'root can read files without read permission' if Process.uid.zero?

    Dir.mktmpdir do |dir|
      path = File.join(dir, 'unreadable.mmdb')
      File.binwrite(path, File.binread(city_db_path))
      File.chmod(0o000, path)

      assert_same_error_class(path, :MODE_FILE, Errno::EACCES)
    end
  end

  def test_directory_path_error_matches_official_gem
    assert_same_error_class(TEST_DATA_DIR, :MODE_FILE, Errno::EISDIR)
    assert_same_error_class(TEST_DATA_DIR, :MODE_MEMORY, Errno::EISDIR)
  end

  def test_invalid_ip_error_matches_official_gem
    with_readers(city_db_path, :MODE_FILE) do |official, rust|
      ['not-an-ip', '', '1.2.3.256'].each do |ip|
        official_error = assert_raises(IPAddr::InvalidAddressError) { official.get(ip) }
        rust_error = assert_raises(IPAddr::InvalidAddressError) { rust.get(ip) }

        assert_equal official_error.message, rust_error.message, ip.inspect
      end
    end
  end

  def test_non_string_ip_error_matches_official_gem
    with_readers(city_db_path, :MODE_FILE) do |official, rust|
      [16_909_060, :'81.2.69.142', Object.new].each do |ip|
        official_error = assert_raises(ArgumentError) { official.get(ip) }
        rust_error = assert_raises(ArgumentError) { rust.get(ip) }

        assert_equal official_error.class, rust_error.class, ip.inspect
      end
    end
  end

  def test_scoped_ipv6_in_ipv4_database_error_matches_official_gem
    with_readers(ipv4_db_path, :MODE_FILE) do |official, rust|
      official_error = assert_raises(ArgumentError) { official.get('fe80::1%eth0') }
      rust_error = assert_raises(ArgumentError) { rust.get('fe80::1%eth0') }

      assert_equal official_error.message, rust_error.message
    end
  end

  def test_cidr_lookup_matches_official_gem
    with_readers(city_db_path, :MODE_FILE) do |official, rust|
      ['81.2.69.142/32', '81.2.69.170/27', '81.2.69.142/26'].each do |ip|
        assert_same_record official.get(ip), rust.get(ip), ip
        assert_equal official.get_with_prefix_length(ip), rust.get_with_prefix_length(ip), ip
      end
    end
  end

  def test_scoped_ipv6_lookup_matches_official_gem
    with_readers(ipv6_db_path, :MODE_FILE) do |official, rust|
      ['fe80::1%eth0', '::1:ffff:ffff%eth0'].each do |ip|
        assert_same_record official.get(ip), rust.get(ip), ip
      end
    end
  end

  def test_metadata_after_close_matches_official_gem
    with_readers(city_db_path, :MODE_FILE) do |official, rust|
      official.close
      rust.close

      assert_equal official.metadata.database_type, rust.metadata.database_type
      assert_equal official.metadata.ip_version, rust.metadata.ip_version
    end
  end

  private

  def assert_same_error_class(database, mode_name, expected)
    official_error = assert_raises(expected) do
      MaxMind::DB.new(database, mode: MaxMind::DB.const_get(mode_name))
    end
    rust_error = assert_raises(expected) do
      MaxMind::DB::Rust::Reader.new(database, mode: MaxMind::DB::Rust.const_get(mode_name))
    end

    assert_equal official_error.class, rust_error.class, mode_name.to_s
  end

  def assert_same_record(expected, actual, message)
    if expected.nil?
      assert_nil actual, message
    else
      assert_equal expected, actual, message
    end
  end

  def with_readers(database, mode_name)
    official = MaxMind::DB.new(database, mode: MaxMind::DB.const_get(mode_name))
    rust = MaxMind::DB::Rust::Reader.new(database, mode: MaxMind::DB::Rust.const_get(mode_name))
    yield official, rust
  ensure
    official&.close
    rust&.close
  end

  def city_db_path
    File.join(TEST_DATA_DIR, 'GeoIP2-City-Test.mmdb')
  end

  def decoder_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-test-decoder.mmdb')
  end

  def ipv4_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-test-ipv4-24.mmdb')
  end

  def ipv6_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-test-ipv6-24.mmdb')
  end
end
