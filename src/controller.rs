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
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};
use tokio_tungstenite::tungstenite::Message;
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
    "increaserate FLOAT set per-second rate users increase\n",
    "increase-time TIME set total time to take increasing users\n",
    "decreaserate FLOAT set per-second rate users decrease\n",
    "decrease-time TIME set total time to take decreasing users\n",
    "users INT          set number of simulated users (1 to 100000)\n",
    "runtime TIME       set how long to run test, (ie 1h30m5s)\n",
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
///     with a `ControlOutcome` whose message
///     every front end shows.
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
    /// Stops a running (or stating) load test.
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
    /// are launched, per second: a finite number above 0.
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
    /// [`GooseUser`](../goose/struct.GooseUser.html)s, in seconds.
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
    /// stopped, per second: a finite number above 0.
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
    /// [`GooseUser`](../goose/struct.GooseUser.html)s, in seconds.
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
    /// Configure how long the load test should run before stopping and
    /// returning to an idle state, in seconds.
    ///
    /// # Example
    /// Tells Goose to run the load test for 1 minute, before automatically stopping.
    /// ```notest
    /// runtime 60
    /// ```
    ///
    /// Goose must be idle to process this command: the run time is read when
    /// the load test starts.
    RunTime(usize),
    /// Define a load test plan, starting 1 to 100000 users in total. This will
    /// replace the previously configured test plan, if any.
    ///
    /// # Example
    /// Tells Goose to launch 10 users in 5 seconds, maintain them for 30 seconds, and then spend 5 seconds
    /// stopping them.
    /// ```notest
    /// testplan "10,5s;10,30s;0,5s"
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

/// A rate: a finite number of users per second above 0.
fn rate(value: &str, error: &'static str) -> Result<f32, &'static str> {
    match value.parse::<f32>() {
        Ok(rate) if rate.is_finite() && rate > 0.0 => Ok(rate),
        _ => Err(error),
    }
}

/// A time span in seconds: whole seconds, or hours, minutes and seconds in
/// that order, short enough for a `usize` in milliseconds.
fn time(value: &str, error: &'static str) -> Result<usize, &'static str> {
    lazy_static! {
        static ref TIME: Regex = Regex::new(r"^([0-9]+|([0-9]+h)?([0-9]+m)?([0-9]+s)?)$").unwrap();
    }
    let value = value.to_ascii_lowercase();
    if value.is_empty() || !TIME.is_match(&value) {
        return Err(error);
    }
    crate::util::parse_timespan(&value).ok_or(error)
}

/// Parse a line from a telnet or WebSocket client into a command, or the
/// error the client is sent back.
impl FromStr for ControllerCommand {
    type Err = &'static str;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let line = line.trim();
        let (verb, value) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(verb, value)| (verb, value.trim()));
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
            "host" | "hostname" | "host_name" | "host-name" => match url::Url::parse(value) {
                Ok(url) if ["http", "https"].contains(&url.scheme()) => {
                    Ok(Self::Host(value.to_string()))
                }
                _ => Err("host must be a valid url starting with http:// or https://"),
            },
            "users" | "user" => value
                .parse()
                .map_err(|_| INVALID_USERS_MSG)
                .and_then(Self::users),
            "increaserate" | "increase_rate" | "increase-rate" => {
                rate(value, "increase_rate must be a number above 0").map(Self::IncreaseRate)
            }
            "decreaserate" | "decrease_rate" | "decrease-rate" => {
                rate(value, "decrease_rate must be a number above 0").map(Self::DecreaseRate)
            }
            "increasetime" | "increase_time" | "increase-time" => time(
                value,
                "increase_time must be a time span such as 1h30m5s",
            )
            .map(Self::IncreaseTime),
            "decreasetime" | "decrease_time" | "decrease-time" => time(
                value,
                "decrease_time must be a time span such as 1h30m5s",
            )
            .map(Self::DecreaseTime),
            "run" | "runtime" | "run_time" | "run-time" => {
                time(value, "run_time must be a time span such as 1h30m5s").map(Self::RunTime)
            }
            "testplan" | "test_plan" | "test-plan" | "plan" => match value.parse::<TestPlan>() {
                Ok(plan) if plan.total_users() <= MAX_CONTROL_USERS => Ok(Self::TestPlan(plan)),
                _ => Err("test-plan must be users,time steps such as 10,5m;10,1h;0,30s, with 1 to 100000 users in total"),
            },
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
    /// Current load test metrics, for `metrics` and `metrics-json`.
    Metrics(Box<GooseMetrics>),
}

/// A command and where to send the parent's reply, on the one channel that
/// telnet, WebSocket and the dashboard share.
pub(crate) type ControlRequest = (ControllerCommand, oneshot::Sender<ControlReply>);

/// Send `command` to the parent and wait up to [`CONTROL_TIMEOUT`] for its
/// reply. Fails with `"unavailable"` when the parent has gone or dropped
/// the request, and `"timeout"` when it did not answer in time.
pub(crate) async fn send_command(
    channel_tx: &flume::Sender<ControlRequest>,
    command: ControllerCommand,
) -> Result<ControlReply, &'static str> {
    let (respond, reply) = oneshot::channel();
    channel_tx
        .send((command, respond))
        .map_err(|_| "unavailable")?;
    match tokio::time::timeout(CONTROL_TIMEOUT, reply).await {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(_)) => Err("unavailable"),
        Err(_) => Err("timeout"),
    }
}

/// The parent process side of the Controller functionality.
impl GooseAttack {
    /// Answer up to 16 queued control requests, from telnet, WebSocket and
    /// the dashboard alike, so a flood cannot hold the main loop. A request
    /// whose client has gone (timed out or disconnected) is skipped, so it
    /// is not applied after the client was told it failed.
    pub(crate) async fn handle_control_requests(
        &mut self,
        goose_attack_run_state: &mut GooseAttackRunState,
    ) -> Result<(), GooseError> {
        let Some(channel_rx) = goose_attack_run_state.controller_channel_rx.clone() else {
            return Ok(());
        };
        for (command, respond) in channel_rx.try_iter().take(16) {
            if respond.is_closed() {
                debug!("[control]: {command:?} skipped, its client has gone");
                continue;
            }
            info!("[control]: {command:?}");
            self.control(goose_attack_run_state, command, respond)
                .await?;
        }
        Ok(())
    }

    /// Apply one command and reply to it. A hard error is answered as an
    /// `internal` outcome, then returned.
    async fn control(
        &mut self,
        rs: &mut GooseAttackRunState,
        command: ControllerCommand,
        respond: oneshot::Sender<ControlReply>,
    ) -> Result<(), GooseError> {
        let idle = self.attack_phase == AttackPhase::Idle;
        let refused = |goose: &Self, rs: &GooseAttackRunState, message| {
            goose.control_outcome(rs, false, Some("invalid_phase"), message, None)
        };
        let outcome = match command {
            ControllerCommand::Config | ControllerCommand::ConfigJson => {
                let config = Box::new(self.configuration.clone());
                let _ = respond.send(ControlReply::Config(config));
                return Ok(());
            }
            ControllerCommand::Metrics | ControllerCommand::MetricsJson => {
                let metrics = self.metrics_for_controller(rs).await;
                let _ = respond.send(ControlReply::Metrics(Box::new(metrics)));
                return Ok(());
            }
            ControllerCommand::Start => match self.control_start_begin(rs) {
                // Reply before control_start_finish, so the client is not
                // blocked on reset_run_state and test_start.
                Ok(ControlStartBegin::Accepted(outcome)) => {
                    let _ = respond.send(ControlReply::Outcome(outcome));
                    return self.control_start_finish(rs).await;
                }
                Ok(ControlStartBegin::Rejected(outcome)) => Ok(outcome),
                Err(e) => Err(e),
            },
            ControllerCommand::Stop => self.control_stop(rs).await,
            ControllerCommand::Quit => Ok(self.control_quit(rs)),
            ControllerCommand::Shutdown => self.control_shutdown(rs).await,
            ControllerCommand::Users(users) => self.control_set_users(rs, users),
            ControllerCommand::TestPlan(plan) => self.control_set_plan(rs, plan),
            ControllerCommand::IncreaseRate(rate) => {
                self.configuration.increase_time = "0".to_string();
                self.configuration.increase_rate = Some(rate.to_string());
                self.configuration.test_plan = None;
                Ok(self.control_outcome(rs, true, None, "increase_rate configured", None))
            }
            ControllerCommand::DecreaseRate(rate) => {
                self.configuration.decrease_time = "0".to_string();
                self.configuration.decrease_rate = Some(rate.to_string());
                self.configuration.test_plan = None;
                Ok(self.control_outcome(rs, true, None, "decrease_rate configured", None))
            }
            ControllerCommand::Host(_) if !idle => Ok(refused(
                self,
                rs,
                "failed to reconfigure host, be sure host is valid and load test is idle",
            )),
            ControllerCommand::IncreaseTime(_) if !idle => Ok(refused(
                self,
                rs,
                "failed to configure increase_time, be sure load test is idle",
            )),
            ControllerCommand::DecreaseTime(_) if !idle => Ok(refused(
                self,
                rs,
                "failed to configure decrease_time, be sure load test is idle",
            )),
            ControllerCommand::RunTime(_) if !idle => Ok(refused(
                self,
                rs,
                "failed to configure run_time, be sure load test is idle",
            )),
            ControllerCommand::Host(host) => {
                self.configuration.host = host;
                Ok(self.control_outcome(rs, true, None, "host configured", None))
            }
            ControllerCommand::IncreaseTime(seconds) => {
                self.configuration.increase_rate = None;
                self.configuration.increase_time = seconds.to_string();
                self.configuration.test_plan = None;
                Ok(self.control_outcome(rs, true, None, "increase_time configured", None))
            }
            ControllerCommand::DecreaseTime(seconds) => {
                self.configuration.decrease_rate = None;
                self.configuration.decrease_time = seconds.to_string();
                self.configuration.test_plan = None;
                Ok(self.control_outcome(rs, true, None, "decrease_time configured", None))
            }
            ControllerCommand::RunTime(seconds) => {
                self.configuration.run_time = seconds.to_string();
                self.configuration.test_plan = None;
                Ok(self.control_outcome(rs, true, None, "run_time configured", None))
            }
        };
        match outcome {
            Ok(outcome) => {
                info!(
                    "[control]: ok={} error={:?} phase={} active_users={} target_users={:?}: {}",
                    outcome.ok,
                    outcome.error,
                    outcome.phase,
                    outcome.active_users,
                    outcome.target_users,
                    outcome.message
                );
                let _ = respond.send(ControlReply::Outcome(outcome));
                Ok(())
            }
            Err(e) => {
                let internal = self.control_outcome(
                    rs,
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
            configuration.telnet_host, configuration.telnet_port
        ),
        ControllerProtocol::WebSocket => format!(
            "{}:{}",
            configuration.websocket_host, configuration.websocket_port
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
            protocol.clone(),
            channel_tx.clone(),
            client,
        ));
    }

    Ok(())
}

/// Goose supports two different Controller protocols: telnet and WebSocket.
#[derive(Clone, Debug)]
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

/// Reply to a WebSocket message that is not a json request.
const INVALID_JSON: &str =
    "invalid json, see Goose book https://book.goose.rs/controller/websocket.html";

impl Transport {
    /// The next line the client sent; `None` once the client has gone,
    /// `Some(Err(reply))` for a message that holds no line.
    async fn read(&mut self) -> Option<Result<String, &'static str>> {
        match self {
            Transport::Telnet(socket) => {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).await.ok().filter(|n| *n > 0)?;
                // Invalid utf-8 is a corrupted request: disconnect.
                let text = str::from_utf8(&buf[..n]).ok()?;
                Some(Ok(text.lines().next().unwrap_or_default().to_string()))
            }
            Transport::WebSocket(stream) => {
                let request = match stream.next().await?.ok()? {
                    Message::Text(text) => {
                        serde_json::from_str::<ControllerWebSocketRequest>(&text).ok()
                    }
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
    async fn close(&mut self, reason: &'static str) {
        if let Transport::WebSocket(stream) = self {
            let frame = CloseFrame {
                code: CloseCode::Normal,
                reason: reason.into(),
            };
            if stream.send(Message::Close(Some(frame))).await.is_err() {
                warn!("failed to write data to stream");
            }
        }
    }
}

/// Talk to one Controller client until it exits, shuts Goose down, or goes.
async fn serve_client(
    socket: TcpStream,
    protocol: ControllerProtocol,
    channel_tx: flume::Sender<ControlRequest>,
    client: String,
) {
    info!("{client} connected");
    let mut transport = match protocol {
        ControllerProtocol::Telnet => Transport::Telnet(socket),
        ControllerProtocol::WebSocket => match tokio_tungstenite::accept_async(socket).await {
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
                transport.close("exit").await;
                break;
            }
            continue;
        }
        let command = match line.parse::<ControllerCommand>() {
            Ok(command) => command,
            Err(UNRECOGNIZED) if websocket => {
                let reply = "unrecognized command, see Goose book https://book.goose.rs/controller/websocket.html";
                transport.write(false, reply).await;
                continue;
            }
            Err(reply) => {
                transport.write(false, reply).await;
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
        let (success, response) = match send_command(&channel_tx, command).await {
            Ok(ControlReply::Outcome(outcome)) => (outcome.ok, outcome.message),
            Ok(ControlReply::Config(config)) if json => (true, to_json(&config)),
            Ok(ControlReply::Config(config)) => (true, format!("{config:#?}")),
            Ok(ControlReply::Metrics(metrics)) if json => (true, to_json(&metrics)),
            Ok(ControlReply::Metrics(metrics)) => (true, metrics.to_string()),
            Err("timeout") => (
                false,
                "control timed out, it may still apply: check before retrying".to_string(),
            ),
            // The parent has gone: disconnect.
            Err(_) => {
                transport
                    .write(false, "parent process has closed the controller channel")
                    .await;
                break;
            }
        };
        transport.write(success, &response).await;
        if shutdown {
            transport.close("shutdown").await;
            break;
        }
    }
    info!("{client} disconnected");
}

/// A reply the Controller gives itself, and whether the client then exits:
/// `help` and `exit` never reach the parent.
fn local_reply(line: &str) -> Option<(&'static str, bool)> {
    match line.trim().to_ascii_lowercase().as_str() {
        "help" | "?" => Some((HELP, false)),
        "exit" | "quit" | "q" => Some(("goodbye!", true)),
        _ => None,
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
mod tests {
    use super::*;

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
        const HOST: &str = "host must be a valid url starting with http:// or https://";
        const INCREASE_RATE: &str = "increase_rate must be a number above 0";
        const DECREASE_RATE: &str = "decrease_rate must be a number above 0";
        const INCREASE_TIME: &str = "increase_time must be a time span such as 1h30m5s";
        const DECREASE_TIME: &str = "decrease_time must be a time span such as 1h30m5s";
        const RUN_TIME: &str = "run_time must be a time span such as 1h30m5s";
        const TEST_PLAN: &str =
            "test-plan must be users,time steps such as 10,5m;10,1h;0,30s, with 1 to 100000 users in total";

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
            ("increaserate 0", INCREASE_RATE),
            ("increaserate 0.0", INCREASE_RATE),
            ("increaserate .", INCREASE_RATE),
            ("increaserate 1.2.3", INCREASE_RATE),
            ("increaserate -1", INCREASE_RATE),
            ("decreaserate 0", DECREASE_RATE),
            ("decreaserate .", DECREASE_RATE),
            ("increasetime .1", INCREASE_TIME),
            ("increasetime 10d", INCREASE_TIME),
            ("increase-time 2m1h", INCREASE_TIME),
            ("increasetime 99999999999999999999h", INCREASE_TIME),
            ("decreasetime 9999999999999999h", DECREASE_TIME),
            ("runtime 10d", RUN_TIME),
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
            ("users 9999999999999999999999999999", USERS),
            ("testplan 9999999999999999999999999999,1s", TEST_PLAN),
        ];

        // Collect every mismatch, so one run shows the whole table.
        let mut mismatches = Vec::new();
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
        // A long line is refused like a short one.
        let long = format!("users {}", "1".repeat(10_000));
        if parse(&long) != Err(USERS.to_string()) {
            mismatches.push(format!("10 KB users line: got {:?}", parse(&long)));
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }
}
