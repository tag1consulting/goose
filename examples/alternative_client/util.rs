//! Helper functions to simplify the use of the Isahc HTTP client.
//!
//! ## License
//!
//! Copyright 2021 Jeremy Andrews
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

use goose::metrics::{GooseRawRequest, GooseRequestMetric};
use goose::prelude::*;

use isahc::prelude::*;
//use rand::Rng;
use regex::Regex;

pub(crate) fn build_request_metric(
    user: &mut GooseUser,
    method: GooseMethod,
    url: &str,
    headers: Option<&http::HeaderMap>,
    error: &str,
    started: std::time::Instant,
    status: http::StatusCode,
) -> GooseRequestMetric {
    let mut h: Vec<String> = Vec::new();
    for header in headers {
        h.push(format!("{:?}", header));
    }

    // Record information about the request.
    let raw_request = GooseRawRequest {
        method,
        url: url.to_string(),
        headers: h,
        //@TODO
        body: "".to_string(),
    };

    GooseRequestMetric {
        elapsed: user.started.elapsed().as_millis() as u64,
        raw: raw_request,
        name: url.to_string(),
        //@TODO
        final_url: url.to_string(),
        //@TODO
        redirected: false,
        response_time: started.elapsed().as_millis() as u64,
        status_code: status.as_u16(),
        success: status.is_success(),
        update: false,
        user: user.weighted_users_index,
        error: error.to_string(),
        // Coordinated Omission is disabled when using a custom client.
        coordinated_omission_elapsed: 0,
        user_cadence: 0,
    }
}

pub async fn load_static_assets(
    user: &mut GooseUser,
    headers: Option<&http::HeaderMap>,
    text: &str,
) {
    let re = Regex::new(r#"src="(.*?)""#).unwrap();
    for url in re.captures_iter(text) {
        if url[1].contains("/misc") || url[1].contains("/themes") {
            let started = std::time::Instant::now();
            let url = user.build_url("/").unwrap();
            match isahc::get_async(&url).await {
                Ok(mut r) => {
                    let status = r.status();
                    let mut request_metric = build_request_metric(
                        user,
                        GooseMethod::Get,
                        &url,
                        headers,
                        "",
                        started,
                        status,
                    );
                    r.consume().await.unwrap();
                    request_metric.name = "static asset".to_string();
                    let _ = user.send_request_metric_to_parent(request_metric);
                }
                Err(e) => {
                    let mut request_metric = build_request_metric(
                        user,
                        GooseMethod::Get,
                        &url,
                        headers,
                        &e.to_string(),
                        started,
                        http::StatusCode::from_u16(500).unwrap(),
                    );
                    request_metric.name = "static asset".to_string();
                    let _ = user.send_request_metric_to_parent(request_metric.clone());
                }
            };
        }
    }
}
