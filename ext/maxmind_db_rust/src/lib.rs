mod decoder;
mod metadata;
mod path;
mod reader;

use magnus::{error::Error, prelude::*, RModule, Value};

pub(crate) const STRING_CACHE_ROOTS_CONST: &str = "__STRING_CACHE_ROOTS__";
const MAP_KEY_ROOTS_CONST: &str = "__MAP_KEY_ROOTS__";
pub(crate) const STRING_CACHE_MAX: usize = 4096;
pub(crate) const STRING_CACHE_MIN_LEN: usize = 2;
pub(crate) const STRING_CACHE_MAX_LEN: usize = 64;

pub(crate) fn rust_module(ruby: &magnus::Ruby) -> RModule {
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
        let roots = ruby.ary_new_capa(STRING_CACHE_MAX);
        for _ in 0..STRING_CACHE_MAX {
            roots.push(ruby.qnil().as_value())?;
        }
        rust.const_set(STRING_CACHE_ROOTS_CONST, roots)?;
    }

    if rust.const_get::<_, Value>(MAP_KEY_ROOTS_CONST).is_ok() {
        let _ = rust.funcall::<_, _, Value>("send", ("remove_const", MAP_KEY_ROOTS_CONST))?;
    }

    // The extension can be loaded more than once from different paths.
    // Reusing previously defined classes avoids typed-data incompatibilities.
    if rust.const_get::<_, Value>("Reader").is_ok() {
        return Ok(());
    }

    reader::define(ruby, rust)?;
    metadata::define(ruby, rust)?;
    Ok(())
}
