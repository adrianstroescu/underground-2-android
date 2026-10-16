//! Readers for the data files shipped with Need for Speed: Underground 2 (PC).
//!
//! Everything in the game is stored as EA "bChunk" trees: `u32 id, u32 size, body`.
//! Ids with the top bit set are containers holding further chunks. This crate only
//! parses; it never needs the original executable.

pub mod audio;
pub mod bytes;
pub mod car;
pub mod carinfo;
pub mod chunk;
pub mod collision;
pub mod compression;
pub mod dxt;
pub mod fs;
pub mod hash;
pub mod light;
pub mod material;
pub mod parts;
pub mod route;
pub mod scenery;
pub mod solid;
pub mod tpk;
pub mod track;
pub mod visible;
pub mod envmap;
pub mod movie;
pub mod lang;

pub use chunk::{Chunk, ChunkIter};
pub use hash::bin_hash;

#[derive(Debug, Clone)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => { return Err($crate::Error(format!($($arg)*))) };
}
