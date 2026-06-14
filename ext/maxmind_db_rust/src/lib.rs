// SAFETY: the `maxminddb` crate is built with the `unsafe-str-decode` feature enabled.
// Ruby validates UTF-8 when we construct `RString`s, so skipping the redundant check in
// the decoder is safe and avoids re-validating every string record twice.
use ::maxminddb as maxminddb_crate;
use arc_swap::{ArcSwapOption, Guard};
use ipnetwork::IpNetwork;
use magnus::{
    error::Error, prelude::*, scan_args::get_kwargs, scan_args::scan_args, typed_data::Obj,
    ExceptionClass, IntoValue, RArray, RClass, RHash, RModule, RString, Symbol, Value,
};
use maxminddb_crate::{MaxMindDbError, PathElement, Reader as MaxMindReader, Within};
use memmap2::Mmap;
use rustc_hash::FxHasher;
use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, VecDeque},
    fmt,
    fs::File,
    hash::{Hash, Hasher},
    io::Read as IoRead,
    net::{IpAddr, Ipv4Addr},
    path::Path,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
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
const PATH_CACHE_MAX_ENTRIES: usize = 64;

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

#[derive(PartialEq, Eq)]
enum OwnedPathElement {
    Key(String),
    Index(usize),
    IndexFromEnd(usize),
}

struct CachedPath {
    hash: u64,
    elements: Arc<[OwnedPathElement]>,
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

// SAFETY: Metadata stores only owned Rust values copied out of the database
// metadata. It contains no Ruby VALUE handles or borrowed database/source data,
// so moving it between Ruby-managed threads cannot invalidate GC or lifetime
// assumptions.
unsafe impl Send for Metadata {}

/// A Ruby wrapper around the MaxMind DB reader
#[derive(Clone)]
#[magnus::wrap(class = "MaxMind::DB::Rust::Reader")]
struct Reader {
    reader: Arc<ArcSwapOption<ReaderSource>>,
    closed: Arc<AtomicBool>,
    path_cache: Arc<Mutex<VecDeque<CachedPath>>>,
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
        lookup_prefix_result_to_array(&ruby, reader.lookup_prefix(parsed_ip))
    }

    fn get_many(&self, ips: Value) -> Result<RArray, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        if let Ok(ips) = RArray::try_convert(ips) {
            let results = ruby.ary_new_capa(ips.len());
            for index in 0..ips.len() {
                let ip = ips.entry::<Value>(index as isize)?;
                results.push(self.lookup_ip_value(&ruby, reader, ip)?)?;
            }
            return Ok(results);
        }

        ensure_enumerable(ips, &ruby, "ips must be an Array or Enumerable")?;
        let results = ruby.ary_new();
        for ip in ips.enumeratorize("each", ()) {
            results.push(self.lookup_ip_value(&ruby, reader, ip?)?)?;
        }
        Ok(results)
    }

    fn get_many_path(&self, ips: Value, path: Value) -> Result<RArray, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");

        let guard = self.get_reader(&ruby)?;
        let reader_option = guard.as_ref();
        let reader = reader_option.as_ref().unwrap();

        let owned_path = self.parse_path(path, &ruby)?;
        let path_elements = path_elements_from_owned_path(owned_path.as_ref());

        if let Ok(ips) = RArray::try_convert(ips) {
            let results = ruby.ary_new_capa(ips.len());
            for index in 0..ips.len() {
                let ip = ips.entry::<Value>(index as isize)?;
                results.push(self.lookup_ip_path_value(&ruby, reader, ip, &path_elements)?)?;
            }
            return Ok(results);
        }

        ensure_enumerable(ips, &ruby, "ips must be an Array or Enumerable")?;
        let results = ruby.ary_new();
        for ip in ips.enumeratorize("each", ()) {
            results.push(self.lookup_ip_path_value(&ruby, reader, ip?, &path_elements)?)?;
        }
        Ok(results)
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
        let candidates = match self.path_cache.lock() {
            Ok(cache) => cache
                .iter()
                .filter(|entry| entry.hash == hash && entry.elements.len() == path.len())
                .map(|entry| entry.elements.clone())
                .collect::<Vec<_>>(),
            Err(_) => return Ok(None),
        };

        for candidate in candidates {
            if path_matches_cached(path, candidate.as_ref())? {
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

/// Helper function to create a Reader from a ReaderSource
fn create_reader(source: ReaderSource) -> Reader {
    let ip_version = source.metadata().ip_version;
    let source = Arc::new(source);
    Reader {
        reader: Arc::new(ArcSwapOption::from(Some(source))),
        closed: Arc::new(AtomicBool::new(false)),
        path_cache: Arc::new(Mutex::new(VecDeque::with_capacity(PATH_CACHE_MAX_ENTRIES))),
        ip_version,
    }
}

fn path_array(path: Value, ruby: &magnus::Ruby) -> Result<RArray, Error> {
    RArray::try_convert(path).map_err(|_| {
        Error::new(
            ruby.exception_arg_error(),
            "Path must be an Array of String and Integer elements",
        )
    })
}

fn parse_path_array(path: RArray, ruby: &magnus::Ruby) -> Result<Vec<OwnedPathElement>, Error> {
    let mut elements = Vec::with_capacity(path.len());
    for index in 0..path.len() {
        let item = path.entry::<Value>(index as isize)?;
        if let Ok(key) = RString::try_convert(item) {
            elements.push(OwnedPathElement::Key(key.to_string()?));
            continue;
        }
        if let Ok(index) = isize::try_convert(item) {
            elements.push(signed_index_to_owned_path_element(index));
            continue;
        }
        return Err(Error::new(
            ruby.exception_arg_error(),
            "Path elements must be Strings or Integers",
        ));
    }

    Ok(elements)
}

#[inline]
fn signed_index_to_owned_path_element(index: isize) -> OwnedPathElement {
    if index >= 0 {
        OwnedPathElement::Index(index as usize)
    } else {
        let index_from_end = index
            .checked_neg()
            .and_then(|index| index.checked_sub(1))
            .map(|index| index as usize)
            .unwrap_or(usize::MAX);
        OwnedPathElement::IndexFromEnd(index_from_end)
    }
}

fn path_cache_hash(path: RArray, ruby: &magnus::Ruby) -> Result<u64, Error> {
    let mut hasher = FxHasher::default();
    path.len().hash(&mut hasher);

    for index in 0..path.len() {
        let item = path.entry::<Value>(index as isize)?;
        if let Ok(key) = RString::try_convert(item) {
            0_u8.hash(&mut hasher);
            hash_path_key(key, &mut hasher)?;
            continue;
        }
        if let Ok(index) = isize::try_convert(item) {
            1_u8.hash(&mut hasher);
            index.hash(&mut hasher);
            continue;
        }
        return Err(Error::new(
            ruby.exception_arg_error(),
            "Path elements must be Strings or Integers",
        ));
    }

    Ok(hasher.finish())
}

fn hash_path_key(key: RString, hasher: &mut FxHasher) -> Result<(), Error> {
    // SAFETY: the borrowed str is used only for immediate hashing and is not
    // stored across any call that could mutate or free the Ruby string.
    if let Some(key_str) = unsafe { key.test_as_str() } {
        key_str.hash(hasher);
    } else {
        key.to_string()?.hash(hasher);
    }
    Ok(())
}

fn path_matches_cached(path: RArray, cached: &[OwnedPathElement]) -> Result<bool, Error> {
    if path.len() != cached.len() {
        return Ok(false);
    }

    for (index, cached_element) in cached.iter().enumerate() {
        let item = path.entry::<Value>(index as isize)?;
        match cached_element {
            OwnedPathElement::Key(expected) => {
                let Ok(key) = RString::try_convert(item) else {
                    return Ok(false);
                };
                if !path_key_matches(key, expected)? {
                    return Ok(false);
                }
            }
            OwnedPathElement::Index(_) | OwnedPathElement::IndexFromEnd(_) => {
                let Ok(index) = isize::try_convert(item) else {
                    return Ok(false);
                };
                if signed_index_to_owned_path_element(index) != *cached_element {
                    return Ok(false);
                }
            }
        }
    }

    Ok(true)
}

fn path_key_matches(key: RString, expected: &str) -> Result<bool, Error> {
    // SAFETY: the borrowed str is used only for immediate comparison and is not
    // stored across any call that could mutate or free the Ruby string.
    if let Some(key_str) = unsafe { key.test_as_str() } {
        Ok(key_str == expected)
    } else {
        Ok(key.to_string()? == expected)
    }
}

fn path_elements_from_owned_path(path: &[OwnedPathElement]) -> Vec<PathElement<'_>> {
    path.iter()
        .map(|element| match element {
            OwnedPathElement::Key(key) => PathElement::Key(key.as_str()),
            OwnedPathElement::Index(index) => PathElement::Index(*index),
            OwnedPathElement::IndexFromEnd(index) => PathElement::IndexFromEnd(*index),
        })
        .collect()
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
) -> Result<RArray, Error> {
    match result {
        Ok((data, prefix)) => {
            let arr = ruby.ary_new();
            arr.push(data.map_or_else(|| ruby.qnil().as_value(), RubyDecodedValue::into_value))?;
            arr.push(prefix.into_value_with(ruby))?;
            Ok(arr)
        }
        Err(err) => Err(lookup_error(ruby, err, "Database lookup failed")),
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
    reader_class.define_method("get_path", magnus::method!(Reader::get_path, 2))?;
    reader_class.define_method(
        "get_with_prefix_length",
        magnus::method!(Reader::get_with_prefix_length, 1),
    )?;
    reader_class.define_method("get_many", magnus::method!(Reader::get_many, 1))?;
    reader_class.define_method("get_many_path", magnus::method!(Reader::get_many_path, 2))?;
    reader_class.define_method("metadata", magnus::method!(Reader::metadata, 0))?;
    reader_class.define_method("close", magnus::method!(Reader::close, 0))?;
    reader_class.define_method("closed", magnus::method!(Reader::closed, 0))?;
    reader_class.define_method("inspect", magnus::method!(Reader::inspect, 0))?;
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
