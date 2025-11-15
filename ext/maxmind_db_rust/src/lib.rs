use ::maxminddb as maxminddb_crate;
use ipnetwork::IpNetwork;
use magnus::{
    error::Error, prelude::*, scan_args::get_kwargs,
    scan_args::scan_args, ExceptionClass, IntoValue, RArray, RClass, RHash, RModule,
    RString, Symbol, Value,
};
use maxminddb_crate::{MaxMindDbError, Reader as MaxMindReader, Within, WithinItem};
use memmap2::Mmap;
use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt,
    fs::File,
    io::Read as IoRead,
    net::IpAddr,
    path::Path,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};

// Error constants
const ERR_CLOSED_DB: &str = "Attempt to read from a closed MaxMind DB.";
const ERR_BAD_DATA: &str =
    "The MaxMind DB file's data section contains bad data (unknown data type or corrupt data)";

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
        RubyValueSeed.deserialize(deserializer)
    }
}

struct RubyValueSeed;

impl<'de> DeserializeSeed<'de> for RubyValueSeed {
    type Value = RubyDecodedValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(RubyValueVisitor)
    }
}

struct RubyValueVisitor;

impl<'de> Visitor<'de> for RubyValueVisitor {
    type Value = RubyDecodedValue;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any valid MaxMind DB value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(&magnus::Ruby::get().unwrap())))
    }

    fn visit_i32<E>(self, value: i32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(&magnus::Ruby::get().unwrap())))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let ruby = magnus::Ruby::get().unwrap();
        if value >= i32::MIN as i64 && value <= i32::MAX as i64 {
            Ok(RubyDecodedValue::new((value as i32).into_value_with(&ruby)))
        } else {
            Ok(RubyDecodedValue::new(value.into_value_with(&ruby)))
        }
    }

    fn visit_u16<E>(self, value: u16) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(&magnus::Ruby::get().unwrap())))
    }

    fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(&magnus::Ruby::get().unwrap())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(&magnus::Ruby::get().unwrap())))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(&magnus::Ruby::get().unwrap())))
    }

    fn visit_f32<E>(self, value: f32) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new((value as f64).into_value_with(&magnus::Ruby::get().unwrap())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(value.into_value_with(&magnus::Ruby::get().unwrap())))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let ruby = magnus::Ruby::get().unwrap();
        Ok(RubyDecodedValue::new(
            ruby.str_new(value).into_value_with(&ruby),
        ))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let ruby = magnus::Ruby::get().unwrap();
        Ok(RubyDecodedValue::new(
            ruby.str_new(&value).into_value_with(&ruby),
        ))
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let ruby = magnus::Ruby::get().unwrap();
        Ok(RubyDecodedValue::new(
            ruby.str_from_slice(value).into_value_with(&ruby),
        ))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let ruby = magnus::Ruby::get().unwrap();
        Ok(RubyDecodedValue::new(
            ruby.str_from_slice(&value).into_value_with(&ruby),
        ))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let ruby = magnus::Ruby::get().unwrap();
        let arr = ruby.ary_new();
        while let Some(elem) = seq.next_element_seed(RubyValueSeed)? {
            arr.push(elem.into_value())
                .map_err(|e| de::Error::custom(e.to_string()))?;
        }
        Ok(RubyDecodedValue::new(arr.into_value_with(&ruby)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let ruby = magnus::Ruby::get().unwrap();
        let hash = ruby.hash_new();
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            let value = map.next_value_seed(RubyValueSeed)?;
            hash.aset(key.as_ref(), value.into_value())
                .map_err(|e| de::Error::custom(e.to_string()))?;
        }
        Ok(RubyDecodedValue::new(hash.into_value_with(&ruby)))
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
            ReaderSource::Mmap(reader) => reader.lookup(ip),
            ReaderSource::Memory(reader) => reader.lookup(ip),
        }
    }

    #[inline]
    fn lookup_prefix(
        &self,
        ip: IpAddr,
    ) -> Result<(Option<RubyDecodedValue>, usize), maxminddb_crate::MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => reader.lookup_prefix(ip),
            ReaderSource::Memory(reader) => reader.lookup_prefix(ip),
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
    fn within(
        &self,
        network: IpNetwork,
    ) -> Result<ReaderWithin, MaxMindDbError> {
        match self {
            ReaderSource::Mmap(reader) => {
                let iter = reader.within::<RubyDecodedValue>(network)?;
                // SAFETY: the iterator holds a reference into `reader`. We'll store an Arc guard
                // alongside it so the reader outlives the transmuted iterator.
                Ok(ReaderWithin::Mmap(unsafe {
                    std::mem::transmute::<Within<'_, RubyDecodedValue, Mmap>, Within<'static, RubyDecodedValue, Mmap>>(
                        iter,
                    )
                }))
            }
            ReaderSource::Memory(reader) => {
                let iter = reader.within::<RubyDecodedValue>(network)?;
                // SAFETY: same as above, the Arc guard keeps the reader alive.
                Ok(ReaderWithin::Memory(unsafe {
                    std::mem::transmute::<
                        Within<'_, RubyDecodedValue, Vec<u8>>,
                        Within<'static, RubyDecodedValue, Vec<u8>>,
                    >(iter)
                }))
            }
        }
    }
}

/// Wrapper enum for Within iterators
enum ReaderWithin {
    Mmap(Within<'static, RubyDecodedValue, Mmap>),
    Memory(Within<'static, RubyDecodedValue, Vec<u8>>),
}

impl ReaderWithin {
    fn next(&mut self) -> Option<Result<WithinItem<RubyDecodedValue>, MaxMindDbError>> {
        match self {
            ReaderWithin::Mmap(iter) => iter.next(),
            ReaderWithin::Memory(iter) => iter.next(),
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
        let ruby = magnus::Ruby::get().unwrap();
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
    reader: Arc<RwLock<Option<Arc<ReaderSource>>>>,
    closed: Arc<AtomicBool>,
    ip_version: u16,
}

impl Reader {
    fn new(args: &[Value]) -> Result<Self, Error> {
        let ruby = magnus::Ruby::get().unwrap();

        let args = scan_args::<(String,), (), (), (), _, ()>(args)?;
        let (database,) = args.required;
        let kw = get_kwargs::<_, (), (Option<Symbol>,), ()>(
            args.keywords,
            &[],
            &["mode"],
        )?;
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
        let ruby = magnus::Ruby::get().unwrap();

        // Check if database is closed
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::new(ruby.exception_runtime_error(), ERR_CLOSED_DB));
        }

        // Parse IP address
        let parsed_ip = parse_ip_address_fast(ip_address, &ruby)?;

        if self.ip_version == 4 && matches!(parsed_ip, IpAddr::V6(_)) {
            return Err(Error::new(
                ruby.exception_arg_error(),
                ipv6_in_ipv4_error(&parsed_ip),
            ));
        }

        let reader = self.get_reader()?;

        // Perform lookup
        match reader.lookup(parsed_ip) {
            Ok(Some(data)) => Ok(data.into_value()),
            Ok(None) => Ok(ruby.qnil().as_value()),
            Err(MaxMindDbError::InvalidDatabase(_)) | Err(MaxMindDbError::Io(_)) => {
                Err(Error::new(
                    ExceptionClass::from_value(invalid_database_error().as_value()).unwrap(),
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
        let ruby = magnus::Ruby::get().unwrap();

        // Check if database is closed
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::new(ruby.exception_runtime_error(), ERR_CLOSED_DB));
        }

        // Parse IP address
        let parsed_ip = parse_ip_address_fast(ip_address, &ruby)?;

        if self.ip_version == 4 && matches!(parsed_ip, IpAddr::V6(_)) {
            return Err(Error::new(
                ruby.exception_arg_error(),
                ipv6_in_ipv4_error(&parsed_ip),
            ));
        }

        let reader = self.get_reader()?;

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
            Err(MaxMindDbError::InvalidDatabase(_)) | Err(MaxMindDbError::Io(_)) => {
                Err(Error::new(
                    ExceptionClass::from_value(invalid_database_error().as_value()).unwrap(),
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
        let ruby = magnus::Ruby::get().unwrap();

        // Check if database is closed
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::new(ruby.exception_runtime_error(), ERR_CLOSED_DB));
        }

        let reader = self.get_reader()?;
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
        self.closed.store(true, Ordering::Release);
        let mut writer = self.reader.write().unwrap();
        *writer = None;
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn each(&self) -> Result<Value, Error> {
        let ruby = magnus::Ruby::get().unwrap();

        // Check if database is closed
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::new(ruby.exception_runtime_error(), ERR_CLOSED_DB));
        }

        // If no block given, return enumerator
        if !ruby.block_given() {
            return Err(Error::new(
                ruby.exception_runtime_error(),
                "Enumerator support not yet implemented, please provide a block",
            ));
        }

        let reader = self.get_reader()?;
        let ip_version = reader.metadata().ip_version;

        // For IPv4 databases, iterate over IPv4 range only
        // For IPv6 databases, iterate over IPv6 range only (includes IPv4-mapped addresses)
        let network_str = if ip_version == 4 {
            "0.0.0.0/0"
        } else {
            "::/0"
        };

        let network = IpNetwork::from_str(network_str).map_err(|e| {
            Error::new(
                ExceptionClass::from_value(invalid_database_error().as_value()).unwrap(),
                format!("Failed to create network: {}", e),
            )
        })?;

        let mut iter = reader.within(network).map_err(|e| {
            Error::new(
                ExceptionClass::from_value(invalid_database_error().as_value()).unwrap(),
                format!("Failed to iterate: {}", e),
            )
        })?;

        // Get IPAddr class
        let ipaddr_class = ruby.class_object().const_get::<_, RClass>("IPAddr")?;

        // Iterate over all networks
        while let Some(result) = iter.next() {
            match result {
                Ok(item) => {
                    // Convert IpNetwork to IPAddr
                    let ip_str = item.ip_net.to_string();
                    let ipaddr = ipaddr_class.funcall::<_, _, Value>("new", (ip_str,))?;

                    // Yield [network, data] to block
                    let values = (ipaddr, item.info.into_value());
                    ruby.yield_values::<(Value, Value), Value>(values)?;
                }
                Err(MaxMindDbError::InvalidDatabase(_)) | Err(MaxMindDbError::Io(_)) => {
                    return Err(Error::new(
                        ExceptionClass::from_value(invalid_database_error().as_value()).unwrap(),
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

    /// Helper method to get the reader from the Arc<RwLock<>>
    fn get_reader(&self) -> Result<Arc<ReaderSource>, Error> {
        let ruby = magnus::Ruby::get().unwrap();
        let reader_lock = self.reader.read().unwrap();
        match reader_lock.as_ref() {
            Some(reader) => Ok(Arc::clone(reader)),
            None => Err(Error::new(ruby.exception_runtime_error(), ERR_CLOSED_DB)),
        }
    }
}

unsafe impl Send for Reader {}

/// Helper function to create a Reader from a ReaderSource
fn create_reader(source: ReaderSource) -> Reader {
    let ip_version = source.metadata().ip_version;
    Reader {
        reader: Arc::new(RwLock::new(Some(Arc::new(source)))),
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
    if let Ok(ipaddr_obj) = value.funcall::<_, _, String>("to_s", ()) {
        return IpAddr::from_str(&ipaddr_obj).map_err(|_| {
            Error::new(
                ruby.exception_arg_error(),
                format!("'{}' does not appear to be an IPv4 or IPv6 address", ipaddr_obj),
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
    let ruby = magnus::Ruby::get().unwrap();

    let file = File::open(Path::new(path)).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            let errno = ruby.class_object().const_get::<_, RModule>("Errno").unwrap();
            let enoent = errno.const_get::<_, RClass>("ENOENT").unwrap();
            Error::new(
                ExceptionClass::from_value(enoent.as_value()).unwrap(),
                e.to_string(),
            )
        }
        _ => Error::new(ruby.exception_io_error(), e.to_string()),
    })?;

    let mmap = unsafe { Mmap::map(&file) }.map_err(|e| {
        Error::new(
            ruby.exception_io_error(),
            format!("Failed to memory-map database file: {}", e),
        )
    })?;

    let reader = MaxMindReader::from_source(mmap).map_err(|_| {
        Error::new(
            ExceptionClass::from_value(invalid_database_error().as_value()).unwrap(),
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
    let ruby = magnus::Ruby::get().unwrap();

    let mut file = File::open(Path::new(path)).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            let errno = ruby.class_object().const_get::<_, RModule>("Errno").unwrap();
            let enoent = errno.const_get::<_, RClass>("ENOENT").unwrap();
            Error::new(
                ExceptionClass::from_value(enoent.as_value()).unwrap(),
                e.to_string(),
            )
        }
        _ => Error::new(ruby.exception_io_error(), e.to_string()),
    })?;

    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).map_err(|e| {
        Error::new(
            ruby.exception_io_error(),
            format!("Failed to read database file: {}", e),
        )
    })?;

    let reader = MaxMindReader::from_source(buffer).map_err(|_| {
        Error::new(
            ExceptionClass::from_value(invalid_database_error().as_value()).unwrap(),
            format!(
                "Error opening database file ({}). Is this a valid MaxMind DB file?",
                path
            ),
        )
    })?;

    Ok(create_reader(ReaderSource::Memory(reader)))
}

/// Get the InvalidDatabaseError class
fn invalid_database_error() -> RClass {
    let ruby = magnus::Ruby::get().unwrap();
    let maxmind = ruby.class_object().const_get::<_, RModule>("MaxMind").unwrap();
    let db = maxmind.const_get::<_, RModule>("DB").unwrap();
    let rust = db.const_get::<_, RModule>("Rust").unwrap();
    rust.const_get::<_, RClass>("InvalidDatabaseError").unwrap()
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
            // Define Rust module directly as a constant on the class using funcall
            let rust_mod = ruby.define_module("MaxMindDBRustTemp")?;
            // Use const_set via funcall on the existing class/module
            let _ = existing.funcall::<_, _, Value>("const_set", ("Rust", rust_mod))?;
            rust_mod
        }
        Ok(existing) => {
            // MaxMind::DB exists as a Module (our gem loaded first)
            let db_mod = RModule::from_value(existing)
                .ok_or_else(|| Error::new(ruby.exception_type_error(), "MaxMind::DB is not a module"))?;
            db_mod.define_module("Rust")?
        }
        Err(_) => {
            // MaxMind::DB doesn't exist, define it as a module
            let db = maxmind.define_module("DB")?;
            db.define_module("Rust")?
        }
    };

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
    reader_class.define_method("each", magnus::method!(Reader::each, 0))?;

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
    metadata_class.define_method("node_byte_size", magnus::method!(Metadata::node_byte_size, 0))?;
    metadata_class.define_method("search_tree_size", magnus::method!(Metadata::search_tree_size, 0))?;

    // Define MODE constants
    rust.const_set("MODE_AUTO", ruby.to_symbol("MODE_AUTO"))?;
    rust.const_set("MODE_MEMORY", ruby.to_symbol("MODE_MEMORY"))?;
    rust.const_set("MODE_MMAP", ruby.to_symbol("MODE_MMAP"))?;

    Ok(())
}
