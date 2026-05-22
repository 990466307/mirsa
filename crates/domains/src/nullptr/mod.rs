pub mod abstract_value;
pub mod access_path;
pub mod condition_path;
pub mod engine;
pub mod state;
pub mod transfer;

pub use abstract_value::NullPtr;
pub use engine::{
    analyze_nullptr, run_nullptr, state_before_location as query_nullptr_before_location,
};
pub use state::NullPtrState;
