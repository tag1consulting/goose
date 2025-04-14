# System Patterns: Goose Load Testing Framework

## System Architecture
Goose is designed as a modular and scalable load testing framework built on Rust's asynchronous programming model. The core architecture consists of:

- **GooseAttack**: The primary coordinator that manages the load test lifecycle
- **Load Test Plan**: Defines the pattern of user scaling over time (ramp-up, steady state, ramp-down)
- **User Simulation**: Creates and manages GooseUser instances that execute transactions
- **Metrics Collection**: Collects, aggregates, and reports on performance metrics
- **Controllers**: Provides telnet and WebSocket interfaces to dynamically control running tests
- **Reporting**: Generates detailed reports in various formats including HTML with graphs

The system operates through multiple attack phases:
- **Idle**: Configuration can be modified, waiting to start
- **Increase**: Users are being added according to the test plan
- **Maintain**: Steady-state load generation with consistent user count
- **Decrease**: Users are being removed according to the test plan
- **Shutdown**: Test is completing and resources are being released

## Key Technical Decisions

- **Rust Language**: Chosen for its performance, memory safety, and zero-cost abstractions
- **Async/Await**: Leverages Tokio for efficient asynchronous I/O and concurrency
- **Reqwest HTTP Client**: Provides robust HTTP capabilities with connection pooling
- **Channel-based Communication**: Uses flume channels for communication between components
- **Modular Design**: Encapsulates functionality in separate modules with clear boundaries
- **Coordinated Omission Mitigation**: Explicit design to prevent this common load testing pitfall
- **Memory-efficient Implementation**: Minimizes overhead per simulated user

## Design Patterns

### User Model
- **GooseUser**: Represents a single simulated user with:
  - HTTP client with its own cookie store and session state
  - Assigned scenario to execute
  - Metrics reporting capabilities
  - Optional custom session data for maintaining state

### Scenario and Transaction Structure
- **Scenario**: Represents a complete user workflow with:
  - Collection of transactions
  - Configurable weighting to control frequency
  - Optional host assignment
  - Configurable wait times between transactions

- **Transaction**: Represents a discrete user action with:
  - Executable async function with application logic
  - Configurable weighting to control frequency
  - Optional sequence number for ordered execution
  - Optional on_start/on_stop flags for setup/teardown operations

### Scheduling Models
- **RoundRobin**: Distributes scenarios and transactions in rotation (default)
- **Serial**: Executes all weighted instances of one type before moving to next
- **Random**: Randomly selects which scenario or transaction to run next

### Request Handling
- **GooseRequest**: Abstraction over HTTP requests with:
  - Method, path, and optional name
  - Expected status code configuration
  - Error handling rules

- **GooseResponse**: Wrapper around HTTP responses with:
  - Original request details
  - Response data and status
  - Success/failure determination

### Metrics Collection
- **Request Metrics**: Tracks individual HTTP request performance
- **Transaction Metrics**: Aggregates metrics for logical user operations
- **Scenario Metrics**: Groups transaction metrics by user type
- **Coordinated Omission Mitigation**: 
  - Tracks expected request cadence
  - Detects delays caused by server processing
  - Creates synthetic measurements to account for "missing" requests

### Throttling
- **Leaky Bucket Algorithm**: Controls request rate to prevent overwhelming target systems
- **Bounded Channel**: Implementation uses a bounded channel as the throttling mechanism

### Test Plan Execution
- **TestPlan**: Defines a sequence of load steps:
  - Each step specifies user count and duration
  - Supports complex patterns like ramp-up, steady state, spike, ramp-down

### Controller Interfaces
- **Telnet Controller**: Text-based interface for monitoring and modifying tests
- **WebSocket Controller**: Web-compatible interface for UI integrations
- **Command Pattern**: Commands sent over channels for test control
