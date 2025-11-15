# frozen_string_literal: true

require 'ipaddr'
require 'maxmind/db/maxmind_db_rust'

module MaxMind
  module DB
    module Rust
      # The native extension defines:
      # - Reader class
      # - Metadata class
      # - InvalidDatabaseError exception
      # - MODE_AUTO, MODE_MEMORY, MODE_MMAP constants
    end
  end
end
