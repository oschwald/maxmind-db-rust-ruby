# frozen_string_literal: true

require 'fileutils'
require 'open3'
require 'pathname'
require 'rbconfig'
require 'tmpdir'
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

  def test_pathname_paths_open_in_path_modes
    skip 'Test database not found' unless File.exist?(test_db_path)

    OFFICIAL_PATH_MODES.each do |mode_name|
      reader = MaxMind::DB::Rust::Reader.new(
        Pathname.new(test_db_path),
        mode: MaxMind::DB::Rust.const_get(mode_name),
      )

      assert_equal 'London', reader.get('81.2.69.142').dig('city', 'names', 'en'), mode_name
      reader.close
    end
  end

  def test_paths_with_null_bytes_are_rejected
    path = "#{test_db_path}\0"
    expected = assert_raises(ArgumentError) { File.open(path) }
    error = assert_raises(ArgumentError) { MaxMind::DB::Rust::Reader.new(path) }
    assert_equal expected.message, error.message
  end

  def test_options_accept_a_positional_hash
    skip 'Test database not found' unless File.exist?(test_db_path)

    options = { mode: MaxMind::DB::Rust::MODE_MEMORY }.freeze
    reader = MaxMind::DB::Rust::Reader.new(test_db_path, options)

    assert_equal({ mode: MaxMind::DB::Rust::MODE_MEMORY }, options)

    assert_equal 'London', reader.get('81.2.69.142').dig('city', 'names', 'en')
    reader.close

    error = assert_raises(ArgumentError) do
      MaxMind::DB::Rust::Reader.new(test_db_path, {}, mode: MaxMind::DB::Rust::MODE_MEMORY)
    end
    assert_includes error.message, 'wrong number of arguments'
  end

  def test_subclasses_are_instantiated_and_initialized
    skip 'Test database not found' unless File.exist?(test_db_path)

    subclass = Class.new(MaxMind::DB::Rust::Reader) do
      attr_reader :initialized_with

      def initialize(database)
        @initialized_with = database
        super(database, mode: MaxMind::DB::Rust::MODE_MEMORY)
      end
    end

    reader = subclass.new(test_db_path)

    assert_instance_of subclass, reader
    assert_equal test_db_path, reader.initialized_with
    assert_equal 'London', reader.get('81.2.69.142').dig('city', 'names', 'en')
    reader.close
  end

  def test_readers_cannot_be_initialized_twice
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    error = assert_raises(RuntimeError) { reader.send(:initialize, test_db_path) }
    assert_includes error.message, 'already initialized'
    reader.close
  end

  def test_reentrant_initialize_is_rejected
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.allocate
    path = test_db_path
    database = Object.new
    database.define_singleton_method(:to_path) do
      reader.send(:initialize, path)
      path
    end

    error = assert_raises(RuntimeError) { reader.send(:initialize, database) }
    assert_includes error.message, 'already initialized'
    assert reader.closed
  end

  def test_failed_initialize_can_be_retried
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.allocate
    assert_raises(Errno::ENOENT) { reader.send(:initialize, '/nonexistent/path/to/database.mmdb') }

    reader.send(:initialize, test_db_path)

    refute reader.closed
    reader.close
  end

  def test_allocated_reader_behaves_as_closed
    reader = MaxMind::DB::Rust::Reader.allocate

    assert reader.closed
    assert_raises(RuntimeError) { reader.get('1.1.1.1') }
    assert_raises(RuntimeError) { reader.metadata }
  end

  def test_readers_cannot_be_copied
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    assert_raises(TypeError) { reader.dup }
    assert_raises(TypeError) { reader.clone }
    reader.close
  end

  def test_unreadable_file_raises_errno_eacces
    skip 'Test database not found' unless File.exist?(test_db_path)
    skip 'root can read files without read permission' if Process.respond_to?(:uid) && Process.uid.zero?
    skip 'file permissions differ on Windows' if Gem.win_platform?

    Dir.mktmpdir do |dir|
      path = File.join(dir, 'unreadable.mmdb')
      File.binwrite(path, File.binread(test_db_path))
      File.chmod(0o000, path)

      OFFICIAL_PATH_MODES.each do |mode_name|
        assert_raises(Errno::EACCES, mode_name.to_s) do
          MaxMind::DB::Rust::Reader.new(path, mode: MaxMind::DB::Rust.const_get(mode_name))
        end
      end
    end
  end

  def test_directory_path_raises_errno_eisdir
    skip 'directories cannot be opened as files on Windows' if Gem.win_platform?

    OFFICIAL_PATH_MODES.each do |mode_name|
      error = assert_raises(Errno::EISDIR, mode_name.to_s) do
        MaxMind::DB::Rust::Reader.new(TEST_DATA_DIR, mode: MaxMind::DB::Rust.const_get(mode_name))
      end
      assert_includes error.message, TEST_DATA_DIR
    end
  end

  def test_file_that_cannot_be_mapped_raises_errno
    skip 'Requires a Linux /dev/null' unless RUBY_PLATFORM.include?('linux')

    error = assert_raises(Errno::ENODEV) { MaxMind::DB::Rust::Reader.new(File::NULL) }
    assert_includes error.message, File::NULL
  end

  def test_path_encoding_matches_file_open
    skip 'Test database not found' unless File.exist?(test_db_path)
    skip 'Linux passes path bytes to the OS unchanged' unless RUBY_PLATFORM.include?('linux')

    Dir.mktmpdir do |dir|
      utf8 = File.join(dir, "caf\u00e9.mmdb")
      FileUtils.cp(test_db_path, utf8)
      # A child process, because the test runner may set the internal encoding.
      script = <<~RUBY
        Encoding.default_internal = nil
        $LOAD_PATH.unshift(#{File.expand_path('../lib', __dir__).dump})
        require 'maxmind/db/rust'
        utf8 = #{utf8.dump}
        latin1 = utf8.encode('ISO-8859-1')

        # The encoded bytes are the file name, so this is a different file.
        [File.method(:open), MaxMind::DB::Rust::Reader.method(:new)].each do |open|
          open.call(latin1)
          raise 'no error'
        rescue Errno::ENOENT
          nil
        end

        File.binwrite(latin1, File.binread(utf8))
        MaxMind::DB::Rust::Reader.new(latin1).close
      RUBY
      _stdout, stderr, status = Open3.capture3(RbConfig.ruby, '-e', script)

      assert_predicate status, :success?, stderr
    end

    path = test_db_path.encode('UTF-16LE')
    without_internal_encoding do
      assert_raises(Encoding::CompatibilityError) { File.open(path) }
      assert_raises(Encoding::CompatibilityError) { MaxMind::DB::Rust::Reader.new(path) }
    end
  end

  def test_open_error_messages_keep_the_path_encoding
    skip 'Linux passes path bytes to the OS unchanged' unless RUBY_PLATFORM.include?('linux')

    without_internal_encoding do
      Dir.mktmpdir do |dir|
        latin1 = File.join(dir, "caf\u00e9.mmdb").encode('ISO-8859-1')
        modes = OFFICIAL_PATH_MODES.map { |name| MaxMind::DB::Rust.const_get(name) }

        expected = assert_raises(Errno::ENOENT) { File.open(latin1) }
        modes.each do |mode|
          error = assert_raises(Errno::ENOENT) { MaxMind::DB::Rust::Reader.new(latin1, mode: mode) }
          assert_equal expected.message.encoding, error.message.encoding
          assert_operator error.message, :end_with?, latin1
        end

        File.binwrite(latin1, 'not a database')
        modes.each do |mode|
          error = assert_raises(MaxMind::DB::Rust::InvalidDatabaseError) do
            MaxMind::DB::Rust::Reader.new(latin1, mode: mode)
          end
          assert_includes error.message, latin1
        end

        Dir.mkdir("#{latin1}.d")
        modes.each do |mode|
          error = assert_raises(Errno::EISDIR) { MaxMind::DB::Rust::Reader.new("#{latin1}.d", mode: mode) }
          assert_operator error.message, :end_with?, "#{latin1}.d"
        end
      end
    end
  end

  def test_windows_open_errors_are_system_call_errors
    skip 'Requires Windows' unless Gem.win_platform?

    # ERROR_INVALID_NAME has no matching Rust error kind.
    OFFICIAL_PATH_MODES.each do |mode_name|
      assert_raises(Errno::EINVAL, Errno::ENOENT, mode_name.to_s) do
        MaxMind::DB::Rust::Reader.new('a<b.mmdb', mode: MaxMind::DB::Rust.const_get(mode_name))
      end
    end
  end

  def test_path_uses_default_internal_encoding_like_file_open
    skip 'Test database not found' unless File.exist?(test_db_path)
    skip 'Requires a Linux file system' unless RUBY_PLATFORM.include?('linux')

    Dir.mktmpdir do |dir|
      path = File.join(dir, "caf\u00e9.mmdb")
      FileUtils.cp(test_db_path, path)
      script = <<~RUBY
        $LOAD_PATH.unshift(#{File.expand_path('../lib', __dir__).dump})
        require 'maxmind/db/rust'
        path = #{path.dump}.encode('ISO-8859-1')
        File.open(path).close
        MaxMind::DB::Rust::Reader.new(path).close
      RUBY
      _stdout, stderr, status = Open3.capture3(
        RbConfig.ruby, '-E', 'UTF-8:UTF-8', '-e', script
      )

      assert_predicate status, :success?, stderr
    end
  end

  def test_private_to_path_is_used_like_file_open
    skip 'Test database not found' unless File.exist?(test_db_path)

    path = test_db_path
    database = Class.new do
      define_method(:to_path) { path }
      private :to_path
    end.new

    MaxMind::DB::Rust::Reader.new(database).close
  end

  def test_subclass_inspect_names_the_subclass
    skip 'Test database not found' unless File.exist?(test_db_path)

    subclass = Class.new(MaxMind::DB::Rust::Reader)
    reader = subclass.new(test_db_path)

    assert_match(/\A##{Regexp.escape("<#{subclass}")}:0x[0-9a-f]+ @closed=false @ip_version=6>\z/, reader.inspect)
    reader.close
  end

  def test_invalid_ip_raises_ipaddr_invalid_address_error
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    ['not-an-ip', '', '1.2.3.256', "\xFF".b].each do |ip|
      assert_raises(IPAddr::InvalidAddressError, ip.inspect) { reader.get(ip) }
      assert_raises(IPAddr::InvalidAddressError, ip.inspect) { reader.get_many([ip]) }
    end
    reader.close
  end

  def test_get_accepts_cidr_strings_like_ipaddr
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    assert_equal reader.get('81.2.69.142'), reader.get('81.2.69.142/32')
    # IPAddr masks the address to the network, so these look up 81.2.69.160
    # and 81.2.69.128.
    refute_nil reader.get('81.2.69.170/27')
    assert_equal reader.get('81.2.69.160'), reader.get('81.2.69.170/27')
    assert_nil reader.get('81.2.69.142/26')
    assert_equal reader.get_with_prefix_length('81.2.69.142'),
                 reader.get_with_prefix_length('81.2.69.142/32')
    reader.close
  end

  def test_get_accepts_scoped_ipv6_addresses
    skip 'Test database not found' unless File.exist?(ipv6_db_path)

    reader = MaxMind::DB::Rust::Reader.new(ipv6_db_path)

    assert_nil reader.get('fe80::1%eth0')
    assert_equal reader.get('::1:ffff:ffff'), reader.get('::1:ffff:ffff%eth0')
    reader.close
  end

  def test_non_string_ip_arguments_go_to_ipaddr
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)

    [16_909_060, :'81.2.69.142', Object.new, nil].each do |ip|
      assert_raises(IPAddr::AddressFamilyError, ip.inspect) { reader.get(ip) }
    end
    reader.close
  end

  def test_ipv6_in_ipv4_database_error_echoes_the_argument
    reader = MaxMind::DB::Rust::Reader.new(File.join(TEST_DATA_DIR, 'MaxMind-DB-test-ipv4-24.mmdb'))

    error = assert_raises(ArgumentError) { reader.get('fe80::1%eth0') }
    assert_includes error.message, 'Error looking up fe80::1%eth0.'
    reader.close
  end

  def test_metadata_is_available_after_close
    skip 'Test database not found' unless File.exist?(test_db_path)

    reader = MaxMind::DB::Rust::Reader.new(test_db_path)
    expected = reader.metadata.database_type
    reader.close

    assert_equal expected, reader.metadata.database_type
    assert_raises(RuntimeError) { reader.get('81.2.69.142') }
  end

  private

  # The test runner may set the internal encoding, which changes path handling.
  def without_internal_encoding
    verbose = $VERBOSE
    internal = Encoding.default_internal
    $VERBOSE = nil
    Encoding.default_internal = nil
    yield
  ensure
    Encoding.default_internal = internal
    $VERBOSE = verbose
  end

  def test_db_path
    File.join(TEST_DATA_DIR, 'GeoIP2-City-Test.mmdb')
  end

  def ipv6_db_path
    File.join(TEST_DATA_DIR, 'MaxMind-DB-test-ipv6-24.mmdb')
  end
end
