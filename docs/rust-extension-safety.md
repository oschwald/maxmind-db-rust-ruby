# Rust Extension Safety Notes

This extension is Ruby-facing Rust code. The important safety boundary is that
Ruby objects must stay under Ruby GC ownership, while long-lived reader state
must be Rust-owned and thread-safe.

## Ruby Values

- Do not store `magnus::Value`, `RString`, `RArray`, `RHash`, or other Ruby
  handles in `Reader`, `Metadata`, caches, or other long-lived Rust structs.
- Ruby values may be borrowed only inside an active Ruby method call and only
  for immediate work.
- Any `RString::as_str`, `RString::test_as_str`, or `RString::as_slice` borrow
  must be used immediately. Do not call back into Ruby, store the borrow, or let
  it outlive the Ruby object it references.
- If bytes or strings are needed after the current expression, copy them into
  owned Rust values first.

## Reader State

`Reader` stores:

- an atomically swapped `Arc<ReaderSource>` for the database source,
- an atomic closed flag,
- a mutex-protected parsed-path cache containing only Rust-owned strings and
  indexes,
- the database IP version copied from metadata.

`Reader` does not store Ruby handles. This is the core invariant behind its
`Send` implementation.

## Metadata

`Metadata` stores only Rust-owned copies of database metadata. It does not
borrow from the database source and does not store Ruby handles. Methods create
Ruby objects on demand when Ruby calls them.

## Close Semantics

`Reader#close` atomically marks the reader closed and swaps the shared source to
`None`. Methods load the source through `get_reader`; if it is gone, they raise
the closed-reader runtime error. A method that already loaded the source may
finish using that `Arc`, while later method calls see the closed state.

## Caches

The string cache is a fixed-size Ruby array owned by `MaxMind::DB::Rust`. It is
both the direct-mapped cache and the GC root for its frozen Ruby strings. Cache
access happens under Ruby's GVL, and thread-local state only memoizes the handle
to the globally rooted array.

MMDB UTF-8 strings and map keys are decoded as borrowed bytes and copied into
UTF-8-tagged Ruby strings without first constructing Rust `str` values. Valid
databases contain valid UTF-8. For corrupt databases, Ruby may safely retain an
invalidly encoded string, matching the official Ruby reader's behavior without
violating Rust string invariants.

The parsed-path cache is per reader and stores only Rust-owned path elements. It
is keyed by path contents, not Ruby array identity, so mutable path arrays remain
correct after mutation and Ruby object ID reuse cannot produce stale lookups.

## Review Checklist

Before adding unsafe code or long-lived state, verify:

- No Ruby handle is stored without a matching mark/root strategy.
- Ruby string borrows are not kept across Ruby calls or stored in structs.
- Shared state is immutable, atomic, or protected by a lock.
- Errors from corrupt database data are mapped to `InvalidDatabaseError`.
- Reader close behavior remains idempotent and concurrent lookups keep a valid
  owned source while they run.
