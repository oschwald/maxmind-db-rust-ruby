use crate::{
    rust_module, STRING_CACHE_MAX, STRING_CACHE_MAX_LEN, STRING_CACHE_MIN_LEN,
    STRING_CACHE_ROOTS_CONST,
};
use ::maxminddb as maxminddb_crate;
use magnus::{prelude::*, IntoValue, RArray, RString, Value};
use rustc_hash::FxHasher;
use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::{
    cell::OnceCell,
    fmt,
    hash::{Hash, Hasher},
};

thread_local! {
    static STRING_CACHE_ROOTS: OnceCell<RArray> = const { OnceCell::new() };
}

#[inline]
fn global_string_cache_roots(ruby: &magnus::Ruby) -> RArray {
    let value = rust_module(ruby)
        .const_get::<_, Value>(STRING_CACHE_ROOTS_CONST)
        .expect("string cache roots constant should exist");
    RArray::from_value(value).expect("string cache roots constant should be an array")
}

#[inline]
fn string_cache_roots(ruby: &magnus::Ruby) -> RArray {
    STRING_CACHE_ROOTS.with(|roots| *roots.get_or_init(|| global_string_cache_roots(ruby)))
}

#[inline]
fn cached_utf8_string(ruby: &magnus::Ruby, value: &[u8]) -> Value {
    if !(STRING_CACHE_MIN_LEN..=STRING_CACHE_MAX_LEN).contains(&value.len()) {
        let string = ruby.enc_str_new(value, ruby.utf8_encoding());
        string.freeze();
        return string.into_value_with(ruby);
    }

    let mut hasher = FxHasher::default();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    let slot = (hash as usize) & (STRING_CACHE_MAX - 1);

    let roots = string_cache_roots(ruby);
    let cached = roots
        .entry::<Value>(slot as isize)
        .expect("string cache roots lookup should succeed");
    if let Some(cached) = RString::from_value(cached) {
        // SAFETY: the bytes are compared immediately while the globally rooted
        // frozen Ruby string remains alive and cannot be mutated.
        if unsafe { cached.as_slice() } == value {
            return cached.into_value_with(ruby);
        }
    }

    let string = ruby.enc_str_new(value, ruby.utf8_encoding());
    string.freeze();
    let cached = string.as_value();
    roots
        .store(slot as isize, cached)
        .expect("string cache roots update should succeed");
    cached
}

/// Wrapper that owns the Ruby value produced by deserializing a MaxMind record.
#[derive(Clone)]
pub(crate) struct RubyDecodedValue {
    value: Value,
}

impl RubyDecodedValue {
    #[inline]
    fn new(value: Value) -> Self {
        Self { value }
    }

    #[inline]
    pub(crate) fn into_value(self) -> Value {
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
        maxminddb_crate::deserialize_any_with_raw_strings(
            deserializer,
            RubyValueVisitor { ruby: self.ruby },
        )
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

    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer
            .deserialize_bytes(RubyUtf8StringVisitor { ruby: self.ruby })
            .map(RubyDecodedValue::new)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(cached_utf8_string(
            self.ruby,
            value.as_bytes(),
        )))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(RubyDecodedValue::new(cached_utf8_string(
            self.ruby,
            value.as_bytes(),
        )))
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
        let mut buffer = [self.ruby.qnil().as_value(); 128];
        let mut buffer_len = 0;
        while let Some(elem) = seq.next_element_seed(RubyValueSeed { ruby: self.ruby })? {
            buffer[buffer_len] = elem.into_value();
            buffer_len += 1;
            if buffer_len == buffer.len() {
                arr.cat(&buffer)
                    .map_err(|e| de::Error::custom(e.to_string()))?;
                buffer_len = 0;
            }
        }
        arr.cat(&buffer[..buffer_len])
            .map_err(|e| de::Error::custom(e.to_string()))?;
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
        let mut buffer = [self.ruby.qnil().as_value(); 128];
        let mut buffer_len = 0;
        while let Some(key_val) = map.next_key_seed(RubyMapKeySeed { ruby: self.ruby })? {
            let value = map.next_value_seed(RubyValueSeed { ruby: self.ruby })?;
            buffer[buffer_len] = key_val;
            buffer[buffer_len + 1] = value.into_value();
            buffer_len += 2;
            if buffer_len == buffer.len() {
                hash.bulk_insert(&buffer)
                    .map_err(|e| de::Error::custom(e.to_string()))?;
                buffer_len = 0;
            }
        }
        hash.bulk_insert(&buffer[..buffer_len])
            .map_err(|e| de::Error::custom(e.to_string()))?;
        Ok(RubyDecodedValue::new(hash.into_value_with(self.ruby)))
    }
}

struct RubyUtf8StringVisitor<'ruby> {
    ruby: &'ruby magnus::Ruby,
}

impl<'de, 'ruby> Visitor<'de> for RubyUtf8StringVisitor<'ruby> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("MMDB UTF-8 string bytes")
    }

    fn visit_borrowed_bytes<E>(self, value: &'de [u8]) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(cached_utf8_string(self.ruby, value))
    }
}

struct RubyMapKeySeed<'ruby> {
    ruby: &'ruby magnus::Ruby,
}

impl<'ruby, 'de> DeserializeSeed<'de> for RubyMapKeySeed<'ruby> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(RubyUtf8StringVisitor { ruby: self.ruby })
    }
}
