pub mod abstract_value;
pub mod engine;
pub mod state;
pub mod transfer;

pub use abstract_value::Interval;
pub use engine::{
    analyze_interval, run_interval, state_before_location as query_interval_before_location,
};
pub use state::{IntervalAnalysisState, IntervalState};
