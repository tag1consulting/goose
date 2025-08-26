//! Example demonstrating GraphQL load testing with Goose.
//!
//! This example shows how to use Goose's GraphQL helper methods to load test
//! a GraphQL API. The GraphQL helpers use the existing HTTP infrastructure
//! and provide convenient methods for making GraphQL queries.
//!
//! ## Usage
//!
//! To compile and run this example:
//!
//! ```bash
//! cargo run --example graphql_loadtest -- --host https://api.example.com
//! ```
//!
//! ## Configuration Options
//!
//! - `--host`: Target host to load test
//! - `--users`: Number of concurrent users (default: 1)
//! - `--run-time`: Duration to run the test (e.g., "30s", "5m")
//!
//! ## GraphQL Helper Methods
//!
//! Goose provides two GraphQL helper methods:
//! - `post_graphql()`: Basic GraphQL queries
//! - `post_graphql_named()`: Named GraphQL queries for better metrics tracking
//!
//! ## Example Commands
//!
//! Basic GraphQL load test:
//! ```bash
//! cargo run --example graphql_loadtest -- --host https://api.github.com --users 5 --run-time 30s
//! ```
//!
//! Using a custom GraphQL endpoint path:
//! ```bash
//! cargo run --example graphql_loadtest -- --host https://api.example.com --users 10 --run-time 1m
//! ```

use goose::prelude::*;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("GraphQL Load Test")
                .set_weight(1)?
                .register_transaction(transaction!(simple_query).set_name("Simple Query"))
                .register_transaction(transaction!(user_query).set_name("User Query"))
                .register_transaction(transaction!(complex_query).set_name("Complex Query"))
                .register_transaction(transaction!(mutation_example).set_name("Mutation")),
        )
        .execute()
        .await?;

    Ok(())
}

/// Simple GraphQL query example
async fn simple_query(user: &mut GooseUser) -> TransactionResult {
    let query = json!({
        "query": "{ __typename }"
    });

    let _goose = user.post_graphql("/graphql", &query).await?;
    Ok(())
}

/// User query with variables
async fn user_query(user: &mut GooseUser) -> TransactionResult {
    let query = json!({
        "query": "query GetUser($id: ID!) { user(id: $id) { id name email } }",
        "variables": {
            "id": "123"
        }
    });

    // Use named query for better metrics tracking
    let _goose = user
        .post_graphql_named("/graphql", &query, "get user by id")
        .await?;
    Ok(())
}

/// Complex query with multiple fields and nested data
async fn complex_query(user: &mut GooseUser) -> TransactionResult {
    let query = json!({
        "query": r#"
            query GetUserWithPosts($userId: ID!, $first: Int!) {
                user(id: $userId) {
                    id
                    name
                    email
                    posts(first: $first) {
                        edges {
                            node {
                                id
                                title
                                content
                                createdAt
                                comments(first: 5) {
                                    edges {
                                        node {
                                            id
                                            content
                                            author {
                                                name
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        "#,
        "variables": {
            "userId": "456",
            "first": 10
        }
    });

    let _goose = user
        .post_graphql_named("/graphql", &query, "complex user posts query")
        .await?;
    Ok(())
}

/// GraphQL mutation example
async fn mutation_example(user: &mut GooseUser) -> TransactionResult {
    let mutation = json!({
        "query": r#"
            mutation CreatePost($input: CreatePostInput!) {
                createPost(input: $input) {
                    id
                    title
                    content
                    author {
                        id
                        name
                    }
                }
            }
        "#,
        "variables": {
            "input": {
                "title": "Load Test Post",
                "content": "This post was created during a load test",
                "authorId": "789"
            }
        }
    });

    let _goose = user
        .post_graphql_named("/graphql", &mutation, "create post")
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_graphql_transactions() {
        // Test that transactions can be created without errors
        let simple_transaction = transaction!(simple_query).set_name("Simple Query");
        let user_transaction = transaction!(user_query).set_name("User Query");
        let complex_transaction = transaction!(complex_query).set_name("Complex Query");
        let mutation_transaction = transaction!(mutation_example).set_name("Mutation");

        assert_eq!(simple_transaction.name, "Simple Query");
        assert_eq!(user_transaction.name, "User Query");
        assert_eq!(complex_transaction.name, "Complex Query");
        assert_eq!(mutation_transaction.name, "Mutation");
    }

    #[test]
    fn test_scenario_creation() {
        // Test that the scenario can be created properly
        let scenario = scenario!("GraphQL Load Test")
            .set_weight(1)
            .unwrap()
            .register_transaction(transaction!(simple_query).set_name("Simple Query"))
            .register_transaction(transaction!(user_query).set_name("User Query"))
            .register_transaction(transaction!(complex_query).set_name("Complex Query"))
            .register_transaction(transaction!(mutation_example).set_name("Mutation"));

        assert_eq!(scenario.name, "GraphQL Load Test");
        assert_eq!(scenario.weight, 1);
        assert_eq!(scenario.transactions.len(), 4);
    }

    #[test]
    fn test_json_query_creation() {
        // Test that JSON queries can be created properly
        let query = json!({
            "query": "{ __typename }"
        });

        assert!(query.is_object());
        assert!(query.get("query").is_some());
        assert_eq!(query["query"], "{ __typename }");
    }

    #[test]
    fn test_complex_json_query() {
        // Test complex query with variables
        let query = json!({
            "query": "query GetUser($id: ID!) { user(id: $id) { id name } }",
            "variables": {
                "id": "123"
            }
        });

        assert!(query.is_object());
        assert!(query.get("query").is_some());
        assert!(query.get("variables").is_some());
        assert_eq!(query["variables"]["id"], "123");
    }
}
