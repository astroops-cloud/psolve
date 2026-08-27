//! The psolve star index: HEALPix-bucketed, magnitude-sorted, mmap-friendly.

mod blind_grid;
pub mod builder;
pub mod error;
pub mod format;
pub mod gaia;
pub mod healpix;
pub mod quad_builder;
pub mod quad_format;
pub mod quad_reader;
pub mod reader;
pub mod record;
pub mod sha256;

pub use error::IndexError;
