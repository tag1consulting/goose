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
    GooseMethod, GooseRequest, GooseUser, Scenario, Transaction, TransactionError,
    TransactionFunction, TransactionResult,
};
pub use crate::metrics::{GooseCoordinatedOmissionMitigation, GooseMetrics};
pub use crate::{scenario, transaction, GooseAttack, GooseError, GooseScheduler};
