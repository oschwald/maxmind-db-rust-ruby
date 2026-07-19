mod decoder;
mod initialization;
mod metadata;
mod path;
mod reader;

use magnus::{error::Error, prelude::*, RModule, Value};

pub(crate) const STRING_CACHE_ROOTS_CONST: &str = "__STRING_CACHE_ROOTS__";
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
    initialization::initialize(ruby)
}
