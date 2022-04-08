//! Conversion of drupal_memcache example to use the Isahc http client instead
//! of Reqwest.
//!
//! To run, you must set up the load test environment as described in the
//! drupal_memcache example.
//!
//! ## License
//!
//! Copyright 2021 Jeremy Andrews
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! <http://www.apache.org/licenses/LICENSE-2.0>
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

mod util;

use goose::prelude::*;

use isahc::prelude::*;
use rand::Rng;
use regex::Regex;

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_taskset(
            taskset!("AnonBrowsingUser")
                .set_weight(4)?
                .register_task(
                    task!(drupal_memcache_front_page)
                        .set_weight(15)?
                        .set_name("(Anon) front page"),
                )
                .register_task(
                    task!(drupal_memcache_node_page)
                        .set_weight(10)?
                        .set_name("(Anon) node page"),
                )
                .register_task(
                    task!(drupal_memcache_profile_page)
                        .set_weight(3)?
                        .set_name("(Anon) user page"),
                ),
        )
        .register_taskset(
            taskset!("AuthBrowsingUser")
                .set_weight(1)?
                .register_task(
                    task!(drupal_memcache_login)
                        .set_on_start()
                        .set_name("(Auth) login"),
                )
                .register_task(
                    task!(drupal_memcache_front_page)
                        .set_weight(15)?
                        .set_name("(Auth) front page"),
                )
                .register_task(
                    task!(drupal_memcache_node_page)
                        .set_weight(10)?
                        .set_name("(Auth) node page"),
                )
                .register_task(
                    task!(drupal_memcache_profile_page)
                        .set_weight(3)?
                        .set_name("(Auth) user page"),
                ),
            /*
                    .register_task(
                        task!(drupal_memcache_post_comment)
                            .set_weight(3)?
                            .set_name("(Auth) comment form"),
                    ),
            */
        )
        .execute()
        .await?
        .print();

    Ok(())
}

/// View the front page.
async fn drupal_memcache_front_page(user: &mut GooseUser) -> GooseTaskResult {
    let started = std::time::Instant::now();
    let url = user.build_url("/").unwrap();
    let response = isahc::get_async(&url).await;

    match response {
        Ok(mut r) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &r.headers().clone();
            match r.text().await {
                Ok(t) => {
                    let status = r.status();
                    let mut request_metric = util::build_request_metric(
                        user,
                        GooseMethod::Get,
                        &url,
                        Some(headers),
                        "",
                        started,
                        status,
                    );
                    request_metric.name = "/".to_string();
                    user.send_request_metric_to_parent(request_metric)?;

                    // Load all static assets.
                    util::load_static_assets(user, Some(headers), &t).await;
                }
                Err(e) => {
                    let status = r.status();
                    let mut request_metric = util::build_request_metric(
                        user,
                        GooseMethod::Get,
                        &url,
                        Some(headers),
                        &e.to_string(),
                        started,
                        status,
                    );
                    request_metric.name = "/".to_string();
                    user.send_request_metric_to_parent(request_metric.clone())?;
                    return user.set_failure(
                        &format!("front page: failed to parse page: {}", e),
                        &mut request_metric,
                        Some(headers),
                        None,
                    );
                }
            }
        }
        Err(e) => {
            let mut request_metric = util::build_request_metric(
                user,
                GooseMethod::Get,
                &url,
                None,
                &e.to_string(),
                started,
                http::StatusCode::from_u16(500).unwrap(),
            );
            request_metric.name = "/".to_string();
            user.send_request_metric_to_parent(request_metric.clone())?;
            return user.set_failure(
                &format!("front page: no response from server: {}", e),
                &mut request_metric,
                None,
                None,
            );
        }
    }

    Ok(())
}

/// View a node from 1 to 10,000, created by preptest.sh.
async fn drupal_memcache_node_page(user: &mut GooseUser) -> GooseTaskResult {
    let started = std::time::Instant::now();
    let nid = rand::thread_rng().gen_range(1..10_000);
    let url = user.build_url(format!("/node/{}", &nid).as_str()).unwrap();
    let response = isahc::get_async(&url).await;

    match response {
        Ok(mut r) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &r.headers().clone();
            let status = r.status();
            let mut request_metric = util::build_request_metric(
                user,
                GooseMethod::Get,
                &url,
                Some(headers),
                "",
                started,
                status,
            );
            request_metric.name = "/node/{}".to_string();
            r.consume().await.unwrap();
            user.send_request_metric_to_parent(request_metric)?;
        }
        Err(e) => {
            let mut request_metric = util::build_request_metric(
                user,
                GooseMethod::Get,
                &url,
                None,
                &e.to_string(),
                started,
                http::StatusCode::from_u16(500).unwrap(),
            );
            request_metric.name = "/node/{}".to_string();
            user.send_request_metric_to_parent(request_metric.clone())?;
            return user.set_failure(
                &format!("node page: no response from server: {}", e),
                &mut request_metric,
                None,
                None,
            );
        }
    }

    Ok(())
}

/// View a profile from 2 to 5,001, created by preptest.sh.
async fn drupal_memcache_profile_page(user: &mut GooseUser) -> GooseTaskResult {
    let started = std::time::Instant::now();
    let uid = rand::thread_rng().gen_range(2..5_001);
    let url = user.build_url(format!("/user/{}", &uid).as_str()).unwrap();
    let response = isahc::get_async(&url).await;

    match response {
        Ok(mut r) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &r.headers().clone();
            let status = r.status();
            let mut request_metric = util::build_request_metric(
                user,
                GooseMethod::Get,
                &url,
                Some(headers),
                "",
                started,
                status,
            );
            request_metric.name = "/user/{}".to_string();
            r.consume().await.unwrap();
            user.send_request_metric_to_parent(request_metric)?;
        }
        Err(e) => {
            let mut request_metric = util::build_request_metric(
                user,
                GooseMethod::Get,
                &url,
                None,
                &e.to_string(),
                started,
                http::StatusCode::from_u16(500).unwrap(),
            );
            request_metric.name = "/user/{}".to_string();
            user.send_request_metric_to_parent(request_metric.clone())?;
            return user.set_failure(
                &format!("user page: no response from server: {}", e),
                &mut request_metric,
                None,
                None,
            );
        }
    }

    Ok(())
}

/// Log in.
async fn drupal_memcache_login(user: &mut GooseUser) -> GooseTaskResult {
    let started = std::time::Instant::now();
    let url = user.build_url("/user").unwrap();
    let response = isahc::get_async(&url).await;

    match response {
        Ok(mut r) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &r.headers().clone();
            let status = r.status();
            let mut request_metric = util::build_request_metric(
                user,
                GooseMethod::Get,
                &url,
                Some(headers),
                "",
                started,
                status,
            );
            request_metric.name = "/user".to_string();
            user.send_request_metric_to_parent(request_metric.clone())?;

            match r.text().await {
                Ok(html) => {
                    let re = Regex::new(r#"name="form_build_id" value=['"](.*?)['"]"#).unwrap();
                    let form_build_id = match re.captures(&html) {
                        Some(f) => f,
                        None => {
                            // This will automatically get written to the error log if enabled, and will
                            // be displayed to stdout if `-v` is enabled when running the load test.
                            return user.set_failure(
                                "login: no form_build_id on page: /user page",
                                &mut request_metric,
                                Some(headers),
                                Some(&html),
                            );
                        }
                    };

                    // Log the user in.
                    let uid: usize = rand::thread_rng().gen_range(3..5_002);
                    let username = format!("user{}", uid);
                    let params = format!(
                        r#"{{
                        "name": "{}",
                        "pass": "12345",
                        "form_build_id": "{}",
                        "form_id": "user_login",
                        "op": "Log+in",
                    }}"#,
                        username.as_str(),
                        &form_build_id[1],
                    );
                    let started = std::time::Instant::now();
                    let response = isahc::post_async(&url, params.as_str()).await;
                    match response {
                        Ok(mut r) => {
                            // Copy the headers so we have them for logging if there are errors.
                            let headers = &r.headers().clone();
                            let status = r.status();
                            let mut request_metric = util::build_request_metric(
                                user,
                                GooseMethod::Post,
                                &url,
                                Some(headers),
                                "",
                                started,
                                status,
                            );
                            request_metric.name = "/user".to_string();
                            r.consume().await.unwrap();
                            user.send_request_metric_to_parent(request_metric)?;
                        }
                        Err(e) => {
                            let mut request_metric = util::build_request_metric(
                                user,
                                GooseMethod::Post,
                                &url,
                                None,
                                &e.to_string(),
                                started,
                                http::StatusCode::from_u16(500).unwrap(),
                            );
                            request_metric.name = "/user".to_string();
                            user.send_request_metric_to_parent(request_metric.clone())?;
                            return user.set_failure(
                                &format!("user page: no response from server: {}", e),
                                &mut request_metric,
                                None,
                                None,
                            );
                        }
                    }
                }
                Err(e) => {
                    let status = r.status();
                    let mut request_metric = util::build_request_metric(
                        user,
                        GooseMethod::Get,
                        &url,
                        Some(headers),
                        &e.to_string(),
                        started,
                        status,
                    );
                    request_metric.name = "/".to_string();
                    user.send_request_metric_to_parent(request_metric.clone())?;
                    return user.set_failure(
                        &format!("login: failed to parse page: {}", e),
                        &mut request_metric,
                        Some(headers),
                        None,
                    );
                }
            }
        }
        Err(e) => {
            let mut request_metric = util::build_request_metric(
                user,
                GooseMethod::Get,
                &url,
                None,
                &e.to_string(),
                started,
                http::StatusCode::from_u16(500).unwrap(),
            );
            request_metric.name = "/user".to_string();
            user.send_request_metric_to_parent(request_metric.clone())?;
            return user.set_failure(
                &format!("login: no response from server: {}", e),
                &mut request_metric,
                None,
                None,
            );
        }
    }

    Ok(())
}
