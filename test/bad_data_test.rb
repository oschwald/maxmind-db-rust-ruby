# frozen_string_literal: true

require 'test_helper'

class BadDataTest < Minitest::Test
  BAD_DATA_PATHS = Dir[File.join(__dir__, 'data', 'MaxMind-DB', 'bad-data', '**', '*.mmdb')].freeze
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

  private

  def assert_bad_database_rejected(path, mode)
    error = assert_raises(MaxMind::DB::Rust::InvalidDatabaseError, "#{path} #{mode}") do
      open_and_lookup_bad_database(path, mode)
    end

    assert_match(/bad data|valid MaxMind DB file/, error.message)
  end

  def open_and_lookup_bad_database(path, mode)
    reader = MaxMind::DB::Rust::Reader.new(path, mode: mode)
    reader.get('1.1.1.1')
  ensure
    reader&.close
  end
end
