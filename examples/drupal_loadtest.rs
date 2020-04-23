//! Conversion of Locust load test used for the Drupal memcache module, from
//! https://github.com/tag1consulting/drupal-loadtest/
//! 
//! ## License
//! 
//! Copyright 2020 Jeremy Andrews
//! 
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//! 
//! http://www.apache.org/licenses/LICENSE-2.0
//! 
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

use rand::Rng;
use scraper::{Html, Selector};
use select::document::Document;
use select::predicate::Name;

use goose::GooseState;
use goose::goose::{GooseTaskSet, GooseClient, GooseTask};

fn main() {
    GooseState::initialize()
        .register_taskset(GooseTaskSet::new("AnonBrowsingUser")
            .set_weight(4)
            .register_task(GooseTask::new(drupal_loadtest_front_page)
                .set_weight(15)
                .set_name("(Anon) front page")
            )
            .register_task(GooseTask::new(drupal_loadtest_node_page)
                .set_weight(10)
                .set_name("(Anon) node page")
            )
            .register_task(GooseTask::new(drupal_loadtest_profile_page)
                .set_weight(3)
                .set_name("(Anon) user page")
            )
        )
        .register_taskset(GooseTaskSet::new("AuthBrowsingUser")
            .set_weight(1)
            .register_task(GooseTask::new(drupal_loadtest_login)
                .set_on_start()
                .set_name("(Auth) login")
            )
            .register_task(GooseTask::new(drupal_loadtest_front_page)
                .set_weight(15)
                .set_name("(Auth) front page")
            )
            .register_task(GooseTask::new(drupal_loadtest_node_page)
                .set_weight(10)
                .set_name("(Auth) node page")
            )
            .register_task(GooseTask::new(drupal_loadtest_profile_page)
                .set_weight(3)
                .set_name("(Auth) user page")
            )
            .register_task(GooseTask::new(drupal_loadtest_post_comment)
                .set_weight(3)
                .set_name("(Auth) comment form")
            )
        )
        .execute();
}

/// View the front page.
fn drupal_loadtest_front_page(client: &mut GooseClient) {
    let response = client.get("/");

    // Grab some static assets from the front page.
    match response {
        Ok(r) => {
            // Grab all src values from image tags.
            Document::from_read(r)
                .unwrap()
                .find(Name("img"))
                .filter_map(|n| n.attr("src"))
                .for_each(|x| {
                    // @TODO: once we're done comparing Goose to Locust, improve this
                    // to do a better job of matching local assets
                    if x.contains("/misc") || x.contains("/themes") {
                        println!("getting {}", x);
                        let _response = client.set_request_name("static asset").get(x);
                    }
                });
        },
        Err(e) => {
            eprintln!("unexpected error when loading / page: {}", e);
            client.set_failure();
        },
    }
}

/// View a node from 1 to 10,000, created by preptest.sh.
fn drupal_loadtest_node_page(client: &mut GooseClient) {
    let nid = rand::thread_rng().gen_range(1, 10_000);
    let _response = client.get(format!("/node/{}", &nid).as_str());
}

/// View a profile from 2 to 5,001, created by preptest.sh.
fn drupal_loadtest_profile_page(client: &mut GooseClient) {
    let uid = rand::thread_rng().gen_range(2, 5_001);
    let _response = client.get(format!("/user/{}", &uid).as_str());
}

/// Log in.
fn drupal_loadtest_login(client: &mut GooseClient) {
    let response = client.get("/user");
    match response {
        Ok(r) => {
            match r.text() {
                Ok(html) => {
                    // Extract the form_build_id from the user login form.
                    let user_page = Html::parse_document(&html);
                    let selector = Selector::parse(r#"input[name='form_build_id']"#).expect("failed to parse selector");
                    let input = match user_page.select(&selector).next() {
                        Some(i) => i,
                        None => {
                            eprintln!("no form_build_id found on /user");
                            client.set_failure();
                            return;
                        }
                    };
                    let form_build_id = input.value().attr("value").expect("failed to get form_build_id value");

                    // Log the user in.
                    let uid = rand::thread_rng().gen_range(3, 5_002);
                    let username = format!("user{}", uid);
                    let params = [
                        ("name", username.as_str()),
                        ("pass", "12345"),
                        ("form_build_id", form_build_id),
                        ("form_id", "user_login"),
                        ("op", "Log+in"),
                    ];
                    let request_builder = client.goose_post("/user");
                    let _response = client.goose_send(request_builder.form(&params));
                    // @TODO: verify that we actually logged in.
                }
                Err(e) => {
                    eprintln!("unexpected error when loading /user page: {}", e);
                    client.set_failure();
                }
            }
        }
        // Goose will catch this error.
        Err(_) => (),
    }
}

/// Post a comment.
fn drupal_loadtest_post_comment(client: &mut GooseClient) {
    let nid = rand::thread_rng().gen_range(1, 10_000);
    let response = client.get(format!("/node/{}", &nid).as_str());
    match response {
        Ok(r) => {
            match r.text() {
                Ok(html) => {
                    // Extract the form_build_id from the user login form.
                    let comment_page = Html::parse_document(&html);

                    let selector = Selector::parse(r#"input[name='form_build_id']"#).expect("failed to parse form_build_id selector");
                    let input = match comment_page.select(&selector).next() {
                        Some(i) => i,
                        None => {
                            eprintln!("no form_build_id found on node/{}", &nid);
                            client.set_failure();
                            return;
                        }
                    };
                    let form_build_id = input.value().attr("value").expect("failed to get form_build_id value");

                    let selector = Selector::parse(r#"input[name='form_token']"#).expect("failed to parse form_token selector");
                    let input = match comment_page.select(&selector).next() {
                        Some(i) => i,
                        None => {
                            eprintln!("no form_token found on node/{}", &nid);
                            client.set_failure();
                            return;
                        }
                    };
                    let form_token = input.value().attr("value").expect("failed to get form_token value");

                    let selector = Selector::parse(r#"input[name='form_id']"#).expect("failed to parse form_token selector");
                    let input = match comment_page.select(&selector).next() {
                        Some(i) => i,
                        None => {
                            eprintln!("no form_id found on node/{}", &nid);
                            client.set_failure();
                            return;
                        }
                    };
                    let form_id = input.value().attr("value").expect("failed to get form_id value");
                    //println!("form_id: {}, form_build_id: {}, form_token: {}", &form_id, &form_build_id, &form_token);

                    let comment_body = "this is a test comment body";
                    let params = [
                        ("subject", "this is a test comment subject"),
                        ("comment_body[und][0][value]", &comment_body),
                        ("comment_body[und][0][format]", "filtered_html"),
                        ("form_build_id", form_build_id),
                        ("form_token", form_token),
                        ("form_id", form_id),
                        ("op", "Save"),
                    ];
                    let request_builder = client.goose_post(format!("/comment/reply/{}", &nid).as_str());
                    let response = client.goose_send(request_builder.form(&params));
                    match response {
                        Ok(r) => {
                            match r.text() {
                                Ok(html) => {
                                    if !html.contains(&comment_body) {
                                        eprintln!("no comment showed up after posting to comment/reply/{}", &nid);
                                        client.set_failure();
                                    }
                                }
                                Err(e) => {
                                    eprintln!("unexpected error when posting to comment/reply/{}: {}", &nid, e);
                                    client.set_failure();
                                }
                            }
                        }
                        // Goose will catch this error.
                        Err(_) => (),
                    }
                }
                Err(e) => {
                    eprintln!("unexpected error when loading node/{} page: {}", &nid, e);
                    client.set_failure();
                }
            }
        }
        // Goose will catch this error.
        Err(_) => (),
    }
}
