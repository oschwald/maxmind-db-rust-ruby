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

- Do not drop an `Err` from a call that can run Ruby code. It may hold a
  pending `throw` or thread kill, and calling Ruby again before returning it
  crashes the interpreter. Use `try_convert_optional` when an argument may be
  one of several types, or `try_convert_array` and `try_convert_hash`.
- Do not keep a Ruby handle, including one inside a `magnus::Error`, in Rust heap
  memory while Ruby code can run, unless a stack frame or a `mark` function pins
  the object. Compaction can move it. Keep an exception in a Ruby array instead.
- A block can outlive the call that created it, and `define_method` can copy it.
  magnus keeps the closure of `proc_from_fn` alive with an instance variable that
  a copy does not have. Use `rb_proc_new` with a Ruby object as its data, as
  `collect_each` does. Do not use `proc_from_fn` at all with magnus 0.9.3: it
  frees a closure's data type before Ruby is done with it. A block also keeps the
  receiver of the method that created it alive, so create it in a method of that
  object.

## Reader State

`Reader` stores:

- an atomically swapped `Arc<ReaderSource>` for the database source,
- a mutex-protected parsed-path cache containing only Rust-owned strings and
  indexes,
- an atomically swapped `Arc<Metadata>` with a Rust-owned copy of the database
  metadata,
- the database IP version copied from metadata, in an atomic integer.

`Reader` does not store Ruby handles. This is the core invariant behind its
`Send` implementation.

`Reader` has an allocator that creates an empty reader with no source, and
`initialize` opens the database and stores the source, metadata and IP version.
This lets Ruby subclasses override `initialize` and call `super`. An allocated
reader that was never initialized is in the closed state. `dup` and `clone`
raise `TypeError` rather than producing an empty copy.

## Metadata

`Metadata` stores only Rust-owned copies of database metadata. It does not
borrow from the database source and does not store Ruby handles. Methods create
Ruby objects on demand when Ruby calls them.

## Close Semantics

`Reader#close` atomically swaps the shared source to `None`, which is also the
reader's closed-state representation. Methods load the source through
`get_reader`; if it is gone, they raise the closed-reader runtime error. A
method that already loaded the source may finish using that `Arc`, while later
method calls see the closed state. `get_many` and `get_many_path` load the source
for each address, so they raise the closed-reader error if the reader is closed
before the last lookup. If it is closed after that, they return the results.
`each` keeps its source in a Ruby object and continues after `close`.

Ruby can abandon a Fiber's stack without unwinding it, so anything Rust owns there
leaks. Lookups and `each` must not have a database source on a stack that calls
Ruby. `each` and batch lookups keep their state in Ruby objects
(`NetworkIterator`, `Batch` and `BatchPath` in `reader.rs`).

Some leaks are known. Parsing an argument can leak its own small buffers. `verify`
and opening in `MODE_MEMORY` can leak the source if a trap handler suspends the
Fiber after they release the GVL, and the Fiber is then abandoned.

`close` leaves the metadata in place, so `Reader#metadata` keeps working after
`close`, as in the official gem.

## Releasing the GVL

`Reader#verify` and the file open and read in `MODE_MEMORY` run through
`gvl::without_gvl`, which calls `rb_thread_call_without_gvl`. Code inside it:

- must not touch Ruby objects or call the Ruby API, including creating
  exceptions. Return Rust values and errors, and convert them after the call.
- must own what it uses, such as a cloned `Arc<ReaderSource>` or an owned
  `PathBuf`, so a concurrent `close` or GC cannot free it.

Panics are caught before they reach the C frame and resumed once the GVL is
held again. No unblocking function is registered, so the work always runs to
the end; pending interrupts such as `Thread#raise` are raised afterwards
through `protect`.

## Memory-Mapped Files

`MODE_AUTO`, `MODE_FILE`, and `MODE_MMAP` create a read-only file-backed memory
map. The underlying file must not be modified or truncated while its reader is
alive. Database refreshes should write a new file and atomically replace the
path, leaving the inode used by existing readers unchanged. Use `MODE_MEMORY`
when that lifecycle cannot be guaranteed.

## Caches

All decoded MMDB strings are frozen. Strings within the configured length range
may also be retained in a fixed-size Ruby array owned by `MaxMind::DB::Rust`.
That array is both the direct-mapped cache and the GC root for its entries.
Cache access happens under Ruby's GVL, and thread-local state only memoizes the
handle to the globally rooted array.

Ruby cannot see or update handles held in Rust thread-locals or other state
outside Ruby objects, so a constant alone is not enough to root such an object:
GC compaction may move it and leave the Rust handle stale. Any Ruby object whose
handle is kept outside Ruby objects must be pinned. The string cache array is
registered with `Ruby::gc_register_mark_object`, which marks it permanently
and keeps it from moving.

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

- No Ruby handle is stored without a matching mark/root strategy, and any
  handle kept outside Ruby objects points at a pinned object.
- Ruby string borrows are not kept across Ruby calls or stored in structs.
- Shared state is immutable, atomic, or protected by a lock.
- Errors from corrupt database data are mapped to `InvalidDatabaseError`.
- Reader close behavior remains idempotent and concurrent lookups keep a valid
  owned source while they run.
