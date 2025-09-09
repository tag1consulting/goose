//! Test to verify that cloning GooseUser with session data doesn't cause stack overflow.
//! This test reproduces the issue from GitHub issue #606.

use goose::config::GooseConfiguration;
use goose::prelude::*;
use gumdrop::Options;

#[derive(Debug, Clone)]
struct Session {
    jwt_token: String,
}

#[tokio::test]
async fn test_clone_user_with_session_data() {
    // Create a GooseUser with session data
    let configuration = GooseConfiguration::parse_args_default(&[
        "--host",
        "http://localhost:8080",
        "--users",
        "1",
        "--hatch-rate",
        "1",
        "--run-time",
        "1",
        "--quiet",
    ])
    .unwrap();
    let mut user =
        GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();

    // Set session data
    user.set_session_data(Session {
        jwt_token: "test_token".to_string(),
    });

    // This should not cause a stack overflow
    let cloned_user = user.clone();

    // Verify that the session data was cloned correctly
    let original_session = user.get_session_data::<Session>().unwrap();
    let cloned_session = cloned_user.get_session_data::<Session>().unwrap();

    assert_eq!(original_session.jwt_token, cloned_session.jwt_token);
    assert_eq!(original_session.jwt_token, "test_token");
}

#[tokio::test]
async fn test_clone_user_without_session_data() {
    // Create a GooseUser without session data
    let configuration = GooseConfiguration::parse_args_default(&[
        "--host",
        "http://localhost:8080",
        "--users",
        "1",
        "--hatch-rate",
        "1",
        "--run-time",
        "1",
        "--quiet",
    ])
    .unwrap();
    let user = GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();

    // This should work fine (and always has)
    let cloned_user = user.clone();

    // Verify that neither has session data
    assert!(user.get_session_data::<Session>().is_none());
    assert!(cloned_user.get_session_data::<Session>().is_none());
}

#[tokio::test]
async fn test_multiple_clones_with_session_data() {
    // Test multiple levels of cloning to ensure no issues
    let configuration = GooseConfiguration::parse_args_default(&[
        "--host",
        "http://localhost:8080",
        "--users",
        "1",
        "--hatch-rate",
        "1",
        "--run-time",
        "1",
        "--quiet",
    ])
    .unwrap();
    let mut user1 =
        GooseUser::single("http://localhost:8080".parse().unwrap(), &configuration).unwrap();

    user1.set_session_data(Session {
        jwt_token: "original_token".to_string(),
    });

    let user2 = user1.clone();
    let user3 = user2.clone();
    let user4 = user3.clone();

    // All should have the same session data
    assert_eq!(
        user1.get_session_data::<Session>().unwrap().jwt_token,
        "original_token"
    );
    assert_eq!(
        user2.get_session_data::<Session>().unwrap().jwt_token,
        "original_token"
    );
    assert_eq!(
        user3.get_session_data::<Session>().unwrap().jwt_token,
        "original_token"
    );
    assert_eq!(
        user4.get_session_data::<Session>().unwrap().jwt_token,
        "original_token"
    );
}
