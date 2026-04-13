pub mod abstract_value;
pub mod condition_path;
pub mod engine;
pub mod state;
pub mod transfer;
pub mod warnings;

pub use abstract_value::NullPtr;
pub use engine::run_nullptr;
pub use state::NullPtrState;
