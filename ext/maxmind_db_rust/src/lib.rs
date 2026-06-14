// SAFETY: the `maxminddb` crate is built with the `unsafe-str-decode` feature enabled.
// Ruby validates UTF-8 when we construct `RString`s, so skipping the redundant check in
// the decoder is safe and avoids re-validating every string record twice.
use ::maxminddb as maxminddb_crate;
use arc_swap::{ArcSwapOption, Guard};
use ipnetwork::IpNetwork;
use magnus::{
    error::Error, prelude::*, scan_args::get_kwargs, scan_args::scan_args, ExceptionClass,
    IntoValue, RArray, RClass, RHash, RModule, RString, Symbol, Value,
};
use maxminddb_crate::{MaxMindDbError, Reader as MaxMindReader, Within};
use memmap2::Mmap;
use rustc_hash::FxHasher;
use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeMap,
    fmt,
    fs::File,
    hash::{Hash, Hasher},
    io::Read as IoRead,
    net::IpAddr,
    path::Path,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

// Error constants
const ERR_CLOSED_DB: &str = "Attempt to read from a closed MaxMind DB.";
const ERR_BAD_DATA: &str =
    "The MaxMind DB file's data section contains bad data (unknown data type or corrupt data)";
const STRING_CACHE_ROOTS_CONST: &str = "__STRING_CACHE_ROOTS__";
const MAP_KEY_ROOTS_CONST: &str = "__MAP_KEY_ROOTS__";
const STRING_CACHE_MAX: usize = 4096;
const STRING_CACHE_MIN_LEN: usize = 2;
const STRING_CACHE_MAX_LEN: usize = 64;

#[derive(Default)]
struct StringCacheEntry {
    hash: u64,
    value: String,
}

struct StringCache {
    entries: Box<[StringCacheEntry]>,
}

impl StringCache {
    fn new() -> Self {
        let entries = (0..STRING_CACHE_MAX)
            .map(|_| StringCacheEntry::default())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self { entries }
    }
}

thread_local! {
    static STRING_CACHE: RefCell<StringCache> = RefCell::new(StringCache::new());
    static STRING_CACHE_ROOTS: OnceCell<RArray> = const { OnceCell::new() };
}

#[inline]
fn string_cache_roots_owner(ruby: &magnus::Ruby) -> RArray {
    let value = rust_module(ruby)
        .const_get::<_, Value>(STRING_CACHE_ROOTS_CONST)
        .expect("string cache roots constant should exist");
    RArray::from_value(value).expect("string cache roots constant should be an array")
}

#[inline]
fn init_thread_string_cache_roots(ruby: &magnus::Ruby) -> RArray {
    let roots = ruby.ary_new_capa(STRING_CACHE_MAX);
    for _ in 0..STRING_CACHE_MAX {
        roots
            .push(ruby.qnil().as_value())
            .expect("string cache roots initialization should succeed");
    }
    string_cache_roots_owner(ruby)
        .push(roots.as_value())
        .expect("string cache roots owner should retain per-thread roots");
    roots
}

#[inline]
fn string_cache_roots(ruby: &magnus::Ruby) -> RArray {
    STRING_CACHE_ROOTS.with(|roots| *roots.get_or_init(|| init_thread_string_cache_roots(ruby)))
}

#[inline]
fn cached_string(ruby: &magnus::Ruby, value: &str) -> Value {
    if !(STRING_CACHE_MIN_LEN..=STRING_CACHE_MAX_LEN).contains(&value.len()) {
        return ruby.str_new(value).into_value_with(ruby);
    }

    let mut hasher = FxHasher::default();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    let slot = (hash as usize) & (STRING_CACHE_MAX - 1);

    STRING_CACHE.with(|cache_cell| {
        let mut cache = cache_cell.borrow_mut();
        let entry = &mut cache.entries[slot];
        if entry.hash == hash && entry.value == value {
            return string_cache_roots(ruby)
                .entry::<Value>(slot as isize)
                .expect("string cache roots lookup should succeed");
        }

        let string = ruby.str_new(value);
        string.freeze();
        let cached = string.as_value();
        string_cache_roots(ruby)
            .store(slot as isize, cached)
            .expect("string cache roots update should succeed");
        entry.hash = hash;
        entry.value.clear();
        entry.value.push_str(value);
        cached
    })
}

/// Wrapper that owns the Ruby value produced by deserializing a MaxMind record
#[derive(Clone)]
struct RubyDecodedValue {
    value: Value,
}

impl RubyDecodedValue {
    #[inline]
    fn new(value: Value) -> Self {
        Self { value }
    }

    #[inline]
    fn into_value(self) -> Value {
        self.value
    }
}

impl<'de> Deserialize<'de> for RubyDecodedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in deserializer");
        RubyValueSeed { ruby: &ruby }.deserialize(deserializer)
    }
}

struct RubyValueSeed<'ruby> {
    ruby: &'ruby magnus::Ruby,
}

impl<'ruby, 'de> DeserializeSeed<'de> for RubyValueSeed<'ruby> {
    type Value = RubyDecodedValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RubyValueVisitor { ruby: self.ruby })
    }
}

struct RubyValueVisitor<'ruby> {
    ruby: &'ruby magnus::Ruby,
}

impl<'de, 'ruby> Visitor<'de> for RubyValueVisitor<'ruby> {
    type Value = RubyDecodedValue;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any valid MaxMind DB value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(self.ruby)))
    }

    fn visit_i32<E>(self, value: i32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(self.ruby)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value >= i32::MIN as i64 && value <= i32::MAX as i64 {
            Ok(RubyDecodedValue::new(
                (value as i32).into_value_with(self.ruby),
            ))
        } else {
            Ok(RubyDecodedValue::new(value.into_value_with(self.ruby)))
        }
    }

    fn visit_u16<E>(self, value: u16) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(self.ruby)))
    }

    fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(self.ruby)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(self.ruby)))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(self.ruby)))
    }

    fn visit_f32<E>(self, value: f32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(
            (value as f64).into_value_with(self.ruby),
        ))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(self.ruby)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(cached_string(self.ruby, value)))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(cached_string(self.ruby, &value)))
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(
            self.ruby.str_from_slice(value).into_value_with(self.ruby),
        ))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(
            self.ruby.str_from_slice(&value).into_value_with(self.ruby),
        ))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let arr = match seq.size_hint() {
            Some(cap) => self.ruby.ary_new_capa(cap),
            None => self.ruby.ary_new(),
        };
        while let Some(elem) = seq.next_element_seed(RubyValueSeed { ruby: self.ruby })? {
            arr.push(elem.into_value())
                .map_err(|e| de::Error::custom(e.to_string()))?;
        }
        Ok(RubyDecodedValue::new(arr.into_value_with(self.ruby)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let hash = match map.size_hint() {
            Some(cap) => self.ruby.hash_new_capa(cap),
            None => self.ruby.hash_new(),
        };
        while let Some(key) = map.next_key::<&'de str>()? {
            let value = map.next_value_seed(RubyValueSeed { ruby: self.ruby })?;
            let key_val = cached_string(self.ruby, key);
            hash.aset(key_val, value.into_value())
                .map_err(|e| de::Error::custom(e.to_string()))?;
        }
        Ok(RubyDecodedValue::new(hash.into_value_with(self.ruby)))
    }
}

/// Enum to handle different reader source types
enum ReaderSource {
    Mmap(MaxMindReader<Mmap>),
    Memory(MaxMindReader<Vec<u8>>),
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
    fn metadata(&self) -> &maxminddb_crate::Metadata {
        match self {
            ReaderSource::Mmap(reader) => &reader.metadata,
            ReaderSource::Memory(reader) => &reader.metadata,
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

/// Metadata about the MaxMind DB database
#[derive(Clone)]
#[magnus::wrap(class = "MaxMind::DB::Rust::Metadata")]
struct Metadata {
    /// The major version number of the binary format used when creating the database.
    binary_format_major_version: u16,
    /// The minor version number of the binary format used when creating the database.
    binary_format_minor_version: u16,
    /// The Unix epoch timestamp for when the database was built.
    build_epoch: u64,
    /// A string identifying the database type (e.g., 'GeoIP2-City', 'GeoLite2-Country').
    database_type: String,
    description_map: BTreeMap<String, String>,
    /// The IP version of the data in a database. A value of 4 means IPv4 only; 6 supports both IPv4 and IPv6.
    ip_version: u16,
    languages_list: Vec<String>,
    /// The number of nodes in the search tree.
    node_count: u32,
    /// The record size in bits (24, 28, or 32).
    record_size: u16,
}

impl Metadata {
    fn binary_format_major_version(&self) -> u16 {
        self.binary_format_major_version
    }

    fn binary_format_minor_version(&self) -> u16 {
        self.binary_format_minor_version
    }

    fn build_epoch(&self) -> u64 {
        self.build_epoch
    }

    fn database_type(&self) -> String {
        self.database_type.clone()
    }

    fn description(&self) -> RHash {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");
        let hash = ruby.hash_new();
        for (k, v) in &self.description_map {
            let _ = hash.aset(k.as_str(), v.as_str());
        }
        hash
    }

    fn ip_version(&self) -> u16 {
        self.ip_version
    }

    fn languages(&self) -> Vec<String> {
        self.languages_list.clone()
    }

    fn node_count(&self) -> u32 {
        self.node_count
    }

    fn record_size(&self) -> u16 {
        self.record_size
    }

    fn node_byte_size(&self) -> u16 {
        self.record_size / 4
    }

    fn search_tree_size(&self) -> u32 {
        self.node_count * (self.record_size as u32 / 4)
    }
}

unsafe impl Send for Metadata {}

/// A Ruby wrapper around the MaxMind DB reader
#[derive(Clone)]
#[magnus::wrap(class = "MaxMind::DB::Rust::Reader")]
struct Reader {
    reader: Arc<ArcSwapOption<ReaderSource>>,
    closed: Arc<AtomicBool>,
    ip_version: u16,
}

impl Reader {
    fn new(args: &[Value]) -> Result<Self, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let args = scan_args::<(String,), (), (), (), _, ()>(args)?;
        let (database,) = args.required;
        let kw = get_kwargs::<_, (), (Option<Symbol>,), ()>(args.keywords, &[], &["mode"])?;
        let (mode,) = kw.optional;

        // Parse mode from options hash
        let mode: Symbol = mode.unwrap_or_else(|| ruby.to_symbol("MODE_AUTO"));

        let mode_str = mode.name()?;
        let mode_str: &str = &mode_str;

        // Determine actual mode to use
        let actual_mode = match mode_str {
            "MODE_AUTO" | "MODE_MMAP" => "MMAP",
            "MODE_MEMORY" => "MEMORY",
            _ => {
                return Err(Error::new(
                    ruby.exception_arg_error(),
                    format!("Unsupported mode: {}", mode_str),
                ))
            }
        };

        // Open database with appropriate mode
        match actual_mode {
            "MMAP" => open_database_mmap(&database),
            "MEMORY" => open_database_memory(&database),
            _ => Err(Error::new(
                ruby.exception_arg_error(),
                format!("Invalid mode: {}", actual_mode),
            )),
        }
    }

    #[inline]
    fn get(&self, ip_address: Value) -> Result<Value, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        // Parse IP address
        let parsed_ip = parse_ip_address_fast(ip_address, &ruby)?;

        if self.ip_version == 4 && matches!(parsed_ip, IpAddr::V6(_)) {
            return Err(Error::new(
                ruby.exception_arg_error(),
                ipv6_in_ipv4_error(&parsed_ip),
            ));
        }

        // Perform lookup
        match reader.lookup(parsed_ip) {
            Ok(Some(data)) => Ok(data.into_value()),
            Ok(None) => Ok(ruby.qnil().as_value()),
            Err(MaxMindDbError::InvalidDatabase { .. }) | Err(MaxMindDbError::Io(_)) => {
                Err(Error::new(
                    ExceptionClass::from_value(invalid_database_error().as_value())
                        .expect("InvalidDatabaseError should convert to ExceptionClass"),
                    ERR_BAD_DATA,
                ))
            }
            Err(e) => Err(Error::new(
                ruby.exception_runtime_error(),
                format!("Database lookup failed: {}", e),
            )),
        }
    }

    #[inline]
    fn get_with_prefix_length(&self, ip_address: Value) -> Result<RArray, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        // Parse IP address
        let parsed_ip = parse_ip_address_fast(ip_address, &ruby)?;

        if self.ip_version == 4 && matches!(parsed_ip, IpAddr::V6(_)) {
            return Err(Error::new(
                ruby.exception_arg_error(),
                ipv6_in_ipv4_error(&parsed_ip),
            ));
        }

        // Perform lookup with prefix
        match reader.lookup_prefix(parsed_ip) {
            Ok((Some(data), prefix)) => {
                let arr = ruby.ary_new();
                arr.push(data.into_value())?;
                arr.push(prefix.into_value_with(&ruby))?;
                Ok(arr)
            }
            Ok((None, prefix)) => {
                let arr = ruby.ary_new();
                arr.push(ruby.qnil().as_value())?;
                arr.push(prefix.into_value_with(&ruby))?;
                Ok(arr)
            }
            Err(MaxMindDbError::InvalidDatabase { .. }) | Err(MaxMindDbError::Io(_)) => {
                Err(Error::new(
                    ExceptionClass::from_value(invalid_database_error().as_value())
                        .expect("InvalidDatabaseError should convert to ExceptionClass"),
                    ERR_BAD_DATA,
                ))
            }
            Err(e) => Err(Error::new(
                ruby.exception_runtime_error(),
                format!("Database lookup failed: {}", e),
            )),
        }
    }

    fn metadata(&self) -> Result<Metadata, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();
        let meta = reader.metadata();

        Ok(Metadata {
            binary_format_major_version: meta.binary_format_major_version,
            binary_format_minor_version: meta.binary_format_minor_version,
            build_epoch: meta.build_epoch,
            database_type: meta.database_type.clone(),
            description_map: meta.description.clone(),
            ip_version: meta.ip_version,
            languages_list: meta.languages.clone(),
            node_count: meta.node_count,
            record_size: meta.record_size,
        })
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.reader.store(None);
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn each(&self, args: &[Value]) -> Result<Value, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        // If no block given, return enumerator
        if !ruby.block_given() {
            return Err(Error::new(
                ruby.exception_runtime_error(),
                "Enumerator support not yet implemented, please provide a block",
            ));
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

        let mut iter = reader.within(network).map_err(|e| {
            Error::new(
                ExceptionClass::from_value(invalid_database_error().as_value())
                    .expect("InvalidDatabaseError should convert to ExceptionClass"),
                format!("Failed to iterate: {}", e),
            )
        })?;
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
                Err(MaxMindDbError::InvalidDatabase { .. }) | Err(MaxMindDbError::Io(_)) => {
                    return Err(Error::new(
                        ExceptionClass::from_value(invalid_database_error().as_value())
                            .expect("InvalidDatabaseError should convert to ExceptionClass"),
                        ERR_BAD_DATA,
                    ));
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
}

unsafe impl Send for Reader {}

/// Helper function to create a Reader from a ReaderSource
fn create_reader(source: ReaderSource) -> Reader {
    let ip_version = source.metadata().ip_version;
    let source = Arc::new(source);
    Reader {
        reader: Arc::new(ArcSwapOption::from(Some(source))),
        closed: Arc::new(AtomicBool::new(false)),
        ip_version,
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

        return IpAddr::from_str(ip_str).map_err(|_| {
            Error::new(
                ruby.exception_arg_error(),
                format!("'{}' does not appear to be an IPv4 or IPv6 address", ip_str),
            )
        });
    }

    // Slow path: Try as IPAddr object
    if let Ok(ipaddr_class) = ruby.class_object().const_get::<_, RClass>("IPAddr") {
        if value.is_kind_of(ipaddr_class) {
            let packed: Value = value.funcall("hton", ())?;
            if let Some(packed_str) = RString::from_value(packed) {
                // SAFETY: `bytes` is used immediately and `packed`/`packed_str` stay alive and
                // unmodified through the end of this match. This block must not introduce calls
                // that could move, collect, or mutate the Ruby string between `as_slice()` and
                // the final byte-pattern match handling.
                let bytes = unsafe { packed_str.as_slice() };
                return match bytes {
                    [a, b, c, d] => Ok(IpAddr::from([*a, *b, *c, *d])),
                    [a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15] => {
                        Ok(IpAddr::from([
                            *a0, *a1, *a2, *a3, *a4, *a5, *a6, *a7, *a8, *a9, *a10, *a11, *a12,
                            *a13, *a14, *a15,
                        ]))
                    }
                    _ => Err(Error::new(
                        ruby.exception_arg_error(),
                        format!("'{}' does not appear to be an IPv4 or IPv6 address", value),
                    )),
                };
            }
        }
    }

    if let Ok(ipaddr_obj) = value.funcall::<_, _, String>("to_s", ()) {
        return IpAddr::from_str(&ipaddr_obj).map_err(|_| {
            Error::new(
                ruby.exception_arg_error(),
                format!(
                    "'{}' does not appear to be an IPv4 or IPv6 address",
                    ipaddr_obj
                ),
            )
        });
    }

    Err(Error::new(
        ruby.exception_arg_error(),
        format!("'{}' does not appear to be an IPv4 or IPv6 address", value),
    ))
}

/// Generate error message for IPv6 in IPv4-only database
fn ipv6_in_ipv4_error(ip: &IpAddr) -> String {
    format!(
        "Error looking up {}. You attempted to look up an IPv6 address in an IPv4-only database",
        ip
    )
}

/// Open a MaxMind DB using memory-mapped I/O (MODE_MMAP)
fn open_database_mmap(path: &str) -> Result<Reader, Error> {
    let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby context");
    let file = open_database_file(path, &ruby)?;

    let mmap = unsafe { Mmap::map(&file) }.map_err(|e| {
        Error::new(
            ruby.exception_io_error(),
            format!("Failed to memory-map database file: {}", e),
        )
    })?;
    let reader = MaxMindReader::from_source(mmap).map_err(|_| {
        Error::new(
            ExceptionClass::from_value(invalid_database_error().as_value())
                .expect("InvalidDatabaseError should convert to ExceptionClass"),
            format!(
                "Error opening database file ({}). Is this a valid MaxMind DB file?",
                path
            ),
        )
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

    let reader = MaxMindReader::from_source(buffer).map_err(|_| {
        Error::new(
            ExceptionClass::from_value(invalid_database_error().as_value())
                .expect("InvalidDatabaseError should convert to ExceptionClass"),
            format!(
                "Error opening database file ({}). Is this a valid MaxMind DB file?",
                path
            ),
        )
    })?;

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

fn rust_module(ruby: &magnus::Ruby) -> RModule {
    let maxmind = ruby
        .class_object()
        .const_get::<_, RModule>("MaxMind")
        .expect("MaxMind module should exist");
    let db = maxmind
        .const_get::<_, Value>("DB")
        .expect("MaxMind::DB constant should exist");
    let rust_value = db
        .funcall::<_, _, Value>("const_get", ("Rust",))
        .expect("MaxMind::DB::Rust constant should exist");
    RModule::from_value(rust_value).expect("MaxMind::DB::Rust should be a module")
}

#[magnus::init]
fn init(ruby: &magnus::Ruby) -> Result<(), Error> {
    // Define module hierarchy: MaxMind::DB::Rust
    // Handle case where official maxmind-db gem may have already defined MaxMind::DB as a Class
    let maxmind = ruby.define_module("MaxMind")?;

    // Try to get or define DB - it might be a Class (official gem) or Module (ours)
    let db_value = maxmind.const_get::<_, Value>("DB");
    let rust = match db_value {
        Ok(existing) if existing.is_kind_of(ruby.class_class()) => {
            // MaxMind::DB exists as a Class (official gem loaded first)
            // Reuse existing Rust constant if present to avoid replacing classes.
            if let Ok(rust_value) = existing.funcall::<_, _, Value>("const_get", ("Rust", false)) {
                RModule::from_value(rust_value).ok_or_else(|| {
                    Error::new(
                        ruby.exception_type_error(),
                        "MaxMind::DB::Rust exists but is not a module",
                    )
                })?
            } else {
                // Define Rust module directly as a constant on the class.
                let rust_value: Value = ruby.module_new().as_value();
                let rust_mod = RModule::from_value(rust_value).ok_or_else(|| {
                    Error::new(
                        ruby.exception_type_error(),
                        "Failed to create anonymous module for MaxMind::DB::Rust",
                    )
                })?;
                let _ = existing.funcall::<_, _, Value>("const_set", ("Rust", rust_mod))?;
                rust_mod
            }
        }
        Ok(existing) => {
            // MaxMind::DB exists as a Module (our gem loaded first)
            let db_mod = RModule::from_value(existing).ok_or_else(|| {
                Error::new(ruby.exception_type_error(), "MaxMind::DB is not a module")
            })?;
            db_mod.define_module("Rust")?
        }
        Err(_) => {
            // MaxMind::DB doesn't exist, define it as a module
            let db = maxmind.define_module("DB")?;
            db.define_module("Rust")?
        }
    };

    if rust
        .const_get::<_, Value>(STRING_CACHE_ROOTS_CONST)
        .is_err()
    {
        rust.const_set(STRING_CACHE_ROOTS_CONST, ruby.ary_new())?;
    }

    if rust.const_get::<_, Value>(MAP_KEY_ROOTS_CONST).is_ok() {
        let _ = rust.funcall::<_, _, Value>("send", ("remove_const", MAP_KEY_ROOTS_CONST))?;
    }

    // The extension can be loaded more than once from different paths.
    // Reusing previously defined classes avoids typed-data incompatibilities.
    if rust.const_get::<_, Value>("Reader").is_ok() {
        return Ok(());
    }
    // Define InvalidDatabaseError
    let runtime_error = ruby.exception_runtime_error();
    rust.define_error("InvalidDatabaseError", runtime_error)?;

    // Define Reader class
    let reader_class = rust.define_class("Reader", ruby.class_object())?;
    reader_class.define_singleton_method("new", magnus::function!(Reader::new, -1))?;
    reader_class.define_method("get", magnus::method!(Reader::get, 1))?;
    reader_class.define_method(
        "get_with_prefix_length",
        magnus::method!(Reader::get_with_prefix_length, 1),
    )?;
    reader_class.define_method("metadata", magnus::method!(Reader::metadata, 0))?;
    reader_class.define_method("close", magnus::method!(Reader::close, 0))?;
    reader_class.define_method("closed", magnus::method!(Reader::closed, 0))?;
    reader_class.define_method("each", magnus::method!(Reader::each, -1))?;

    // Include Enumerable module
    let enumerable = ruby.class_object().const_get::<_, RModule>("Enumerable")?;
    reader_class.include_module(enumerable)?;

    // Define Metadata class
    let metadata_class = rust.define_class("Metadata", ruby.class_object())?;
    metadata_class.define_method(
        "binary_format_major_version",
        magnus::method!(Metadata::binary_format_major_version, 0),
    )?;
    metadata_class.define_method(
        "binary_format_minor_version",
        magnus::method!(Metadata::binary_format_minor_version, 0),
    )?;
    metadata_class.define_method("build_epoch", magnus::method!(Metadata::build_epoch, 0))?;
    metadata_class.define_method("database_type", magnus::method!(Metadata::database_type, 0))?;
    metadata_class.define_method("description", magnus::method!(Metadata::description, 0))?;
    metadata_class.define_method("ip_version", magnus::method!(Metadata::ip_version, 0))?;
    metadata_class.define_method("languages", magnus::method!(Metadata::languages, 0))?;
    metadata_class.define_method("node_count", magnus::method!(Metadata::node_count, 0))?;
    metadata_class.define_method("record_size", magnus::method!(Metadata::record_size, 0))?;
    metadata_class.define_method(
        "node_byte_size",
        magnus::method!(Metadata::node_byte_size, 0),
    )?;
    metadata_class.define_method(
        "search_tree_size",
        magnus::method!(Metadata::search_tree_size, 0),
    )?;

    // Define MODE constants
    rust.const_set("MODE_AUTO", ruby.to_symbol("MODE_AUTO"))?;
    rust.const_set("MODE_MEMORY", ruby.to_symbol("MODE_MEMORY"))?;
    rust.const_set("MODE_MMAP", ruby.to_symbol("MODE_MMAP"))?;

    Ok(())
}
