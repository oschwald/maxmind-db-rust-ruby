use crate::{
    convert::{try_convert_array, try_convert_hash, try_convert_optional},
    decoder::RubyDecodedValue,
    gvl::without_gvl,
    metadata::Metadata,
    path::{
        parse_path_array, path_array, path_cache_hash, path_elements_from_owned_path,
        path_matches_cached, CachedPath, OwnedPathElement, PATH_CACHE_MAX_ENTRIES,
    },
    rust_module,
};
use ::maxminddb as maxminddb_crate;
use arc_swap::{ArcSwapOption, Guard};
use ipnetwork::IpNetwork;
use magnus::{
    error::{Error, OpaqueError},
    prelude::*,
    rb_sys::{protect, AsRawValue, FromRawValue},
    scan_args::get_kwargs,
    scan_args::scan_args,
    typed_data::Obj,
    value::Opaque,
    Exception, ExceptionClass, IntoValue, RArray, RClass, RHash, RModule, RString, Symbol, Value,
};
use maxminddb_crate::{MaxMindDbError, PathElement, Reader as MaxMindReader, Within};
use memmap2::Mmap;
use std::{
    collections::VecDeque,
    fs::File,
    io::Read as IoRead,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, AtomicU16, Ordering},
        Arc, Mutex,
    },
};

const ERR_CLOSED_DB: &str = "Attempt to read from a closed MaxMind DB.";
const ERR_BAD_DATA: &str =
    "The MaxMind DB file's data section contains bad data (unknown data type or corrupt data)";

/// Enum to handle different reader source types
enum ReaderSource {
    Mmap(MaxMindReader<Mmap>),
    Memory(MaxMindReader<Vec<u8>>),
}

#[derive(Copy, Clone)]
enum OpenMode {
    Mmap,
    Memory,
    Buffer,
}

impl OpenMode {
    fn from_symbol(mode: Symbol, ruby: &magnus::Ruby) -> Result<Self, Error> {
        let mode_name = mode.name()?;
        match mode_name.as_ref() {
            // MODE_FILE is the official gem's file-backed mode; use the
            // existing mmap reader for the same path-backed behavior.
            "MODE_AUTO" | "MODE_FILE" | "MODE_MMAP" => Ok(Self::Mmap),
            "MODE_MEMORY" => Ok(Self::Memory),
            "MODE_PARAM_IS_BUFFER" => Ok(Self::Buffer),
            _ => Err(Error::new(
                ruby.exception_arg_error(),
                format!("Unsupported mode: {}", mode_name),
            )),
        }
    }
}

impl ReaderSource {
    #[inline]
    fn lookup(
        &self,
        ip: IpAddr,
    ) -> Result<Option<RubyDecodedValue>, maxminddb_crate::MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => reader.lookup(ip)?.decode(),
            ReaderSource::Memory(reader) => reader.lookup(ip)?.decode(),
        }
    }

    #[inline]
    fn lookup_prefix(
        &self,
        ip: IpAddr,
    ) -> Result<(Option<RubyDecodedValue>, usize), maxminddb_crate::MaxMindDbError> {
        let (result, prefix_len) = match self {
            ReaderSource::Mmap(reader) => {
                let result = reader.lookup(ip)?;
                let network = result.network()?;
                (result.decode()?, prefix_len_for_ip_network(ip, network))
            }
            ReaderSource::Memory(reader) => {
                let result = reader.lookup(ip)?;
                let network = result.network()?;
                (result.decode()?, prefix_len_for_ip_network(ip, network))
            }
        };
        Ok((result, prefix_len))
    }

    #[inline]
    fn lookup_path(
        &self,
        ip: IpAddr,
        path_elements: &[PathElement<'_>],
    ) -> Result<Option<RubyDecodedValue>, maxminddb_crate::MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => reader.lookup(ip)?.decode_path(path_elements),
            ReaderSource::Memory(reader) => reader.lookup(ip)?.decode_path(path_elements),
        }
    }

    #[inline]
    fn metadata(&self) -> &maxminddb_crate::Metadata {
        match self {
            ReaderSource::Mmap(reader) => reader.metadata(),
            ReaderSource::Memory(reader) => reader.metadata(),
        }
    }

    fn verify(&self) -> Result<(), MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => reader.verify(),
            ReaderSource::Memory(reader) => reader.verify(),
        }
    }

    #[inline]
    fn within(&self, network: IpNetwork) -> Result<ReaderWithin<'_>, MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => Ok(ReaderWithin::Mmap(
                reader.within(network, Default::default())?,
            )),
            ReaderSource::Memory(reader) => Ok(ReaderWithin::Memory(
                reader.within(network, Default::default())?,
            )),
        }
    }
}

/// Wrapper enum for Within iterators
enum ReaderWithin<'reader> {
    Mmap(Within<'reader, Mmap>),
    Memory(Within<'reader, Vec<u8>>),
}

impl ReaderWithin<'_> {
    fn next(&mut self) -> Option<Result<(IpNetwork, RubyDecodedValue), MaxMindDbError>> {
        match self {
            ReaderWithin::Mmap(iter) => next_within_result(iter),
            ReaderWithin::Memory(iter) => next_within_result(iter),
        }
    }
}

self_cell::self_cell!(
    /// A `ReaderWithin` together with the `ReaderSource` it borrows from.
    struct OwnedWithin {
        owner: Arc<ReaderSource>,

        #[covariant]
        dependent: ReaderWithin,
    }
);

/// Iteration state for `Reader#each`, owned by a Ruby object.
///
/// An external enumerator runs `each` in a Fiber. When the caller stops early,
/// Ruby frees the Fiber without unwinding its native stack, so anything Rust
/// owns on that stack is never dropped. Keeping the database handle and the
/// iterator in a Ruby object lets the garbage collector drop them instead.
/// When `each` returns normally, by `break` or by an exception, it releases
/// the state right away so `close` still frees the database at once.
#[magnus::wrap(class = "MaxMind::DB::Rust::Reader::NetworkIterator", free_immediately)]
struct NetworkIterator {
    iter: Mutex<Option<OwnedWithin>>,
}

/// Releases a `NetworkIterator`'s state when `each` leaves its loop.
struct ReleaseOnExit<'a>(&'a NetworkIterator);

impl Drop for ReleaseOnExit<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl NetworkIterator {
    fn new(source: Arc<ReaderSource>, network: IpNetwork) -> Result<Self, MaxMindDbError> {
        let iter = OwnedWithin::try_new(source, |source| source.within(network))?;
        Ok(Self {
            iter: Mutex::new(Some(iter)),
        })
    }

    /// Drops the iterator and its database handle.
    fn release(&self) {
        let iter = match self.iter.lock() {
            Ok(mut iter) => iter.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        drop(iter);
    }

    /// Advances one network. The lock is released before calling into Ruby.
    fn next_entry(
        &self,
        ruby: &magnus::Ruby,
        ipaddr_class: RClass,
    ) -> Result<Option<(Value, Value)>, Error> {
        let result = {
            let mut iter = self.iter.lock().map_err(|_| {
                Error::new(
                    ruby.exception_runtime_error(),
                    "Database iteration failed: iterator lock poisoned",
                )
            })?;
            match iter.as_mut() {
                Some(iter) => iter.with_dependent_mut(|_, iter| iter.next()),
                None => None,
            }
        };

        match result {
            None => Ok(None),
            Some(Ok((network, data))) => {
                let ipaddr = ipaddr_class.funcall::<_, _, Value>("new", (network.to_string(),))?;
                Ok(Some((ipaddr, data.into_value())))
            }
            Some(Err(MaxMindDbError::InvalidDatabase { .. }))
            | Some(Err(MaxMindDbError::Decoding { .. }))
            | Some(Err(MaxMindDbError::ResourceLimit { .. }))
            | Some(Err(MaxMindDbError::Io(_))) => Err(invalid_database_exception(ERR_BAD_DATA)),
            Some(Err(e)) => Err(Error::new(
                ruby.exception_runtime_error(),
                format!("Database iteration failed: {}", e),
            )),
        }
    }
}

#[inline]
// prefix_len_for_ip_network uses 0 as a sentinel for ip.is_ipv4() && network.is_ipv6().
// In this case, 0 is not a real prefix length; it signals an IPv4-in-IPv6 mapping path,
// and callers must treat it specially (distinct from "no network found").
fn prefix_len_for_ip_network(ip: IpAddr, network: IpNetwork) -> usize {
    if ip.is_ipv4() && network.is_ipv6() {
        0
    } else {
        network.prefix() as usize
    }
}

#[inline]
fn next_within_result<S: AsRef<[u8]>>(
    iter: &mut Within<'_, S>,
) -> Option<Result<(IpNetwork, RubyDecodedValue), MaxMindDbError>> {
    loop {
        match iter.next() {
            None => return None,
            Some(Err(e)) => return Some(Err(e)),
            Some(Ok(lookup_result)) => {
                let network = match lookup_result.network() {
                    Ok(n) => n,
                    Err(e) => return Some(Err(e)),
                };
                match lookup_result.decode::<RubyDecodedValue>() {
                    Ok(Some(data)) => return Some(Ok((network, data))),
                    Ok(None) => continue, // Skip networks without data
                    Err(e) => return Some(Err(e)),
                }
            }
        }
    }
}

/// A Ruby wrapper around the MaxMind DB reader
///
/// The class has an allocator and an `initialize` method, like a Ruby class,
/// so `Reader.new` returns an instance of the receiver and subclasses can
/// override `initialize` and call `super`. An allocated reader that was never
/// initialized behaves like a closed reader.
#[magnus::wrap(class = "MaxMind::DB::Rust::Reader")]
struct Reader {
    reader: ArcSwapOption<ReaderSource>,
    // Kept separately from `reader` so `metadata` still works after `close`,
    // as it does in the official gem.
    metadata: ArcSwapOption<Metadata>,
    path_cache: Mutex<VecDeque<CachedPath>>,
    ip_version: AtomicU16,
    // Set by the first `initialize` call before it runs any Ruby code, and
    // cleared again if that call fails.
    initialized: AtomicBool,
}

impl Default for Reader {
    fn default() -> Self {
        Reader {
            reader: ArcSwapOption::empty(),
            metadata: ArcSwapOption::empty(),
            path_cache: Mutex::new(VecDeque::with_capacity(PATH_CACHE_MAX_ENTRIES)),
            ip_version: AtomicU16::new(0),
            initialized: AtomicBool::new(false),
        }
    }
}

impl Reader {
    fn initialize(&self, args: &[Value]) -> Result<(), Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        // The source, metadata and IP version are stored separately, so
        // swapping them on a reader that is in use could let a lookup see a
        // mix of two databases. Only allow initializing a fresh reader. The
        // flag is claimed before any Ruby code runs (`dup`, `to_path`), so a
        // reentrant or concurrent call is rejected too.
        if self
            .initialized
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(Error::new(
                ruby.exception_runtime_error(),
                "MaxMind::DB::Rust::Reader is already initialized",
            ));
        }

        let result = self.open(&ruby, args);
        if result.is_err() {
            self.initialized.store(false, Ordering::Release);
        }
        result
    }

    fn open(&self, ruby: &magnus::Ruby, args: &[Value]) -> Result<(), Error> {
        // Accept the options both as keywords and as a positional Hash, like
        // the official gem's `initialize(database, options = {})`.
        let args = scan_args::<(Value,), (Option<Value>,), (), (), RHash, ()>(args)?;
        let (database,) = args.required;
        let (positional_options,) = args.optional;
        let options = match positional_options {
            Some(_) if !args.keywords.is_empty() => {
                return Err(Error::new(
                    ruby.exception_arg_error(),
                    "wrong number of arguments (given 3, expected 1..2)",
                ));
            }
            // `get_kwargs` removes the keys it reads, so work on a copy rather
            // than emptying the caller's Hash.
            Some(options) => {
                let Some(options) = try_convert_hash(options, ruby)? else {
                    return Err(Error::new(
                        ruby.exception_type_error(),
                        "options must be a Hash",
                    ));
                };
                // Check the result without naming its class (see try_convert_hash).
                let copy = options.funcall::<_, _, Value>("dup", ())?;
                RHash::from_value(copy).ok_or_else(|| {
                    Error::new(ruby.exception_type_error(), "options.dup must be a Hash")
                })?
            }
            None => args.keywords,
        };
        let kw = get_kwargs::<_, (), (Option<Symbol>,), ()>(options, &[], &["mode"])?;
        let (mode,) = kw.optional;

        // Parse mode from options hash
        let mode: Symbol = mode.unwrap_or_else(|| ruby.to_symbol("MODE_AUTO"));

        let open_mode = OpenMode::from_symbol(mode, ruby)?;

        // Open database with appropriate mode
        let source = match open_mode {
            OpenMode::Mmap => open_database_mmap(&database_path(database, ruby)?)?,
            OpenMode::Memory => open_database_memory(&database_path(database, ruby)?)?,
            OpenMode::Buffer => open_database_buffer(database_buffer(database)?)?,
        };

        let meta = source.metadata();
        self.ip_version.store(meta.ip_version, Ordering::Relaxed);
        self.metadata
            .store(Some(Arc::new(Metadata::from_maxmind(meta))));
        self.reader.store(Some(Arc::new(source)));
        Ok(())
    }

    fn initialize_copy(&self, _other: Value) -> Result<(), Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");
        Err(Error::new(
            ruby.exception_type_error(),
            "can't copy MaxMind::DB::Rust::Reader",
        ))
    }

    #[inline]
    fn get(&self, ip_address: Value) -> Result<Value, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let parsed_ip = self.parse_lookup_ip(ip_address, &ruby)?;

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        lookup_result_to_value(&ruby, reader.lookup(parsed_ip), "Database lookup failed")
    }

    #[inline]
    fn get_path(&self, ip_address: Value, path: Value) -> Result<Value, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let parsed_ip = self.parse_lookup_ip(ip_address, &ruby)?;
        let owned_path = self.parse_path(path, &ruby)?;
        let path_elements = path_elements_from_owned_path(owned_path.as_ref());

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        lookup_result_to_value(
            &ruby,
            reader.lookup_path(parsed_ip, &path_elements),
            "Database lookup failed",
        )
    }

    #[inline]
    fn get_with_prefix_length(&self, ip_address: Value) -> Result<RArray, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let parsed_ip = self.parse_lookup_ip(ip_address, &ruby)?;

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        // Perform lookup with prefix
        lookup_prefix_result_to_array(
            &ruby,
            reader.lookup_prefix(parsed_ip),
            "Database lookup failed",
        )
    }

    fn get_many(rb_self: Obj<Self>, ips: Value) -> Result<RArray, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");
        let reader_self = &*rb_self;

        // Each lookup loads the reader itself so no guard is held while
        // calling into Ruby (see NetworkIterator).
        reader_self.get_reader(&ruby)?;

        if let Some(ip_array) = try_convert_array(ips, &ruby)? {
            return ruby.ary_try_from_iter((0..ip_array.len()).map(|index| {
                let ip = ip_array.entry::<Value>(index as isize)?;
                reader_self.lookup_ip_value(&ruby, ip)
            }));
        }

        ensure_enumerable(ips, &ruby, "ips must be an Array or Enumerable")?;
        // The block holds the Reader object, not the database, so a block that
        // each keeps (or a stack that is never unwound) can't keep it open.
        let reader_ref = Opaque::from(rb_self);
        let results = collect_each(&ruby, ips, move |ruby, ip| {
            ruby.get_inner(reader_ref).lookup_ip_value(ruby, ip)
        });
        // The block holds the Reader only as `Opaque`; keep it on this stack.
        gc_guard(&rb_self);
        results
    }

    fn get_many_path(rb_self: Obj<Self>, ips: Value, path: Value) -> Result<RArray, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");
        let reader_self = &*rb_self;

        // Each lookup loads the reader itself so no guard is held while
        // calling into Ruby (see NetworkIterator).
        reader_self.get_reader(&ruby)?;

        let owned_path = reader_self.parse_path(path, &ruby)?;
        let path_elements = path_elements_from_owned_path(owned_path.as_ref());

        if let Some(ip_array) = try_convert_array(ips, &ruby)? {
            return ruby.ary_try_from_iter((0..ip_array.len()).map(|index| {
                let ip = ip_array.entry::<Value>(index as isize)?;
                reader_self.lookup_ip_path_value(&ruby, ip, &path_elements)
            }));
        }

        ensure_enumerable(ips, &ruby, "ips must be an Array or Enumerable")?;
        // See get_many: the block holds the Reader object, not the database.
        let reader_ref = Opaque::from(rb_self);
        let results = collect_each(&ruby, ips, move |ruby, ip| {
            let path_elements = path_elements_from_owned_path(owned_path.as_ref());
            ruby.get_inner(reader_ref)
                .lookup_ip_path_value(ruby, ip, &path_elements)
        });
        gc_guard(&rb_self);
        results
    }

    fn metadata(&self) -> Result<Metadata, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        match self.metadata.load().as_ref() {
            Some(meta) => Ok(Metadata::clone(meta)),
            None => Err(Error::new(ruby.exception_runtime_error(), ERR_CLOSED_DB)),
        }
    }

    fn verify(&self) -> Result<bool, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");
        // Own a reference to the reader so a concurrent `close` cannot free it
        // while verification runs without the GVL.
        let reader = self
            .reader
            .load_full()
            .ok_or_else(|| Error::new(ruby.exception_runtime_error(), ERR_CLOSED_DB))?;

        // Verification only decodes in Rust, so other Ruby threads can run.
        without_gvl(|| reader.verify())?.map_err(|err| {
            invalid_database_exception(&format!("Database verification failed: {err}"))
        })?;
        Ok(true)
    }

    fn close(&self) {
        self.reader.store(None);
    }

    fn closed(&self) -> bool {
        self.reader.load().is_none()
    }

    fn inspect(rb_self: Obj<Self>) -> String {
        let reader_self = &*rb_self;
        format!(
            "#<{}:0x{:x} @closed={} @ip_version={}>",
            // SAFETY: the name is copied before Ruby code can run.
            unsafe { rb_self.classname() },
            reader_self as *const Self as usize,
            reader_self.closed(),
            reader_self.ip_version.load(Ordering::Relaxed),
        )
    }

    fn each(ruby: &magnus::Ruby, rb_self: Obj<Self>, args: &[Value]) -> Result<Value, Error> {
        let reader_self = &*rb_self;

        // Check for a closed reader now, but don't hold it while the network
        // argument below calls into Ruby.
        reader_self.get_reader(ruby)?;

        // Check the arguments before returning an enumerator, so a bad network
        // fails here and not when iteration starts.
        let network = reader_self.parse_each_network(ruby, args)?;

        // If no block given, return enumerator. It gets the parsed network, not
        // the original argument, so iterating does not convert it again.
        if !ruby.block_given() {
            let enumerator = match network {
                Some(network) => rb_self.enumeratorize("each", (network.to_string(),)),
                None => rb_self.enumeratorize("each", ()),
            };
            return Ok(enumerator.as_value());
        }

        let network = network.unwrap_or_else(|| {
            if reader_self.ip_version.load(Ordering::Relaxed) == 4 {
                IpNetwork::V4(ipnetwork::Ipv4Network::new(Ipv4Addr::UNSPECIFIED, 0).unwrap())
            } else {
                IpNetwork::V6(ipnetwork::Ipv6Network::new(Ipv6Addr::UNSPECIFIED, 0).unwrap())
            }
        });

        let iter = {
            let guard = reader_self.get_reader(ruby)?;
            NetworkIterator::new(Arc::clone(guard.as_ref().as_ref().unwrap()), network)
                .map_err(|e| invalid_database_exception(&format!("Failed to iterate: {}", e)))?
        };
        let iter = ruby.obj_wrap(iter);
        // Nothing Rust owns may stay on this stack while yielding (see
        // NetworkIterator). The Ruby object above owns the iteration state.
        let _release = ReleaseOnExit(&iter);

        // Get IPAddr class
        let ipaddr_class = ruby.class_object().const_get::<_, RClass>("IPAddr")?;

        // Iterate over all networks, yielding [network, data] to the block
        while let Some(values) = iter.next_entry(ruby, ipaddr_class)? {
            ruby.yield_values::<(Value, Value), Value>(values)?;
        }

        Ok(ruby.qnil().as_value())
    }

    /// Parses the optional network argument to `each`. Returns `None` when the
    /// network is missing or nil, which means the whole database.
    fn parse_each_network(
        &self,
        ruby: &magnus::Ruby,
        args: &[Value],
    ) -> Result<Option<IpNetwork>, Error> {
        let args = scan_args::<(), (Option<Value>,), (), (), (), ()>(args)?;
        let network_arg = match args.optional.0 {
            Some(value) if !value.is_nil() => value,
            _ => return Ok(None),
        };

        // Accept both String and IPAddr objects
        let network_str = if let Some(s) = try_convert_optional::<RString>(network_arg, ruby)? {
            s.to_string()?
        } else {
            let ipaddr_class = ruby.class_object().const_get::<_, RClass>("IPAddr")?;
            if network_arg.is_kind_of(ipaddr_class) {
                // IPAddr#to_s drops the prefix, so add it back as CIDR notation
                let ip_str: String = network_arg.funcall("to_s", ())?;
                let prefix_len: u8 = network_arg.funcall("prefix", ())?;
                format!("{}/{}", ip_str, prefix_len)
            } else {
                // Try to call to_s on it (works for other objects)
                let to_s_result: Value = network_arg.funcall("to_s", ())?;
                try_convert_optional::<RString>(to_s_result, ruby)?
                    .ok_or_else(|| {
                        Error::new(
                            ruby.exception_arg_error(),
                            "Network parameter must be a String or IPAddr",
                        )
                    })?
                    .to_string()?
            }
        };

        let network = IpNetwork::from_str(&network_str).map_err(|e| {
            Error::new(
                ruby.exception_arg_error(),
                format!("Invalid network CIDR '{}': {}", network_str, e),
            )
        })?;

        // Validate network matches database IP version
        // IPv4 in IPv6 DB is OK (IPv4-mapped), IPv6 in IPv6 DB is OK
        if let (4, IpNetwork::V6(_)) = (self.ip_version.load(Ordering::Relaxed), network) {
            return Err(Error::new(
                ruby.exception_arg_error(),
                format!(
                    "Cannot search for IPv6 network '{}' in an IPv4-only database",
                    network_str
                ),
            ));
        }

        Ok(Some(network))
    }

    /// Helper method to get the reader from the ArcSwapOption
    ///
    /// Parsing arguments can call into Ruby, and no guard may be on the stack
    /// then (see NetworkIterator). So before such a call, methods call this and
    /// drop the guard, which reports a closed reader ahead of a bad argument.
    /// After parsing they call this again for the lookup.
    fn get_reader(&self, ruby: &magnus::Ruby) -> Result<Guard<Option<Arc<ReaderSource>>>, Error> {
        let guard = self.reader.load();
        if guard.is_none() {
            return Err(Error::new(ruby.exception_runtime_error(), ERR_CLOSED_DB));
        }
        Ok(guard)
    }

    #[inline]
    fn parse_lookup_ip(&self, ip_address: Value, ruby: &magnus::Ruby) -> Result<IpAddr, Error> {
        // A plain String is parsed without calling Ruby. Everything else can
        // call Ruby, so check for a closed reader first (see get_reader).
        let parsed_ip = match parse_plain_ip(ip_address) {
            Some(parsed_ip) => parsed_ip,
            None => {
                self.get_reader(ruby)?;
                parse_ip_with_ipaddr(ip_address, ruby)?
            }
        };

        if self.ip_version.load(Ordering::Relaxed) == 4 && parsed_ip.is_ipv6() {
            self.get_reader(ruby)?;
            // Echo the argument as given, like the official gem, rather than
            // the parsed address, which drops an IPv6 zone and normalizes.
            return Err(ipv6_in_ipv4_error(ruby, ip_address));
        }
        Ok(parsed_ip)
    }

    #[inline]
    fn lookup_ip_value(&self, ruby: &magnus::Ruby, ip: Value) -> Result<Value, Error> {
        let parsed_ip = self.parse_lookup_ip(ip, ruby)?;
        let result = {
            let guard = self.get_reader(ruby)?;
            guard.as_ref().as_ref().unwrap().lookup(parsed_ip)
        };
        lookup_result_to_value(ruby, result, "Database lookup failed")
    }

    #[inline]
    fn lookup_ip_path_value(
        &self,
        ruby: &magnus::Ruby,
        ip: Value,
        path_elements: &[PathElement<'_>],
    ) -> Result<Value, Error> {
        let parsed_ip = self.parse_lookup_ip(ip, ruby)?;
        let result = {
            let guard = self.get_reader(ruby)?;
            guard
                .as_ref()
                .as_ref()
                .unwrap()
                .lookup_path(parsed_ip, path_elements)
        };
        lookup_result_to_value(ruby, result, "Database lookup failed")
    }

    fn parse_path(
        &self,
        path: Value,
        ruby: &magnus::Ruby,
    ) -> Result<Arc<[OwnedPathElement]>, Error> {
        // A plain path is hashed without calling Ruby. Anything else can call
        // Ruby or fail, so check for a closed reader first (see get_reader).
        let path = match RArray::from_value(path) {
            Some(path) => path,
            None => {
                self.get_reader(ruby)?;
                path_array(path, ruby)?
            }
        };
        let hash = path_cache_hash(path, ruby, || self.get_reader(ruby).map(drop))?;

        if let Some(cached) = self.cached_path(path, hash, ruby)? {
            return Ok(cached);
        }

        let parsed_path: Arc<[OwnedPathElement]> = parse_path_array(path, ruby)?.into();
        self.store_cached_path(hash, parsed_path.clone());
        Ok(parsed_path)
    }

    fn cached_path(
        &self,
        path: RArray,
        hash: u64,
        ruby: &magnus::Ruby,
    ) -> Result<Option<Arc<[OwnedPathElement]>>, Error> {
        // Comparing a candidate can call Ruby `to_str` or `to_int` methods,
        // which may reenter this reader or release the GVL. Collect the
        // candidates and release the lock before comparing them so that no
        // Ruby code runs while the mutex is held. The first candidate is kept
        // separately so the usual case does not allocate.
        let (first, rest) = {
            let cache = match self.path_cache.lock() {
                Ok(cache) => cache,
                Err(_) => return Ok(None),
            };

            let mut first = None;
            let mut rest = Vec::new();
            for entry in cache
                .iter()
                .filter(|entry| entry.hash == hash && entry.elements.len() == path.len())
            {
                if first.is_none() {
                    first = Some(entry.elements.clone());
                } else {
                    rest.push(entry.elements.clone());
                }
            }
            (first, rest)
        };

        for candidate in first.into_iter().chain(rest) {
            if path_matches_cached(path, candidate.as_ref(), ruby)? {
                return Ok(Some(candidate));
            }
        }

        Ok(None)
    }

    fn store_cached_path(&self, hash: u64, elements: Arc<[OwnedPathElement]>) {
        if let Ok(mut cache) = self.path_cache.lock() {
            if cache
                .iter()
                .any(|entry| entry.hash == hash && entry.elements.as_ref() == elements.as_ref())
            {
                return;
            }

            cache.push_back(CachedPath { hash, elements });
            while cache.len() > PATH_CACHE_MAX_ENTRIES {
                cache.pop_front();
            }
        }
    }
}

// SAFETY: Reader does not store Ruby VALUE handles. The database source is
// owned by ReaderSource and is read-only after construction; close atomically
// swaps the shared source to None. The path cache contains only Rust-owned path
// elements behind a Mutex. All Ruby object access happens inside method calls
// while the Ruby VM is active.
unsafe impl Send for Reader {}

/// Lookup state shared by the block that `collect_each` passes to `each`.
struct BatchState<F> {
    /// Cleared when `collect_each` returns. The block may outlive the call.
    running: AtomicBool,
    /// Taken out while a lookup runs, and dropped when the batch ends so a
    /// kept block doesn't hold the database open.
    lookup: Mutex<Option<F>>,
    /// The first lookup or reentry error, raised once `each` has unwound.
    error: Mutex<Option<OpaqueError>>,
}

const ERR_BATCH_FINISHED: &str =
    "the block that get_many or get_many_path passed to each was called after it returned";
const ERR_BATCH_REENTERED: &str =
    "the block that get_many or get_many_path passed to each was called while already running";

/// Calls `ips.each` with a native block and collects `lookup` of each yielded
/// value.
///
/// Iterating with a block, rather than an external enumerator, lets any
/// exception from `each`, including `StopIteration`, reach the caller instead
/// of looking like the end of the input.
///
/// A lookup error stops the iteration with `throw`, which `each` can't rescue,
/// and is raised after `each` unwinds, so the result can't silently come back
/// short.
///
/// `each` can keep the block and call it after this returns, so the block owns
/// what it uses and only appends to `results` while this call is running.
/// The Ruby values the closures use stay on this frame's stack until then,
/// which keeps them alive for the garbage collector.
fn collect_each<F>(ruby: &magnus::Ruby, ips: Value, lookup: F) -> Result<RArray, Error>
where
    F: FnMut(&magnus::Ruby, Value) -> Result<Value, Error> + Send + 'static,
{
    let results = ruby.ary_new();
    // Keeps a stored lookup exception alive while `each` unwinds.
    let error_values = ruby.ary_new();
    let tag = ruby.class_object().new_instance(())?;
    let state = Arc::new(BatchState {
        running: AtomicBool::new(true),
        lookup: Mutex::new(Some(lookup)),
        error: Mutex::new(None),
    });

    let block_state = Arc::clone(&state);
    let results_ref = Opaque::from(results);
    let error_values_ref = Opaque::from(error_values);
    let tag_ref = Opaque::from(tag);
    let block = ruby.proc_from_fn(move |ruby, args, _block| -> Result<Value, Error> {
        // Read the captures up front; the lookup below can run Ruby code.
        let state = Arc::clone(&block_state);
        let results = ruby.get_inner(results_ref);
        let error_values = ruby.get_inner(error_values_ref);
        let tag = ruby.get_inner(tag_ref);

        if !state.running.load(Ordering::Acquire) {
            return Err(Error::new(
                ruby.exception_runtime_error(),
                ERR_BATCH_FINISHED,
            ));
        }
        // An earlier lookup failed and `each` kept going. Skip the work; the
        // batch already fails.
        if lock_ignoring_poison(&state.error).is_some() {
            return ruby.module_kernel().funcall::<_, _, Value>("throw", (tag,));
        }
        // Stores the error, so `each` can't hide it by rescuing, and stops
        // `each`. The first error wins, except that one such as Interrupt or
        // SystemExit replaces a StandardError.
        let fail = |err: Error| -> Result<Value, Error> {
            // A jump such as `break` or `throw` isn't an exception, and `each`
            // can't rescue it, so let it continue.
            let Some(error_value) = err.value() else {
                return Err(err);
            };
            error_values.push(error_value)?;
            let is_standard = err.is_kind_of(ruby.exception_standard_error());
            let mut stored = lock_ignoring_poison(&state.error);
            if stored.is_none() || !is_standard {
                *stored = Some(err.into());
            }
            drop(stored);
            ruby.module_kernel().funcall::<_, _, Value>("throw", (tag,))
        };
        let Some(mut lookup) = lock_ignoring_poison(&state.lookup).take() else {
            return fail(Error::new(
                ruby.exception_runtime_error(),
                ERR_BATCH_REENTERED,
            ));
        };

        // Match Enumerator#next: no values yield nil, several yield an Array.
        let ip = match args {
            [] => ruby.qnil().as_value(),
            [ip] => *ip,
            ips => ruby.ary_new_from_values(ips).as_value(),
        };
        let result = lookup(ruby, ip);

        // The lookup can run Ruby code, so check again. Nothing between this
        // check and the push below can end the batch.
        if !state.running.load(Ordering::Acquire) {
            // Don't drop an error, which may be a thread kill or Interrupt.
            result?;
            return Err(Error::new(
                ruby.exception_runtime_error(),
                ERR_BATCH_FINISHED,
            ));
        }
        *lock_ignoring_poison(&state.lookup) = Some(lookup);

        match result {
            Ok(value) => {
                results.push(value)?;
                Ok(ruby.qnil().as_value())
            }
            Err(err) => fail(err),
        }
    });

    let ips_ref = Opaque::from(ips);
    let block_ref = Opaque::from(block);
    let body = ruby.proc_from_fn(move |ruby, _args, _block| -> Result<Value, Error> {
        ruby.get_inner(ips_ref).funcall_with_block::<_, _, Value>(
            "each",
            (),
            ruby.get_inner(block_ref),
        )
    });
    let outcome = ruby
        .module_kernel()
        .funcall_with_block::<_, _, Value>("catch", (tag,), body);

    // The closures hold these only as `Opaque`, which the garbage collector
    // doesn't see, so keep them on this frame's stack until `each` is done.
    gc_guard(&results);
    gc_guard(&error_values);
    gc_guard(&tag);
    gc_guard(&block);

    state.running.store(false, Ordering::Release);
    drop(lock_ignoring_poison(&state.lookup).take());
    let lookup_error = lock_ignoring_poison(&state.error).take();

    match (outcome, lookup_error) {
        // A jump such as `break` or `throw` from the input still wins. So does
        // an exception such as Interrupt, SystemExit or Timeout's, because the
        // caller may rescue the lookup error and keep running.
        (Err(err), _) if !err.is_kind_of(ruby.exception_standard_error()) => Err(err),
        // The lookup error wins over a StandardError that `each` raised after
        // it, such as from an `ensure`, or UncaughtThrowError when the block
        // ran on another thread or fiber than the `catch`. An asynchronous
        // StandardError, as from `Timeout.timeout(1, MyError)`, looks the same
        // as one from an `ensure`, so it loses too.
        (_, Some(err)) => Err(OpaqueError::into_error_with(err, ruby)),
        (Err(err), None) => Err(err),
        (Ok(_), None) => Ok(results),
    }
}

/// Keeps `value` in this frame's memory up to this point, like Ruby's
/// `RB_GC_GUARD`, so the garbage collector's stack scan finds it.
#[inline(always)]
fn gc_guard<T: Copy>(value: &T) {
    // SAFETY: `value` is a valid reference to an initialized `T`.
    unsafe {
        std::ptr::read_volatile(value);
    }
}

fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn ensure_enumerable(value: Value, ruby: &magnus::Ruby, error_message: &str) -> Result<(), Error> {
    if value.respond_to("each", false)? {
        Ok(())
    } else {
        Err(Error::new(
            ruby.exception_arg_error(),
            error_message.to_owned(),
        ))
    }
}

/// Parse a plain IPv4 or IPv6 String without calling Ruby. Returns `None` for
/// anything else.
#[inline(always)]
fn parse_plain_ip(value: Value) -> Option<IpAddr> {
    let rstring = RString::from_value(value)?;
    // SAFETY: the &str is only used for parsing before any Ruby code can run
    // and modify the string. An invalidly encoded string is not a plain IP.
    unsafe { rstring.test_as_str() }.and_then(parse_ip_string)
}

/// Parse an IP address that is not a plain String with `IPAddr`
///
/// Anything that is not an `IPAddr` is handed to `IPAddr.new`, as the official
/// gem does, so CIDR strings, scoped IPv6 addresses and the
/// `IPAddr::InvalidAddressError` raised for invalid input all match it.
fn parse_ip_with_ipaddr(value: Value, ruby: &magnus::Ruby) -> Result<IpAddr, Error> {
    if value.is_kind_of(ipaddr_class(ruby)?) {
        return ipaddr_to_ip(value, ruby);
    }

    ipaddr_to_ip(ipaddr_new(value, ruby)?, ruby)
}

fn ipaddr_class(ruby: &magnus::Ruby) -> Result<RClass, Error> {
    ruby.class_object().const_get::<_, RClass>("IPAddr")
}

fn ipaddr_new(value: Value, ruby: &magnus::Ruby) -> Result<Value, Error> {
    ipaddr_class(ruby)?.new_instance((value,))
}

fn ipaddr_to_ip(ipaddr: Value, ruby: &magnus::Ruby) -> Result<IpAddr, Error> {
    let address: u128 = ipaddr.funcall("to_i", ())?;
    let is_ipv4: bool = ipaddr.funcall("ipv4?", ())?;
    if is_ipv4 {
        let address = u32::try_from(address).map_err(|_| {
            interpolated_arg_error(ruby, "'", ipaddr, "' does not appear to be an IPv4 address")
        })?;
        return Ok(IpAddr::V4(Ipv4Addr::from(address)));
    }
    Ok(IpAddr::V6(Ipv6Addr::from(address)))
}

#[inline(always)]
fn parse_ip_string(s: &str) -> Option<IpAddr> {
    if let Some(ip) = parse_ipv4_string(s.as_bytes()) {
        return Some(IpAddr::V4(ip));
    }

    IpAddr::from_str(s).ok()
}

#[inline(always)]
fn parse_ipv4_string(bytes: &[u8]) -> Option<Ipv4Addr> {
    let mut octets = [0u8; 4];
    let mut octet_index = 0;
    let mut value: u16 = 0;
    let mut digits = 0;

    for &byte in bytes {
        if byte == b'.' {
            if digits == 0 || octet_index == 3 {
                return None;
            }
            octets[octet_index] = value as u8;
            octet_index += 1;
            value = 0;
            digits = 0;
            continue;
        }

        if !byte.is_ascii_digit() {
            return None;
        }
        if digits == 1 && value == 0 {
            return None;
        }

        digits += 1;
        if digits > 3 {
            return None;
        }
        value = value * 10 + u16::from(byte - b'0');
        if value > u16::from(u8::MAX) {
            return None;
        }
    }

    if octet_index != 3 || digits == 0 {
        return None;
    }
    octets[octet_index] = value as u8;

    Some(Ipv4Addr::from(octets))
}

#[inline]
fn lookup_result_to_value(
    ruby: &magnus::Ruby,
    result: Result<Option<RubyDecodedValue>, MaxMindDbError>,
    error_context: &str,
) -> Result<Value, Error> {
    match result {
        Ok(Some(data)) => Ok(data.into_value()),
        Ok(None) => Ok(ruby.qnil().as_value()),
        Err(err) => Err(lookup_error(ruby, err, error_context)),
    }
}

#[inline]
fn lookup_prefix_result_to_array(
    ruby: &magnus::Ruby,
    result: Result<(Option<RubyDecodedValue>, usize), MaxMindDbError>,
    error_context: &str,
) -> Result<RArray, Error> {
    match result {
        Ok((data, prefix)) => {
            let arr = ruby.ary_new();
            arr.push(data.map_or_else(|| ruby.qnil().as_value(), RubyDecodedValue::into_value))?;
            arr.push(prefix.into_value_with(ruby))?;
            Ok(arr)
        }
        Err(err) => Err(lookup_error(ruby, err, error_context)),
    }
}

#[inline]
fn lookup_error(ruby: &magnus::Ruby, err: MaxMindDbError, context: &str) -> Error {
    match err {
        MaxMindDbError::InvalidDatabase { .. }
        | MaxMindDbError::Decoding { .. }
        | MaxMindDbError::ResourceLimit { .. }
        | MaxMindDbError::Io(_) => invalid_database_exception(ERR_BAD_DATA),
        other => Error::new(
            ruby.exception_runtime_error(),
            format!("{}: {}", context, other),
        ),
    }
}

/// The error for an IPv6 address in an IPv4-only database.
fn ipv6_in_ipv4_error(ruby: &magnus::Ruby, ip: Value) -> Error {
    interpolated_arg_error(
        ruby,
        "Error looking up ",
        ip,
        ". You attempted to look up an IPv6 address in an IPv4-only database.",
    )
}

/// An `ArgumentError` whose message is `"#{prefix}#{value}#{suffix}"`.
///
/// The message is built the way Ruby string interpolation builds it: a String
/// is used as is and anything else goes through `to_s`. So, as in the
/// `maxmind-db` gem, an exception or `throw` from `to_s`, or an
/// `Encoding::CompatibilityError` from joining the text, is what the caller
/// gets. Formatting the `Value` with `{}` would instead ignore those and show
/// `#<Class:0x...>`.
fn interpolated_arg_error(ruby: &magnus::Ruby, prefix: &str, value: Value, suffix: &str) -> Error {
    let build = || -> Result<Exception, Error> {
        let message = ruby.str_new(prefix);
        message.buf_append(value.to_r_string()?)?;
        message.cat(suffix);
        ruby.exception_arg_error().new_instance((message,))
    };
    match build() {
        Ok(exception) => exception.into(),
        Err(err) => err,
    }
}

/// Convert a database path argument the way Ruby's `File.open` does, including
/// `to_path`, the encoding and the check for null bytes.
fn database_path(database: Value, ruby: &magnus::Ruby) -> Result<PathBuf, Error> {
    // SAFETY: `rb_get_path` is called with a live VALUE and returns a String.
    // `protect` catches anything it raises or throws.
    let path = unsafe { Value::from_raw(protect(|| rb_sys::rb_get_path(database.as_raw()))?) };
    path_from_string(RString::try_convert(path)?, ruby)
}

#[cfg(unix)]
fn path_from_string(path: RString, _ruby: &magnus::Ruby) -> Result<PathBuf, Error> {
    use std::os::unix::ffi::OsStringExt;

    // macOS file names are UTF-8, and Ruby converts the path before it opens
    // the file.
    #[cfg(target_os = "macos")]
    let path = path.conv_enc(_ruby.utf8_encoding())?;

    // SAFETY: the bytes are copied before Ruby can mutate or free the string.
    let bytes = unsafe { path.as_slice() }.to_vec();
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

#[cfg(not(unix))]
fn path_from_string(path: RString, _ruby: &magnus::Ruby) -> Result<PathBuf, Error> {
    Ok(PathBuf::from(path.to_string()?))
}

fn database_buffer(database: Value) -> Result<Vec<u8>, Error> {
    let string = RString::try_convert(database)?;
    // SAFETY: the slice is copied into an owned Vec before Ruby can mutate or
    // free the string, and the reader only ever sees the owned bytes.
    Ok(unsafe { string.as_slice() }.to_vec())
}

/// Open a MaxMind DB using memory-mapped I/O (MODE_MMAP)
fn open_database_mmap(path: &Path) -> Result<ReaderSource, Error> {
    let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby context");
    let file = open_database_file(path, &ruby)?;

    // SAFETY: the mapping is read-only, and MODE_MMAP's documented contract
    // requires callers not to modify or truncate the mapped file while this
    // reader is alive. Database updates should atomically replace the path.
    let mmap = unsafe { Mmap::map(&file) }.map_err(|e| io_error(&ruby, &e, path))?;
    let reader = MaxMindReader::from_source(mmap).map_err(|_| {
        invalid_database_exception(&format!(
            "Error opening database file ({}). Is this a valid MaxMind DB file?",
            path.display()
        ))
    })?;

    Ok(ReaderSource::Mmap(reader))
}

/// Open a MaxMind DB by loading entire file into memory (MODE_MEMORY)
fn open_database_memory(path: &Path) -> Result<ReaderSource, Error> {
    let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby context");

    // Opening and reading the file can block for a long time, so do it
    // without the GVL and build any Ruby exception afterwards.
    let buffer = without_gvl(|| {
        let mut file = File::open(path).map_err(MemoryReadError::Io)?;
        // Opening a directory succeeds on Unix; raise Errno::EISDIR up front,
        // as `open_database_file` does.
        if file.metadata().map_err(MemoryReadError::Io)?.is_dir() {
            return Err(MemoryReadError::IsDirectory);
        }
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).map_err(MemoryReadError::Io)?;
        Ok(buffer)
    })?
    .map_err(|err| match err {
        MemoryReadError::Io(e) => io_error(&ruby, &e, path),
        MemoryReadError::IsDirectory => errno_error(&ruby, "EISDIR", path),
    })?;

    reader_from_buffer(
        buffer,
        format!(
            "Error opening database file ({}). Is this a valid MaxMind DB file?",
            path.display()
        ),
    )
}

fn open_database_buffer(buffer: Vec<u8>) -> Result<ReaderSource, Error> {
    reader_from_buffer(
        buffer,
        "Error opening database from buffer. Is this a valid MaxMind DB file?".to_owned(),
    )
}

fn reader_from_buffer(buffer: Vec<u8>, invalid_message: String) -> Result<ReaderSource, Error> {
    let reader = MaxMindReader::from_source(buffer)
        .map_err(|_| invalid_database_exception(invalid_message.as_str()))?;

    Ok(ReaderSource::Memory(reader))
}

/// Errors from the GVL-free part of `open_database_memory`, converted to Ruby
/// exceptions once the GVL is held again.
enum MemoryReadError {
    Io(std::io::Error),
    IsDirectory,
}

fn open_database_file(path: &Path, ruby: &magnus::Ruby) -> Result<File, Error> {
    let file = File::open(path).map_err(|e| io_error(ruby, &e, path))?;
    // Opening a directory succeeds on Unix, and the error only shows up later
    // as an unhelpful mmap or read failure. Raise Errno::EISDIR up front, as
    // the official gem does.
    let is_dir = file
        .metadata()
        .map_err(|e| io_error(ruby, &e, path))?
        .is_dir();
    if is_dir {
        return Err(errno_error(ruby, "EISDIR", path));
    }
    Ok(file)
}

/// Convert an I/O error into the matching `Errno` exception, so callers that
/// rescue `SystemCallError` see the same classes as with the official gem.
/// Errors without an OS error code fall back to `IOError`.
fn io_error(ruby: &magnus::Ruby, err: &std::io::Error, path: &Path) -> Error {
    #[cfg(unix)]
    if let Some(code) = err.raw_os_error() {
        return system_call_error(ruby, code, path);
    }

    let errno_name = match err.kind() {
        std::io::ErrorKind::NotFound => Some("ENOENT"),
        std::io::ErrorKind::PermissionDenied => Some("EACCES"),
        std::io::ErrorKind::IsADirectory => Some("EISDIR"),
        _ => None,
    };
    match errno_name {
        Some(name) => errno_error(ruby, name, path),
        None => Error::new(
            ruby.exception_io_error(),
            format!("{} - {}", err, path.display()),
        ),
    }
}

/// `SystemCallError.new(path, code)`, which Ruby turns into the `Errno`
/// subclass for that code.
#[cfg(unix)]
fn system_call_error(ruby: &magnus::Ruby, code: i32, path: &Path) -> Error {
    let exception = ruby
        .exception_system_call_error()
        .new_instance((path.display().to_string(), code));
    match exception {
        Ok(exception) => Error::from(exception),
        Err(error) => error,
    }
}

fn errno_error(ruby: &magnus::Ruby, name: &str, path: &Path) -> Error {
    let class = ruby
        .class_object()
        .const_get::<_, RModule>("Errno")
        .and_then(|errno| errno.const_get::<_, RClass>(name));
    let exception = class.and_then(|class| class.new_instance((path.display().to_string(),)));
    match exception.map(Exception::from_value) {
        Ok(Some(exception)) => Error::from(exception),
        Ok(None) => Error::new(ruby.exception_io_error(), path.display().to_string()),
        Err(error) => error,
    }
}

/// Get the InvalidDatabaseError class
fn invalid_database_error() -> RClass {
    let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby context");
    let rust = rust_module(&ruby);
    rust.const_get::<_, RClass>("InvalidDatabaseError")
        .expect("InvalidDatabaseError class should exist")
}

fn invalid_database_exception(message: &str) -> Error {
    Error::new(
        ExceptionClass::from_value(invalid_database_error().as_value())
            .expect("InvalidDatabaseError should convert to ExceptionClass"),
        message.to_owned(),
    )
}

pub(crate) fn define(ruby: &magnus::Ruby, rust: RModule) -> Result<(), Error> {
    let runtime_error = ruby.exception_runtime_error();
    rust.define_error("InvalidDatabaseError", runtime_error)?;

    let reader_class = rust.define_class("Reader", ruby.class_object())?;
    reader_class.define_alloc_func::<Reader>();
    reader_class.define_method("initialize", magnus::method!(Reader::initialize, -1))?;
    reader_class.define_method(
        "initialize_copy",
        magnus::method!(Reader::initialize_copy, 1),
    )?;
    reader_class.define_method("get", magnus::method!(Reader::get, 1))?;
    reader_class.define_method("get_path", magnus::method!(Reader::get_path, 2))?;
    reader_class.define_method(
        "get_with_prefix_length",
        magnus::method!(Reader::get_with_prefix_length, 1),
    )?;
    reader_class.define_method("get_many", magnus::method!(Reader::get_many, 1))?;
    reader_class.define_method("get_many_path", magnus::method!(Reader::get_many_path, 2))?;
    reader_class.define_method("metadata", magnus::method!(Reader::metadata, 0))?;
    reader_class.define_method("verify", magnus::method!(Reader::verify, 0))?;
    reader_class.define_method("close", magnus::method!(Reader::close, 0))?;
    reader_class.define_method("closed", magnus::method!(Reader::closed, 0))?;
    reader_class.define_method("inspect", magnus::method!(Reader::inspect, 0))?;
    reader_class.define_method("each", magnus::method!(Reader::each, -1))?;

    // Internal holder for Reader#each state; not meant to be used directly.
    let iterator_class = reader_class.define_class("NetworkIterator", ruby.class_object())?;
    iterator_class.undef_default_alloc_func();

    let enumerable = ruby.class_object().const_get::<_, RModule>("Enumerable")?;
    reader_class.include_module(enumerable)?;

    rust.const_set("MODE_AUTO", ruby.to_symbol("MODE_AUTO"))?;
    rust.const_set("MODE_FILE", ruby.to_symbol("MODE_FILE"))?;
    rust.const_set("MODE_MEMORY", ruby.to_symbol("MODE_MEMORY"))?;
    rust.const_set("MODE_MMAP", ruby.to_symbol("MODE_MMAP"))?;
    rust.const_set(
        "MODE_PARAM_IS_BUFFER",
        ruby.to_symbol("MODE_PARAM_IS_BUFFER"),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_ipv4_string;
    use std::net::Ipv4Addr;

    #[test]
    fn parses_strict_ipv4_strings() {
        assert_eq!(
            parse_ipv4_string(b"0.1.2.255"),
            Some(Ipv4Addr::new(0, 1, 2, 255))
        );
        assert_eq!(
            parse_ipv4_string(b"192.0.2.1"),
            Some(Ipv4Addr::new(192, 0, 2, 1))
        );
    }

    #[test]
    fn rejects_ipv4_strings_that_std_parser_rejects() {
        for value in [
            b"01.2.3.4".as_slice(),
            b"1.02.3.4".as_slice(),
            b"1.2.3.04".as_slice(),
            b"1.2.3".as_slice(),
            b"1.2.3.4.5".as_slice(),
            b"1..2.3".as_slice(),
            b"256.1.1.1".as_slice(),
            b"1.2.3.4 ".as_slice(),
            b" 1.2.3.4".as_slice(),
            b"2001:db8::1".as_slice(),
        ] {
            assert_eq!(parse_ipv4_string(value), None);
        }
    }
}
