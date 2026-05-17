//! HTTP proxy server. See spec sections 2.1 and 5.

pub mod compress;
pub mod forward;
pub mod intercept;
pub mod mitm;
pub mod server;
pub mod session;

pub use intercept::InterceptState;

#[cfg(test)]
mod session_tests;

#[cfg(test)]
mod tests;
