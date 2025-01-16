#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/op-rs/hilo/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

#[macro_use]
extern crate tracing;

mod errors;
pub use errors::NodeError;

mod node;
pub use node::Node;

mod sync;
pub use sync::SyncMode;

mod config;
pub use config::{Config, ConfigError};
