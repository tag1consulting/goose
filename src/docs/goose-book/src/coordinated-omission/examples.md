# Practical Examples

This chapter provides real-world examples of when and how to use different Coordinated Omission mitigation strategies. Each example includes the command to run, expected output, and interpretation guidance.

## Example 1: Microservice SLA Validation

**Scenario**: You're testing a payment processing microservice that must respond within 100ms for 99% of requests.

**Goal**: Detect any SLA violations, no matter how brief.

**Strategy**: Use `minimum` cadence to catch even momentary slowdowns.

```bash
# Test command
cargo run --example payment_service -- \
    --host https://payments.api.company.com \
    --users 20 \
    --run-time 5m \
    --co-mitigation minimum \
    --report-file payment_test.html

# Expected healthy output:
# === COORDINATED OMISSION METRICS ===
# Duration: 300 seconds
# Total CO Events: 2
# Events per minute: 0.40
# 
# Request Breakdown:
#   Actual requests: 18,450
#   Synthetic requests: 12 (0.1%)
# 
# Severity Distribution:
#   Minor: 2
#   Moderate: 0
#   Severe: 0
#   Critical: 0
```

**Interpretation**: 
- ✅ **Excellent**: Only 2 minor CO events in 5 minutes
- ✅ **SLA Met**: 0.1% synthetic requests indicates 99.9% of requests met timing expectations
- ✅ **No Critical Issues**: No severe or critical events

**Red Flag Example**:
```bash
# Problematic output:
# === COORDINATED OMISSION METRICS ===
# Duration: 300 seconds
# Total CO Events: 45
# Events per minute: 9.00
# 
# Request Breakdown:
#   Actual requests: 18,450
#   Synthetic requests: 892 (4.6%)
# 
# Severity Distribution:
#   Minor: 38
#   Moderate: 6
#   Severe: 1
#   Critical: 0
```

**Action Required**: 4.6% synthetic requests and frequent CO events indicate the service is struggling to meet SLA requirements consistently.

## Example 2: E-commerce Website Load Testing

**Scenario**: Testing an e-commerce site during Black Friday preparation. Users can tolerate some variability, but you want to understand overall performance.

**Goal**: Simulate realistic user behavior while detecting significant performance issues.

**Strategy**: Use `average` cadence for balanced detection.

```bash
# Test command
cargo run --example ecommerce_site -- \
    --host https://shop.company.com \
    --users 500 \
    --run-time 15m \
    --co-mitigation average \
    --report-file blackfriday_test.html

# Expected healthy output:
# === COORDINATED OMISSION METRICS ===
# Duration: 900 seconds
# Total CO Events: 12
# Events per minute: 0.80
# 
# Request Breakdown:
#   Actual requests: 145,230
#   Synthetic requests: 234 (0.2%)
# 
# Severity Distribution:
#   Minor: 8
#   Moderate: 3
#   Severe: 1
#   Critical: 0
```

**Interpretation**:
- ✅ **Good Performance**: Low CO event rate (0.8/minute)
- ✅ **Minimal Impact**: Only 0.2% synthetic requests
- ⚠️ **Monitor**: One severe event warrants investigation

**Concerning Example**:
```bash
# Problematic output:
# === COORDINATED OMISSION METRICS ===
# Duration: 900 seconds
# Total CO Events: 156
# Events per minute: 10.40
# 
# Request Breakdown:
#   Actual requests: 145,230
#   Synthetic requests: 12,450 (7.9%)
# 
# Severity Distribution:
#   Minor: 89
#   Moderate: 45
#   Severe: 18
#   Critical: 4
```

**Action Required**: High CO event rate and 7.9% synthetic requests indicate the site will struggle under Black Friday load. Scale up resources or optimize performance.

## Example 3: API Gateway Performance Testing

**Scenario**: Testing an API gateway that routes requests to multiple backend services. You want to understand how backend slowdowns affect the gateway.

**Goal**: Detect when backend issues cause gateway performance degradation.

**Strategy**: Use `average` cadence with longer test duration to capture intermittent issues.

```bash
# Test command
cargo run --example api_gateway -- \
    --host https://gateway.api.company.com \
    --users 100 \
    --run-time 30m \
    --co-mitigation average \
    --report-file gateway_test.html

# Healthy distributed system output:
# === COORDINATED OMISSION METRICS ===
# Duration: 1800 seconds
# Total CO Events: 23
# Events per minute: 0.77
# 
# Request Breakdown:
#   Actual requests: 324,500
#   Synthetic requests: 445 (0.1%)
# 
# Severity Distribution:
#   Minor: 18
#   Moderate: 4
#   Severe: 1
#   Critical: 0
```

**Interpretation**:
- ✅ **Stable Gateway**: Low synthetic percentage indicates good overall performance
- ✅ **Resilient**: Minor events suggest the gateway handles backend hiccups well
- ✅ **Scalable**: Consistent performance over 30 minutes

## Example 4: Database Connection Pool Testing

**Scenario**: Testing an application's database connection pool under load to ensure it doesn't become a bottleneck.

**Goal**: Detect connection pool exhaustion or database slowdowns.

**Strategy**: Use `minimum` cadence to catch any database-related delays immediately.

```bash
# Test command
cargo run --example database_app -- \
    --host https://app.company.com \
    --users 200 \
    --run-time 10m \
    --co-mitigation minimum \
    --report-file db_pool_test.html

# Healthy connection pool output:
# === COORDINATED OMISSION METRICS ===
# Duration: 600 seconds
# Total CO Events: 8
# Events per minute: 0.80
# 
# Request Breakdown:
#   Actual requests: 89,450
#   Synthetic requests: 67 (0.1%)
# 
# Severity Distribution:
#   Minor: 6
#   Moderate: 2
#   Severe: 0
#   Critical: 0
```

**Pool Exhaustion Example**:
```bash
# Connection pool exhaustion:
# === COORDINATED OMISSION METRICS ===
# Duration: 600 seconds
# Total CO Events: 234
# Events per minute: 23.40
# 
# Request Breakdown:
#   Actual requests: 89,450
#   Synthetic requests: 8,920 (9.1%)
# 
# Severity Distribution:
#   Minor: 45
#   Moderate: 123
#   Severe: 56
#   Critical: 10
```

**Action Required**: High CO event rate and 9.1% synthetic requests indicate connection pool exhaustion. Increase pool size or optimize database queries.

## Example 5: CDN Performance Validation

**Scenario**: Testing how your application performs when the CDN is slow or unavailable.

**Goal**: Understand the impact of CDN issues on user experience.

**Strategy**: Use `average` cadence to simulate realistic user tolerance.

```bash
# Test command with CDN issues simulated
cargo run --example cdn_test -- \
    --host https://app.company.com \
    --users 150 \
    --run-time 10m \
    --co-mitigation average \
    --report-file cdn_impact_test.html

# CDN issues detected:
# === COORDINATED OMISSION METRICS ===
# Duration: 600 seconds
# Total CO Events: 89
# Events per minute: 8.90
# 
# Request Breakdown:
#   Actual requests: 67,230
#   Synthetic requests: 2,340 (3.4%)
# 
# Severity Distribution:
#   Minor: 34
#   Moderate: 38
#   Severe: 15
#   Critical: 2
```

**Interpretation**:
- ⚠️ **CDN Impact**: 3.4% synthetic requests show CDN issues affect user experience
- ⚠️ **User Frustration**: Moderate and severe events indicate noticeable delays
- 📊 **Business Impact**: Use this data to justify CDN redundancy or optimization

## Example 6: Baseline Testing (No CO Expected)

**Scenario**: Testing a well-optimized system under normal load to establish performance baselines.

**Goal**: Confirm the system performs consistently without CO events.

**Strategy**: Use `disabled` to get pure measurements, then compare with `average` mode.

```bash
# First, test with CO disabled for baseline
cargo run --example baseline_test -- \
    --host https://optimized.company.com \
    --users 100 \
    --run-time 10m \
    --co-mitigation disabled \
    --report-file baseline_raw.html

# Then test with CO detection enabled
cargo run --example baseline_test -- \
    --host https://optimized.company.com \
    --users 100 \
    --run-time 10m \
    --co-mitigation average \
    --report-file baseline_co.html

# Expected output (CO enabled):
# === COORDINATED OMISSION METRICS ===
# Duration: 600 seconds
# Total CO Events: 0
# Events per minute: 0.00
# 
# Request Breakdown:
#   Actual requests: 45,670
#   Synthetic requests: 0 (0.0%)
```

**Perfect Baseline**: Zero CO events and 0% synthetic requests indicate the system performs consistently under this load level.

## Interpreting Results Across Examples

### Green Flags (Healthy System)
- CO events per minute < 2
- Synthetic request percentage < 1%
- Mostly Minor severity events
- Consistent performance across test duration

### Yellow Flags (Monitor Closely)
- CO events per minute 2-10
- Synthetic request percentage 1-5%
- Some Moderate severity events
- Occasional performance dips

### Red Flags (Action Required)
- CO events per minute > 10
- Synthetic request percentage > 5%
- Frequent Severe or any Critical events
- Degrading performance over time

### Using CO Metrics for Capacity Planning

1. **Find Breaking Point**: Gradually increase load until CO events spike
2. **Set Alerts**: Monitor CO metrics in production to detect issues early
3. **Compare Environments**: Use CO metrics to validate staging vs production performance
4. **Track Trends**: Monitor CO metrics over time to detect performance regression

## Best Practices Summary

1. **Choose the Right Mode**:
   - `minimum` for strict SLA validation
   - `average` for realistic user simulation
   - `disabled` for baseline measurements

2. **Set Appropriate Test Duration**:
   - Short tests (5-10 min) for quick validation
   - Long tests (30+ min) for stability assessment

3. **Monitor Key Metrics**:
   - Events per minute rate
   - Synthetic request percentage
   - Severity distribution
   - Trends over time

4. **Take Action Based on Results**:
   - < 1% synthetic: System healthy
   - 1-5% synthetic: Monitor and investigate
   - > 5% synthetic: Performance issues need attention

These examples provide a foundation for understanding how CO metrics help identify and quantify performance issues in different scenarios. Use them as templates for your own testing strategies.
