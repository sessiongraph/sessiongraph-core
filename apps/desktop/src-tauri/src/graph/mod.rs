//! Session graph extraction and injection. See spec sections 2.4–2.6.

pub mod extractor;
pub mod injector;
pub mod schema;

pub use schema::SessionGraph;

#[cfg(test)]
mod tests;
