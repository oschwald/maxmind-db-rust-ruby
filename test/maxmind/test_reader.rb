# frozen_string_literal: true

# Copyright (c) 2018-2025 MaxMind, Inc.
#
# Licensed under the Apache License, Version 2.0 or the MIT License.
# This file is adapted from the official MaxMind-DB-Reader-ruby project.
# Original: https://github.com/maxmind/MaxMind-DB-Reader-ruby
#
# Modifications for maxmind-db-rust:
# - Changed require 'maxmind/db' to require 'maxmind/db/rust'
# - Changed MaxMind::DB to MaxMind::DB::Rust
# - Changed MODE_FILE to MODE_MMAP (MODE_FILE not supported)
# - Updated test data paths to test/data/MaxMind-DB/test-data/
# - Removed MODE_PARAM_IS_BUFFER tests (not supported)
# - Removed test_reader_inspect (not implemented)
# - Updated error messages to match Rust implementation
# - Removed internal method tests (read_node)

require 'maxmind/db/rust'
require 'minitest/autorun'
require_relative 'mmdb_util'

class MaxMindReaderTest < Minitest::Test
  def test_reader
    modes = [
      MaxMind::DB::Rust::MODE_MMAP,
      MaxMind::DB::Rust::MODE_MEMORY,
    ]

    modes.each do |mode|
      record_sizes = [24, 28, 32]
      record_sizes.each do |record_size|
        ip_versions = [4, 6]
        ip_versions.each do |ip_version|
          filename = "test/data/MaxMind-DB/test-data/MaxMind-DB-test-ipv#{ip_version}-#{record_size}.mmdb"
          reader = MaxMind::DB::Rust::Reader.new(filename, mode: mode)
          check_metadata(reader, ip_version, record_size)
          if ip_version == 4
            check_ipv4(reader, filename)
          else
            check_ipv6(reader, filename)
          end
          reader.close
        end
      end
    end
  end

  def test_get_with_prefix_len
    decoder_record = {
      'array' => [1, 2, 3],
      'boolean' => true,
      'bytes' => "\x00\x00\x00*",
      'double' => 42.123456,
      'float' => 1.100000023841858,
      'int32' => -268_435_456,
      'map' => {
        'mapX' => {
          'arrayX' => [7, 8, 9],
          'utf8_stringX' => 'hello',
        },
      },
      'uint128' => 1_329_227_995_784_915_872_903_807_060_280_344_576,
      'uint16' => 0x64,
      'uint32' => 0x10000000,
      'uint64' => 0x1000000000000000,
      'utf8_string' => 'unicode! ☯ - ♫',
    }

    tests = [{
      'ip' => '1.1.1.1',
      'file_name' => 'MaxMind-DB-test-ipv6-32.mmdb',
      'expected_prefix_length' => 8,
      'expected_record' => nil,
    }, {
      'ip' => '::1:ffff:ffff',
      'file_name' => 'MaxMind-DB-test-ipv6-24.mmdb',
      'expected_prefix_length' => 128,
      'expected_record' => {
        'ip' => '::1:ffff:ffff'
      },
    }, {
      'ip' => '::2:0:1',
      'file_name' => 'MaxMind-DB-test-ipv6-24.mmdb',
      'expected_prefix_length' => 122,
      'expected_record' => {
        'ip' => '::2:0:0'
      },
    }, {
      'ip' => '1.1.1.1',
      'file_name' => 'MaxMind-DB-test-ipv4-24.mmdb',
      'expected_prefix_length' => 32,
      'expected_record' => {
        'ip' => '1.1.1.1'
      },
    }, {
      'ip' => '1.1.1.3',
      'file_name' => 'MaxMind-DB-test-ipv4-24.mmdb',
      'expected_prefix_length' => 31,
      'expected_record' => {
        'ip' => '1.1.1.2'
      },
    }, {
      'ip' => '1.1.1.3',
      'file_name' => 'MaxMind-DB-test-decoder.mmdb',
      'expected_prefix_length' => 24,
      'expected_record' => decoder_record,
    }, {
      'ip' => '::ffff:1.1.1.128',
      'file_name' => 'MaxMind-DB-test-decoder.mmdb',
      'expected_prefix_length' => 120,
      'expected_record' => decoder_record,
    }, {
      'ip' => '::1.1.1.128',
      'file_name' => 'MaxMind-DB-test-decoder.mmdb',
      'expected_prefix_length' => 120,
      'expected_record' => decoder_record,
    }, {
      'ip' => '200.0.2.1',
      'file_name' => 'MaxMind-DB-no-ipv4-search-tree.mmdb',
      'expected_prefix_length' => 0,
      'expected_record' => '::/64',
    }, {
      'ip' => '::200.0.2.1',
      'file_name' => 'MaxMind-DB-no-ipv4-search-tree.mmdb',
      'expected_prefix_length' => 64,
      'expected_record' => '::/64',
    }, {
      'ip' => '0:0:0:0:ffff:ffff:ffff:ffff',
      'file_name' => 'MaxMind-DB-no-ipv4-search-tree.mmdb',
      'expected_prefix_length' => 64,
      'expected_record' => '::/64',
    }, {
      'ip' => 'ef00::',
      'file_name' => 'MaxMind-DB-no-ipv4-search-tree.mmdb',
      'expected_prefix_length' => 1,
      'expected_record' => nil,
    }]

    tests.each do |test|
      reader = MaxMind::DB::Rust::Reader.new("test/data/MaxMind-DB/test-data/#{test['file_name']}")
      record, prefix_length = reader.get_with_prefix_length(test['ip'])

      assert_equal(test['expected_prefix_length'], prefix_length,
                   format('expected prefix_length of %d for %s in %s but got %p',
                          test['expected_prefix_length'], test['ip'],
                          test['file_name'], prefix_length))

      msg = format('expected_record for %s in %s', test['ip'], test['file_name'])
      if test['expected_record'].nil?
        assert_nil(record, msg)
      else
        assert_equal(test['expected_record'], record, msg)
      end
    end
  end

  def test_decoder
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    record = reader.get('::1.1.1.0')

    assert_equal([1, 2, 3], record['array'])
    assert(record['boolean'])
    assert_equal("\x00\x00\x00*".b, record['bytes'])
    assert_in_delta(42.123456, record['double'])
    assert_in_delta(1.1, record['float'])
    assert_equal(-268_435_456, record['int32'])
    assert_equal(
      {
        'mapX' => {
          'arrayX' => [7, 8, 9],
          'utf8_stringX' => 'hello',
        },
      },
      record['map'],
    )
    assert_equal(100, record['uint16'])
    assert_equal(268_435_456, record['uint32'])
    assert_equal(1_152_921_504_606_846_976, record['uint64'])
    assert_equal('unicode! ☯ - ♫', record['utf8_string'])
    assert_equal(1_329_227_995_784_915_872_903_807_060_280_344_576, record['uint128'])
    reader.close
  end

  def test_metadata_pointers
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-metadata-pointers.mmdb'
    )

    assert_equal('Lots of pointers in metadata', reader.metadata.database_type)
    reader.close
  end

  def test_no_ipv4_search_tree
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-no-ipv4-search-tree.mmdb'
    )

    # Both "::0/64" and "::/64" are valid representations of the same IPv6 network
    assert_equal('::/64', reader.get('1.1.1.1'))
    assert_equal('::/64', reader.get('192.1.1.1'))
    reader.close
  end

  def test_ipv6_address_in_ipv4_database
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-ipv4-24.mmdb'
    )
    e = assert_raises ArgumentError do
      reader.get('2001::')
    end
    # Error message matches the official gem format
    assert_equal(
      'Error looking up 2001::. You attempted to look up an IPv6 address in an IPv4-only database',
      e.message,
    )
    reader.close
  end

  def test_bad_ip_parameter
    reader = MaxMind::DB::Rust::Reader.new('test/data/MaxMind-DB/test-data/GeoIP2-City-Test.mmdb')
    e = assert_raises ArgumentError do
      reader.get(Object.new)
    end
    # Our implementation will have a different error message
    assert_includes(e.message, 'does not appear to be')
    reader.close
  end

  def test_broken_database
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/GeoIP2-City-Test-Broken-Double-Format.mmdb'
    )
    e = assert_raises MaxMind::DB::Rust::InvalidDatabaseError do
      reader.get('2001:220::')
    end
    assert_equal(
      'The MaxMind DB file\'s data section contains bad data (unknown data type or corrupt data)',
      e.message,
    )
    reader.close
  end

  def test_ip_validation
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    e = assert_raises ArgumentError do
      reader.get('not_ip')
    end
    assert_includes(e.message, 'does not appear to be')
    reader.close
  end

  def test_missing_database
    e = assert_raises SystemCallError do
      MaxMind::DB::Rust::Reader.new('file-does-not-exist.mmdb')
    end
    assert_includes(e.message, 'No such file or directory')
  end

  def test_nondatabase
    e = assert_raises MaxMind::DB::Rust::InvalidDatabaseError do
      MaxMind::DB::Rust::Reader.new('README.md')
    end
    assert_includes(e.message, 'valid MaxMind DB file')
  end

  def test_too_many_constructor_args
    e = assert_raises ArgumentError do
      MaxMind::DB::Rust::Reader.new('README.md', {}, 'blah')
    end
    assert_includes(e.message, 'wrong number of arguments')
  end

  def test_no_constructor_args
    e = assert_raises ArgumentError do
      MaxMind::DB::Rust::Reader.new
    end
    assert_includes(e.message, 'wrong number of arguments')
  end

  def test_too_many_get_args
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    e = assert_raises ArgumentError do
      reader.get('1.1.1.1', 'blah')
    end
    assert_includes(e.message, 'wrong number of arguments')
    reader.close
  end

  def test_no_get_args
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    e = assert_raises ArgumentError do
      reader.get
    end
    assert_includes(e.message, 'wrong number of arguments')
    reader.close
  end

  def test_metadata_args
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    e = assert_raises ArgumentError do
      reader.metadata('hi')
    end
    assert_includes(e.message, 'wrong number of arguments')
    reader.close
  end

  def test_metadata_unknown_attribute
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    assert_raises NoMethodError do
      reader.metadata.what
    end
    reader.close
  end

  def test_close
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    reader.close
  end

  def test_double_close
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    reader.close
    reader.close
  end

  def test_closed_get
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    reader.close
    e = assert_raises RuntimeError do
      reader.get('1.1.1.1')
    end
    assert_includes(e.message, 'closed')
  end

  def test_closed_metadata
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/MaxMind-DB-test-decoder.mmdb'
    )
    reader.close

    # Our implementation raises an error on accessing metadata after close
    # rather than caching it
    assert_raises RuntimeError do
      reader.metadata.description
    end
  end

  def test_threads
    reader = MaxMind::DB::Rust::Reader.new(
      'test/data/MaxMind-DB/test-data/GeoIP2-Domain-Test.mmdb'
    )

    num_threads = 16
    num_lookups = 32
    thread_lookups = []
    num_threads.times do
      thread_lookups << []
    end

    threads = []
    num_threads.times do |i|
      threads << Thread.new do
        num_lookups.times do |j|
          thread_lookups[i] << reader.get("65.115.240.#{j}")
          thread_lookups[i] << reader.get("2a02:2770:3::#{j}")
        end
      end
    end

    threads.each(&:join)

    thread_lookups.each do |a|
      assert_equal(num_lookups * 2, a.length)
      thread_lookups.each do |b|
        assert_equal(a, b)
      end
    end

    reader.close
  end

  def check_metadata(reader, ip_version, record_size)
    metadata = reader.metadata

    assert_equal(2, metadata.binary_format_major_version, 'major_version')
    assert_equal(0, metadata.binary_format_minor_version, 'minor_version')
    assert_operator(metadata.build_epoch, :>, 1_373_571_901, 'build_epoch')
    assert_equal('Test', metadata.database_type, 'database_type')
    assert_equal(
      {
        'en' => 'Test Database',
        'zh' => 'Test Database Chinese',
      },
      metadata.description,
      'description',
    )
    assert_equal(ip_version, metadata.ip_version, 'ip_version')
    assert_equal(%w[en zh], metadata.languages, 'languages')
    assert_operator(metadata.node_count, :>, 36, 'node_count')
    assert_equal(record_size, metadata.record_size, 'record_size')
  end

  def check_ipv4(reader, filename)
    6.times do |i|
      address = "1.1.1.#{2**i}"

      assert_equal(
        { 'ip' => address },
        reader.get(address),
        "found expected data record for #{address} in #{filename}",
      )
    end

    pairs = {
      '1.1.1.3' => '1.1.1.2',
      '1.1.1.5' => '1.1.1.4',
      '1.1.1.7' => '1.1.1.4',
      '1.1.1.9' => '1.1.1.8',
      '1.1.1.15' => '1.1.1.8',
      '1.1.1.17' => '1.1.1.16',
      '1.1.1.31' => '1.1.1.16',
    }
    pairs.each do |key_address, value_address|
      data = { 'ip' => value_address }

      assert_equal(
        data,
        reader.get(key_address),
        "found expected data record for #{key_address} in #{filename}",
      )
    end

    ['1.1.1.33', '255.254.253.123'].each do |ip|
      assert_nil(
        reader.get(ip),
        "#{ip} is not in #{filename}",
      )
    end
  end

  def check_ipv6(reader, filename)
    subnets = [
      '::1:ffff:ffff', '::2:0:0', '::2:0:40', '::2:0:50', '::2:0:58',
    ]

    subnets.each do |address|
      assert_equal(
        { 'ip' => address },
        reader.get(address),
        "found expected data record for #{address} in #{filename}",
      )
    end

    pairs = {
      '::2:0:1' => '::2:0:0',
      '::2:0:33' => '::2:0:0',
      '::2:0:39' => '::2:0:0',
      '::2:0:41' => '::2:0:40',
      '::2:0:49' => '::2:0:40',
      '::2:0:52' => '::2:0:50',
      '::2:0:57' => '::2:0:50',
      '::2:0:59' => '::2:0:58',
    }

    pairs.each do |key_address, value_address|
      assert_equal(
        { 'ip' => value_address },
        reader.get(key_address),
        "found expected data record for #{key_address} in #{filename}",
      )
    end

    ['1.1.1.33', '255.254.253.123', '89fa::'].each do |ip|
      assert_nil(
        reader.get(ip),
        "#{ip} is not in #{filename}",
      )
    end
  end
end
