pub mod backend;
pub mod guid;
pub mod internal;
pub mod registry;
pub mod variants;

pub use backend::DesktopBackend;
pub use guid::Guid;
pub use internal::{Apartment, InternalBackend};
