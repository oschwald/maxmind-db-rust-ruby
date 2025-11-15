# frozen_string_literal: true

Gem::Specification.new do |s|
  s.authors     = ['Gregory Oschwald']
  s.files       = Dir['lib/**/*.rb', 'ext/**/*.{rs,toml,rb}', 'README.md', 'LICENSE', 'CHANGELOG.md', 'CONTRIBUTING.md']
  s.name        = 'maxmind-db-rust'
  s.summary     = 'Unofficial high-performance Rust-based MaxMind DB reader for Ruby'
  s.version     = '0.1.2'

  s.description = 'An unofficial high-performance Rust-based gem for reading MaxMind DB files. ' \
                  'Provides API compatibility with the official maxmind-db gem while leveraging Rust ' \
                  'for superior performance. This library is not endorsed by MaxMind.'
  s.email       = 'oschwald@gmail.com'
  s.homepage    = 'https://github.com/oschwald/maxmind-db-rust-ruby'
  s.licenses    = ['ISC']

  s.metadata = {
    'bug_tracker_uri' => 'https://github.com/oschwald/maxmind-db-rust-ruby/issues',
    'changelog_uri' => 'https://github.com/oschwald/maxmind-db-rust-ruby/blob/main/CHANGELOG.md',
    'documentation_uri' => 'https://www.rubydoc.info/gems/maxmind-db-rust',
    'homepage_uri' => 'https://github.com/oschwald/maxmind-db-rust-ruby',
    'source_code_uri' => 'https://github.com/oschwald/maxmind-db-rust-ruby',
    'rubygems_mfa_required' => 'true'
  }

  s.required_ruby_version = '>= 3.2'
  s.extensions = ['ext/maxmind_db_rust/extconf.rb']

  s.add_dependency 'rb_sys', '~> 0.9'

  s.add_development_dependency 'minitest', '~> 5.0'
  s.add_development_dependency 'rake', '~> 13.0'
  s.add_development_dependency 'rake-compiler', '~> 1.2'
  s.add_development_dependency 'rubocop', '~> 1.0'
  s.add_development_dependency 'rubocop-minitest', '~> 0.36'
  s.add_development_dependency 'rubocop-performance', '~> 1.0'
  s.add_development_dependency 'rubocop-rake', '~> 0.6'
  s.add_development_dependency 'rubocop-thread_safety', '~> 0.6'
end
