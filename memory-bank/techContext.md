# Technical Context: Goose Load Testing Framework

## Technologies Used

### Core Technologies
- **Rust (2018 edition)**: Primary implementation language
- **Tokio**: Async runtime for efficient concurrency
- **Reqwest**: HTTP client library for making requests
- **Serde**: Serialization/deserialization framework
- **Flume**: Channel implementation for internal communication
- **Gumdrop**: Command-line argument parsing

### Supporting Libraries
- **Chrono**: Date and time functionality
- **Log**: Logging infrastructure
- **Simplelog**: Logger implementation
- **Regex**: Regular expression support
- **Url**: URL parsing and manipulation
- **Rand**: Random number generation
- **Strum**: Enumeration utilities
- **Tungstenite**: WebSocket protocol implementation

### Optional Features
- **rustls-tls**: backwards-compatible feature for enabling rustls (feature flag)
- **gaggle**: Distributed load testing capability (feature flag)

## Development Setup

### Build System
- **Cargo**: Rust's package manager and build tool
- **Feature Flags**: Optional functionality through conditional compilation
  - `default`: Uses rustls for HTTPS
  - `rustls-tls`: additionally enables rustls for websockets
  - `gaggle`: Enables distributed load testing

### Testing
- **HTTPMock**: For mocking HTTP responses in tests
- **Serial_test**: For tests that cannot run in parallel
- **Rustls**: For testing TLS functionality
- **Nix**: For signal handling in tests

## Technical Constraints

### Performance Considerations
- Balancing number of users vs. system resources
- Network I/O as potential bottleneck
- Metrics collection overhead
- Memory usage per simulated user

### Platform Compatibility
- Designed to work across major operating systems
- TLS implementation variations between platforms
- Terminal support for controllers varies by platform

### Dependency Management
- Careful versioning of external dependencies
- Minimal dependency footprint where possible
- Feature flags for optional dependencies

## Development Workflow

### Code Organization
- Modular structure with clear separation of concerns:
  - `goose.rs`: Core load testing primitives
  - `lib.rs`: Main library entry point and coordination
  - `config.rs`: Configuration management
  - `controller.rs`: Runtime control interfaces
  - `metrics.rs`: Performance metrics collection
  - `report.rs`: Report generation
  - `test_plan.rs`: Load pattern management
  - `user.rs`: User simulation logic

### Testing Approach
- Unit tests for core functionality
- Integration tests for end-to-end scenarios
- Mocked HTTP responses for deterministic testing

### Documentation
- In-code documentation with rustdoc
- The Goose Book for comprehensive guidance
- Examples demonstrating various usage patterns

### Code Quality Process
- Manual code reviews for all pull requests
- Clippy for static code analysis
- Formatting verification with rustfmt
- GooseBot AI-assisted code reviews for automated feedback on:
  - Documentation clarity and PR quality (implemented in Phase 1)
  - Code quality and best practices (planned for Phase 2)
  - Performance considerations (planned for Phase 3)
  - Security implications (planned for Phase 3)
  
### GooseBot AI Code Review
- **Implementation**: Python-based GitHub Actions workflow
- **Dependencies**: Anthropic Claude API, PyGithub
- **Integration**: Automated PR reviews triggered by PR events
- **Context-Aware**: Reads memory-bank for project understanding
- **Prompt Templates**: Versioned templates with explicit scope
- **File Filtering**: Configurable patterns to include/exclude files
- **Token Management**: Budget tracking to control API costs
- **Model Versioning Strategy**: Uses specific model versions (e.g., claude-3-sonnet-20240229) rather than "-latest" suffix for:
  - Predictable responses and consistent behavior
  - Stable token usage and cost management
  - Avoiding unexpected breaking changes
  - Reproducible reviews and debugging
  - Note: Requires manual updates when models are deprecated (current model expires July 2025)
