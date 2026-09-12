# frozen_string_literal: true

require 'test_helper'

class BadDataTest < Minitest::Test
  TEST_DATA_DIR = File.join(__dir__, 'data', 'MaxMind-DB', 'test-data')
  EMPTY_METADATA_PATHS = Dir[File.join(__dir__, 'data', 'MaxMind-DB', 'bad-data', 'libmaxminddb',
                                       'libmaxminddb-empty-*-last-in-metadata.mmdb')].freeze
  BAD_DATA_PATHS = (Dir[File.join(__dir__, 'data', 'MaxMind-DB', 'bad-data', '**', '*.mmdb')] - EMPTY_METADATA_PATHS).freeze
  PATH_MODES = [
    MaxMind::DB::Rust::MODE_MMAP,
    MaxMind::DB::Rust::MODE_MEMORY,
  ].freeze

  def test_bad_data_corpus_raises_invalid_database_error
    refute_empty BAD_DATA_PATHS

    PATH_MODES.each do |mode|
      BAD_DATA_PATHS.each do |path|
        assert_bad_database_rejected(path, mode)
      end
    end
  end

  def test_empty_containers_at_end_of_metadata_can_be_read
    assert_equal 2, EMPTY_METADATA_PATHS.length

    PATH_MODES.each do |mode|
      EMPTY_METADATA_PATHS.each do |path|
        reader = MaxMind::DB::Rust::Reader.new(path, mode: mode)

        assert_empty reader.metadata.description
        assert_empty reader.metadata.languages
        assert_equal({ 'ip' => 'test' }, reader.get('1.1.1.1'))
      ensure
        reader&.close
      end
    end
  end

  def test_lookup_resource_limits_raise_invalid_database_error
    reader = resource_limited_reader
    ip = '1.2.3.4'

    [
      [:get, ip],
      [:get_with_prefix_length, ip],
      [:get_path, ip, []],
      [:get_many, [ip]],
      [:get_many, [ip].each],
      [:get_many_path, [ip], []],
      [:get_many_path, [ip].each, []],
    ].each do |method, *args|
      error = assert_raises(MaxMind::DB::Rust::InvalidDatabaseError, method.to_s) do
        reader.public_send(method, *args)
      end

      assert_match(/bad data/, error.message)
    end
  ensure
    reader&.close
  end

  def test_iteration_resource_limits_raise_invalid_database_error
    reader = resource_limited_reader

    error = assert_raises(MaxMind::DB::Rust::InvalidDatabaseError) { reader.each.to_a }

    assert_match(/bad data/, error.message)
  ensure
    reader&.close
  end

  def test_payload_resource_limit_raises_invalid_database_error
    path = File.join(TEST_DATA_DIR, 'MaxMind-DB-test-decoder-payload-limit-over.mmdb')
    reader = MaxMind::DB::Rust::Reader.new(path)

    assert_raises(MaxMind::DB::Rust::InvalidDatabaseError) { reader.get('1.2.3.4') }
  ensure
    reader&.close
  end

  def test_records_at_resource_limits_can_be_read
    %w[value payload].each do |limit|
      path = File.join(TEST_DATA_DIR, "MaxMind-DB-test-decoder-#{limit}-limit.mmdb")
      reader = MaxMind::DB::Rust::Reader.new(path)

      refute_nil reader.get('1.2.3.4')
    ensure
      reader&.close
    end
  end

  def test_metadata_resource_limit_raises_invalid_database_error
    path = File.join(TEST_DATA_DIR, 'MaxMind-DB-test-metadata-payload-limit.mmdb')

    PATH_MODES.each do |mode|
      assert_raises(MaxMind::DB::Rust::InvalidDatabaseError) do
        MaxMind::DB::Rust::Reader.new(path, mode: mode)
      end
    end
  end

  private

  def resource_limited_reader
    path = File.join(TEST_DATA_DIR, 'MaxMind-DB-test-decoder-value-limit-over.mmdb')
    MaxMind::DB::Rust::Reader.new(path)
  end

  def assert_bad_database_rejected(path, mode)
    error = assert_raises(MaxMind::DB::Rust::InvalidDatabaseError, "#{path} #{mode}") do
      open_lookup_and_verify_bad_database(path, mode)
    end

    assert_match(/bad data|valid MaxMind DB file|Database verification failed/, error.message)
  end

  def open_lookup_and_verify_bad_database(path, mode)
    reader = MaxMind::DB::Rust::Reader.new(path, mode: mode)
    reader.get('1.1.1.1')
    reader.get('128.0.0.1')
    # Some corpus files only contain corruption outside these lookup paths.
    reader.verify
  ensure
    reader&.close
  end
end
