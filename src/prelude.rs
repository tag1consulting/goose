//! A list of things that typically must be imported to write a Goose load test.
//!
//! Instead of manually importing everything each time you write a Goose load test,
//! you can simply import this prelude as follows:
//!
//! ```rust
//! use goose::prelude::*;
//! ```

pub use crate::config::{GooseDefault, GooseDefaultType};
pub use crate::goose::{
    GooseMethod, GooseRequest, GooseTask, GooseTaskError, GooseTaskFunction, GooseTaskResult,
    GooseTaskSet, GooseUser,
};
pub use crate::metrics::{GooseCoordinatedOmissionMitigation, GooseMetrics};
pub use crate::{task, taskset, GooseAttack, GooseError, GooseScheduler};
