//! Optional telnet and WebSocket Controller threads.
//!
//! By default, Goose launches both a telnet Controller and a WebSocket Controller, allowing
//! real-time control of the running load test.

use crate::config::GooseConfiguration;
use crate::metrics::{GooseMetrics, MetricsCommand};
use crate::test_plan::TestPlan;
use crate::GooseError;
use crate::{AttackPhase, ControlOutcome, ControlStartBegin, GooseAttack, GooseAttackRunState};

use futures::{SinkExt, StreamExt};
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io;
use std::str::{self, FromStr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::protocol::{
    frame::coding::CloseCode, CloseFrame, WebSocketConfig,
};
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};
use tokio_tungstenite::WebSocketStream;

/// Most users any front end can set: `users`, a test plan's total, and the
/// dashboard's users control. One command allocates the added users on the
/// main loop, so a larger value can exhaust memory or overflow the user
/// arithmetic.
pub(crate) const MAX_CONTROL_USERS: usize = 100_000;

/// Refusal for a user count outside 1 to [`MAX_CONTROL_USERS`].
pub(crate) const INVALID_USERS_MSG: &str = "users must be an integer between 1 and 100000";

/// How long a client waits for the parent to answer a command.
///
/// On a timeout the client is told so, and the parent skips the command if
/// it is still queued. If the parent had already started applying it (a
/// large users change), it can still apply after the timeout, so clients
/// must check the phase and users before retrying.
///
/// 15s allows non-trivial `weight_scenario_users` work; Start still replies
/// before `control_start_finish`, so a client usually hears much sooner.
pub(crate) const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);

/// Reply to a line that is not a command.
const UNRECOGNIZED: &str = "unrecognized command";

/// The help a telnet or WebSocket client gets for `help` or `?`.
const HELP: &str = concat!(
    env!("CARGO_PKG_NAME"),
    " ",
    env!("CARGO_PKG_VERSION"),
    " controller commands:\n",
    "help               this help\n",
    "exit               exit controller\n\n",
    "start              start an idle load test\n",
    "stop               stop a running load test and return to idle state\n",
    "shutdown           shutdown load test and exit controller\n\n",
    "host HOST          set host to load test, (ie https://web.site/)\n",
    "increaserate FLOAT set per-second rate users increase (0.01 to 100000)\n",
    "increase-time TIME set total time to take increasing users (0 clears)\n",
    "decreaserate FLOAT set per-second rate users decrease (0.01 to 100000)\n",
    "decrease-time TIME set total time to take decreasing users (0 clears)\n",
    "users INT          set number of simulated users (1 to 100000)\n",
    "runtime TIME       set how long to hold users after the ramp up, (ie 1h30m5s, 0 clears)\n",
    "test-plan PLAN     define or replace test-plan, (ie 10,5m;10,1h;0,30s)\n\n",
    "config             display load test configuration\n",
    "config-json        display load test configuration in json format\n",
    "metrics            display metrics for current load test\n",
    "metrics-json       display metrics for current load test in json format",
);

/// All commands Goose can be controlled with, each carrying its parsed and
/// bounded value.
///
/// Telnet and WebSocket clients send a command as text, which
/// [`FromStr`](#impl-FromStr-for-ControllerCommand) parses; commands and
/// their aliases are not case sensitive. When sending commands to the
/// WebSocket Controller, they must be formatted as json as defined by
/// [ControllerWebSocketRequest](./struct.ControllerWebSocketRequest.html).
/// The dashboard builds the same commands from its control requests. `help`
/// and `exit` are answered by the Controller itself, so are not commands.
///
/// GOOSE DEVELOPER NOTE: The following steps are required to add a new command:
///  1. Define the new command here, carrying its value already parsed and
///     bounded.
///  2. Parse it in `ControllerCommand::from_str`, with a value parser that
///     returns the error a client is sent, and add a line to `HELP`.
///  3. Apply it in `GooseAttack::control` (also in this file), which answers
///     with a `ControlOutcome` whose message every front end shows.
///  4. Add it to the parse table test in this file, and a test for the new
///     command in tests/controller.rs.
#[derive(Clone, Debug)]
pub enum ControllerCommand {
    /// Start an idle test.
    ///
    /// # Example
    /// Starts an idle load test.
    /// ```notest
    /// start
    /// ```
    ///
    /// Goose must be idle to process this command.
    Start,
    /// Stop a running test, putting it into an idle state.
    ///
    /// # Example
    /// Stops a running (or starting) load test.
    /// ```notest
    /// stop
    /// ```
    ///
    /// Goose must be running (or starting) to process this command.
    Stop,
    /// Tell the load test to shut down (which will disconnect the controller).
    ///
    /// # Example
    /// Terminates the Goose process, cleanly shutting down the load test if running.
    /// ```notest
    /// shutdown
    /// ```
    ///
    /// Goose can process this command at any time.
    Shutdown,
    /// Shut Goose down from Idle: the dashboard's Quit. Controllers have no
    /// text for it, as `quit` disconnects a Controller client.
    ///
    /// Goose must be idle to process this command.
    Quit,
    /// Configure the host to load test, a valid http or https url.
    ///
    /// # Example
    /// Tells Goose to generate load against <http://example.com/>.
    /// ```notest
    /// host http://example.com/
    /// ```
    ///
    /// Goose must be idle to process this command.
    Host(String),
    /// Configure how quickly new [`GooseUser`](../goose/struct.GooseUser.html)s
    /// are launched, per second: from 0.01 to 100000.
    ///
    /// # Example
    /// Tells Goose to launch a new user every 1.25 seconds.
    /// ```notest
    /// increaserate 1.25
    /// ```
    ///
    /// Goose can be idle or running when processing this command.
    IncreaseRate(f32),
    /// Configure how long to take to launch all
    /// [`GooseUser`](../goose/struct.GooseUser.html)s, in seconds: from 1
    /// second to 30 days, or 0 to clear it.
    ///
    /// # Example
    /// Tells Goose to increase users over 30 seconds.
    /// ```notest
    /// increasetime 30s
    /// ```
    ///
    /// Goose must be idle to process this command.
    IncreaseTime(usize),
    /// Configure how quickly [`GooseUser`](../goose/struct.GooseUser.html)s are
    /// stopped, per second: from 0.01 to 100000.
    ///
    /// # Example
    /// Tells Goose to stop one user every 2 seconds.
    /// ```notest
    /// decreaserate 0.5
    /// ```
    ///
    /// Goose can be idle or running when processing this command.
    DecreaseRate(f32),
    /// Configure how long to take to stop all
    /// [`GooseUser`](../goose/struct.GooseUser.html)s, in seconds: from 1
    /// second to 30 days, or 0 to clear it.
    ///
    /// # Example
    /// Tells Goose to decrease users over 30 seconds.
    /// ```notest
    /// decreasetime 30s
    /// ```
    ///
    /// Goose must be idle to process this command.
    DecreaseTime(usize),
    /// Configure how many [`GooseUser`](../goose/struct.GooseUser.html)s to
    /// launch, 1 to 100000.
    ///
    /// # Example
    /// Tells Goose to simulate 100 concurrent users.
    /// ```notest
    /// users 100
    /// ```
    ///
    /// Can be configured on an idle or running load test.
    Users(usize),
    /// Configure how long to hold all users after the ramp up, before the
    /// ramp down, in seconds: from 1 second to 30 days, or 0 to clear it.
    ///
    /// # Example
    /// Tells Goose to hold all users for 1 minute, then ramp down.
    /// ```notest
    /// runtime 60
    /// ```
    ///
    /// Goose must be idle to process this command: the run time is read when
    /// the load test starts.
    RunTime(usize),
    /// Define a load test plan: at most 100 steps of up to 30 days each, in
    /// at most 4096 bytes, starting 1 to 100000 users in total. This will
    /// replace the previously configured test plan, if any.
    ///
    /// # Example
    /// Tells Goose to launch 10 users in 5 seconds, maintain them for 30 seconds, and then spend 5 seconds
    /// stopping them.
    /// ```notest
    /// testplan 10,5s;10,30s;0,5s
    /// ```
    ///
    /// Can be configured on an idle or running load test.
    TestPlan(TestPlan),
    /// Display the current [`GooseConfiguration`](../struct.GooseConfiguration.html)s.
    ///
    /// # Example
    /// Returns the current Goose configuration.
    /// ```notest
    /// config
    /// ```
    Config,
    /// Display the current [`GooseConfiguration`](../struct.GooseConfiguration.html)s in json format.
    ///
    /// # Example
    /// Returns the current Goose configuration in json format.
    /// ```notest
    /// configjson
    /// ```
    ///
    /// This command can be run at any time.
    ConfigJson,
    /// Display the current [`GooseMetric`](../metrics/struct.GooseMetrics.html)s.
    ///
    /// # Example
    /// Returns the current Goose metrics.
    /// ```notest
    /// metrics
    /// ```
    ///
    /// This command can be run at any time.
    Metrics,
    /// Display the current [`GooseMetric`](../metrics/struct.GooseMetrics.html)s in json format.
    ///
    /// # Example
    /// Returns the current Goose metrics in json format.
    /// ```notest
    /// metricsjson
    /// ```
    ///
    /// This command can be run at any time.
    MetricsJson,
}

impl ControllerCommand {
    /// `users`, refused unless 1 to [`MAX_CONTROL_USERS`].
    pub(crate) fn users(users: u64) -> Result<Self, &'static str> {
        if (1..=MAX_CONTROL_USERS as u64).contains(&users) {
            Ok(Self::Users(users as usize))
        } else {
            Err(INVALID_USERS_MSG)
        }
    }
}

lazy_static! {
    // The values the Controller regexes on main accepted, in ASCII digits
    // only: a value outside them is refused before it is parsed, so no other
    // text reaches a parser.
    static ref USERS: Regex = Regex::new(r"^[0-9]+$").unwrap();
    static ref RATE: Regex = Regex::new(r"^[0-9]*(\.[0-9]*)?$").unwrap();
    static ref TIME: Regex = Regex::new(r"(?i-u)^([0-9]+|([0-9]+h)?([0-9]+m)?([0-9]+s)?)$").unwrap();
    static ref PLAN: Regex = Regex::new(
        r"(?-u)^(([0-9]+)\s*,\s*([0-9]+|([0-9]+h)?([0-9]+m)?([0-9]+s)?)*;*)+$"
    )
    .unwrap();
    static ref HOST: Regex = Regex::new(r"(?i)^https?://.+$").unwrap();
}

/// Slowest rate any front end can set, in users per second.
pub(crate) const MIN_CONTROL_RATE: f32 = 0.01;

/// Fastest rate any front end can set, in users per second.
pub(crate) const MAX_CONTROL_RATE: f32 = 100_000.0;

/// Longest increase time, decrease time, run time or test plan step any
/// front end can set: 30 days, in seconds.
pub(crate) const MAX_CONTROL_SECS: usize = 30 * 24 * 60 * 60;

/// Most steps in a test plan any front end can set.
pub(crate) const MAX_CONTROL_PLAN_STEPS: usize = 100;

/// Longest test plan text any front end can set, in bytes.
pub(crate) const MAX_CONTROL_PLAN_BYTES: usize = 4 * 1024;

/// Refusal for a host that is not an http or https url, or that carries a
/// user name or password.
pub(crate) const INVALID_HOST_MSG: &str =
    "host must be a valid url starting with http:// or https://, without a user name or password";

/// Refusal for a test plan outside the bounds.
pub(crate) const INVALID_PLAN_MSG: &str = "test-plan must be users,time steps such as 10,5m;10,1h;0,30s: at most 100 steps of up to 30 days each, 4096 bytes, and 1 to 100000 users in total";

/// `value` when `regex` accepts it, otherwise `error`.
fn grammar<'a>(
    regex: &Regex,
    value: &'a str,
    error: &'static str,
) -> Result<&'a str, &'static str> {
    regex.is_match(value).then_some(value).ok_or(error)
}

/// A host to load test: an http or https url with no user name or password,
/// which would show to observers and in reports.
pub(crate) fn host(value: &str) -> Result<String, &'static str> {
    let host = grammar(&HOST, value, INVALID_HOST_MSG)?;
    match url::Url::parse(host) {
        Ok(url) if url.username().is_empty() && url.password().is_none() => Ok(host.to_string()),
        _ => Err(INVALID_HOST_MSG),
    }
}

/// A rate in users per second, from [`MIN_CONTROL_RATE`] to
/// [`MAX_CONTROL_RATE`] once read as the `f32` Goose ramps with.
pub(crate) fn rate(value: &str, error: &'static str) -> Result<f32, &'static str> {
    let rate = grammar(&RATE, value, error)?
        .parse::<f32>()
        .map_err(|_| error)?;
    bounded_rate(rate, error)
}

/// `rate` when it is from [`MIN_CONTROL_RATE`] to [`MAX_CONTROL_RATE`];
/// `NaN` and infinity are not.
pub(crate) fn bounded_rate(rate: f32, error: &'static str) -> Result<f32, &'static str> {
    if (MIN_CONTROL_RATE..=MAX_CONTROL_RATE).contains(&rate) {
        Ok(rate)
    } else {
        Err(error)
    }
}

/// A time span in seconds: whole seconds, or hours, minutes and seconds in
/// that order, up to [`MAX_CONTROL_SECS`]. 0 is accepted, and means unset.
pub(crate) fn time(value: &str, error: &'static str) -> Result<usize, &'static str> {
    if value.is_empty() {
        return Err(error);
    }
    let value = grammar(&TIME, value, error)?.to_ascii_lowercase();
    match crate::util::parse_timespan(&value) {
        Some(seconds) if seconds <= MAX_CONTROL_SECS => Ok(seconds),
        _ => Err(error),
    }
}

/// A test plan: at most [`MAX_CONTROL_PLAN_STEPS`] steps of up to
/// [`MAX_CONTROL_SECS`] each, [`MAX_CONTROL_PLAN_BYTES`] of text, and 1 to
/// [`MAX_CONTROL_USERS`] users in total.
pub(crate) fn test_plan(value: &str) -> Result<TestPlan, &'static str> {
    if value.len() > MAX_CONTROL_PLAN_BYTES {
        return Err(INVALID_PLAN_MSG);
    }
    // The quiet parse: the command line's prints the line it refuses.
    let plan =
        TestPlan::parse(grammar(&PLAN, value, INVALID_PLAN_MSG)?).map_err(|_| INVALID_PLAN_MSG)?;
    let bounded = plan.steps.len() <= MAX_CONTROL_PLAN_STEPS
        && plan
            .steps
            .iter()
            .all(|&(_, ms)| ms <= MAX_CONTROL_SECS * 1_000)
        && plan.total_users() <= MAX_CONTROL_USERS;
    if bounded {
        Ok(plan)
    } else {
        Err(INVALID_PLAN_MSG)
    }
}

/// Refusal for an increase rate outside the bounds.
pub(crate) const INVALID_INCREASE_RATE_MSG: &str =
    "increase_rate must be a number from 0.01 to 100000";
/// Refusal for a decrease rate outside the bounds.
pub(crate) const INVALID_DECREASE_RATE_MSG: &str =
    "decrease_rate must be a number from 0.01 to 100000";
/// Refusal for an increase time outside the bounds.
pub(crate) const INVALID_INCREASE_TIME_MSG: &str =
    "increase_time must be a time span such as 1h30m5s, from 1 second to 30 days, or 0 to clear";
/// Refusal for a decrease time outside the bounds.
pub(crate) const INVALID_DECREASE_TIME_MSG: &str =
    "decrease_time must be a time span such as 1h30m5s, from 1 second to 30 days, or 0 to clear";
/// Refusal for a run time outside the bounds.
pub(crate) const INVALID_RUN_TIME_MSG: &str =
    "run_time must be a time span such as 1h30m5s, from 1 second to 30 days, or 0 to clear";

/// Parse a line from a telnet or WebSocket client into a command, or the
/// error the client is sent back.
impl FromStr for ControllerCommand {
    type Err = &'static str;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        // A verb, then one space and its value if it takes one.
        let line = line.trim();
        let (verb, value) = line.split_once(' ').unwrap_or((line, ""));
        // A command that takes no value.
        let alone = |command| match value {
            "" => Ok(command),
            _ => Err(UNRECOGNIZED),
        };
        match verb.to_ascii_lowercase().as_str() {
            "start" => alone(Self::Start),
            "stop" => alone(Self::Stop),
            "shutdown" => alone(Self::Shutdown),
            "config" => alone(Self::Config),
            "configjson" | "config-json" => alone(Self::ConfigJson),
            "metrics" | "stats" => alone(Self::Metrics),
            "metricsjson" | "metrics-json" | "statsjson" | "stats-json" => alone(Self::MetricsJson),
            "host" | "hostname" | "host_name" | "host-name" => host(value).map(Self::Host),
            "users" | "user" => grammar(&USERS, value, INVALID_USERS_MSG)?
                .parse()
                .map_err(|_| INVALID_USERS_MSG)
                .and_then(Self::users),
            "increaserate" | "increase_rate" | "increase-rate" => {
                rate(value, INVALID_INCREASE_RATE_MSG).map(Self::IncreaseRate)
            }
            "decreaserate" | "decrease_rate" | "decrease-rate" => {
                rate(value, INVALID_DECREASE_RATE_MSG).map(Self::DecreaseRate)
            }
            "increasetime" | "increase_time" | "increase-time" => {
                time(value, INVALID_INCREASE_TIME_MSG).map(Self::IncreaseTime)
            }
            "decreasetime" | "decrease_time" | "decrease-time" => {
                time(value, INVALID_DECREASE_TIME_MSG).map(Self::DecreaseTime)
            }
            "run" | "runtime" | "run_time" | "run-time" => {
                time(value, INVALID_RUN_TIME_MSG).map(Self::RunTime)
            }
            "testplan" | "test_plan" | "test-plan" | "plan" => test_plan(value).map(Self::TestPlan),
            _ => Err(UNRECOGNIZED),
        }
    }
}

/// The parent's answer to a [`ControllerCommand`].
pub(crate) enum ControlReply {
    /// Whether the command was applied, and the message every front end shows.
    Outcome(ControlOutcome),
    /// The load test configuration, for `config` and `config-json`.
    Config(Box<GooseConfiguration>),
    /// Current load test metrics, for `metrics` and `metrics-json`, shared
    /// by every client that asked in the same tick.
    Metrics(Arc<GooseMetrics>),
}

/// A command and where to send the parent's reply, on the one channel that
/// telnet, WebSocket and the dashboard share.
pub(crate) type ControlRequest = (ControllerCommand, oneshot::Sender<ControlReply>);

/// Why [`send_command`] has no reply.
pub(crate) enum SendError {
    /// The parent has gone or dropped the request.
    Unavailable,
    /// The parent did not answer within [`CONTROL_TIMEOUT`].
    Timeout,
}

/// Send `command` to the parent and wait up to `timeout`, [`CONTROL_TIMEOUT`]
/// outside tests, for its reply.
pub(crate) async fn send_command(
    channel_tx: &flume::Sender<ControlRequest>,
    command: ControllerCommand,
    timeout: Duration,
) -> Result<ControlReply, SendError> {
    let (respond, reply) = oneshot::channel();
    channel_tx
        .send((command, respond))
        .map_err(|_| SendError::Unavailable)?;
    match tokio::time::timeout(timeout, reply).await {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(_)) => Err(SendError::Unavailable),
        Err(_) => Err(SendError::Timeout),
    }
}

/// Most requests the parent answers in one turn, so a flood cannot hold the
/// main loop: a tick has two control turns, the dashboard's and then the
/// Controllers', and one turn on the dashboard's snapshot channel.
pub(crate) const MAX_DRAIN: usize = 16;

/// The parent process side of the Controller functionality.
impl GooseAttack {
    /// Answer up to [`MAX_DRAIN`] queued dashboard commands, then up to
    /// [`MAX_DRAIN`] more, telnet and WebSocket commands among them: the
    /// dashboard first, and again before each Controller command, as the
    /// Controllers take no authentication. A request whose client has gone
    /// (timed out or disconnected) is skipped, so it is not applied after the
    /// client was told it failed, and does not count toward [`MAX_DRAIN`]; no
    /// more of them are skipped in a turn than were queued when it began.
    ///
    /// Every `metrics` and `metrics-json` in the tick is answered from one
    /// metrics processor request, made after the tick's commands, so it shows
    /// the metrics as they stand at the end of the tick.
    pub(crate) async fn handle_control_requests(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        let mut metrics_waiters = Vec::new();
        let mut result = Ok(());
        let pending = |rx: &Option<flume::Receiver<ControlRequest>>| rx.as_ref()?.try_recv().ok();
        let queued = |rx: &Option<flume::Receiver<ControlRequest>>| {
            rx.as_ref().map_or(0, flume::Receiver::len)
        };
        'drain: for dashboard in [true, false] {
            // A skipped request is not counted, so requests whose clients
            // have gone leave the queue at once, but no more of them than
            // were queued when the turn began, so the turn ends however fast
            // more arrive.
            let mut skips = queued(&goose_attack_run_state.dashboard_control_rx)
                + if dashboard {
                    0
                } else {
                    queued(&goose_attack_run_state.controller_channel_rx)
                };
            let mut answered = 0;
            while answered < MAX_DRAIN {
                // The dashboard's next request, else in the Controllers' turn
                // theirs, so no queued Controller command runs ahead of a
                // dashboard command.
                let state = &*goose_attack_run_state;
                let next = match pending(&state.dashboard_control_rx) {
                    None if !dashboard => pending(&state.controller_channel_rx),
                    next => next,
                };
                let Some((command, respond)) = next else {
                    break;
                };
                if respond.is_closed() {
                    debug!("[control]: {command:?} skipped, its client has gone");
                    if skips == 0 {
                        break;
                    }
                    skips -= 1;
                    continue;
                }
                answered += 1;
                info!("[control]: {command:?}");
                result = self
                    .control(
                        goose_attack_run_state,
                        command,
                        respond,
                        &mut metrics_waiters,
                    )
                    .await;
                if result.is_err() {
                    break 'drain;
                }
            }
        }
        // Answered even after a hard error, so no client waits for nothing.
        if !metrics_waiters.is_empty() {
            let metrics = Arc::new(self.metrics_for_controller(goose_attack_run_state).await);
            for respond in metrics_waiters {
                let _ = respond.send(ControlReply::Metrics(Arc::clone(&metrics)));
            }
        }
        result
    }

    /// Apply one command and reply to it. A hard error is answered as an
    /// `internal` outcome, then returned. `metrics` and `metrics-json` are
    /// added to `metrics_waiters`, all answered at the end of the tick.
    async fn control(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
        command: ControllerCommand,
        respond: oneshot::Sender<ControlReply>,
        metrics_waiters: &mut Vec<oneshot::Sender<ControlReply>>,
    ) -> Result<(), GooseError> {
        // Any command may change the configuration: the snapshot's run
        // settings are built again when next asked for.
        #[cfg(feature = "dashboard")]
        {
            self.settings = None;
        }
        // An accepted Start is answered first and then finished, so the
        // client is not blocked on reset_run_state and test_start.
        let mut start = false;
        let outcome = match command {
            ControllerCommand::Config | ControllerCommand::ConfigJson => {
                let config = Box::new(self.configuration.clone());
                let _ = respond.send(ControlReply::Config(config));
                return Ok(());
            }
            ControllerCommand::Metrics | ControllerCommand::MetricsJson => {
                metrics_waiters.push(respond);
                return Ok(());
            }
            ControllerCommand::Start => match self.control_start_begin(goose_attack_run_state) {
                Ok(ControlStartBegin::Accepted(outcome)) => {
                    start = true;
                    Ok(outcome)
                }
                Ok(ControlStartBegin::Rejected(outcome)) => Ok(outcome),
                Err(e) => Err(e),
            },
            ControllerCommand::Stop => self.control_stop(goose_attack_run_state).await,
            ControllerCommand::Quit => Ok(self.control_quit(goose_attack_run_state)),
            ControllerCommand::Shutdown => self.control_shutdown(goose_attack_run_state).await,
            ControllerCommand::Users(users) => {
                self.control_set_users(goose_attack_run_state, users)
            }
            ControllerCommand::TestPlan(plan) => {
                self.control_set_plan(goose_attack_run_state, plan)
            }
            command => Ok(self.control_setting(goose_attack_run_state, command)),
        };
        match outcome {
            Ok(outcome) => {
                debug!(
                    "[control]: ok={} error={:?} phase={} active_users={} target_users={:?}: {}",
                    outcome.ok,
                    outcome.error,
                    outcome.phase,
                    outcome.active_users,
                    outcome.target_users,
                    outcome.message
                );
                let _ = respond.send(ControlReply::Outcome(outcome));
                if !start {
                    return Ok(());
                }
                self.control_start_finish(goose_attack_run_state)
                    .await
                    .inspect_err(|e| warn!("[control]: start failed after it was accepted: {e}"))
            }
            Err(e) => {
                let internal = self.control_outcome(
                    goose_attack_run_state,
                    false,
                    Some("internal"),
                    "internal control error",
                    None,
                );
                let _ = respond.send(ControlReply::Outcome(internal));
                Err(e)
            }
        }
    }

    /// Apply host, a rate, a time or the run time, refused when the phase
    /// cannot change it (`invalid_phase`), in test plan mode without users,
    /// for a rate while its own time is set on a running test, or by the
    /// command line's cross field rules (`conflicting_settings`). Every
    /// refusal names the setting at fault.
    fn control_setting(
        &mut self,
        goose_attack_run_state: &GooseAttackRunState,
        command: ControllerCommand,
    ) -> ControlOutcome {
        let field = match &command {
            ControllerCommand::Host(_) => "host",
            ControllerCommand::IncreaseRate(_) => "increase_rate",
            ControllerCommand::DecreaseRate(_) => "decrease_rate",
            ControllerCommand::IncreaseTime(_) => "increase_time",
            ControllerCommand::DecreaseTime(_) => "decrease_time",
            ControllerCommand::RunTime(_) => "run_time",
            // Every other command has its own arm in `control`.
            _ => "",
        };
        let refuse = |goose: &Self, error, field, message: String| {
            goose
                .control_outcome(goose_attack_run_state, false, Some(error), message, None)
                .with_field(field)
        };
        let idle = self.attack_phase == AttackPhase::Idle;
        let rate = matches!(
            command,
            ControllerCommand::IncreaseRate(_) | ControllerCommand::DecreaseRate(_)
        );

        // The phase: host and the times only in Idle; the rates also on a
        // running users ramp, but not during its ramp down.
        let phase = if idle {
            None
        } else if !rate {
            Some(match command {
                ControllerCommand::Host(_) => {
                    "failed to reconfigure host, load test is not idle".to_string()
                }
                _ => format!("failed to configure {field}, be sure load test is idle"),
            })
        } else if goose_attack_run_state.stopping || self.attack_phase == AttackPhase::Shutdown {
            Some(format!(
                "failed to configure {field}, load test is stopping"
            ))
        } else {
            match &self.run_shape {
                None => Some(format!(
                    "failed to configure {field}, a test plan is running"
                )),
                Some(shape) if shape.ramp_down_started => Some(format!(
                    "failed to configure {field}, load test is ramping down"
                )),
                Some(_) => None,
            }
        };
        if let Some(message) = phase {
            return refuse(self, "invalid_phase", field, message);
        }

        // The mode: a ramp setting needs users, which a test plan leaves
        // unset; starting with none would never finish weighting users.
        let clears = matches!(
            command,
            ControllerCommand::IncreaseTime(0)
                | ControllerCommand::DecreaseTime(0)
                | ControllerCommand::RunTime(0)
        );
        let ramp = !matches!(command, ControllerCommand::Host(_)) && !clears;
        if ramp && self.configuration.test_plan.is_some() {
            return refuse(
                self,
                "conflicting_settings",
                "users",
                format!("failed to configure {field}, a test plan is configured: set users first to switch to a users ramp"),
            );
        }

        // A time cannot change while running, so neither can a rate that
        // would clear it.
        let own_time = match command {
            ControllerCommand::IncreaseRate(_) => self.configuration.increase_time_secs() > 0,
            ControllerCommand::DecreaseRate(_) => self.configuration.decrease_time_secs() > 0,
            _ => false,
        };
        if !idle && own_time {
            let time = field.replace("rate", "time");
            return refuse(
                self,
                "conflicting_settings",
                field,
                format!("failed to configure {field}, {time} is set and changes only when idle"),
            );
        }

        // The command line's cross field rules: run time is refused with
        // --iterations in every phase, and a time needs at least 2 users in
        // Idle, where it shapes the next Start.
        let cross = match command {
            ControllerCommand::RunTime(seconds)
                if seconds > 0 && self.configuration.iterations() > 0 =>
            {
                Some("it can not be set with --iterations")
            }
            ControllerCommand::IncreaseTime(seconds) | ControllerCommand::DecreaseTime(seconds)
                if seconds > 0 && self.configuration.users.is_some_and(|users| users < 2) =>
            {
                Some("it needs at least 2 users")
            }
            _ => None,
        };
        if let Some(reason) = cross {
            return refuse(
                self,
                "conflicting_settings",
                field,
                format!("failed to configure {field}, {reason}"),
            );
        }

        let message = match command {
            ControllerCommand::Host(host) => {
                self.configuration.host = Some(host);
                "host configured"
            }
            ControllerCommand::IncreaseRate(rate) => {
                self.configuration.increase_time = None;
                self.configuration.increase_rate = Some(rate.to_string());
                "increase_rate configured"
            }
            ControllerCommand::DecreaseRate(rate) => {
                self.configuration.decrease_time = None;
                self.configuration.decrease_rate = Some(rate.to_string());
                // On a running users ramp with a hold that ends, the ramp
                // down still ahead is generated again at the new rate, or
                // added where all users would stop at once. A run holding
                // until stopped keeps holding: the rate is for the next
                // Users decrease and the next Start.
                if let Some(shape) = self.run_shape.as_mut() {
                    if !matches!(shape.hold, crate::run_shape::Hold::UntilStopped { .. }) {
                        shape.ramp_down = crate::run_shape::RampDown::Rate(rate);
                        shape.regenerate_tail(&mut self.test_plan.steps, self.test_plan.current);
                    }
                }
                "decrease_rate configured"
            }
            // 0 clears a time, as `null` does on the dashboard: back to
            // Goose's default, leaving the rate and any test plan alone.
            ControllerCommand::IncreaseTime(0) => {
                self.configuration.increase_time = None;
                "increase_time cleared"
            }
            ControllerCommand::DecreaseTime(0) => {
                self.configuration.decrease_time = None;
                "decrease_time cleared"
            }
            ControllerCommand::RunTime(0) => {
                self.configuration.run_time = None;
                "run_time cleared"
            }
            ControllerCommand::IncreaseTime(seconds) => {
                self.configuration.increase_rate = None;
                self.configuration.increase_time = Some(seconds.to_string());
                "increase_time configured"
            }
            ControllerCommand::DecreaseTime(seconds) => {
                self.configuration.decrease_rate = None;
                self.configuration.decrease_time = Some(seconds.to_string());
                "decrease_time configured"
            }
            ControllerCommand::RunTime(seconds) => {
                self.configuration.run_time = Some(seconds.to_string());
                "run_time configured"
            }
            _ => "",
        };
        self.control_outcome(goose_attack_run_state, true, None, message, None)
    }

    /// Current metrics from the dedicated metrics processor, or the main
    /// loop's copy when the processor is not running.
    async fn metrics_for_controller(
        &mut self,
        goose_attack_run_state: &GooseAttackRunState,
    ) -> GooseMetrics {
        self.update_duration();
        let (respond_tx, respond_rx) = oneshot::channel();
        let sent = goose_attack_run_state
            .metrics_cmd_tx
            .send(MetricsCommand::GetMetrics {
                duration: self.metrics.duration,
                total_users: self.metrics.total_users,
                maximum_users: self.metrics.maximum_users,
                history: self.metrics.history.clone(),
                respond: respond_tx,
            });
        let snapshot = if sent.is_ok() {
            respond_rx.await.ok()
        } else {
            None
        };
        snapshot.unwrap_or_else(|| {
            warn!("metrics processor unavailable, returning stale metrics to controller");
            self.metrics.clone()
        })
    }

    /// Shut the load test down: from Idle at once, as Ctrl-C does there,
    /// otherwise by canceling the run. Nothing to do once shutting down.
    async fn control_shutdown(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<ControlOutcome, GooseError> {
        match self.attack_phase {
            AttackPhase::Idle => self.shutdown_from_idle(goose_attack_run_state),
            AttackPhase::Shutdown => {}
            _ => {
                self.set_ended_by(crate::runs::EndedBy::Stopped, None);
                goose_attack_run_state.shutdown_after_stop = true;
                self.cancel_attack(goose_attack_run_state).await?;
            }
        }
        Ok(self.control_outcome(
            goose_attack_run_state,
            true,
            None,
            "load test shut down",
            None,
        ))
    }
}

/// The control loop listens for connections on the configured TCP port. Each connection
/// spawns a new thread so multiple clients can connect. Handles incoming connections for
/// both telnet and WebSocket clients.
///  -  @TODO: optionally limit how many controller connections are allowed
///  -  @TODO: optionally require client authentication
///  -  @TODO: optionally ssl-encrypt client communication
pub(crate) async fn controller_main(
    // Expose load test configuration to controller thread.
    configuration: GooseConfiguration,
    // For sending requests to the parent process.
    channel_tx: flume::Sender<ControlRequest>,
    // Which type of controller to launch.
    protocol: ControllerProtocol,
) -> io::Result<()> {
    // Build protocol-appropriate address.
    let address = match &protocol {
        ControllerProtocol::Telnet => format!(
            "{}:{}",
            configuration.telnet_host(),
            configuration.telnet_port()
        ),
        ControllerProtocol::WebSocket => format!(
            "{}:{}",
            configuration.websocket_host(),
            configuration.websocket_port()
        ),
    };

    // All controllers use a TcpListener port.
    debug!("[controller]: preparing to bind {protocol:?} to: {address}");
    let listener = TcpListener::bind(&address).await?;
    info!("[controller]: {protocol:?} listening on: {address}");

    // Counter increments each time a controller client connects with this protocol.
    let mut client_id: u32 = 0;

    // Wait for a connection.
    while let Ok((stream, _)) = listener.accept().await {
        client_id += 1;

        // Identify the client ip and port, used primarily for debug logging.
        let peer_address = stream
            .peer_addr()
            .map_or("UNKNOWN ADDRESS".to_string(), |p| p.to_string());
        let client = format!("{protocol:?} client [{client_id}] from {peer_address}");

        // Spawn a new thread to communicate with a client. The returned JoinHandle is
        // ignored as the thread simply runs until the client exits or Goose shuts down.
        // Don't .await the tokio::spawn or Goose can't handle multiple simultaneous
        // connections.
        let _ignored_joinhandle = tokio::spawn(serve_client(
            stream,
            protocol,
            channel_tx.clone(),
            client,
            CONTROL_TIMEOUT,
        ));
    }

    Ok(())
}

/// Goose supports two different Controller protocols: telnet and WebSocket.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ControllerProtocol {
    /// Allows control of Goose via telnet.
    Telnet,
    /// Allows control of Goose via a WebSocket.
    WebSocket,
}

/// A connected Controller client: what differs between telnet and
/// WebSocket is how a line arrives and how a reply is written.
enum Transport {
    Telnet(TcpStream),
    WebSocket(Box<WebSocketStream<TcpStream>>),
}

/// Longest WebSocket message a Controller reads, in bytes. It matches Linux's
/// limit on one command line argument (`MAX_ARG_STRLEN`, 128 KiB with 4 KiB
/// pages), so a test plan the command line takes there fits, less the few
/// bytes of the JSON request around it. A longer
/// message gets a 1009 close. A connection accepts messages and frames of up
/// to twice this, so it reads into about 1 MiB at most: a message being
/// assembled, its next frame, and one read ahead.
pub(crate) const MAX_CONTROLLER_MESSAGE: usize = 128 * 1024;

/// Reason in the close a WebSocket message over [`MAX_CONTROLLER_MESSAGE`]
/// gets.
const MESSAGE_TOO_BIG: &str = "message too big, at most 131072 bytes";
// The reason names the limit, so the two change together.
const _: () = assert!(MAX_CONTROLLER_MESSAGE == 131_072);

/// Reply to a WebSocket message that is not a json request.
const INVALID_JSON: &str =
    "invalid json, see Goose book https://book.goose.rs/controller/websocket.html";

/// Reply to a WebSocket request that is not a command.
const WS_UNRECOGNIZED: &str =
    "unrecognized command, see Goose book https://book.goose.rs/controller/websocket.html";

impl Transport {
    /// The next line the client sent; `None` once the client has gone,
    /// `Some(Err(reply))` for a message that holds no line.
    async fn read(&mut self) -> Option<Result<String, &'static str>> {
        match self {
            Transport::Telnet(socket) => {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).await.ok().filter(|n| *n > 0)?;
                // Invalid utf-8 is a corrupted request: disconnect.
                let text = str::from_utf8(&buf[..n])
                    .inspect_err(|e| info!("ignoring unexpected input from telnet controller: {e}"))
                    .ok()?;
                Some(Ok(text.lines().next().unwrap_or_default().to_string()))
            }
            Transport::WebSocket(stream) => {
                let message = match stream.next().await? {
                    Ok(message) if message.len() <= MAX_CONTROLLER_MESSAGE => message,
                    // Too big: refused unread, or over the hard bound and
                    // never buffered; a close can usually still be written.
                    Ok(_) | Err(WebSocketError::Capacity(_)) => {
                        info!("WebSocket Controller message over {MAX_CONTROLLER_MESSAGE} bytes, closing");
                        self.close(CloseCode::Size, MESSAGE_TOO_BIG).await;
                        return None;
                    }
                    Err(_) => return None,
                };
                let request = match message {
                    Message::Text(text) => {
                        serde_json::from_str::<ControllerWebSocketRequest>(&text).ok()
                    }
                    // The client is closing: there is nothing to answer.
                    Message::Close(_) => return None,
                    _ => None,
                };
                Some(request.map(|r| r.request).ok_or(INVALID_JSON))
            }
        }
    }

    /// Send a reply: telnet as text followed by a prompt, WebSocket as a
    /// [`ControllerWebSocketResponse`].
    async fn write(&mut self, success: bool, response: &str) {
        let written = match self {
            Transport::Telnet(socket) => socket
                .write_all(format!("{response}\ngoose> ").as_bytes())
                .await
                .map_err(|e| e.to_string()),
            Transport::WebSocket(stream) => {
                let response = ControllerWebSocketResponse {
                    response: response.to_string(),
                    success,
                };
                let json = serde_json::to_string(&response).unwrap_or_default();
                stream
                    .send(Message::Text(json.into()))
                    .await
                    .map_err(|e| e.to_string())
            }
        };
        if let Err(e) = written {
            info!("failed to write to controller client: {e}");
        }
    }

    /// Tell a WebSocket client the connection is closing, and why.
    async fn close(&mut self, code: CloseCode, reason: &'static str) {
        if let Transport::WebSocket(stream) = self {
            let frame = CloseFrame {
                code,
                reason: reason.into(),
            };
            if stream.send(Message::Close(Some(frame))).await.is_err() {
                warn!("failed to write data to stream");
            }
        }
    }
}

/// Talk to one Controller client until it exits, shuts Goose down, or goes,
/// waiting up to `timeout` for the parent to answer each command.
async fn serve_client(
    socket: TcpStream,
    protocol: ControllerProtocol,
    channel_tx: flume::Sender<ControlRequest>,
    client: String,
    timeout: Duration,
) {
    info!("{client} connected");
    let mut transport = match protocol {
        ControllerProtocol::Telnet => Transport::Telnet(socket),
        ControllerProtocol::WebSocket => match tokio_tungstenite::accept_async_with_config(
            socket,
            Some(
                WebSocketConfig::default()
                    .max_message_size(Some(2 * MAX_CONTROLLER_MESSAGE))
                    .max_frame_size(Some(2 * MAX_CONTROLLER_MESSAGE)),
            ),
        )
        .await
        {
            Ok(stream) => Transport::WebSocket(Box::new(stream)),
            Err(e) => {
                info!("invalid WebSocket handshake: {e}");
                return;
            }
        },
    };
    let websocket = matches!(transport, Transport::WebSocket(_));
    // Display initial goose> prompt.
    if let Transport::Telnet(socket) = &mut transport {
        let _ = socket.write_all(b"goose> ").await;
    }

    while let Some(line) = transport.read().await {
        let line = match line {
            Ok(line) => line,
            Err(reply) => {
                transport.write(false, reply).await;
                continue;
            }
        };
        if let Some((reply, exit)) = local_reply(&line) {
            transport.write(true, reply).await;
            if exit {
                transport.close(CloseCode::Normal, "exit").await;
                break;
            }
            continue;
        }
        let command = match line.parse::<ControllerCommand>() {
            Ok(command) => command,
            Err(reply) => {
                let book = websocket && reply == UNRECOGNIZED;
                transport
                    .write(false, if book { WS_UNRECOGNIZED } else { reply })
                    .await;
                continue;
            }
        };
        let shutdown = matches!(command, ControllerCommand::Shutdown);
        // The WebSocket Controller always returns json.
        let json = websocket
            || matches!(
                command,
                ControllerCommand::ConfigJson | ControllerCommand::MetricsJson
            );
        let (success, response) = match send_command(&channel_tx, command, timeout).await {
            Ok(ControlReply::Outcome(outcome)) => (outcome.ok, outcome.message),
            Ok(ControlReply::Config(config)) if json => (true, to_json(&config)),
            Ok(ControlReply::Config(config)) => (true, format!("{config:#?}")),
            Ok(ControlReply::Metrics(metrics)) if json => (true, to_json(metrics.as_ref())),
            Ok(ControlReply::Metrics(metrics)) => (true, metrics.to_string()),
            Err(SendError::Timeout) => (
                false,
                "control timed out, it may still apply: check before retrying".to_string(),
            ),
            // The parent has gone: disconnect.
            Err(SendError::Unavailable) => {
                transport
                    .write(false, "parent process has closed the controller channel")
                    .await;
                break;
            }
        };
        transport.write(success, &response).await;
        if shutdown {
            transport.close(CloseCode::Normal, "shutdown").await;
            break;
        }
    }
    info!("{client} disconnected");
}

/// A reply the Controller gives itself, and whether the client then exits:
/// `help` and `exit` never reach the parent.
fn local_reply(line: &str) -> Option<(&'static str, bool)> {
    let is = |words: &[&str]| words.iter().any(|w| line.trim().eq_ignore_ascii_case(w));
    if is(&["help", "?"]) {
        Some((HELP, false))
    } else if is(&["exit", "quit", "q"]) {
        Some(("goodbye!", true))
    } else {
        None
    }
}

/// Configuration and metrics as json for a Controller client.
fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| e.to_string())
}

/// This structure defines the required json format of any request sent to the WebSocket
/// Controller.
///
/// Requests must be made in the following format:
/// ```json
/// {
///     "request": String,
/// }
///
/// ```
///
/// The request "String" value must be a valid
/// [`ControllerCommand`](./enum.ControllerCommand.html).
///
/// # Example
/// The following request will shut down the load test:
/// ```json
/// {
///     "request": "shutdown",
/// }
/// ```
///
/// Responses will be formatted as defined in
/// [ControllerWebSocketResponse](./struct.ControllerWebSocketResponse.html).
#[derive(Debug, Deserialize, Serialize)]
pub struct ControllerWebSocketRequest {
    /// A valid command string.
    pub request: String,
}

/// This structure defines the json format of any response returned from the WebSocket
/// Controller.
///
/// Responses are in the following format:
/// ```json
/// {
///     "response": String,
///     "success": bool,
/// }
/// ```
///
/// # Example
/// The following response will be returned when a request is made to shut down the
/// load test:
/// ```json
/// {
///     "response": "load test shut down",
///     "success": true
/// }
/// ```
///
/// Requests must be formatted as defined in
/// [ControllerWebSocketRequest](./struct.ControllerWebSocketRequest.html).
#[derive(Debug, Deserialize, Serialize)]
pub struct ControllerWebSocketResponse {
    /// The response from the controller.
    pub response: String,
    /// Whether the request was successful or not.
    pub success: bool,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::goose::{GooseUser, Scenario, Transaction};
    use crate::runs;
    use crate::test_plan::TestPlanStepAction;
    use crate::ControlOutcome;
    use crate::{scenario, transaction};

    /// A telnet client that goes ends the task serving it.
    #[tokio::test]
    async fn telnet_client_gone_ends_serve_client() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        let (channel_tx, _channel_rx) = flume::unbounded();
        let served = tokio::spawn(serve_client(
            socket,
            ControllerProtocol::Telnet,
            channel_tx,
            "telnet client".to_string(),
            CONTROL_TIMEOUT,
        ));
        let mut prompt = [0; 7];
        client.read_exact(&mut prompt).await.unwrap();
        assert_eq!(&prompt, b"goose> ");
        // End of file, with the client still reading, so only the read ends it.
        client.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), served)
            .await
            .expect("serve_client must end once its client has gone")
            .unwrap();
    }

    /// What a client line becomes: the typed command written out, or the
    /// error the client is sent back.
    fn parse(line: &str) -> Result<String, String> {
        if let Some((_, exit)) = local_reply(line) {
            return Ok(if exit { "Exit" } else { "Help" }.to_string());
        }
        line.parse::<ControllerCommand>()
            .map(|command| format!("{command:?}"))
            .map_err(str::to_string)
    }

    /// Every command and alias parses to its typed value, and every bad
    /// input to the error its client is sent.
    #[test]
    fn parse_table() {
        const UNRECOGNIZED: &str = "unrecognized command";
        const USERS: &str = "users must be an integer between 1 and 100000";
        const HOST: &str = "host must be a valid url starting with http:// or https://, without a user name or password";
        const INCREASE_RATE: &str = "increase_rate must be a number from 0.01 to 100000";
        const DECREASE_RATE: &str = "decrease_rate must be a number from 0.01 to 100000";
        const INCREASE_TIME: &str = "increase_time must be a time span such as 1h30m5s, from 1 second to 30 days, or 0 to clear";
        const DECREASE_TIME: &str = "decrease_time must be a time span such as 1h30m5s, from 1 second to 30 days, or 0 to clear";
        const RUN_TIME: &str =
            "run_time must be a time span such as 1h30m5s, from 1 second to 30 days, or 0 to clear";
        const FORTY_DIGITS: &str = "increaserate 1000000000000000000000000000000000000000";
        const TEST_PLAN: &str = "test-plan must be users,time steps such as 10,5m;10,1h;0,30s: at most 100 steps of up to 30 days each, 4096 bytes, and 1 to 100000 users in total";
        // 100 steps of 1,1s are accepted and 101 refused; so is a plan of
        // more than 4096 bytes.
        let steps_100 = format!("testplan {}", vec!["1,1s"; 100].join(";"));
        let steps_101 = format!("testplan {}", vec!["1,1s"; 101].join(";"));
        let bytes_4097 = format!("testplan 1,{}1s", "0".repeat(4093));

        let ok: &[(&str, &str)] = &[
            ("help", "Help"),
            ("?", "Help"),
            ("HELP", "Help"),
            ("exit", "Exit"),
            ("quit", "Exit"),
            ("q", "Exit"),
            ("Exit", "Exit"),
            ("start", "Start"),
            ("START", "Start"),
            ("stop", "Stop"),
            ("shutdown", "Shutdown"),
            ("config", "Config"),
            ("configjson", "ConfigJson"),
            ("config-json", "ConfigJson"),
            ("metrics", "Metrics"),
            ("stats", "Metrics"),
            ("metricsjson", "MetricsJson"),
            ("metrics-json", "MetricsJson"),
            ("statsjson", "MetricsJson"),
            ("stats-json", "MetricsJson"),
            ("host http://example.com/", "Host(\"http://example.com/\")"),
            ("hostname https://example.com", "Host(\"https://example.com\")"),
            ("host_name http://127.0.0.1:8080/", "Host(\"http://127.0.0.1:8080/\")"),
            ("host-name http://example.com/", "Host(\"http://example.com/\")"),
            ("HOST HTTP://EXAMPLE.COM/", "Host(\"HTTP://EXAMPLE.COM/\")"),
            ("users 5", "Users(5)"),
            ("user 1", "Users(1)"),
            ("users 100000", "Users(100000)"),
            ("increaserate 1.25", "IncreaseRate(1.25)"),
            ("increase_rate .1", "IncreaseRate(0.1)"),
            ("increase-rate 25", "IncreaseRate(25.0)"),
            ("decreaserate 0.5", "DecreaseRate(0.5)"),
            ("decrease_rate 2", "DecreaseRate(2.0)"),
            ("decrease-rate 0.90", "DecreaseRate(0.9)"),
            // The slowest and fastest rates.
            ("increaserate 0.01", "IncreaseRate(0.01)"),
            ("decreaserate .01", "DecreaseRate(0.01)"),
            ("increaserate 100000", "IncreaseRate(100000.0)"),
            ("decreaserate 100000.0", "DecreaseRate(100000.0)"),
            // 30 days is the longest time, and 0 clears a time.
            ("runtime 720h", "RunTime(2592000)"),
            ("increasetime 2592000", "IncreaseTime(2592000)"),
            ("decreasetime 43200m", "DecreaseTime(2592000)"),
            ("runtime 0", "RunTime(0)"),
            ("increasetime 0s", "IncreaseTime(0)"),
            (
                "testplan 10,720h;0,0",
                "TestPlan(TestPlan { steps: [(10, 2592000000), (0, 0)], current: 0 })",
            ),
            ("host https://example.com:8443/path", "Host(\"https://example.com:8443/path\")"),
            ("increasetime 30s", "IncreaseTime(30)"),
            ("increase_time 1h2m3s", "IncreaseTime(3723)"),
            ("increase-time 10", "IncreaseTime(10)"),
            ("decreasetime 5m", "DecreaseTime(300)"),
            ("decrease_time 1h", "DecreaseTime(3600)"),
            ("decrease-time 30", "DecreaseTime(30)"),
            ("runtime 60", "RunTime(60)"),
            ("run 1h", "RunTime(3600)"),
            ("run_time 1h2s", "RunTime(3602)"),
            ("run-time 10s", "RunTime(10)"),
            ("runtime 1H2M3S", "RunTime(3723)"),
            (
                "testplan 10,5s;10,30s;0,5s",
                "TestPlan(TestPlan { steps: [(10, 5000), (10, 30000), (0, 5000)], current: 0 })",
            ),
            (
                "test_plan 10,2s;10,30m5s;0,1h2s",
                "TestPlan(TestPlan { steps: [(10, 2000), (10, 1805000), (0, 3602000)], current: 0 })",
            ),
            (
                "test-plan 100000,1s",
                "TestPlan(TestPlan { steps: [(100000, 1000)], current: 0 })",
            ),
            (
                "plan 50000,1s;100000,1s",
                "TestPlan(TestPlan { steps: [(50000, 1000), (100000, 1000)], current: 0 })",
            ),
            // Main's grammar allows ASCII white space around the comma.
            (
                "testplan 10 , 5s;0,\t1s",
                "TestPlan(TestPlan { steps: [(10, 5000), (0, 1000)], current: 0 })",
            ),
        ];
        let refused: &[(&str, &str)] = &[
            ("", UNRECOGNIZED),
            ("foo", UNRECOGNIZED),
            ("start now", UNRECOGNIZED),
            ("users", USERS),
            ("users 0", USERS),
            ("users 100001", USERS),
            ("users 1.1", USERS),
            ("users -1", USERS),
            ("users 99999999999999999999", USERS),
            ("host foobar", HOST),
            ("host http://$[foo", HOST),
            ("host ftp://example.com/", HOST),
            // A user name or password would show to observers and in reports.
            ("host http://user:secret@example.com/", HOST),
            ("host https://user@example.com/", HOST),
            ("host https://:secret@example.com/", HOST),
            ("increaserate 0", INCREASE_RATE),
            ("increaserate 0.0", INCREASE_RATE),
            ("increaserate .", INCREASE_RATE),
            ("increaserate 1.2.3", INCREASE_RATE),
            ("increaserate -1", INCREASE_RATE),
            ("decreaserate 0", DECREASE_RATE),
            ("decreaserate .", DECREASE_RATE),
            // Above 0, but slower than 0.01 or faster than 100000.
            ("increaserate 0.0000001", INCREASE_RATE),
            ("increaserate 0.009", INCREASE_RATE),
            ("decreaserate 0.0000116", DECREASE_RATE),
            ("decreaserate 0.0099", DECREASE_RATE),
            ("increaserate 100000.01", INCREASE_RATE),
            ("decreaserate 100001", DECREASE_RATE),
            // Longer than 30 days.
            ("runtime 720h1s", RUN_TIME),
            ("increasetime 2592001", INCREASE_TIME),
            ("decreasetime 43201m", DECREASE_TIME),
            ("testplan 10,720h1s", TEST_PLAN),
            ("testplan 10,1s;0,2592001", TEST_PLAN),
            ("increasetime .1", INCREASE_TIME),
            ("increasetime 10d", INCREASE_TIME),
            ("increase-time 2m1h", INCREASE_TIME),
            ("increasetime 99999999999999999999h", INCREASE_TIME),
            ("decreasetime 9999999999999999h", DECREASE_TIME),
            ("runtime 10d", RUN_TIME),
            ("runtime", RUN_TIME),
            ("increasetime", INCREASE_TIME),
            ("decreasetime", DECREASE_TIME),
            ("runtime 99999999999999999999h", RUN_TIME),
            ("runtime 18446744073709552", RUN_TIME),
            ("testplan 10", TEST_PLAN),
            ("testplan 0,1s", TEST_PLAN),
            ("testplan 0,1s;0,1h", TEST_PLAN),
            ("testplan 99999999999999999999,1s", TEST_PLAN),
            ("testplan 10,99999999999999999999h", TEST_PLAN),
            ("testplan 10,18446744073709552", TEST_PLAN),
            ("testplan 100001,1s", TEST_PLAN),
            ("testplan 60000,1s;0,1s;60000,1s", TEST_PLAN),
            // Only ASCII digits: a regex `\d` also matches other scripts' digits.
            ("testplan ١,1s", TEST_PLAN),
            ("testplan 10,١s", TEST_PLAN),
            ("users ١", USERS),
            ("increaserate ١", INCREASE_RATE),
            ("runtime ١h", RUN_TIME),
            // ASCII case and spaces only: Unicode `(?i)` folds `ſ` to `s`, and
            // Unicode `\s` matches a no break space.
            ("runtime 1\u{17f}", RUN_TIME),
            ("testplan 10,\u{a0}1s", TEST_PLAN),
            ("users 9999999999999999999999999999", USERS),
            ("testplan 9999999999999999999999999999,1s", TEST_PLAN),
            // Only what the Controller regexes on main accepted: a sign, a second
            // space, an exponent, a word for a number, or a host without `//`.
            ("users +5", USERS),
            ("users  5", USERS),
            ("increaserate 1e3", INCREASE_RATE),
            ("increaserate inf", INCREASE_RATE),
            ("increaserate NaN", INCREASE_RATE),
            ("host http:foo", HOST),
            ("testplan 10,1s\u{1b}[2J", TEST_PLAN),
            ("testplan  10,1s", TEST_PLAN),
            ("testplan 10,1s ;5,1s", TEST_PLAN),
            // In the grammar, but infinite as an f32: the finite check refuses it.
            (FORTY_DIGITS, INCREASE_RATE),
        ];

        // Collect every mismatch, so one run shows the whole table.
        let mut mismatches = Vec::new();
        for (line, expected) in [
            (steps_100.as_str(), Ok(())),
            (steps_101.as_str(), Err(TEST_PLAN.to_string())),
            (bytes_4097.as_str(), Err(TEST_PLAN.to_string())),
        ] {
            let got = parse(line).map(|_| ());
            if got != expected {
                mismatches.push(format!(
                    "{}...: expected {expected:?}, got {got:?}",
                    &line[..20]
                ));
            }
        }
        // 4096 bytes of plan are accepted.
        let bytes_4096 = format!("testplan 1,{}1s", "0".repeat(4092));
        assert_eq!(bytes_4096.len() - "testplan ".len(), 4096);
        assert!(parse(&bytes_4096).is_ok());
        let rows = ok
            .iter()
            .map(|(line, typed)| (*line, Ok(typed.to_string())))
            .chain(
                refused
                    .iter()
                    .map(|(line, error)| (*line, Err(error.to_string()))),
            );
        for (line, expected) in rows {
            let got = std::panic::catch_unwind(|| parse(line))
                .unwrap_or_else(|_| Err("panicked".to_string()));
            if got != expected {
                mismatches.push(format!("{line:?}: expected {expected:?}, got {got:?}"));
            }
        }
        // The grammar, not a parser, refuses the escape sequence, so it never
        // reaches a parser that could print it; and the forty digit rate is
        // in the grammar, so the finite check is what refuses it.
        assert!(!PLAN.is_match("10,1s\u{1b}[2J"));
        assert!(RATE.is_match(&FORTY_DIGITS[13..]));

        // A long line is refused like a short one.
        let long = format!("users {}", "1".repeat(10_000));
        if parse(&long) != Err(USERS.to_string()) {
            mismatches.push(format!("10 KB users line: got {:?}", parse(&long)));
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }

    /// Users outside 1 to MAX_CONTROL_USERS is refused while idle and while
    /// running, and leaves the configuration and test plan as they were. The
    /// bound is in the one constructor every front end builds `users` with.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_set_users_out_of_bounds_refused() {
        use crate::controller::{ControllerCommand, INVALID_USERS_MSG, MAX_CONTROL_USERS};

        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-save",
            "--no-telnet",
            "--no-websocket",
            "--test-plan",
            "10,1s",
        ])
        .unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        assert_eq!(
            INVALID_USERS_MSG,
            format!("users must be an integer between 1 and {MAX_CONTROL_USERS}")
        );
        let steps = goose_attack.test_plan.steps.clone();

        for phase in [AttackPhase::Idle, AttackPhase::Maintain] {
            goose_attack.set_attack_phase(&mut run_state, phase);
            for users in [0, MAX_CONTROL_USERS as u64 + 1, u64::MAX] {
                let line = format!("users {users}");
                let refused = !control(&mut goose_attack, &mut run_state, &line).await;
                assert!(refused, "{:?} must refuse {} users", phase, users);
                assert!(
                    matches!(ControllerCommand::users(users), Err(INVALID_USERS_MSG)),
                    "{} users",
                    users
                );
                assert_eq!(goose_attack.configuration.users, None);
                // Accepting users clears the configured test plan.
                assert!(goose_attack.configuration.test_plan.is_some());
                assert_eq!(goose_attack.test_plan.steps, steps);
            }
        }

        // The bounds themselves are accepted.
        goose_attack.set_attack_phase(&mut run_state, AttackPhase::Idle);
        for users in [1, MAX_CONTROL_USERS] {
            let line = format!("users {users}");
            let accepted = control(&mut goose_attack, &mut run_state, &line).await;
            assert!(accepted, "idle must accept {} users", users);
            assert_eq!(goose_attack.configuration.users, Some(users));
        }
    }

    /// A controller `shutdown` while idle cancels nothing, but the run is
    /// stopping all the same: Users is refused until it shuts down.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_shutdown_while_idle_sets_stopping() {
        async fn noop(_user: &mut GooseUser) -> crate::goose::TransactionResult {
            Ok(())
        }

        // A scenario and host let a Users that wrongly gets through launch
        // users and fail the assertion, rather than loop with nothing to weight.
        let configuration = GooseConfiguration::parse_args_default(&[
            "--no-save",
            "--no-telnet",
            "--no-websocket",
            "--host",
            "http://127.0.0.1",
        ])
        .unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario!("Idle").register_transaction(transaction!(noop)));
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        assert_eq!(
            reply(&mut goose_attack, &mut run_state, "shutdown").await,
            (true, "load test shut down".to_string())
        );
        assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
        assert!(run_state.stopping);
        let outcome = goose_attack.control_set_users(&mut run_state, 5).unwrap();
        assert!(!outcome.ok, "shutdown must refuse users: {:?}", outcome);
        assert_eq!(outcome.error, Some("invalid_phase"));
    }

    /// A controller `users` value too large for a `usize` is refused with an
    /// error reply instead of panicking the main loop.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_controller_users_overflow_refused() {
        let configuration =
            GooseConfiguration::parse_args_default(&["--no-telnet", "--no-websocket", "--no-save"])
                .unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
        let mut run_state = goose_attack.initialize_attack().await.unwrap();
        let accepted = control(
            &mut goose_attack,
            &mut run_state,
            "users 99999999999999999999",
        )
        .await;
        assert!(!accepted, "users overflow must be refused");
        assert_eq!(goose_attack.configuration.users, None);
    }

    /// Run `test` on a thread and runtime of its own and fail if it does not
    /// finish within 20 seconds, so a dispatcher that never returns fails the
    /// test instead of hanging the suite. A hung test leaves its thread
    /// spinning until the test binary exits, which is why the tests that use
    /// this register no scenario: weighting users then allocates nothing.
    fn finishes<F, Fut>(test: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()>,
    {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(test());
            let _ = done_tx.send(());
        });
        match done_rx.recv_timeout(Duration::from_secs(20)) {
            Ok(()) => handle.join().unwrap(),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if let Err(panic) = handle.join() {
                    std::panic::resume_unwind(panic);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("the control dispatcher did not return within 20 seconds")
            }
        }
    }

    /// An idle load test with no scenario, built from `args`.
    async fn idle_attack(args: &[&str]) -> (GooseAttack, GooseAttackRunState) {
        let mut all = vec!["--no-telnet", "--no-websocket", "--no-save", "--quiet"];
        all.extend_from_slice(args);
        let configuration = GooseConfiguration::parse_args_default(&all).unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration).unwrap();
        let run_state = goose_attack.initialize_attack().await.unwrap();
        (goose_attack, run_state)
    }

    /// Send `line` to the parent as a telnet or WebSocket client does, and
    /// return whether it was accepted. A line the client refuses never
    /// reaches the parent and counts as refused.
    pub(crate) async fn control(
        goose_attack: &mut GooseAttack,
        run_state: &mut GooseAttackRunState,
        line: &str,
    ) -> bool {
        reply(goose_attack, run_state, line).await.0
    }

    /// As [`control`], with the message the client is sent: the outcome's,
    /// the client's own refusal, or empty for configuration and metrics.
    async fn reply(
        goose_attack: &mut GooseAttack,
        run_state: &mut GooseAttackRunState,
        line: &str,
    ) -> (bool, String) {
        use crate::controller::{ControlReply, ControllerCommand};

        let command = match line.parse::<ControllerCommand>() {
            Ok(command) => command,
            Err(refusal) => return (false, refusal.to_string()),
        };
        let (controller_tx, controller_rx) = flume::unbounded();
        run_state.controller_channel_rx = Some(controller_rx);
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        controller_tx.send((command, response_tx)).unwrap();
        goose_attack
            .handle_control_requests(run_state)
            .await
            .unwrap();
        match response_rx.await.expect("the parent must reply") {
            ControlReply::Outcome(outcome) => (outcome.ok, outcome.message),
            _ => (true, String::new()),
        }
    }

    /// Gap 1: a test plan with no users is refused, idle or running, as
    /// weighting zero users never ends.
    #[test]
    #[serial_test::serial]
    fn test_control_test_plan_without_users_refused() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            for line in ["testplan 0,1s", "test-plan 0,1s;0,1h"] {
                assert!(
                    !control(&mut goose_attack, &mut run_state, line).await,
                    "{}",
                    line
                );
                assert!(goose_attack.configuration.test_plan.is_none(), "{}", line);
            }
            goose_attack.set_attack_phase(&mut run_state, AttackPhase::Maintain);
            assert!(!control(&mut goose_attack, &mut run_state, "testplan 0,1s").await);
            assert!(goose_attack.test_plan.steps.is_empty());
        });
    }

    /// Gaps 2 and 3: users or time spans too large for a `usize`, or a step
    /// too long in milliseconds, are refused instead of panicking the parent.
    #[test]
    #[serial_test::serial]
    fn test_control_overflow_refused() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let configuration = format!("{:?}", goose_attack.configuration);
            for line in [
                "testplan 99999999999999999999,1s",
                "testplan 10,99999999999999999999h",
                "testplan 10,9999999999999999h",
                "testplan 10,18446744073709552",
                "testplan 18446744073709551615,1s;0,1s;1,1s",
                "increasetime 99999999999999999999h",
                "increasetime 18446744073709552",
                "decreasetime 9999999999999999h",
                // Digits of other scripts, and a users count no integer holds.
                "testplan ١,1s",
                "users ١",
                "runtime ١h",
                "users 9999999999999999999999999999",
                "testplan 9999999999999999999999999999,1s",
            ] {
                assert!(
                    !control(&mut goose_attack, &mut run_state, line).await,
                    "{}",
                    line
                );
                assert_eq!(
                    format!("{:?}", goose_attack.configuration),
                    configuration,
                    "{}",
                    line
                );
            }
        });
    }

    /// Gap 4: a run time too large is refused when it is set, instead of
    /// panicking the next Start.
    #[test]
    #[serial_test::serial]
    fn test_control_run_time_overflow_refused() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let run_time = goose_attack.configuration.run_time.clone();
            for line in ["runtime 99999999999999999999h", "run 18446744073709552"] {
                assert!(
                    !control(&mut goose_attack, &mut run_state, line).await,
                    "{}",
                    line
                );
                assert_eq!(goose_attack.configuration.run_time, run_time, "{}", line);
            }
        });
    }

    /// Gap 5: a rate must be at least one user a day, as on the command line.
    #[test]
    #[serial_test::serial]
    fn test_control_rate_zero_refused() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            for line in [
                "increaserate 0",
                "increaserate 0.0",
                "increaserate 00",
                "increaserate .",
                "decreaserate 0",
                "decreaserate .",
            ] {
                assert!(
                    !control(&mut goose_attack, &mut run_state, line).await,
                    "{}",
                    line
                );
                assert_eq!(goose_attack.configuration.increase_rate, None, "{}", line);
                assert_eq!(goose_attack.configuration.decrease_rate, None, "{}", line);
            }
            assert!(control(&mut goose_attack, &mut run_state, "increaserate .5").await);
        });
    }

    /// Gap 6: a test plan goes through the users bound and the stopping
    /// check that `users` does, and is refused in Shutdown.
    #[test]
    #[serial_test::serial]
    fn test_control_test_plan_users_bound_and_stopping() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            // More than 100000 users in total, in one step or over several.
            for line in ["testplan 100001,1s", "testplan 60000,1s;0,1s;60000,1s"] {
                assert!(
                    !control(&mut goose_attack, &mut run_state, line).await,
                    "{}",
                    line
                );
                assert!(goose_attack.configuration.test_plan.is_none(), "{}", line);
            }
            assert!(
                control(
                    &mut goose_attack,
                    &mut run_state,
                    "testplan 50000,1s;100000,1s"
                )
                .await
            );

            // Shutdown refuses instead of reaching `unreachable!`.
            goose_attack.set_attack_phase(&mut run_state, AttackPhase::Shutdown);
            assert!(!control(&mut goose_attack, &mut run_state, "testplan 10,1s").await);

            // A cancel keeps its ramp to zero.
            goose_attack.set_attack_phase(&mut run_state, AttackPhase::Idle);
            goose_attack.test_plan = TestPlan {
                steps: vec![(10, 0), (0, 10_000)],
                current: 0,
            };
            run_state.active_users = 10;
            goose_attack.advance_test_plan(&mut run_state);
            goose_attack.cancel_attack(&mut run_state).await.unwrap();
            assert!(run_state.stopping);
            assert!(!control(&mut goose_attack, &mut run_state, "testplan 10,1s").await);
            assert_eq!(goose_attack.test_plan.steps.last(), Some(&(0, 0)));
        });
    }

    /// Gap 7: run time is only read at Start, so it is refused unless idle,
    /// as increase and decrease time and host are.
    #[test]
    #[serial_test::serial]
    fn test_control_run_time_refused_unless_idle() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let configuration = format!("{:?}", goose_attack.configuration);
            for phase in [
                AttackPhase::Increase,
                AttackPhase::Maintain,
                AttackPhase::Decrease,
            ] {
                goose_attack.set_attack_phase(&mut run_state, phase);
                for (line, refusal) in [
                    (
                        "runtime 10",
                        "failed to configure run_time, be sure load test is idle",
                    ),
                    (
                        "increasetime 10",
                        "failed to configure increase_time, be sure load test is idle",
                    ),
                    (
                        "decreasetime 10",
                        "failed to configure decrease_time, be sure load test is idle",
                    ),
                    (
                        "host http://127.0.0.1/",
                        "failed to reconfigure host, load test is not idle",
                    ),
                ] {
                    assert_eq!(
                        reply(&mut goose_attack, &mut run_state, line).await,
                        (false, refusal.to_string()),
                        "{} in {:?}",
                        line,
                        phase
                    );
                    assert_eq!(
                        format!("{:?}", goose_attack.configuration),
                        configuration,
                        "{} in {:?}",
                        line,
                        phase
                    );
                }
            }
            goose_attack.set_attack_phase(&mut run_state, AttackPhase::Idle);
            assert!(control(&mut goose_attack, &mut run_state, "runtime 10").await);
            assert_eq!(goose_attack.configuration.run_time.as_deref(), Some("10"));
        });
    }

    /// A Controller time of 0, with or without units, clears the time as
    /// `null` does on the dashboard, and leaves the rate alone.
    #[test]
    #[serial_test::serial]
    fn test_control_zero_time_is_unset() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) =
                idle_attack(&["--users", "4", "--run-time", "10", "--decrease-time", "5"]).await;
            goose_attack.configuration.increase_rate = Some("2".to_string());
            for (line, reply_text) in [
                ("runtime 0s", "run_time cleared"),
                ("increasetime 0m", "increase_time cleared"),
                ("decreasetime 00", "decrease_time cleared"),
            ] {
                assert_eq!(
                    reply(&mut goose_attack, &mut run_state, line).await,
                    (true, reply_text.to_string()),
                    "{}",
                    line
                );
            }
            assert_eq!(goose_attack.configuration.run_time, None);
            assert_eq!(goose_attack.configuration.increase_time, None);
            assert_eq!(goose_attack.configuration.decrease_time, None);
            assert_eq!(
                goose_attack.configuration.increase_rate.as_deref(),
                Some("2")
            );
        });
    }

    /// Gap 8: a controller `shutdown` from Idle shuts down as Ctrl-C and
    /// dashboard Quit do there, with a plan that has nothing left to run, and
    /// does not hold the main loop.
    #[test]
    #[serial_test::serial]
    fn test_control_shutdown_from_idle() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) =
                idle_attack(&["--no-autostart", "--test-plan", "10,1s"]).await;
            goose_attack.test_plan = TestPlan::build(&goose_attack.configuration).unwrap();
            let started = std::time::Instant::now();
            assert!(control(&mut goose_attack, &mut run_state, "shutdown").await);
            assert!(
                started.elapsed() < Duration::from_millis(250),
                "shutdown held the main loop for {:?}",
                started.elapsed()
            );
            assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
            assert!(run_state.stopping);
            assert!(run_state.shutdown_after_stop);
            assert_eq!(goose_attack.test_plan.steps, vec![(0, 0)]);
            assert_eq!(goose_attack.test_plan.current, 0);
        });
    }

    /// A `shutdown` once Goose is shutting down is answered and changes
    /// nothing.
    #[test]
    #[serial_test::serial]
    fn test_control_shutdown_in_shutdown_phase() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&["--no-autostart"]).await;
            goose_attack.set_attack_phase(&mut run_state, AttackPhase::Shutdown);
            let steps = goose_attack.test_plan.steps.clone();
            assert_eq!(
                reply(&mut goose_attack, &mut run_state, "shutdown").await,
                (true, "load test shut down".to_string())
            );
            assert_eq!(goose_attack.attack_phase, AttackPhase::Shutdown);
            assert!(!run_state.stopping);
            assert!(!run_state.shutdown_after_stop);
            assert_eq!(goose_attack.test_plan.steps, steps);
        });
    }

    /// A cancel with no user running, as a second `shutdown` from Idle or a
    /// `stop` in the tick of its `start` sends, ends the load test instead
    /// of maintaining zero users with no end.
    #[test]
    #[serial_test::serial]
    fn test_control_cancel_without_users_ends() {
        // Run the main loop's phase work until the cancel ends in `until`,
        // as a plan maintaining zero users never does.
        async fn ends_in(
            goose_attack: &mut GooseAttack,
            run_state: &mut GooseAttackRunState,
            until: AttackPhase,
        ) {
            for _ in 0..4 {
                match goose_attack.attack_phase {
                    AttackPhase::Maintain => goose_attack.maintain_attack(run_state).await,
                    AttackPhase::Decrease => goose_attack.decrease_attack(run_state).await,
                    _ => break,
                }
                .unwrap();
            }
            assert_eq!(goose_attack.attack_phase, until);
        }

        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&["--no-autostart"]).await;
            assert!(control(&mut goose_attack, &mut run_state, "shutdown").await);
            assert!(control(&mut goose_attack, &mut run_state, "shutdown").await);
            assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
            assert_eq!(goose_attack.test_plan.steps, vec![(0, 0)]);
            ends_in(&mut goose_attack, &mut run_state, AttackPhase::Shutdown).await;

            let (mut goose_attack, mut run_state) = idle_attack(&["--no-autostart"]).await;
            // As `start` leaves it before its first user launches.
            goose_attack.set_attack_phase(&mut run_state, AttackPhase::Increase);
            goose_attack.run_in_progress = true;
            run_state.active_users = 0;
            assert!(control(&mut goose_attack, &mut run_state, "stop").await);
            assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
            assert_eq!(goose_attack.test_plan.steps, vec![(0, 0)]);
            assert!(run_state.stopping);
            assert!(!run_state.shutdown_after_stop);
            assert!(matches!(
                goose_attack.ended_by,
                Some((runs::EndedBy::Stopped, _))
            ));
            ends_in(&mut goose_attack, &mut run_state, AttackPhase::Idle).await;
            // The run ended through report writing, recorded as canceled.
            assert!(!goose_attack.run_in_progress);
            assert!(!run_state.stopping);
            let actions: Vec<_> = goose_attack
                .metrics
                .history
                .iter()
                .map(|step| &step.action)
                .collect();
            assert!(
                matches!(
                    actions[..],
                    [TestPlanStepAction::Canceling, TestPlanStepAction::Finished]
                ),
                "{:?}",
                actions
            );

            // A stop while maintaining no users, as a plan step to 0 users
            // leaves the run, ends it too.
            let (mut goose_attack, mut run_state) = idle_attack(&["--no-autostart"]).await;
            goose_attack.set_attack_phase(&mut run_state, AttackPhase::Maintain);
            goose_attack.run_in_progress = true;
            run_state.active_users = 0;
            assert!(control(&mut goose_attack, &mut run_state, "stop").await);
            assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
            assert_eq!(goose_attack.test_plan.steps, vec![(0, 0)]);
            ends_in(&mut goose_attack, &mut run_state, AttackPhase::Idle).await;
            assert!(!run_state.stopping);
        });
    }

    /// Gap 10: on a `--test-plan` launch, a run time or rate used to clear
    /// the test plan and leave no users, so Start weighted zero users
    /// forever. It is refused now, and Start still answers.
    #[test]
    #[serial_test::serial]
    fn test_control_start_after_test_plan_cleared_answers() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) =
                idle_attack(&["--no-autostart", "--test-plan", "2,1s"]).await;
            // A ramp setting no longer clears the plan, so the plan's users
            // are still there to start.
            for line in ["runtime 10", "increaserate 2"] {
                assert!(
                    !control(&mut goose_attack, &mut run_state, line).await,
                    "{}",
                    line
                );
                assert_eq!(
                    TestPlan::build(&goose_attack.configuration)
                        .unwrap()
                        .total_users(),
                    2,
                    "{}",
                    line
                );
            }
        });
    }

    /// The parent answers every queued controller request in one tick, up
    /// to a batch, and skips a request whose client has gone.
    #[test]
    #[serial_test::serial]
    fn test_control_drains_batch_and_skips_gone_clients() {
        use crate::controller::{ControlReply, ControllerCommand};

        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let (controller_tx, controller_rx) = flume::unbounded();
            run_state.controller_channel_rx = Some(controller_rx);
            let mut replies = Vec::new();
            for (users, gone) in [(6, false), (7, false), (5, true)] {
                let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                controller_tx
                    .send((ControllerCommand::Users(users), response_tx))
                    .unwrap();
                if !gone {
                    replies.push(response_rx);
                }
            }
            goose_attack
                .handle_control_requests(&mut run_state)
                .await
                .unwrap();
            for mut reply in replies {
                assert!(matches!(
                    reply.try_recv(),
                    Ok(ControlReply::Outcome(ControlOutcome { ok: true, .. }))
                ));
            }
            assert_eq!(goose_attack.configuration.users, Some(7));
        });
    }

    /// The parent answers at most 16 commands a tick, so a flood cannot
    /// hold the main loop; the rest wait for the next tick.
    #[test]
    #[serial_test::serial]
    fn test_control_drain_cap() {
        use crate::controller::{ControlReply, ControllerCommand};
        use tokio::sync::oneshot::error::TryRecvError;

        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let (controller_tx, controller_rx) = flume::unbounded();
            run_state.controller_channel_rx = Some(controller_rx);
            let mut replies = Vec::new();
            for users in 1..=17 {
                let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                controller_tx
                    .send((ControllerCommand::Users(users), response_tx))
                    .unwrap();
                replies.push(response_rx);
            }
            goose_attack
                .handle_control_requests(&mut run_state)
                .await
                .unwrap();
            let mut last = replies.pop().unwrap();
            for mut reply in replies {
                assert!(matches!(
                    reply.try_recv(),
                    Ok(ControlReply::Outcome(ControlOutcome { ok: true, .. }))
                ));
            }
            assert!(matches!(last.try_recv(), Err(TryRecvError::Empty)));
            assert_eq!(goose_attack.configuration.users, Some(16));

            goose_attack
                .handle_control_requests(&mut run_state)
                .await
                .unwrap();
            assert!(matches!(
                last.try_recv(),
                Ok(ControlReply::Outcome(ControlOutcome { ok: true, .. }))
            ));
            assert_eq!(goose_attack.configuration.users, Some(17));
        });
    }

    /// Dashboard control is answered before queued Controller commands, so
    /// Controller clients, which take no authentication, can not delay it.
    #[test]
    #[serial_test::serial]
    fn test_control_dashboard_drained_first() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let (controller_tx, controller_rx) = flume::unbounded();
            run_state.controller_channel_rx = Some(controller_rx);
            let (dashboard_tx, dashboard_rx) = flume::unbounded();
            run_state.dashboard_control_rx = Some(dashboard_rx);
            let mut queued = Vec::new();
            for users in 1..=MAX_DRAIN + 1 {
                let (respond, reply) = oneshot::channel();
                controller_tx
                    .send((ControllerCommand::Users(users), respond))
                    .unwrap();
                queued.push(reply);
            }
            let (respond, mut reply) = oneshot::channel();
            dashboard_tx
                .send((ControllerCommand::Users(42), respond))
                .unwrap();
            goose_attack
                .handle_control_requests(&mut run_state)
                .await
                .unwrap();
            assert!(matches!(
                reply.try_recv(),
                Ok(ControlReply::Outcome(ControlOutcome { ok: true, .. }))
            ));
            // The dashboard's command ran first, so the last Controller
            // command answered in the tick stands.
            assert_eq!(goose_attack.configuration.users, Some(MAX_DRAIN));
        });
    }

    /// A dashboard command beyond a full dashboard turn is still answered in
    /// the tick, ahead of the Controllers': the dashboard is checked before
    /// each Controller command, not only in its own turn.
    #[test]
    #[serial_test::serial]
    fn test_control_dashboard_checked_before_each_controller_command() {
        use tokio::sync::oneshot::error::TryRecvError;

        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let (controller_tx, controller_rx) = flume::unbounded();
            run_state.controller_channel_rx = Some(controller_rx);
            let (dashboard_tx, dashboard_rx) = flume::unbounded();
            run_state.dashboard_control_rx = Some(dashboard_rx);
            let mut controller = Vec::new();
            for users in 1..=MAX_DRAIN {
                let (respond, reply) = oneshot::channel();
                controller_tx
                    .send((ControllerCommand::Users(users), respond))
                    .unwrap();
                controller.push(reply);
            }
            let mut dashboard = Vec::new();
            for users in 100..=100 + MAX_DRAIN {
                let (respond, reply) = oneshot::channel();
                dashboard_tx
                    .send((ControllerCommand::Users(users), respond))
                    .unwrap();
                dashboard.push(reply);
            }
            goose_attack
                .handle_control_requests(&mut run_state)
                .await
                .unwrap();
            for mut reply in dashboard {
                assert!(matches!(
                    reply.try_recv(),
                    Ok(ControlReply::Outcome(ControlOutcome { ok: true, .. }))
                ));
            }
            // The 17th dashboard command took a Controller slot.
            let mut last = controller.pop().unwrap();
            assert!(matches!(last.try_recv(), Err(TryRecvError::Empty)));
            assert_eq!(goose_attack.configuration.users, Some(MAX_DRAIN - 1));
        });
    }

    /// Requests whose clients have gone do not use up a tick: a live request
    /// behind more than [`MAX_DRAIN`] of them is answered in the same tick.
    #[test]
    #[serial_test::serial]
    fn test_control_gone_clients_do_not_count() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let (controller_tx, controller_rx) = flume::unbounded();
            run_state.controller_channel_rx = Some(controller_rx);
            for users in 1..=MAX_DRAIN + 1 {
                // The reply's receiver is dropped: the client has gone.
                let (respond, _) = oneshot::channel();
                controller_tx
                    .send((ControllerCommand::Users(users), respond))
                    .unwrap();
            }
            let (respond, mut reply) = oneshot::channel();
            controller_tx
                .send((ControllerCommand::Users(42), respond))
                .unwrap();
            goose_attack
                .handle_control_requests(&mut run_state)
                .await
                .unwrap();
            assert!(matches!(
                reply.try_recv(),
                Ok(ControlReply::Outcome(ControlOutcome { ok: true, .. }))
            ));
            assert_eq!(goose_attack.configuration.users, Some(42));
        });
    }

    /// Every `metrics` and `metrics-json` queued in one tick is answered from
    /// one metrics processor request.
    #[test]
    #[serial_test::serial]
    fn test_control_metrics_fetched_once_per_tick() {
        use crate::metrics::MetricsCommand;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let (controller_tx, controller_rx) = flume::unbounded();
            run_state.controller_channel_rx = Some(controller_rx);
            let (metrics_cmd_tx, metrics_cmd_rx) = flume::unbounded();
            run_state.metrics_cmd_tx = metrics_cmd_tx;
            let requests = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&requests);
            tokio::spawn(async move {
                while let Ok(command) = metrics_cmd_rx.recv_async().await {
                    if let MetricsCommand::GetMetrics { respond, .. } = command {
                        counted.fetch_add(1, Ordering::SeqCst);
                        let _ = respond.send(GooseMetrics::default());
                    }
                }
            });
            let mut replies = Vec::new();
            for command in [
                ControllerCommand::Metrics,
                ControllerCommand::MetricsJson,
                ControllerCommand::Metrics,
                ControllerCommand::MetricsJson,
                ControllerCommand::Metrics,
            ] {
                let (respond, reply) = oneshot::channel();
                controller_tx.send((command, respond)).unwrap();
                replies.push(reply);
            }
            goose_attack
                .handle_control_requests(&mut run_state)
                .await
                .unwrap();
            assert_eq!(requests.load(Ordering::SeqCst), 1);
            for mut reply in replies {
                assert!(matches!(reply.try_recv(), Ok(ControlReply::Metrics(_))));
            }
        });
    }

    /// A rate or a time in test plan mode is refused naming users, and
    /// leaves the plan: with no users, Start would weight users forever.
    /// Users switches to a users ramp, and the rate is then accepted.
    #[test]
    #[serial_test::serial]
    fn test_control_ramp_setting_in_test_plan_mode_refused() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) =
                idle_attack(&["--no-autostart", "--test-plan", "2,1s"]).await;
            for (line, field) in [
                ("increaserate 2", "increase_rate"),
                ("decreaserate 2", "decrease_rate"),
                ("runtime 10", "run_time"),
                ("increasetime 10", "increase_time"),
                ("decreasetime 10", "decrease_time"),
            ] {
                assert_eq!(
                    reply(&mut goose_attack, &mut run_state, line).await,
                    (
                        false,
                        format!("failed to configure {field}, a test plan is configured: set users first to switch to a users ramp")
                    ),
                    "{}",
                    line
                );
                assert!(goose_attack.configuration.test_plan.is_some(), "{}", line);
                assert_eq!(goose_attack.configuration.users, None, "{}", line);
            }
            // Clearing a time is not a ramp setting, so test plan mode keeps it.
            assert_eq!(
                reply(&mut goose_attack, &mut run_state, "runtime 0").await,
                (true, "run_time cleared".to_string())
            );
            assert!(goose_attack.configuration.test_plan.is_some());
            assert!(control(&mut goose_attack, &mut run_state, "users 2").await);
            assert!(goose_attack.configuration.test_plan.is_none());
            assert_eq!(
                reply(&mut goose_attack, &mut run_state, "increaserate 2").await,
                (true, "increase_rate configured".to_string())
            );
        });
    }

    /// Run `goose_attack` as a users ramp from its configuration, with
    /// `active` users running and the step machine on its first step.
    pub(crate) fn run_users_ramp(
        goose_attack: &mut GooseAttack,
        run_state: &mut GooseAttackRunState,
        active: usize,
    ) {
        goose_attack.test_plan = TestPlan::build(&goose_attack.configuration).unwrap();
        goose_attack.run_shape =
            crate::run_shape::RunShape::for_configuration(&goose_attack.configuration);
        assert!(goose_attack.run_shape.is_some());
        goose_attack.set_attack_phase(run_state, AttackPhase::Increase);
        goose_attack.step_started = Some(std::time::Instant::now());
        run_state.active_users = active;
    }

    /// A rate set on a running load test is used by the next `users` change.
    /// Users go down, as weighting added users with no scenario never ends;
    /// with no decrease rate set, the increase rate paces it.
    #[test]
    #[serial_test::serial]
    fn test_control_rate_then_users_on_running_test() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&["--users", "6"]).await;
            run_users_ramp(&mut goose_attack, &mut run_state, 6);
            assert_eq!(
                reply(&mut goose_attack, &mut run_state, "increaserate 2").await,
                (true, "increase_rate configured".to_string())
            );
            assert!(control(&mut goose_attack, &mut run_state, "users 2").await);
            // Four fewer users at 2 a second take 2 seconds.
            assert_eq!(goose_attack.test_plan.steps[1], (2, 2_000));
        });
    }

    /// As [`reply`], with the whole outcome, so a test sees the error code
    /// and the field named.
    async fn outcome(
        goose_attack: &mut GooseAttack,
        run_state: &mut GooseAttackRunState,
        line: &str,
    ) -> ControlOutcome {
        let command = line.parse::<ControllerCommand>().expect("a command");
        let (controller_tx, controller_rx) = flume::unbounded();
        run_state.controller_channel_rx = Some(controller_rx);
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        controller_tx.send((command, response_tx)).unwrap();
        goose_attack
            .handle_control_requests(run_state)
            .await
            .unwrap();
        match response_rx.await.expect("the parent must reply") {
            ControlReply::Outcome(outcome) => outcome,
            _ => panic!("{}: not an outcome", line),
        }
    }

    /// A load test with one scenario and a host, so a users increase can
    /// weight its users, built from `args`.
    async fn ramp_attack(args: &[&str]) -> (GooseAttack, GooseAttackRunState) {
        async fn noop(_user: &mut GooseUser) -> crate::goose::TransactionResult {
            Ok(())
        }
        let mut all = vec![
            "--no-telnet",
            "--no-websocket",
            "--no-save",
            "--quiet",
            "--host",
            "http://127.0.0.1",
        ];
        all.extend_from_slice(args);
        let configuration = GooseConfiguration::parse_args_default(&all).unwrap();
        let mut goose_attack = GooseAttack::initialize_with_config(configuration)
            .unwrap()
            .register_scenario(scenario!("Ramp").register_transaction(transaction!(noop)));
        let run_state = goose_attack.initialize_attack().await.unwrap();
        (goose_attack, run_state)
    }

    /// The run's stage and seconds until its hold ends.
    fn stage(goose_attack: &GooseAttack) -> (&'static str, Option<u64>) {
        let shape = goose_attack.run_shape.as_ref().expect("a users ramp");
        (
            shape.stage().as_str(),
            shape.hold_ends_in_secs(std::time::Instant::now()),
        )
    }

    /// Behaviours 1 and 7: Users before the hold ramps to the new count,
    /// then holds for the full run time, then ramps down for the new count;
    /// the stage stays ramp_up, with no countdown, until the hold starts.
    #[test]
    #[serial_test::serial]
    fn test_shape_users_before_hold() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = ramp_attack(&[
                "--users",
                "10",
                "--increase-rate",
                "10",
                "--run-time",
                "60",
                "--decrease-rate",
                "5",
            ])
            .await;
            run_users_ramp(&mut goose_attack, &mut run_state, 4);
            assert_eq!(stage(&goose_attack), ("ramp_up", None));

            // Up to 6: two more users at 10 a second.
            assert!(control(&mut goose_attack, &mut run_state, "users 6").await);
            let steps = &goose_attack.test_plan.steps;
            assert_eq!(steps[1..], [(6, 200), (6, 60_000), (0, 1_200)]);
            assert_eq!(goose_attack.weighted_users.len(), 2);
            assert_eq!(goose_attack.test_plan.current, 1);
            assert_eq!(stage(&goose_attack), ("ramp_up", None));
            assert_eq!(goose_attack.current_target_users(), 6);

            // The ramp ends and the full hold starts.
            run_state.active_users = 6;
            goose_attack.advance_test_plan(&mut run_state);
            assert_eq!(goose_attack.attack_phase, AttackPhase::Maintain);
            assert_eq!(stage(&goose_attack), ("hold", Some(60)));
            assert_eq!(goose_attack.test_plan.steps[2], (6, 60_000));

            // Then the ramp down for 6 users at 5 a second.
            goose_attack.advance_test_plan(&mut run_state);
            assert_eq!(goose_attack.attack_phase, AttackPhase::Decrease);
            assert_eq!(goose_attack.test_plan.steps[3], (0, 1_200));
            assert_eq!(stage(&goose_attack), ("ramping_down", None));
        });
    }

    /// Behaviours 2 and 7: Users during the hold keeps the hold's end, and
    /// a ramp that outlasts it goes straight on to the ramp down.
    #[test]
    #[serial_test::serial]
    fn test_shape_users_during_hold() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) =
                ramp_attack(&["--users", "10", "--increase-rate", "1", "--run-time", "60"]).await;
            run_users_ramp(&mut goose_attack, &mut run_state, 10);
            goose_attack.advance_test_plan(&mut run_state);
            assert_eq!(stage(&goose_attack), ("hold", Some(60)));
            let end = match goose_attack.run_shape.as_ref().unwrap().hold {
                crate::run_shape::Hold::Timed { end, .. } => end,
                hold => panic!("{:?}", hold),
            };

            // Five more users at 1 a second: the hold keeps its end.
            assert!(control(&mut goose_attack, &mut run_state, "users 15").await);
            assert_eq!(
                goose_attack.test_plan.steps[1..],
                [(15, 5_000), (15, 0), (0, 0)]
            );
            assert_eq!(stage(&goose_attack), ("hold", Some(60)));
            match goose_attack.run_shape.as_ref().unwrap().hold {
                crate::run_shape::Hold::Timed { end: kept, .. } => assert_eq!(kept, end),
                hold => panic!("{:?}", hold),
            }
            // What is left of the hold is set as the ramp ends.
            run_state.active_users = 15;
            goose_attack.advance_test_plan(&mut run_state);
            let left = goose_attack.test_plan.steps[2].1;
            assert!((59_000..=60_000).contains(&left), "{}", left);

            // A ramp that outlasts the hold: the countdown stays at 0, and
            // the hold's step is empty, so the ramp down comes next.
            goose_attack.run_shape.as_mut().unwrap().hold = crate::run_shape::Hold::Timed {
                start: end - Duration::from_secs(60),
                end: std::time::Instant::now(),
            };
            assert!(control(&mut goose_attack, &mut run_state, "users 12").await);
            assert_eq!(stage(&goose_attack), ("hold", Some(0)));
            run_state.active_users = 12;
            goose_attack.advance_test_plan(&mut run_state);
            assert_eq!(goose_attack.test_plan.steps[2], (12, 0));
            goose_attack.advance_test_plan(&mut run_state);
            assert_eq!(stage(&goose_attack), ("ramping_down", None));
        });
    }

    /// Behaviours 3 and 7: Users during the ramp down abandons it and holds
    /// at the new count until stopped; the run is running again, so rates
    /// are accepted, and a second Users still holds until stopped.
    #[test]
    #[serial_test::serial]
    fn test_shape_users_during_ramp_down() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) =
                ramp_attack(&["--users", "10", "--run-time", "1", "--decrease-rate", "1"]).await;
            run_users_ramp(&mut goose_attack, &mut run_state, 10);
            goose_attack.advance_test_plan(&mut run_state);
            goose_attack.advance_test_plan(&mut run_state);
            assert_eq!(stage(&goose_attack), ("ramping_down", None));
            run_state.active_users = 8;
            assert_eq!(
                outcome(&mut goose_attack, &mut run_state, "increaserate 2")
                    .await
                    .error,
                Some("invalid_phase")
            );

            assert!(control(&mut goose_attack, &mut run_state, "users 5").await);
            assert_eq!(goose_attack.test_plan.steps[1..], [(5, 3_000)]);
            assert_eq!(stage(&goose_attack), ("hold_until_stopped", None));
            assert!(control(&mut goose_attack, &mut run_state, "increaserate 2").await);
            // Four fewer of 8 users, at the decrease rate of 1 a second.
            assert!(control(&mut goose_attack, &mut run_state, "users 4").await);
            assert_eq!(goose_attack.test_plan.steps[1..], [(4, 4_000)]);
            assert_eq!(stage(&goose_attack), ("hold_until_stopped", None));
            // The ramp ends with nothing after it: the run holds.
            goose_attack.advance_test_plan(&mut run_state);
            assert_eq!(goose_attack.attack_phase, AttackPhase::Maintain);
            assert_eq!(stage(&goose_attack), ("hold_until_stopped", None));
        });
    }

    /// Behaviours 4 and 7: a run with no run time and no ramp down ramps up,
    /// then holds until stopped; Users ramps to the new count and keeps
    /// holding.
    #[test]
    #[serial_test::serial]
    fn test_shape_users_holding_until_stopped() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) =
                ramp_attack(&["--users", "10", "--increase-rate", "5"]).await;
            run_users_ramp(&mut goose_attack, &mut run_state, 10);
            assert_eq!(stage(&goose_attack), ("ramp_up", None));
            goose_attack.advance_test_plan(&mut run_state);
            assert_eq!(goose_attack.attack_phase, AttackPhase::Maintain);
            assert_eq!(stage(&goose_attack), ("hold_until_stopped", None));

            assert!(control(&mut goose_attack, &mut run_state, "users 20").await);
            assert_eq!(goose_attack.test_plan.steps[1..], [(20, 2_000)]);
            assert_eq!(stage(&goose_attack), ("hold_until_stopped", None));
        });
    }

    /// Behaviour 5: a decrease rate while running regenerates the ramp down
    /// still ahead, adds one where all users would stop at once, and only
    /// sets the configuration on a run holding until stopped.
    #[test]
    #[serial_test::serial]
    fn test_shape_decrease_rate_while_running() {
        finishes(|| async {
            // A ramp down still ahead, regenerated during the ramp up and
            // during the hold; the ramp up in progress is not re-paced.
            let (mut goose_attack, mut run_state) = ramp_attack(&[
                "--users",
                "10",
                "--increase-time",
                "5",
                "--run-time",
                "60",
                "--decrease-rate",
                "5",
            ])
            .await;
            run_users_ramp(&mut goose_attack, &mut run_state, 4);
            assert!(control(&mut goose_attack, &mut run_state, "decreaserate 2").await);
            assert_eq!(
                goose_attack.test_plan.steps,
                [(10, 5_000), (10, 60_000), (0, 5_000)]
            );
            goose_attack.advance_test_plan(&mut run_state);
            assert!(control(&mut goose_attack, &mut run_state, "decreaserate 10").await);
            assert_eq!(goose_attack.test_plan.steps[2], (0, 1_000));
            assert_eq!(stage(&goose_attack), ("hold", Some(60)));

            // A timed hold with no ramp down gets one, and still holds first.
            let (mut goose_attack, mut run_state) =
                ramp_attack(&["--users", "10", "--run-time", "60"]).await;
            run_users_ramp(&mut goose_attack, &mut run_state, 10);
            goose_attack.advance_test_plan(&mut run_state);
            assert_eq!(goose_attack.test_plan.steps[2], (0, 0));
            assert!(control(&mut goose_attack, &mut run_state, "decreaserate 2").await);
            assert_eq!(goose_attack.test_plan.steps[2], (0, 5_000));
            assert_eq!(stage(&goose_attack), ("hold", Some(60)));

            // Holding until stopped: only the configuration changes.
            let (mut goose_attack, mut run_state) = ramp_attack(&["--users", "10"]).await;
            run_users_ramp(&mut goose_attack, &mut run_state, 10);
            goose_attack.advance_test_plan(&mut run_state);
            let steps = goose_attack.test_plan.steps.clone();
            assert!(control(&mut goose_attack, &mut run_state, "decreaserate 2").await);
            assert_eq!(goose_attack.test_plan.steps, steps);
            assert_eq!(
                goose_attack.configuration.decrease_rate.as_deref(),
                Some("2")
            );
            assert_eq!(stage(&goose_attack), ("hold_until_stopped", None));
        });
    }

    /// Behaviour 6: a rate alone clears its own time in Idle; while running,
    /// a rate is refused while its own time is set, naming the rate.
    #[test]
    #[serial_test::serial]
    fn test_shape_rate_with_its_time() {
        finishes(|| async {
            let args = [
                "--users",
                "10",
                "--increase-time",
                "5",
                "--decrease-time",
                "5",
            ];
            let (mut goose_attack, mut run_state) = ramp_attack(&args).await;
            assert!(control(&mut goose_attack, &mut run_state, "increaserate 2").await);
            assert_eq!(goose_attack.configuration.increase_time, None);

            let (mut goose_attack, mut run_state) = ramp_attack(&args).await;
            run_users_ramp(&mut goose_attack, &mut run_state, 4);
            let steps = goose_attack.test_plan.steps.clone();
            for (line, field, time) in [
                ("increaserate 2", "increase_rate", "increase_time"),
                ("decreaserate 2", "decrease_rate", "decrease_time"),
            ] {
                let refused = outcome(&mut goose_attack, &mut run_state, line).await;
                assert!(!refused.ok, "{}", line);
                assert_eq!(refused.error, Some("conflicting_settings"), "{}", line);
                assert_eq!(refused.field, Some(field), "{}", line);
                assert_eq!(
                    refused.message,
                    format!(
                        "failed to configure {field}, {time} is set and changes only when idle"
                    )
                );
            }
            assert_eq!(goose_attack.test_plan.steps, steps);
            assert_eq!(
                goose_attack.configuration.increase_time.as_deref(),
                Some("5")
            );
        });
    }

    /// Decision 5: Users is refused while a test plan runs, naming users;
    /// the rates are refused there too, and a test plan replaces it.
    #[test]
    #[serial_test::serial]
    fn test_users_refused_during_test_plan() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) =
                ramp_attack(&["--test-plan", "10,1s;10,1m;0,10s"]).await;
            goose_attack.test_plan = TestPlan::build(&goose_attack.configuration).unwrap();
            goose_attack.set_attack_phase(&mut run_state, AttackPhase::Increase);
            run_state.active_users = 4;
            let steps = goose_attack.test_plan.steps.clone();
            for line in ["users 5", "increaserate 2", "decreaserate 2"] {
                let refused = outcome(&mut goose_attack, &mut run_state, line).await;
                assert!(!refused.ok, "{}", line);
                assert_eq!(refused.error, Some("invalid_phase"), "{}", line);
                assert_eq!(goose_attack.test_plan.steps, steps, "{}", line);
            }
            let refused = outcome(&mut goose_attack, &mut run_state, "users 5").await;
            assert_eq!(refused.field, Some("users"));
            assert_eq!(
                refused.message,
                "failed to reconfigure users, the test plan sets users: edit the plan, or stop and switch mode"
            );
            assert_eq!(goose_attack.configuration.users, None);
            assert!(goose_attack.configuration.test_plan.is_some());

            // The first step's end no longer panics: no ramp setting got
            // through, and the guard holds with no users either way.
            goose_attack.reset_metrics(&mut run_state).await.unwrap();
            run_state.all_users_spawned = false;
            goose_attack.configuration.test_plan = None;
            goose_attack.reset_metrics(&mut run_state).await.unwrap();
        });
    }

    /// The command line's cross field rules reach Controller commands: the
    /// launch flags in every phase, the 2 user minimum for a time in Idle.
    #[test]
    #[serial_test::serial]
    fn test_control_cross_field_rules() {
        finishes(|| async {
            let refused = |outcome: ControlOutcome, field: &str| {
                assert!(!outcome.ok, "{:?}", outcome);
                assert_eq!(outcome.error, Some("conflicting_settings"), "{:?}", outcome);
                assert_eq!(outcome.field, Some(field), "{:?}", outcome);
            };
            let (mut goose_attack, mut run_state) =
                ramp_attack(&["--users", "4", "--increase-time", "5"]).await;
            refused(
                outcome(&mut goose_attack, &mut run_state, "users 1").await,
                "users",
            );
            assert_eq!(goose_attack.configuration.users, Some(4));
            assert!(control(&mut goose_attack, &mut run_state, "users 2").await);
            // Mid run, times do not shape a Users change.
            run_users_ramp(&mut goose_attack, &mut run_state, 2);
            assert!(control(&mut goose_attack, &mut run_state, "users 1").await);

            let (mut goose_attack, mut run_state) = ramp_attack(&["--users", "1"]).await;
            for (line, field) in [
                ("increasetime 5", "increase_time"),
                ("decreasetime 5", "decrease_time"),
            ] {
                refused(
                    outcome(&mut goose_attack, &mut run_state, line).await,
                    field,
                );
            }
            assert!(control(&mut goose_attack, &mut run_state, "increasetime 0").await);

            for flag in ["--iterations", "--no-reset-metrics"] {
                let args: &[&str] = if flag == "--iterations" {
                    &["--users", "4", "--iterations", "2"]
                } else {
                    &["--users", "4", "--no-reset-metrics"]
                };
                let (mut goose_attack, mut run_state) = ramp_attack(args).await;
                refused(
                    outcome(&mut goose_attack, &mut run_state, "testplan 2,1s").await,
                    "test_plan",
                );
                if flag == "--iterations" {
                    refused(
                        outcome(&mut goose_attack, &mut run_state, "runtime 10").await,
                        "run_time",
                    );
                }
                run_users_ramp(&mut goose_attack, &mut run_state, 4);
                refused(
                    outcome(&mut goose_attack, &mut run_state, "testplan 2,1s").await,
                    "test_plan",
                );
                assert!(goose_attack.configuration.test_plan.is_none(), "{}", flag);
            }
        });
    }

    /// The rates are refused while stopping, in Shutdown, during the ramp
    /// down and during a running test plan, as `invalid_phase`.
    #[test]
    #[serial_test::serial]
    fn test_control_rates_refused_by_phase() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = ramp_attack(&["--users", "4"]).await;
            run_users_ramp(&mut goose_attack, &mut run_state, 4);
            goose_attack.cancel_attack(&mut run_state).await.unwrap();
            for line in ["increaserate 2", "decreaserate 2"] {
                let refused = outcome(&mut goose_attack, &mut run_state, line).await;
                assert_eq!(refused.error, Some("invalid_phase"), "{}", line);
                assert!(
                    refused.message.ends_with("load test is stopping"),
                    "{}",
                    line
                );
            }
            goose_attack.set_attack_phase(&mut run_state, AttackPhase::Shutdown);
            assert_eq!(
                outcome(&mut goose_attack, &mut run_state, "increaserate 2")
                    .await
                    .error,
                Some("invalid_phase")
            );
        });
    }

    /// Read from a telnet Controller until its prompt.
    async fn telnet_reply(client: &mut TcpStream) -> String {
        let mut text = Vec::new();
        let mut buf = [0; 1024];
        while !text.ends_with(b"goose> ") {
            let n = tokio::time::timeout(Duration::from_secs(10), client.read(&mut buf))
                .await
                .expect("the telnet Controller must answer")
                .unwrap();
            assert!(n > 0, "telnet Controller closed: {:?}", text);
            text.extend_from_slice(&buf[..n]);
        }
        String::from_utf8(text).unwrap()
    }

    /// A telnet command sent with no line ending runs.
    #[tokio::test]
    async fn telnet_command_without_line_ending_runs() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        let (channel_tx, channel_rx) = flume::unbounded::<ControlRequest>();
        tokio::spawn(serve_client(
            socket,
            ControllerProtocol::Telnet,
            channel_tx,
            "telnet client".to_string(),
            CONTROL_TIMEOUT,
        ));
        assert_eq!(telnet_reply(&mut client).await, "goose> ");
        client.write_all(b"users 5").await.unwrap();
        let (command, respond) =
            tokio::time::timeout(Duration::from_secs(10), channel_rx.recv_async())
                .await
                .expect("the command must reach the parent")
                .unwrap();
        assert!(matches!(command, ControllerCommand::Users(5)));
        let _ = respond.send(ControlReply::Outcome(ControlOutcome {
            ok: true,
            error: None,
            message: "users configured".to_string(),
            phase: "idle".to_string(),
            active_users: 0,
            target_users: Some(5),
            field: None,
        }));
        assert_eq!(telnet_reply(&mut client).await, "users configured\ngoose> ");
    }

    /// A Controller client whose command the parent never answers is told
    /// so once the timeout passes.
    #[tokio::test]
    async fn telnet_command_unanswered_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        // Held and never read, so the command is never answered.
        let (channel_tx, _channel_rx) = flume::unbounded::<ControlRequest>();
        tokio::spawn(serve_client(
            socket,
            ControllerProtocol::Telnet,
            channel_tx,
            "telnet client".to_string(),
            Duration::from_millis(100),
        ));
        assert_eq!(telnet_reply(&mut client).await, "goose> ");
        client.write_all(b"users 5\n").await.unwrap();
        assert_eq!(
            telnet_reply(&mut client).await,
            "control timed out, it may still apply: check before retrying\ngoose> "
        );
    }

    /// A WebSocket Controller client served by [`serve_client`], and the
    /// channel its commands arrive on.
    async fn websocket_client() -> (WebSocketStream<TcpStream>, flume::Receiver<ControlRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (channel_tx, channel_rx) = flume::unbounded();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            serve_client(
                socket,
                ControllerProtocol::WebSocket,
                channel_tx,
                "WebSocket client".to_string(),
                CONTROL_TIMEOUT,
            )
            .await;
        });
        let stream = TcpStream::connect(address).await.unwrap();
        let (client, _) = tokio_tungstenite::client_async(format!("ws://{address}/"), stream)
            .await
            .unwrap();
        (client, channel_rx)
    }

    /// The next frame a WebSocket client gets, `None` once the connection
    /// has ended.
    async fn next_frame(client: &mut WebSocketStream<TcpStream>) -> Option<Message> {
        tokio::time::timeout(Duration::from_secs(10), client.next())
            .await
            .expect("the WebSocket Controller must answer or end the connection")
            .and_then(Result::ok)
    }

    /// A WebSocket text or binary message longer than [`MAX_CONTROLLER_MESSAGE`]
    /// is answered with a 1009 close naming the limit, and the connection ends.
    #[tokio::test]
    async fn websocket_message_too_big_closed() {
        for message in [
            Message::Text("x".repeat(131_073).into()),
            Message::Binary(vec![b'x'; 131_073].into()),
        ] {
            let (mut client, _channel_rx) = websocket_client().await;
            client.send(message).await.unwrap();
            match next_frame(&mut client).await {
                Some(Message::Close(Some(frame))) => {
                    assert_eq!(frame.code, CloseCode::Size);
                    assert_eq!(frame.reason, "message too big, at most 131072 bytes");
                }
                other => panic!("expected a 1009 close, got {:?}", other),
            }
            assert!(next_frame(&mut client).await.is_none());
        }
    }

    /// A WebSocket frame announcing more than twice the limit ends the
    /// connection from its header, before any of it is buffered: with the
    /// same 1009 close if one can still be written.
    #[tokio::test]
    async fn websocket_message_over_hard_bound_ends_connection() {
        let (mut client, _channel_rx) = websocket_client().await;
        // A masked text frame header announcing 300 KiB, and no payload: only
        // a limit read from the header answers it; a buffering server waits
        // for the payload and the test times out.
        let mut header = vec![0x81, 0xFF];
        header.extend_from_slice(&(300u64 * 1024).to_be_bytes());
        header.extend_from_slice(&[1, 2, 3, 4]);
        client.get_mut().write_all(&header).await.unwrap();
        match next_frame(&mut client).await {
            None => {}
            Some(Message::Close(frame)) => {
                if let Some(frame) = frame {
                    assert_eq!(frame.code, CloseCode::Size);
                    assert_eq!(frame.reason, "message too big, at most 131072 bytes");
                }
                assert!(next_frame(&mut client).await.is_none());
            }
            Some(other) => panic!("expected the connection to end, got {:?}", other),
        }
    }

    /// A test plan of exactly [`MAX_CONTROL_PLAN_BYTES`] in a WebSocket
    /// message is parsed and applied, and one byte more is refused.
    #[test]
    #[serial_test::serial]
    fn websocket_test_plan_at_limit_applied() {
        finishes(|| async {
            let (mut goose_attack, mut run_state) = idle_attack(&[]).await;
            let (mut client, channel_rx) = websocket_client().await;
            run_state.controller_channel_rx = Some(channel_rx);
            // 100 steps, the most a plan has, padded to 4096 bytes.
            let mut plan = "1,1s;0,1s;".repeat(49);
            plan.push_str("1,1s;0,");
            let padding = MAX_CONTROL_PLAN_BYTES - plan.len() - 2;
            plan.push_str(&"0".repeat(padding));
            plan.push_str("1s");
            assert_eq!(plan.len(), MAX_CONTROL_PLAN_BYTES);
            for (plan, reply) in [
                (format!("{plan}0"), INVALID_PLAN_MSG),
                (plan.clone(), "test-plan configured"),
            ] {
                let request = format!(r#"{{"request":"testplan {}"}}"#, plan);
                client.send(Message::Text(request.into())).await.unwrap();
                // Answer commands as the main loop does until the reply arrives.
                let answered = async {
                    loop {
                        goose_attack
                            .handle_control_requests(&mut run_state)
                            .await
                            .unwrap();
                        if let Ok(frame) =
                            tokio::time::timeout(Duration::from_millis(10), next_frame(&mut client))
                                .await
                        {
                            return frame;
                        }
                    }
                };
                match tokio::time::timeout(Duration::from_secs(10), answered).await {
                    Ok(Some(Message::Text(text))) => assert_eq!(
                        serde_json::from_str::<ControllerWebSocketResponse>(&text)
                            .unwrap()
                            .response,
                        reply
                    ),
                    other => panic!("expected a reply, got {:?}", other),
                }
            }
            let applied = goose_attack.configuration.test_plan.as_ref().unwrap();
            assert_eq!(applied.steps.len(), MAX_CONTROL_PLAN_STEPS);
            assert_eq!(applied.total_users(), 50);
        });
    }

    /// A binary WebSocket message is answered as main answered it.
    #[tokio::test]
    async fn websocket_binary_message_refused() {
        let (mut client, _channel_rx) = websocket_client().await;
        client
            .send(Message::Binary(b"{\"request\":\"config\"}".to_vec().into()))
            .await
            .unwrap();
        match next_frame(&mut client).await {
            Some(Message::Text(text)) => {
                let response: ControllerWebSocketResponse = serde_json::from_str(&text).unwrap();
                assert!(!response.success);
                assert_eq!(response.response, INVALID_JSON);
            }
            other => panic!("expected a reply, got {:?}", other),
        }
    }
}
