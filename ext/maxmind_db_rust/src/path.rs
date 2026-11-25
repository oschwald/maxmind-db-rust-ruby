use crate::convert::{try_convert_array, try_convert_optional};
use magnus::{
    error::{Error, ErrorType},
    prelude::*,
    rb_sys::AsRawValue,
    Float, Integer, RArray, RComplex, RString, Symbol, Value,
};
use maxminddb::PathElement;
use rustc_hash::FxHasher;
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

pub(crate) const PATH_CACHE_MAX_ENTRIES: usize = 64;

#[derive(PartialEq, Eq)]
pub(crate) enum OwnedPathElement {
    Key(String),
    Index(usize),
    IndexFromEnd(usize),
}

pub(crate) struct CachedPath {
    pub(crate) hash: u64,
    pub(crate) elements: Arc<[OwnedPathElement]>,
}

pub(crate) fn path_array(path: Value, ruby: &magnus::Ruby) -> Result<RArray, Error> {
    try_convert_array(path, ruby)?.ok_or_else(|| {
        Error::new(
            ruby.exception_arg_error(),
            "Path must be an Array of String and Integer elements",
        )
    })
}

pub(crate) fn parse_path_array(
    path: RArray,
    ruby: &magnus::Ruby,
) -> Result<Vec<OwnedPathElement>, Error> {
    let mut elements = Vec::with_capacity(path.len());
    for index in 0..path.len() {
        let item = path.entry::<Value>(index as isize)?;
        if let Some(key) = try_convert_optional::<RString>(item, ruby)? {
            elements.push(OwnedPathElement::Key(key.to_string()?));
            continue;
        }
        if let Some(index) = path_index(item, ruby)? {
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

/// Returns `None` if `item` is not an Integer or is too large for an index.
#[inline]
fn path_index(item: Value, ruby: &magnus::Ruby) -> Result<Option<isize>, Error> {
    let index = match try_convert_optional::<Integer>(item, ruby) {
        Ok(Some(index)) => index,
        Ok(None) => return Ok(None),
        // A number such as Infinity, NaN or Complex(1, 1) has no Integer value.
        // Ruby's own `to_int` raises then, so this is not a caller's error.
        Err(err) if err.is_kind_of(ruby.exception_range_error()) && is_ruby_number(item) => {
            return Ok(None)
        }
        Err(err) if is_missing_to_i(&err, item, ruby)? => return Ok(None),
        Err(err) => return Err(err),
    };
    match index.to_isize() {
        Ok(index) => Ok(Some(index)),
        Err(err) if err.is_kind_of(ruby.exception_range_error()) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Whether `item` is a number other than an Integer whose `to_int` is Ruby's.
fn is_ruby_number(item: Value) -> bool {
    Float::from_value(item).is_some()
        || RComplex::from_value(item).is_some()
        // SAFETY: the name is used before any Ruby code can run.
        || unsafe { item.classname() } == "BigDecimal"
}

/// Whether `err` is from Numeric#to_int calling a `to_i` that `item`, an instance
/// of a subclass, does not have.
fn is_missing_to_i(err: &Error, item: Value, ruby: &magnus::Ruby) -> Result<bool, Error> {
    let exception = match err.error_type() {
        ErrorType::Exception(exception)
            if exception.is_kind_of(ruby.exception_no_method_error())
                && item.is_kind_of(ruby.class_numeric()) =>
        {
            *exception
        }
        _ => return Ok(false),
    };
    let name = Symbol::from_value(exception.funcall("name", ())?);
    if name != Some(ruby.to_symbol("to_i")) {
        return Ok(false);
    }
    match exception.funcall::<_, _, Value>("receiver", ()) {
        Ok(receiver) => Ok(receiver.as_raw() == item.as_raw()),
        // The exception was created without a receiver.
        Err(err) if err.is_kind_of(ruby.exception_arg_error()) => Ok(false),
        Err(err) => Err(err),
    }
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

/// `before_convert` runs before handling an element that is not a UTF-8 String,
/// because converting it can call Ruby or fail.
pub(crate) fn path_cache_hash(
    path: RArray,
    ruby: &magnus::Ruby,
    before_convert: impl Fn() -> Result<(), Error>,
) -> Result<u64, Error> {
    let mut hasher = FxHasher::default();
    path.len().hash(&mut hasher);

    for index in 0..path.len() {
        let item = path.entry::<Value>(index as isize)?;
        if RString::from_value(item).is_none() {
            before_convert()?;
        }
        if let Some(key) = try_convert_optional::<RString>(item, ruby)? {
            0_u8.hash(&mut hasher);
            hash_path_key(key, &mut hasher, &before_convert)?;
            continue;
        }
        if let Some(index) = path_index(item, ruby)? {
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

fn hash_path_key(
    key: RString,
    hasher: &mut FxHasher,
    before_convert: impl Fn() -> Result<(), Error>,
) -> Result<(), Error> {
    // SAFETY: the borrowed str is used only for immediate hashing and is not
    // stored across any call that could mutate or free the Ruby string.
    if let Some(key_str) = unsafe { key.test_as_str() } {
        key_str.hash(hasher);
    } else {
        before_convert()?;
        key.to_string()?.hash(hasher);
    }
    Ok(())
}

pub(crate) fn path_matches_cached(
    path: RArray,
    cached: &[OwnedPathElement],
    ruby: &magnus::Ruby,
) -> Result<bool, Error> {
    if path.len() != cached.len() {
        return Ok(false);
    }

    for (index, cached_element) in cached.iter().enumerate() {
        let item = path.entry::<Value>(index as isize)?;
        match cached_element {
            OwnedPathElement::Key(expected) => {
                let Some(key) = try_convert_optional::<RString>(item, ruby)? else {
                    return Ok(false);
                };
                if !path_key_matches(key, expected)? {
                    return Ok(false);
                }
            }
            OwnedPathElement::Index(_) | OwnedPathElement::IndexFromEnd(_) => {
                let Some(index) = path_index(item, ruby)? else {
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

pub(crate) fn path_elements_from_owned_path(path: &[OwnedPathElement]) -> Vec<PathElement<'_>> {
    path.iter()
        .map(|element| match element {
            OwnedPathElement::Key(key) => PathElement::Key(key.as_str()),
            OwnedPathElement::Index(index) => PathElement::Index(*index),
            OwnedPathElement::IndexFromEnd(index) => PathElement::IndexFromEnd(*index),
        })
        .collect()
}
