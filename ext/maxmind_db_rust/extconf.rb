# frozen_string_literal: true

require 'mkmf'
require 'rb_sys/mkmf'

create_rust_makefile('maxmind/db/maxmind_db_rust') do |r|
  # Build with the dependency versions in the shipped Cargo.lock.
  r.extra_cargo_args += ['--locked']
end
