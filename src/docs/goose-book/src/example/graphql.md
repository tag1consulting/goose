# GraphQL

Goose provides helper methods for load testing GraphQL APIs, new in 0.19, that simplify sending GraphQL queries and mutations.

## GraphQL Helper Methods

Goose includes two GraphQL helper methods:

- [`post_graphql()`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.post_graphql): POST a GraphQL query as JSON to a path; the metrics name the request by its path, for example `POST /graphql`
- [`post_graphql_named()`](https://docs.rs/goose/*/goose/goose/struct.GooseUser.html#method.post_graphql_named): the same, with a name for the request in the metrics, for example `POST get user by id`

Both take the query as anything that implements `serde::Serialize`, usually a `serde_json::Value` built with `json!()`, and return a [`GooseResponse`](https://docs.rs/goose/*/goose/goose/struct.GooseResponse.html), like the other request helpers.

## GraphQL Endpoint Configuration

There is no configured GraphQL endpoint: the path is the first argument of each call, and Goose prepends the host as it does for every request. By convention, most GraphQL APIs use `/graphql`, but you can specify any path. In this fragment `query` is a `serde_json::Value`:

```rust,ignore
// Using the conventional /graphql endpoint
let _goose = user.post_graphql("/graphql", &query).await?;

// Using a custom endpoint
let _goose = user.post_graphql("/api/graphql", &query).await?;
```

## Example: GraphQL Load Test

The [`examples/graphql_loadtest.rs`](https://github.com/tag1consulting/goose/blob/main/examples/graphql_loadtest.rs) example defines one scenario, `GraphQL Load Test`, with four transactions. None sets a weight or a wait time, so each user runs the four in turn, without pausing, for as long as the load test runs. Each one POSTs to `/graphql`:

| Transaction | Sends | Request name in the metrics |
|---|---|---|
| `Simple Query` | the query `{ __typename }`, with `post_graphql()` | `POST /graphql` |
| `User Query` | a `GetUser` query with the variable `id` | `POST get user by id` |
| `Complex Query` | a `GetUserWithPosts` query for a user's posts and their comments, with the variables `userId` and `first` | `POST complex user posts query` |
| `Mutation` | a `CreatePost` mutation with an `input` variable | `POST create post` |

The full source is at the end of this page.

## Key Features

### JSON Query Construction
GraphQL queries are constructed as JSON objects using `serde_json::json!()`, making it easy to build complex queries with variables.

### Variables Support
GraphQL variables are fully supported, allowing you to create dynamic queries that simulate real user behavior.

### Named Requests
Using `post_graphql_named()` allows you to give meaningful names to your GraphQL operations, making metrics easier to understand. Every request made with `post_graphql()` to the same path shares one row in the metrics.

### Weighted Transactions
Different types of GraphQL operations can be weighted differently with `set_weight()` to simulate realistic usage patterns (e.g., more reads than writes). The example leaves every transaction at the default weight of 1.

## Advanced Usage

### Error Handling
Goose judges a GraphQL request by its HTTP status code, like any other request, so a GraphQL error returned with a `200` counts as a success. To catch those, read the response body and call `set_failure`:

```rust,ignore
async fn validated_graphql_transaction(user: &mut GooseUser) -> TransactionResult {
    let query = json!({
        "query": "query GetUser($id: ID!) { user(id: $id) { id name } }",
        "variables": { "id": "invalid-id" }
    });

    let mut goose = user
        .post_graphql_named("/graphql", &query, "get user with validation")
        .await?;

    if let Ok(response) = goose.response {
        match response.json::<serde_json::Value>().await {
            Ok(json) => {
                if json.get("errors").is_some() {
                    return user.set_failure("GraphQL errors returned", &mut goose.request, None, None);
                }
            }
            Err(_) => {
                return user.set_failure("Invalid JSON response", &mut goose.request, None, None);
            }
        }
    }

    Ok(())
}
```

### Custom Headers
If your GraphQL API requires authentication or custom headers, you can use the lower-level request building approach:

```rust,ignore
async fn authenticated_graphql_transaction(user: &mut GooseUser) -> TransactionResult {
    let query = json!({
        "query": "query GetPrivateData { privateData { id value } }"
    });

    // Build custom request with authentication header
    let request_builder = user
        .get_request_builder(&GooseMethod::Post, "/graphql")?
        .header("Authorization", "Bearer your-token-here")
        .json(&query);

    let goose_request = GooseRequest::builder()
        .set_request_builder(request_builder)
        .name("authenticated query")
        .build();

    let _goose = user.request(goose_request).await?;
    Ok(())
}
```

Both are fragments: they need `use goose::prelude::*;` and `use serde_json::json;`, and a scenario that registers them.

## Sample Output

Running the example for 30 seconds with 5 users, against a small local server that answers every POST to `/graphql` with a `200` and a JSON body:

```bash
$ cargo run --release --example graphql_loadtest -- --host http://localhost:8080 --users 5 --run-time 30s

20:58:15 [INFO] Output verbosity level: INFO
20:58:15 [INFO] Logfile verbosity level: WARN
20:58:15 [INFO] users = 5
20:58:15 [INFO] run_time = 30
20:58:15 [INFO] global host configured: http://localhost:8080
20:58:15 [INFO] [scheduler]: allocating transactions and scenarios with RoundRobin scheduler
20:58:15 [INFO] [scheduler]: initializing 5 user states...
20:58:15 [INFO] entering GooseAttack phase: Increase
20:58:15 [INFO] [user 1]: launching user from GraphQL Load Test
20:58:16 [INFO] [user 2]: launching user from GraphQL Load Test
20:58:17 [INFO] [user 3]: launching user from GraphQL Load Test
20:58:18 [INFO] [user 4]: launching user from GraphQL Load Test
20:58:19 [INFO] [user 5]: launching user from GraphQL Load Test
20:58:20 [INFO] printing running metrics after 5 seconds...
```

Goose starts one user a second, and when all 5 have launched it prints the metrics so far and resets them. Those tables are left out here. The rest of the output follows:

```bash
All 5 users launched, resetting metrics (disable with --no-reset-metrics).

20:58:20 [INFO] entering GooseAttack phase: Maintain
20:58:50 [INFO] entering GooseAttack phase: Decrease
20:58:50 [INFO] [user 4]: exiting from GraphQL Load Test
20:58:50 [INFO] [user 2]: exiting from GraphQL Load Test
20:58:50 [INFO] [user 3]: exiting from GraphQL Load Test
20:58:50 [INFO] [user 5]: exiting from GraphQL Load Test
20:58:50 [INFO] [user 1]: exiting from GraphQL Load Test
20:58:50 [INFO] entering GooseAttack phase: Shutdown
20:58:50 [INFO] printing final metrics after 30 seconds...

 === PER SCENARIO METRICS ===
 ------------------------------------------------------------------------------
 Name                     |  # users |  # times run | scenarios/s | iterations
 ------------------------------------------------------------------------------
 1: GraphQL Load Test     |        5 |         5211 |      173.70 |       1042
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
   1: GraphQL Load Test   |          29 |       0.28 |         142 |         26

 === PER TRANSACTION METRICS ===
 ------------------------------------------------------------------------------
 Name                     |   # times run |        # fails |  trans/s |  fail/s
 ------------------------------------------------------------------------------
 1: GraphQL Load Test    
   1: Simple Query        |         5,213 |         0 (0%) |   173.77 |    0.00
   2: User Query          |         5,213 |         0 (0%) |   173.77 |    0.00
   3: Complex Query       |         5,211 |         0 (0%) |   173.70 |    0.00
   4: Mutation            |         5,211 |         0 (0%) |   173.70 |    0.00
 -------------------------+---------------+----------------+----------+--------
 Aggregated               |        20,848 |         0 (0%) |   694.93 |    0.00
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 1: GraphQL Load Test    
   1: Simple Query        |        7.21 |       0.06 |          66 |       6.56
   2: User Query          |        7.25 |       0.07 |          98 |       6.62
   3: Complex Query       |         7.2 |       0.06 |          95 |        6.5
   4: Mutation            |        7.21 |       0.06 |          73 |        6.5
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |        7.22 |       0.06 |          98 |       6.56

 === PER REQUEST METRICS ===
 ------------------------------------------------------------------------------
 Name                     |        # reqs |        # fails |    req/s |  fail/s
 ------------------------------------------------------------------------------
 POST /graphql            |         5,216 |         0 (0%) |   173.87 |    0.00
 POST complex user post.. |         5,211 |         0 (0%) |   173.70 |    0.00
 POST create post         |         5,211 |         0 (0%) |   173.70 |    0.00
 POST get user by id      |         5,213 |         0 (0%) |   173.77 |    0.00
 -------------------------+---------------+----------------+----------+--------
 Aggregated               |        20,851 |         0 (0%) |   695.03 |    0.00
 ------------------------------------------------------------------------------
 Name                     |    Avg (ms) |        Min |         Max |     Median
 ------------------------------------------------------------------------------
 POST /graphql            |        7.15 |       0.05 |          66 |        6.5
 POST complex user post.. |        7.14 |       0.05 |          95 |       6.43
 POST create post         |        7.15 |       0.06 |          73 |        6.5
 POST get user by id      |         7.2 |       0.06 |          98 |       6.56
 -------------------------+-------------+------------+-------------+-----------
 Aggregated               |        7.16 |       0.05 |          98 |        6.5
 ------------------------------------------------------------------------------
 Name                     |                                        Status codes 
 ------------------------------------------------------------------------------
 POST /graphql            |                                         5,216 [200]
 POST complex user post.. |                                         5,211 [200]
 POST create post         |                                         5,211 [200]
 POST get user by id      |                                         5,213 [200]
 -------------------------+----------------------------------------------------
 Aggregated               |                                        20,851 [200] 
```

### Key Observations

- **Named Operations**: Each request made with `post_graphql_named()` appears with its custom name (e.g., "get user by id", "create post"), and the one made with `post_graphql()` appears by its path, `POST /graphql`
- **Request Grouping**: All GraphQL requests show as POST requests
- **Transaction Timing**: The transaction tables show each transaction by the name given with `set_name()`, and how long it took
- **Even Distribution**: With no weights set, each transaction runs about as often as the others, so the four requests have about the same counts
- **Status Codes**: Every request returned HTTP 200. A GraphQL server usually answers a GraphQL error with a 200 too, with the error in the response body, so Goose counts it as a success unless the transaction checks the body as in [Error Handling](#error-handling)

## Complete Source Code

```rust,ignore
{{#include ../../../../../examples/graphql_loadtest.rs}}
```
