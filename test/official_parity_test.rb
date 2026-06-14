# frozen_string_literal: true

$LOAD_PATH.unshift File.expand_path('../lib', __dir__)

require 'minitest/autorun'

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

  private

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
