//! The psolve solve pipeline: FITS bytes in, a verified TAN WCS out.
//!
//! This crate deliberately has NO filesystem access and NO dependencies. It
//! takes `&[u8]` and returns values, which is what makes "the solver cannot
//! modify your data" a structural property rather than a promise, and what
//! lets the whole pipeline be tested without fixtures on disk.

pub mod error;
pub mod fits;
pub mod background;
pub mod extract;
pub mod quad;
pub mod project;
pub mod fit;
pub mod match_;
pub mod pairmatch;
pub mod verify;
pub mod solve;
pub mod blind;

pub use error::{ReasonCode, SolveError};
pub use fits::{field_height_deg, field_width_deg};
pub use solve::{solve, CatalogStar, Outcome, SolveOptions, Solution, Timings};
