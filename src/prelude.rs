//! A list of things that typically must be imported to write a Goose load test.
//!
//! Instead of manually importing everything each time you write a Goose load test,
//! you can simply import this prelude as follows:
//!
//! ```rust
//! use goose::prelude::*;
//! ```

pub use crate::goose::{
    GooseTask, GooseTaskError, GooseTaskFunction, GooseTaskResult, GooseTaskSet, GooseUser,
};
pub use crate::metrics::GooseMetrics;
pub use crate::{
    task, taskset, GooseAttack, GooseDefault, GooseDefaultType, GooseError, GooseTaskSetScheduler,
};
