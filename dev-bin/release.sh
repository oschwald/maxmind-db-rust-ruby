#!/bin/bash

set -eu -o pipefail

changelog=$(cat CHANGELOG.md)

regex='
## \[([0-9]+\.[0-9]+\.[0-9]+)\] - ([0-9]{4}-[0-9]{2}-[0-9]{2})

((.|
)*)
'

if [[ ! $changelog =~ $regex ]]; then
      echo "Could not find date line in change log!"
      exit 1
fi

version="${BASH_REMATCH[1]}"
date="${BASH_REMATCH[2]}"
notes="$(echo "${BASH_REMATCH[3]}" | sed -n -E '/^## \[[0-9]+\.[0-9]+\.[0-9]+\]/,$!p')"

echo "$notes"
if [[ "$date" != "$(date +"%Y-%m-%d")" ]]; then
    echo "$date is not today!"
    exit 1
fi

tag="v$version"

if [ -n "$(git status --porcelain)" ]; then
    echo ". is not clean." >&2
    exit 1
fi

# Update version in gemspec
perl -pi -e "s/(?<=s\.version\s{,20}=\s{,20}\').+?(?=\')/$version/g" maxmind-db-rust.gemspec

echo $"Test results:"

# Run tests and lints
bundle exec rake test
bundle exec rubocop
cd ext/maxmind_db_rust
cargo clippy -- -D warnings
cargo fmt --check
cd ../..

echo $'\nDiff:'
git diff

echo $'\nRelease notes:'
echo "$notes"

read -e -p "Commit changes and push to origin? " should_push

if [ "$should_push" != "y" ]; then
    echo "Aborting"
    exit 1
fi

git commit -m "Update for $tag" -a

git push

gh release create --target "$(git branch --show-current)" -t "$version" -n "$notes" "$tag"
