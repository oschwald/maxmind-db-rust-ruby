# MaxMind Official Tests

This directory contains tests adapted from the official MaxMind-DB-Reader-ruby project.

## Copyright and License

These tests are derived from:
https://github.com/maxmind/MaxMind-DB-Reader-ruby

Copyright (c) 2018-2025 MaxMind, Inc.

Licensed under the Apache License, Version 2.0 or the MIT License, at your option.

See the original repository for full license details:

- Apache License 2.0: https://github.com/maxmind/MaxMind-DB-Reader-ruby/blob/main/LICENSE-APACHE
- MIT License: https://github.com/maxmind/MaxMind-DB-Reader-ruby/blob/main/LICENSE-MIT

## Modifications

The tests have been adapted to work with the maxmind-db-rust implementation:

- Changed `require 'maxmind/db'` to `require 'maxmind/db/rust'`
- Changed `MaxMind::DB` to `MaxMind::DB::Rust`
- Removed `MODE_FILE` tests (not supported, use `MODE_MMAP` instead)
- Adjusted file paths to match our test data location
- Minor adaptations for API compatibility

## Syncing with Upstream

To update these tests from the official repository:

1. Check out the latest version in the submodule:

   ```bash
   cd test/maxmind-db-reader-ruby
   git pull origin main
   cd ../..
   ```

2. Review changes to the test files:

   ```bash
   git diff test/maxmind-db-reader-ruby/test/
   ```

3. Manually apply relevant changes to the files in `test/maxmind/`, making
   necessary adaptations for our implementation.

## Test Data

These tests use the test databases from the MaxMind-DB repository, located in:
`test/data/MaxMind-DB/test-data/`
