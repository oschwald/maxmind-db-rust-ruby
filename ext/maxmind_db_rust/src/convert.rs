//! Converting Ruby arguments that may be one of several types.

use magnus::{
    error::Error,
    rb_sys::{protect, AsRawValue, FromRawValue},
    RArray, RHash, TryConvert, Value,
};

/// Converts `value`, or returns `None` if it is not convertible to `T`.
///
/// Conversion can call Ruby methods such as `to_str`. Only a `TypeError` means
/// "not convertible". Any other error must reach the caller: dropping a pending
/// `throw` or thread kill and then calling Ruby again crashes the interpreter,
/// and dropping `Interrupt` would ignore Ctrl-C.
///
/// Use `try_convert_array` and `try_convert_hash` for those types.
#[inline]
pub(crate) fn try_convert_optional<T: TryConvert>(
    value: Value,
    ruby: &magnus::Ruby,
) -> Result<Option<T>, Error> {
    none_if_type_error(T::try_convert(value), ruby)
}

/// Like `try_convert_optional` for an Array.
///
/// The error from magnus's `RArray::try_convert` names the class of `value`. It
/// calls the class's `to_s` for that and drops anything `to_s` throws.
#[inline]
pub(crate) fn try_convert_array(
    value: Value,
    ruby: &magnus::Ruby,
) -> Result<Option<RArray>, Error> {
    if let Some(array) = RArray::from_value(value) {
        return Ok(Some(array));
    }
    // SAFETY: `value` is live, and `protect` catches anything that is raised or
    // thrown. The result is an Array or nil.
    let converted = protect(|| unsafe { rb_sys::rb_check_array_type(value.as_raw()) });
    let converted = none_if_type_error(converted, ruby)?;
    Ok(converted.and_then(|raw| RArray::from_value(unsafe { Value::from_raw(raw) })))
}

/// Like `try_convert_array` for a Hash, where magnus has the same problem.
pub(crate) fn try_convert_hash(value: Value, ruby: &magnus::Ruby) -> Result<Option<RHash>, Error> {
    // SAFETY: as in `try_convert_array`. The result is a Hash or nil.
    let converted = protect(|| unsafe { rb_sys::rb_check_hash_type(value.as_raw()) });
    let converted = none_if_type_error(converted, ruby)?;
    Ok(converted.and_then(|raw| RHash::from_value(unsafe { Value::from_raw(raw) })))
}

fn none_if_type_error<T>(
    result: Result<T, Error>,
    ruby: &magnus::Ruby,
) -> Result<Option<T>, Error> {
    match result {
        Ok(converted) => Ok(Some(converted)),
        Err(err) if err.is_kind_of(ruby.exception_type_error()) => Ok(None),
        Err(err) => Err(err),
    }
}
