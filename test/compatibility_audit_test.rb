# frozen_string_literal: true

require 'open3'
require 'rbconfig'
require 'test_helper'

class CompatibilityAuditTest < Minitest::Test
  TEST_DATA_DIR = File.join(__dir__, 'data', 'MaxMind-DB', 'test-data')
  OFFICIAL_PATH_MODES = %i[MODE_AUTO MODE_FILE MODE_MEMORY].freeze

  def test_official_mode_constants_are_supported
    expected_constants = {
      MODE_AUTO: :MODE_AUTO,
      MODE_FILE: :MODE_FILE,
      MODE_MEMORY: :MODE_MEMORY,
      MODE_PARAM_IS_BUFFER: :MODE_PARAM_IS_BUFFER,
    }

    expected_constants.each do |name, value|
      assert_equal value, MaxMind::DB::Rust.const_get(name)
    end
  end

  def test_path_backed_official_modes_return_same_record
    skip 'Test database not found' unless File.exist?(test_db_path)

    readers = OFFICIAL_PATH_MODES.map do |mode_name|
      MaxMind::DB::Rust::Reader.new(
        test_db_path,
        mode: MaxMind::DB::Rust.const_get(mode_name),
      )
    end

    records = readers.map { |reader| reader.get('81.2.69.142') }

    assert_equal [records.first] * records.length, records

    readers.each(&:close)
  end

  def test_buffer_mode_returns_same_record_as_path_mode
    skip 'Test database not found' unless File.exist?(test_db_path)

    path_reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    buffer_reader = MaxMind::DB::Rust::Reader.new(
      File.binread(test_db_path),
      mode: MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER,
    )

    assert_equal path_reader.get('81.2.69.142'), buffer_reader.get('81.2.69.142')

    path_reader.close
    buffer_reader.close
  end

  def test_loading_after_maxmind_db_class_preserves_class_namespace
    skip 'Test database not found' unless File.exist?(test_db_path)

    lib_dir = File.expand_path('../lib', __dir__)
    script = <<~RUBY
      module MaxMind
        class DB
        end
      end

      $LOAD_PATH.unshift(#{lib_dir.dump})
      require 'maxmind/db/rust'

      raise 'MaxMind::DB was replaced' unless MaxMind::DB.is_a?(Class)
      raise 'missing Rust namespace' unless MaxMind::DB.const_defined?(:Rust)
      raise 'missing Reader' unless MaxMind::DB::Rust.const_defined?(:Reader)
      raise 'missing buffer mode' unless MaxMind::DB::Rust::MODE_PARAM_IS_BUFFER == :MODE_PARAM_IS_BUFFER

      reader = MaxMind::DB::Rust::Reader.new(#{test_db_path.dump})
      begin
        raise 'lookup failed' unless reader.get('81.2.69.142').is_a?(Hash)
      ensure
        reader.close
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
