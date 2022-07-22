pub struct GaggleEcho {
    _sequence: u32,
    _acknowledge: Option<u32>,
}

/// Commands sent to/from Works and Managers to control a Gaggle.
pub enum GaggleCommand {
    ManagerShuttingDown,
    Shutdown,
    WorkerShuttingDown,
    /// Notification that a Worker is standing by and ready to start the load test.
    WorkerIsReady,
}

pub enum GaggleCommands {
    Control(GaggleCommand),
    Echo(GaggleEcho),
    // Not Gaggle-specific
    //Error(GooseErrorMetrics),
    //Request(GooseRequestMetrics),
    //Scenario(ScenarioMetrics),
    //Transaction(TransactionMetrics),
}

/// Constant defining Goose's default port when running a Gaggle.
pub(crate) const DEFAULT_GAGGLE_PORT: &str = "5115";

/// Constant defining Goose's default manager_host when running a Gaggle.
pub(crate) const DEFAULT_GAGGLE_HOST: &str = "127.0.0.1";
