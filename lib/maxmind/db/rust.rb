# frozen_string_literal: true

require 'ipaddr'
require 'maxmind/db/maxmind_db_rust'

module MaxMind
  # Check if DB is already defined as a Class (official gem)
  if const_defined?(:DB) && DB.is_a?(Class)
    # Official gem loaded - Rust module is already attached to the class by native extension
    # Just add documentation here
    class << DB
      # The Rust module provides a high-performance Rust-based implementation for reading
      # {MaxMind DB files}[https://maxmind.github.io/MaxMind-DB/].
      #
      # MaxMind DB is a binary file format that stores data indexed by IP address
      # subnets (IPv4 or IPv6).
      #
      # This is a Rust-based implementation that provides significant performance
      # improvements over pure Ruby implementations while maintaining API compatibility
      # with the official MaxMind::DB gem.
      #
      # == Example
      #
      #   require 'maxmind/db/rust'
      #
      #   reader = MaxMind::DB::Rust::Reader.new(
      #     'GeoIP2-City.mmdb',
      #     mode: MaxMind::DB::Rust::MODE_MEMORY
      #   )
      #
      #   record = reader.get('1.1.1.1')
      #   if record.nil?
      #     puts '1.1.1.1 was not found in the database'
      #   else
      #     puts record['country']['iso_code']
      #     puts record['country']['names']['en']
      #   end
      #
      #   reader.close
      #
      # == Using the Iterator
      #
      #   require 'maxmind/db/rust'
      #
      #   reader = MaxMind::DB::Rust::Reader.new('GeoLite2-Country.mmdb')
      #
      #   # Iterate over all networks in the database
      #   reader.each do |network, data|
      #     puts "#{network}: #{data['country']['iso_code']}"
      #   end
      #
      #   # Iterate over networks within a specific subnet
      #   reader.each('192.168.0.0/16') do |network, data|
      #     puts "#{network}: #{data['city']['names']['en']}"
      #   end
      #
      #   # Also accepts IPAddr objects
      #   subnet = IPAddr.new('10.0.0.0/8')
      #   reader.each(subnet) do |network, data|
      #     puts "#{network}: #{data['country']['iso_code']}"
      #   end
      #
      #   reader.close
      #
      # The Rust module (MaxMind::DB::Rust) defines:
      # - Reader class
      # - Metadata class
      # - InvalidDatabaseError exception
      # - MODE_AUTO, MODE_MEMORY, MODE_MMAP constants
    end
  else
    # Official gem not loaded - define DB as a module
    module DB
      # Rust provides a high-performance Rust-based implementation for reading
      # {MaxMind DB files}[https://maxmind.github.io/MaxMind-DB/].
      #
      # MaxMind DB is a binary file format that stores data indexed by IP address
      # subnets (IPv4 or IPv6).
      #
      # This is a Rust-based implementation that provides significant performance
      # improvements over pure Ruby implementations while maintaining API compatibility
      # with the official MaxMind::DB gem.
      #
      # == Example
      #
      #   require 'maxmind/db/rust'
      #
      #   reader = MaxMind::DB::Rust::Reader.new(
      #     'GeoIP2-City.mmdb',
      #     mode: MaxMind::DB::Rust::MODE_MEMORY
      #   )
      #
      #   record = reader.get('1.1.1.1')
      #   if record.nil?
      #     puts '1.1.1.1 was not found in the database'
      #   else
      #     puts record['country']['iso_code']
      #     puts record['country']['names']['en']
      #   end
      #
      #   reader.close
      #
      # == Using the Iterator
      #
      #   require 'maxmind/db/rust'
      #
      #   reader = MaxMind::DB::Rust::Reader.new('GeoLite2-Country.mmdb')
      #
      #   # Iterate over all networks in the database
      #   reader.each do |network, data|
      #     puts "#{network}: #{data['country']['iso_code']}"
      #   end
      #
      #   # Iterate over networks within a specific subnet
      #   reader.each('192.168.0.0/16') do |network, data|
      #     puts "#{network}: #{data['city']['names']['en']}"
      #   end
      #
      #   # Also accepts IPAddr objects
      #   subnet = IPAddr.new('10.0.0.0/8')
      #   reader.each(subnet) do |network, data|
      #     puts "#{network}: #{data['country']['iso_code']}"
      #   end
      #
      #   reader.close
      module Rust
        # The native extension defines:
        # - Reader class
        # - Metadata class
        # - InvalidDatabaseError exception
        # - MODE_AUTO, MODE_MEMORY, MODE_MMAP constants
      end
    end
  end
end
