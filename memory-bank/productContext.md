# Product Context: Goose Load Testing Framework

## Why This Project Exists
Goose was created to provide a robust and efficient load testing solution for web applications. The project aims to fill the gap between existing tools by offering a Rust-based framework that is both powerful and easy to use. While Python-based tools like Locust provide good usability, they can be limited in performance. Goose leverages Rust's performance advantages while maintaining a user-friendly API.

## Problems It Solves
- **Performance Bottlenecks**: Identifies performance issues before they affect users in production environments
- **Resource Optimization**: Helps optimize server resources and infrastructure by simulating real-world loads
- **User Experience**: Ensures smooth and responsive user interactions under load by testing realistic user flows
- **Coordinated Omission**: Avoids the statistical problem where slow server responses create misleading metrics
- **Scalability Testing**: Tests how applications perform as the number of users increases, including controlled ramp-up/down scenarios
- **Realistic Simulation**: Ensures test scenarios match real-world usage patterns with configurable weights and sequences

## Common Use Cases
- **Pre-release Testing**: Validating application performance before deploying to production
- **Capacity Planning**: Determining how many servers/resources are needed for expected traffic
- **Regression Testing**: Ensuring new code changes don't degrade performance
- **Breaking Point Analysis**: Finding the maximum capacity of a system
- **Continuous Integration**: Automated performance testing as part of CI/CD pipelines

## How It Works
Goose operates by creating and managing virtual users (GooseUsers) that perform defined transactions against the target system:

1. **Test Definition**: Developers define scenarios and transactions in Rust code
2. **Test Execution**: Goose launches and manages virtual users according to the test plan
3. **Load Generation**: Users execute transactions with configurable think time between requests
4. **Metrics Collection**: Runtime metrics are collected and processed
5. **Reporting**: Results are displayed and can be exported to reports

Goose provides multiple options for controlling test execution:
- Command-line parameters for quick configuration
- Default settings that can be overridden at runtime
- Controller interfaces (telnet and WebSocket) for real-time adjustments
- Test plans for defining complex load patterns over time

The framework's architecture allows for both simple tests (single file with a few transactions) and complex, realistic user simulations with multiple scenarios, weighted behaviors, and stateful user sessions.
