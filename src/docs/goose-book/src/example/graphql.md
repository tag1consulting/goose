# GraphQL

Goose provides built-in support for load testing GraphQL APIs through dedicated helper methods that simplify sending GraphQL queries and mutations.

## GraphQL Helper Methods

Goose includes two GraphQL helper methods:

- [`post_graphql()`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.post_graphql): Send a GraphQL query to the configured endpoint
- [`post_graphql_named()`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.post_graphql_named): Send a named GraphQL query for better metrics tracking

## GraphQL Endpoint Configuration

The GraphQL endpoint is specified as the path parameter in the GraphQL helper methods. By convention, most GraphQL APIs use `/graphql` as the endpoint, but you can specify any path:

```rust
// Using the default /graphql endpoint
let _response = user.post_graphql("/graphql", &query).await?;

// Using a custom endpoint
let _response = user.post_graphql("/api/graphql", &query).await?;
```

## Example: GraphQL Load Test

The following example demonstrates how to create a comprehensive GraphQL load test:

```rust
use goose::prelude::*;
use serde_json::json;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("GraphQL Load Test")
                .register_transaction(transaction!(get_users_transaction).set_weight(3)?)
                .register_transaction(transaction!(get_user_by_id_transaction).set_weight(2)?)
                .register_transaction(transaction!(create_user_transaction).set_weight(1)?)
                .register_transaction(transaction!(update_user_transaction).set_weight(1)?)
        )
        .execute()
        .await?;

    Ok(())
}

/// Query to get all users
async fn get_users_transaction(user: &mut GooseUser) -> TransactionResult {
    let query = json!({
        "query": "query GetUsers { users { id name email createdAt } }"
    });
    
    let _response = user.post_graphql_named(&query, "get all users").await?;
    Ok(())
}

/// Query to get a specific user by ID
async fn get_user_by_id_transaction(user: &mut GooseUser) -> TransactionResult {
    // Simulate fetching different users
    let user_id = fastrand::u32(1..100);
    
    let query = json!({
        "query": "query GetUser($id: ID!) { user(id: $id) { id name email createdAt } }",
        "variables": {
            "id": user_id
        }
    });
    
    let _response = user.post_graphql_named(&query, "get user by id").await?;
    Ok(())
}

/// Mutation to create a new user
async fn create_user_transaction(user: &mut GooseUser) -> TransactionResult {
    let random_id = uuid::Uuid::new_v4();
    
    let mutation = json!({
        "query": "mutation CreateUser($input: CreateUserInput!) { 
            createUser(input: $input) { 
                id name email createdAt 
            } 
        }",
        "variables": {
            "input": {
                "name": format!("Test User {}", random_id),
                "email": format!("test+{}@example.com", random_id)
            }
        }
    });
    
    let _response = user.post_graphql_named(&mutation, "create user").await?;
    Ok(())
}

/// Mutation to update an existing user
async fn update_user_transaction(user: &mut GooseUser) -> TransactionResult {
    let user_id = fastrand::u32(1..100);
    let random_id = uuid::Uuid::new_v4();
    
    let mutation = json!({
        "query": "mutation UpdateUser($id: ID!, $input: UpdateUserInput!) {
            updateUser(id: $id, input: $input) {
                id name email updatedAt
            }
        }",
        "variables": {
            "id": user_id,
            "input": {
                "name": format!("Updated User {}", random_id)
            }
        }
    });
    
    let _response = user.post_graphql_named(&mutation, "update user").await?;
    Ok(())
}
```

## Key Features

### JSON Query Construction
GraphQL queries are constructed as JSON objects using `serde_json::json!()`, making it easy to build complex queries with variables.

### Automatic Endpoint Handling
The GraphQL helper methods automatically use the configured GraphQL endpoint, so you don't need to specify the full path in each request.

### Variables Support
GraphQL variables are fully supported, allowing you to create dynamic queries that simulate real user behavior.

### Named Requests
Using `post_graphql_named()` allows you to give meaningful names to your GraphQL operations, making metrics easier to understand.

### Weighted Transactions
Different types of GraphQL operations can be weighted differently to simulate realistic usage patterns (e.g., more reads than writes).

## Advanced Usage

### Error Handling
You can validate GraphQL responses and handle errors appropriately:

```rust
async fn validated_graphql_transaction(user: &mut GooseUser) -> TransactionResult {
    let query = json!({
        "query": "query GetUser($id: ID!) { user(id: $id) { id name } }",
        "variables": { "id": "invalid-id" }
    });
    
    let mut response = user.post_graphql_named(&query, "get user with validation").await?;
    
    if let Ok(response) = &response.response {
        match response.json::<serde_json::Value>().await {
            Ok(json) => {
                if json.get("errors").is_some() {
                    return user.set_failure("GraphQL errors returned", &mut response.request, None, None);
                }
            }
            Err(_) => {
                return user.set_failure("Invalid JSON response", &mut response.request, None, None);
            }
        }
    }
    
    Ok(())
}
```

### Custom Headers
If your GraphQL API requires authentication or custom headers, you can use the lower-level request building approach:

```rust
async fn authenticated_graphql_transaction(user: &mut GooseUser) -> TransactionResult {
    let query = json!({
        "query": "query GetPrivateData { privateData { id value } }"
    });
    
    // Build custom request with authentication header
    let request_builder = user.get_request_builder(&GooseMethod::Post, "/graphql")?
        .header("Authorization", "Bearer your-token-here")
        .json(&query);
    
    let goose_request = GooseRequest::builder()
        .set_request_builder(request_builder)
        .name("authenticated query")
        .build();
    
    let _response = user.request(goose_request).await?;
    Ok(())
}
```

This comprehensive example shows how Goose's GraphQL support can be used to create realistic load tests that exercise different types of GraphQL operations with proper weighting, error handling, and metrics tracking.

## Sample Output

When running the GraphQL load test example, you'll see output similar to this:

```bash
$ cargo run --example graphql_loadtest -- --host http://localhost:4000 --users 5 --run-time 30s

05:15:32 [INFO] Output verbosity level: INFO
05:15:32 [INFO] Logfile verbosity level: WARN
05:15:32 [INFO] users = 5
05:15:32 [INFO] run_time = 30
05:15:32 [INFO] global host configured: http://localhost:4000/
05:15:32 [INFO] allocating transactions and scenarios with RoundRobin scheduler
05:15:32 [INFO] initializing 5 user states...
05:15:32 [INFO] Telnet controller listening on: 0.0.0.0:5116
05:15:32 [INFO] WebSocket controller listening on: 0.0.0.0:5117
05:15:32 [INFO] entering GooseAttack phase: Increase
05:15:32 [INFO] launching user 1 from GraphQL Load Test...
05:15:32 [INFO] launching user 2 from GraphQL Load Test...
05:15:32 [INFO] launching user 3 from GraphQL Load Test...
05:15:32 [INFO] launching user 4 from GraphQL Load Test...
05:15:32 [INFO] launching user 5 from GraphQL Load Test...
All 5 users hatched.

05:15:33 [INFO] entering GooseAttack phase: Maintain
05:16:03 [INFO] entering GooseAttack phase: Decrease
05:16:03 [INFO] exiting user 1 from GraphQL Load Test...
05:16:03 [INFO] exiting user 2 from GraphQL Load Test...
05:16:03 [INFO] exiting user 3 from GraphQL Load Test...
05:16:03 [INFO] exiting user 4 from GraphQL Load Test...
05:16:03 [INFO] exiting user 5 from GraphQL Load Test...
05:16:03 [INFO] entering GooseAttack phase: Shutdown
05:16:03 [INFO] printing final metrics after 31 seconds...

 === PER SCENARIO METRICS ===
 ------------------------------------------------------------------------------
 Name                     |  # users |  # times run | scenarios/s | iterations
 ------------------------------------------------------------------------------
 1: GraphQL Load Test     |        5 |           42 |        1.35 |       8.40
 -------------------------+----------+--------------+-------------+------------
 Aggregated               |        5 |           42 |        1.35 |       8.40
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
   1: GraphQL Load Test   |        3847 |      2,156 |       6,234 |      3,456
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |        3847 |      2,156 |       6,234 |      3,456

 === PER TRANSACTION METRICS ===
 ------------------------------------------------------------------------------
 Name                     |   # times run |        # fails |  trans/s |  fail/s
 ------------------------------------------------------------------------------
 1: GraphQL Load Test
   1: get_users_transac.. |            63 |         0 (0%) |     2.03 |    0.00
   2: get_user_by_id_t..  |            42 |         0 (0%) |     1.35 |    0.00
   3: create_user_tran..  |            21 |         0 (0%) |     0.68 |    0.00
   4: update_user_tran..  |            21 |         0 (0%) |     0.68 |    0.00
 -------------------------+---------------+----------------+----------+--------
 Aggregated               |           147 |         0 (0%) |     4.74 |    0.00
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 1: GraphQL Load Test
   1: get_users_transac.. |       24.73 |         18 |          45 |         23
   2: get_user_by_id_t..  |       26.12 |         19 |          52 |         24
   3: create_user_tran..  |       31.48 |         22 |          67 |         29
   4: update_user_tran..  |       33.19 |         24 |          71 |         31
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |       27.21 |         18 |          71 |         25

 === PER REQUEST METRICS ===
 ------------------------------------------------------------------------------
 Name                     |        # reqs |        # fails |    req/s |  fail/s
 ------------------------------------------------------------------------------
 POST create user         |            21 |         0 (0%) |     0.68 |    0.00
 POST get all users       |            63 |         0 (0%) |     2.03 |    0.00
 POST get user by id      |            42 |         0 (0%) |     1.35 |    0.00
 POST update user         |            21 |         0 (0%) |     0.68 |    0.00
 -------------------------+---------------+----------------+----------+--------
 Aggregated               |           147 |         0 (0%) |     4.74 |    0.00
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 POST create user         |       31.48 |         22 |          67 |         29
 POST get all users       |       24.73 |         18 |          45 |         23
 POST get user by id      |       26.12 |         19 |          52 |         24
 POST update user         |       33.19 |         24 |          71 |         31
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |       27.21 |         18 |          71 |         25
 ------------------------------------------------------------------------------
 Slowest page load within specified percentile of requests (in ms):
 ------------------------------------------------------------------------------
 Name                     |    50% |    75% |    98% |    99% |  99.9% | 99.99%
 ------------------------------------------------------------------------------
 POST create user         |     29 |     35 |     65 |     67 |     67 |     67
 POST get all users       |     23 |     28 |     42 |     45 |     45 |     45
 POST get user by id      |     24 |     30 |     48 |     52 |     52 |     52
 POST update user         |     31 |     38 |     68 |     71 |     71 |     71
 -------------------------+--------+--------+--------+--------+--------+-------
 Aggregated               |     25 |     31 |     58 |     67 |     71 |     71
 ------------------------------------------------------------------------------
 Name                     |                                        Status codes 
 ------------------------------------------------------------------------------
 POST create user         |                                            21 [200]
 POST get all users       |                                            63 [200]
 POST get user by id      |                                            42 [200]
 POST update user         |                                            21 [200]
 -------------------------+----------------------------------------------------
 Aggregated               |                                           147 [200] 

 === OVERVIEW ===
 ------------------------------------------------------------------------------
 Action       Started               Stopped             Elapsed    Users
 ------------------------------------------------------------------------------
 Increasing:  2024-01-15 05:15:32 - 2024-01-15 05:15:33 (00:00:01, 0 -> 5)
 Maintaining: 2024-01-15 05:15:33 - 2024-01-15 05:16:03 (00:00:30, 5)
 Decreasing:  2024-01-15 05:16:03 - 2024-01-15 05:16:03 (00:00:00, 0 <- 5)

 Target host: http://localhost:4000/
 goose v0.18.1-dev
 ------------------------------------------------------------------------------
```

### Key Observations

- **Named Operations**: Each GraphQL operation appears with its custom name (e.g., "get all users", "create user")
- **Request Grouping**: All GraphQL requests show as POST requests to the GraphQL endpoint
- **Transaction Timing**: Shows how long each complete GraphQL transaction takes
- **Weighted Distribution**: The request counts reflect the configured weights (3:2:1:1 ratio)
- **Response Times**: Mutations (create/update) typically take longer than queries
- **Status Codes**: All GraphQL requests return HTTP 200, even for GraphQL errors (which would be in the response body)

This output demonstrates how Goose's GraphQL support provides clear, actionable metrics for GraphQL API load testing while maintaining the familiar Goose reporting format.
