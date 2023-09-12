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

pub enum GagglePhase {
    WaitingForWorkers,
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
