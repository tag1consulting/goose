# Project Brief: Goose Load Testing Framework

## Overview
Goose is a powerful load testing framework inspired by Locust, designed to simulate real-world user behavior on web applications. It is built with Rust for speed and scalability, offering significant performance advantages over Python-based alternatives.

## Objectives
- To provide a flexible and customizable load testing tool that scales efficiently
- To enable accurate simulation of realistic user behaviors like logging in, filling out forms, and navigating through applications
- To help identify performance bottlenecks and optimize resource allocation to ensure a seamless user experience
- To deliver accurate performance metrics without falling prey to coordinated omission problems

## Key Features
- **Fast and Scalable**: Built with Rust for optimal performance, utilizing async programming for efficient resource usage
- **Sophisticated Scheduling**: Multiple scheduling strategies (RoundRobin, Serial, Random) for realistic user simulation
- **Test Plans**: Configure complex load patterns with controlled ramp-up and ramp-down phases
- **Flexible and Customizable**: Supports both simple and complex load tests tailored to specific needs
- **Realistic User Behavior Simulation**: 
  - Scenarios: Groups of related tasks representing user workflows
  - Transactions: Individual actions within scenarios
  - Weighted distribution: Control frequency of different user behaviors
  - Session management: Maintain state across requests
- **Advanced Metrics**:
  - Coordinated Omission Mitigation prevents misleading metrics
  - Detailed reporting on response times, status codes, and request rates
  - Support for HTML report generation
- **Runtime Control**: Telnet and WebSocket controllers for modifying tests while running
- **Request Throttling**: Prevent overwhelming target systems with configurable rate limits
