use crate::{metadata, reader, STRING_CACHE_MAX, STRING_CACHE_ROOTS_CONST};
use magnus::{
    error::Error,
    prelude::*,
    rb_sys::{protect, AsRawId, AsRawValue, FromRawValue},
    RModule, Value,
};

const MAP_KEY_ROOTS_CONST: &str = "__MAP_KEY_ROOTS__";

pub(crate) fn initialize(ruby: &magnus::Ruby) -> Result<(), Error> {
    // Define module hierarchy: MaxMind::DB::Rust
    // Handle case where official maxmind-db gem may have already defined MaxMind::DB as a Class
    let maxmind = ruby.define_module("MaxMind")?;

    // Try to get or define DB - it might be a Class (official gem) or Module (ours)
    let rust = match own_constant(maxmind.as_value(), "DB")? {
        Some(existing) if existing.is_kind_of(ruby.class_class()) => {
            // MaxMind::DB exists as a Class (official gem loaded first)
            // Reuse existing Rust constant if present to avoid replacing classes.
            if let Some(rust_value) = own_constant(existing, "Rust")? {
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
        Some(existing) => {
            // MaxMind::DB exists as a Module (our gem loaded first)
            let db_mod = RModule::from_value(existing).ok_or_else(|| {
                Error::new(ruby.exception_type_error(), "MaxMind::DB is not a module")
            })?;
            db_mod.define_module("Rust")?
        }
        None => {
            // MaxMind::DB doesn't exist, define it as a module
            let db = maxmind.define_module("DB")?;
            db.define_module("Rust")?
        }
    };

    let roots = match own_constant(rust.as_value(), STRING_CACHE_ROOTS_CONST)? {
        Some(existing) => existing,
        None => {
            let roots = ruby.ary_new_capa(STRING_CACHE_MAX);
            for _ in 0..STRING_CACHE_MAX {
                roots.push(ruby.qnil().as_value())?;
            }
            rust.const_set(STRING_CACHE_ROOTS_CONST, roots)?;
            roots.as_value()
        }
    };
    // Decoder threads keep this array's handle in a Rust thread-local that Ruby
    // cannot see or update. Registering it as a permanent mark object pins it,
    // so GC compaction can never move it and leave that handle stale. The
    // constant only makes it reachable from Ruby. Register on every load, since
    // an existing array may come from an earlier build that did not pin it;
    // registering the same object again is harmless.
    ruby.gc_register_mark_object(roots);

    if own_constant(rust.as_value(), MAP_KEY_ROOTS_CONST)?.is_some() {
        let _ = rust.funcall::<_, _, Value>("send", ("remove_const", MAP_KEY_ROOTS_CONST))?;
    }

    // The extension can be loaded more than once from different paths.
    // Reusing previously defined classes avoids typed-data incompatibilities.
    if own_constant(rust.as_value(), "Reader")?.is_some() {
        return Ok(());
    }

    reader::define(ruby, rust)?;
    metadata::define(ruby, rust)?;
    Ok(())
}

/// Returns the constant `name` defined in `module` itself, if any.
///
/// A plain `const_get` on a module also finds top-level constants, such as an
/// application's own `DB`. Its error can also come from an autoload.
fn own_constant(module: Value, name: &str) -> Result<Option<Value>, Error> {
    let ruby = magnus::Ruby::get_with(module);
    let id = ruby.intern(name).as_raw();
    let module = module.as_raw();
    let mut defined = false;
    // SAFETY: `module` is a live Module or Class, `id` is an interned symbol,
    // and `protect` catches anything that an autoload raises or throws.
    let constant = protect(|| unsafe {
        // Run an autoload first. It may not define the constant, and then
        // getting the constant would call `const_missing`.
        rb_sys::rb_autoload_load(module, id);
        defined = rb_sys::rb_const_defined_at(module, id) != 0;
        if defined {
            rb_sys::rb_const_get_at(module, id)
        } else {
            rb_sys::Qnil as rb_sys::VALUE
        }
    })?;
    if !defined {
        return Ok(None);
    }
    // SAFETY: `constant` is the live value of the constant.
    Ok(Some(unsafe { Value::from_raw(constant) }))
}
