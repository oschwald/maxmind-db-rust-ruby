use magnus::{error::Error, prelude::*, RHash, RModule};
use std::collections::BTreeMap;

/// Metadata about the MaxMind DB database.
#[derive(Clone)]
#[magnus::wrap(class = "MaxMind::DB::Rust::Metadata")]
pub(crate) struct Metadata {
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
    pub(crate) fn from_maxmind(meta: &maxminddb::Metadata) -> Self {
        Self {
            binary_format_major_version: meta.binary_format_major_version,
            binary_format_minor_version: meta.binary_format_minor_version,
            build_epoch: meta.build_epoch,
            database_type: meta.database_type.clone(),
            description_map: meta.description.clone(),
            ip_version: meta.ip_version,
            languages_list: meta.languages.clone(),
            node_count: meta.node_count,
            record_size: meta.record_size,
        }
    }

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

    fn description(&self) -> Result<RHash, Error> {
        let ruby = magnus::Ruby::get().expect("Ruby VM should be available in Ruby method");
        let hash = ruby.hash_new();
        for (k, v) in &self.description_map {
            hash.aset(k.as_str(), v.as_str())?;
        }
        Ok(hash)
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

pub(crate) fn define(ruby: &magnus::Ruby, rust: RModule) -> Result<(), Error> {
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
    Ok(())
}
