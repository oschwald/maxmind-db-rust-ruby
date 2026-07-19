use crate::{
    decoder::RubyDecodedValue,
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
    error::Error, prelude::*, scan_args::get_kwargs, scan_args::scan_args, typed_data::Obj,
    ExceptionClass, IntoValue, RArray, RClass, RModule, RString, Symbol, Value,
};
use maxminddb_crate::{MaxMindDbError, PathElement, Reader as MaxMindReader, Within};
use memmap2::Mmap;
use std::{
    collections::VecDeque,
    fs::File,
    io::Read as IoRead,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex},
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
#[magnus::wrap(class = "MaxMind::DB::Rust::Reader")]
struct Reader {
    reader: ArcSwapOption<ReaderSource>,
    path_cache: Mutex<VecDeque<CachedPath>>,
    ip_version: u16,
}

impl Reader {
    fn new(args: &[Value]) -> Result<Self, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let args = scan_args::<(Value,), (), (), (), _, ()>(args)?;
        let (database,) = args.required;
        let kw = get_kwargs::<_, (), (Option<Symbol>,), ()>(args.keywords, &[], &["mode"])?;
        let (mode,) = kw.optional;

        // Parse mode from options hash
        let mode: Symbol = mode.unwrap_or_else(|| ruby.to_symbol("MODE_AUTO"));

        let open_mode = OpenMode::from_symbol(mode, &ruby)?;

        // Open database with appropriate mode
        match open_mode {
            OpenMode::Mmap => open_database_mmap(&database_path(database)?),
            OpenMode::Memory => open_database_memory(&database_path(database)?),
            OpenMode::Buffer => open_database_buffer(database_buffer(database)?),
        }
    }

    #[inline]
    fn get(&self, ip_address: Value) -> Result<Value, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        let parsed_ip = self.parse_lookup_ip(ip_address, &ruby)?;

        lookup_result_to_value(&ruby, reader.lookup(parsed_ip), "Database lookup failed")
    }

    #[inline]
    fn get_path(&self, ip_address: Value, path: Value) -> Result<Value, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        let parsed_ip = self.parse_lookup_ip(ip_address, &ruby)?;
        let owned_path = self.parse_path(path, &ruby)?;
        let path_elements = path_elements_from_owned_path(owned_path.as_ref());

        lookup_result_to_value(
            &ruby,
            reader.lookup_path(parsed_ip, &path_elements),
            "Database lookup failed",
        )
    }

    #[inline]
    fn get_with_prefix_length(&self, ip_address: Value) -> Result<RArray, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        let parsed_ip = self.parse_lookup_ip(ip_address, &ruby)?;

        // Perform lookup with prefix
        lookup_prefix_result_to_array(
            &ruby,
            reader.lookup_prefix(parsed_ip),
            "Database lookup failed",
        )
    }

    fn get_many(&self, ips: Value) -> Result<RArray, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        if let Ok(ip_array) = RArray::try_convert(ips) {
            return ruby.ary_try_from_iter((0..ip_array.len()).map(|index| {
                let ip = ip_array.entry::<Value>(index as isize)?;
                self.lookup_ip_value(&ruby, reader, ip)
            }));
        }

        ensure_enumerable(ips, &ruby, "ips must be an Array or Enumerable")?;
        ruby.ary_try_from_iter(
            ips.enumeratorize("each", ())
                .map(|ip| self.lookup_ip_value(&ruby, reader, ip?)),
        )
    }

    fn get_many_path(&self, ips: Value, path: Value) -> Result<RArray, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        let owned_path = self.parse_path(path, &ruby)?;
        let path_elements = path_elements_from_owned_path(owned_path.as_ref());

        if let Ok(ip_array) = RArray::try_convert(ips) {
            return ruby.ary_try_from_iter((0..ip_array.len()).map(|index| {
                let ip = ip_array.entry::<Value>(index as isize)?;
                self.lookup_ip_path_value(&ruby, reader, ip, &path_elements)
            }));
        }

        ensure_enumerable(ips, &ruby, "ips must be an Array or Enumerable")?;
        ruby.ary_try_from_iter(
            ips.enumeratorize("each", ())
                .map(|ip| self.lookup_ip_path_value(&ruby, reader, ip?, &path_elements)),
        )
    }

    fn metadata(&self) -> Result<Metadata, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();
        let meta = reader.metadata();

        Ok(Metadata::from_maxmind(meta))
    }

    fn verify(&self) -> Result<bool, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");
        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        reader.verify().map_err(|err| {
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

    fn inspect(&self) -> String {
        format!(
            "#<MaxMind::DB::Rust::Reader:0x{:x} @closed={} @ip_version={}>",
            self as *const Self as usize,
            self.closed(),
            self.ip_version,
        )
    }

    fn each(ruby: &magnus::Ruby, rb_self: Obj<Self>, args: &[Value]) -> Result<Value, Error> {
        let reader_self = &*rb_self;

        let guard = reader_self.get_reader(ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        // If no block given, return enumerator
        if !ruby.block_given() {
            return Ok(rb_self.enumeratorize("each", args).as_value());
        }

        let ip_version = reader.metadata().ip_version;

        // Determine the network to iterate over
        let network_str = if args.is_empty() {
            // No argument: use default (full database)
            if ip_version == 4 {
                "0.0.0.0/0".to_string()
            } else {
                "::/0".to_string()
            }
        } else {
            // Argument provided: extract network CIDR string
            let network_arg = args[0];

            // Try to get string representation
            // Accept both String and IPAddr objects
            let network_str_val = if let Ok(s) = RString::try_convert(network_arg) {
                // It's already a string
                s.to_string()?
            } else {
                // Check if it's an IPAddr object
                let ipaddr_class = ruby.class_object().const_get::<_, RClass>("IPAddr")?;
                if network_arg.is_kind_of(ipaddr_class) {
                    // It's an IPAddr - need to get both address and prefix
                    let ip_str: String = network_arg.funcall("to_s", ())?;

                    // Get the prefix length from IPAddr
                    // IPAddr stores prefix as a netmask, need to convert
                    let prefix_len: u8 = network_arg.funcall("prefix", ())?;

                    // Construct CIDR notation
                    format!("{}/{}", ip_str, prefix_len)
                } else {
                    // Try to call to_s on it (works for other objects)
                    let to_s_result: Value = network_arg.funcall("to_s", ())?;
                    RString::try_convert(to_s_result)
                        .map_err(|_| {
                            Error::new(
                                ruby.exception_arg_error(),
                                "Network parameter must be a String or IPAddr",
                            )
                        })?
                        .to_string()?
                }
            };

            network_str_val
        };

        let network = IpNetwork::from_str(&network_str).map_err(|e| {
            Error::new(
                ruby.exception_arg_error(),
                format!("Invalid network CIDR '{}': {}", network_str, e),
            )
        })?;

        // Validate network matches database IP version
        // IPv4 in IPv6 DB is OK (IPv4-mapped), IPv6 in IPv6 DB is OK
        if let (4, IpNetwork::V6(_)) = (ip_version, network) {
            return Err(Error::new(
                ruby.exception_arg_error(),
                format!(
                    "Cannot search for IPv6 network '{}' in an IPv4-only database",
                    network_str
                ),
            ));
        }

        let mut iter = reader
            .within(network)
            .map_err(|e| invalid_database_exception(&format!("Failed to iterate: {}", e)))?;
        // Get IPAddr class
        let ipaddr_class = ruby.class_object().const_get::<_, RClass>("IPAddr")?;

        // Iterate over all networks
        while let Some(result) = iter.next() {
            match result {
                Ok((network, data)) => {
                    // Convert IpNetwork to IPAddr
                    let ip_str = network.to_string();
                    let ipaddr = ipaddr_class.funcall::<_, _, Value>("new", (ip_str,))?;

                    // Yield [network, data] to block
                    let values = (ipaddr, data.into_value());
                    ruby.yield_values::<(Value, Value), Value>(values)?;
                }
                Err(MaxMindDbError::InvalidDatabase { .. })
                | Err(MaxMindDbError::Decoding { .. })
                | Err(MaxMindDbError::Io(_)) => {
                    return Err(invalid_database_exception(ERR_BAD_DATA));
                }
                Err(e) => {
                    return Err(Error::new(
                        ruby.exception_runtime_error(),
                        format!("Database iteration failed: {}", e),
                    ));
                }
            }
        }

        Ok(ruby.qnil().as_value())
    }

    /// Helper method to get the reader from the ArcSwapOption
    fn get_reader(&self, ruby: &magnus::Ruby) -> Result<Guard<Option<Arc<ReaderSource>>>, Error> {
        let guard = self.reader.load();
        if guard.is_none() {
            return Err(Error::new(ruby.exception_runtime_error(), ERR_CLOSED_DB));
        }
        Ok(guard)
    }

    #[inline]
    fn parse_lookup_ip(&self, ip_address: Value, ruby: &magnus::Ruby) -> Result<IpAddr, Error> {
        let parsed_ip = parse_ip_address_fast(ip_address, ruby)?;
        self.validate_lookup_ip(parsed_ip, ruby)
    }

    #[inline]
    fn validate_lookup_ip(&self, parsed_ip: IpAddr, ruby: &magnus::Ruby) -> Result<IpAddr, Error> {
        if self.ip_version == 4 && matches!(parsed_ip, IpAddr::V6(_)) {
            Err(Error::new(
                ruby.exception_arg_error(),
                ipv6_in_ipv4_error(&parsed_ip),
            ))
        } else {
            Ok(parsed_ip)
        }
    }

    #[inline]
    fn lookup_ip_value(
        &self,
        ruby: &magnus::Ruby,
        reader: &ReaderSource,
        ip: Value,
    ) -> Result<Value, Error> {
        let parsed_ip = self.parse_lookup_ip(ip, ruby)?;
        lookup_result_to_value(ruby, reader.lookup(parsed_ip), "Database lookup failed")
    }

    #[inline]
    fn lookup_ip_path_value(
        &self,
        ruby: &magnus::Ruby,
        reader: &ReaderSource,
        ip: Value,
        path_elements: &[PathElement<'_>],
    ) -> Result<Value, Error> {
        let parsed_ip = self.parse_lookup_ip(ip, ruby)?;
        lookup_result_to_value(
            ruby,
            reader.lookup_path(parsed_ip, path_elements),
            "Database lookup failed",
        )
    }

    fn parse_path(
        &self,
        path: Value,
        ruby: &magnus::Ruby,
    ) -> Result<Arc<[OwnedPathElement]>, Error> {
        let path = path_array(path, ruby)?;
        let hash = path_cache_hash(path, ruby)?;

        if let Some(cached) = self.cached_path(path, hash)? {
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
    ) -> Result<Option<Arc<[OwnedPathElement]>>, Error> {
        let cache = match self.path_cache.lock() {
            Ok(cache) => cache,
            Err(_) => return Ok(None),
        };

        for entry in cache
            .iter()
            .filter(|entry| entry.hash == hash && entry.elements.len() == path.len())
        {
            if path_matches_cached(path, entry.elements.as_ref())? {
                return Ok(Some(entry.elements.clone()));
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

/// Helper function to create a Reader from a ReaderSource
fn create_reader(source: ReaderSource) -> Reader {
    let ip_version = source.metadata().ip_version;
    let source = Arc::new(source);
    Reader {
        reader: ArcSwapOption::from(Some(source)),
        path_cache: Mutex::new(VecDeque::with_capacity(PATH_CACHE_MAX_ENTRIES)),
        ip_version,
    }
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

/// Parse IP address from Ruby value (String or IPAddr) - optimized version
#[inline(always)]
fn parse_ip_address_fast(value: Value, ruby: &magnus::Ruby) -> Result<IpAddr, Error> {
    // Fast path: Try as RString first (most common case) - zero-copy
    if let Some(rstring) = RString::from_value(value) {
        // SAFETY: as_str() returns a &str that's valid as long as the Ruby string isn't modified
        // We use it immediately for parsing, so this is safe
        let ip_str = unsafe { rstring.as_str() }.map_err(|e| {
            Error::new(
                ruby.exception_arg_error(),
                format!("Invalid UTF-8 in IP address string: {}", e),
            )
        })?;

        return parse_ip_string(ip_str, ruby);
    }

    // Slow path: Try as IPAddr object
    if let Ok(ipaddr_class) = ruby.class_object().const_get::<_, RClass>("IPAddr") {
        if value.is_kind_of(ipaddr_class) {
            let address: u128 = value.funcall("to_i", ())?;
            let is_ipv4: bool = value.funcall("ipv4?", ())?;
            if is_ipv4 {
                let address = u32::try_from(address).map_err(|_| {
                    Error::new(
                        ruby.exception_arg_error(),
                        format!("'{}' does not appear to be an IPv4 address", value),
                    )
                })?;
                return Ok(IpAddr::V4(Ipv4Addr::from(address)));
            }
            return Ok(IpAddr::V6(Ipv6Addr::from(address)));
        }
    }

    if let Ok(ipaddr_obj) = value.funcall::<_, _, String>("to_s", ()) {
        return parse_ip_string(&ipaddr_obj, ruby);
    }

    Err(Error::new(
        ruby.exception_arg_error(),
        format!("'{}' does not appear to be an IPv4 or IPv6 address", value),
    ))
}

#[inline(always)]
fn parse_ip_string(s: &str, ruby: &magnus::Ruby) -> Result<IpAddr, Error> {
    if let Some(ip) = parse_ipv4_string(s.as_bytes()) {
        return Ok(IpAddr::V4(ip));
    }

    IpAddr::from_str(s).map_err(|_| {
        Error::new(
            ruby.exception_arg_error(),
            format!("'{}' does not appear to be an IPv4 or IPv6 address", s),
        )
    })
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
        | MaxMindDbError::Io(_) => invalid_database_exception(ERR_BAD_DATA),
        other => Error::new(
            ruby.exception_runtime_error(),
            format!("{}: {}", context, other),
        ),
    }
}

/// Generate error message for IPv6 in IPv4-only database
fn ipv6_in_ipv4_error(ip: &IpAddr) -> String {
    format!(
        "Error looking up {}. You attempted to look up an IPv6 address in an IPv4-only database.",
        ip
    )
}

fn database_path(database: Value) -> Result<String, Error> {
    RString::try_convert(database)?.to_string()
}

fn database_buffer(database: Value) -> Result<Vec<u8>, Error> {
    let string = RString::try_convert(database)?;
    // SAFETY: the slice is copied into an owned Vec before Ruby can mutate or
    // free the string, and the reader only ever sees the owned bytes.
    Ok(unsafe { string.as_slice() }.to_vec())
}

/// Open a MaxMind DB using memory-mapped I/O (MODE_MMAP)
fn open_database_mmap(path: &str) -> Result<Reader, Error> {
    let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby context");
    let file = open_database_file(path, &ruby)?;

    // SAFETY: the mapping is read-only, and MODE_MMAP's documented contract
    // requires callers not to modify or truncate the mapped file while this
    // reader is alive. Database updates should atomically replace the path.
    let mmap = unsafe { Mmap::map(&file) }.map_err(|e| {
        Error::new(
            ruby.exception_io_error(),
            format!("Failed to memory-map database file: {}", e),
        )
    })?;
    let reader = MaxMindReader::from_source(mmap).map_err(|_| {
        invalid_database_exception(&format!(
            "Error opening database file ({}). Is this a valid MaxMind DB file?",
            path
        ))
    })?;

    Ok(create_reader(ReaderSource::Mmap(reader)))
}

/// Open a MaxMind DB by loading entire file into memory (MODE_MEMORY)
fn open_database_memory(path: &str) -> Result<Reader, Error> {
    let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby context");
    let mut file = open_database_file(path, &ruby)?;

    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).map_err(|e| {
        Error::new(
            ruby.exception_io_error(),
            format!("Failed to read database file: {}", e),
        )
    })?;

    reader_from_buffer(
        buffer,
        format!(
            "Error opening database file ({}). Is this a valid MaxMind DB file?",
            path
        ),
    )
}

fn open_database_buffer(buffer: Vec<u8>) -> Result<Reader, Error> {
    reader_from_buffer(
        buffer,
        "Error opening database from buffer. Is this a valid MaxMind DB file?".to_owned(),
    )
}

fn reader_from_buffer(buffer: Vec<u8>, invalid_message: String) -> Result<Reader, Error> {
    let reader = MaxMindReader::from_source(buffer)
        .map_err(|_| invalid_database_exception(invalid_message.as_str()))?;

    Ok(create_reader(ReaderSource::Memory(reader)))
}

fn open_database_file(path: &str, ruby: &magnus::Ruby) -> Result<File, Error> {
    File::open(Path::new(path)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            open_not_found_error(ruby, e)
        } else {
            Error::new(ruby.exception_io_error(), e.to_string())
        }
    })
}

fn open_not_found_error(ruby: &magnus::Ruby, err: std::io::Error) -> Error {
    let errno = ruby
        .class_object()
        .const_get::<_, RModule>("Errno")
        .expect("Errno module should exist");
    let enoent = errno
        .const_get::<_, RClass>("ENOENT")
        .expect("Errno::ENOENT should exist");
    Error::new(
        ExceptionClass::from_value(enoent.as_value())
            .expect("ENOENT should convert to ExceptionClass"),
        err.to_string(),
    )
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
    reader_class.define_singleton_method("new", magnus::function!(Reader::new, -1))?;
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
