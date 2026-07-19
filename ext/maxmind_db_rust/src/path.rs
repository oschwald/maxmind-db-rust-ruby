use magnus::{error::Error, prelude::*, RArray, RString, Value};
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
    RArray::try_convert(path).map_err(|_| {
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

pub(crate) fn path_cache_hash(path: RArray, ruby: &magnus::Ruby) -> Result<u64, Error> {
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

pub(crate) fn path_matches_cached(
    path: RArray,
    cached: &[OwnedPathElement],
) -> Result<bool, Error> {
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

pub(crate) fn path_elements_from_owned_path(path: &[OwnedPathElement]) -> Vec<PathElement<'_>> {
    path.iter()
        .map(|element| match element {
            OwnedPathElement::Key(key) => PathElement::Key(key.as_str()),
            OwnedPathElement::Index(index) => PathElement::Index(*index),
            OwnedPathElement::IndexFromEnd(index) => PathElement::IndexFromEnd(*index),
        })
        .collect()
}
