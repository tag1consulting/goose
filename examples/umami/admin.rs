use goose::prelude::*;

use crate::common;

use rand::seq::SliceRandom;
use std::env;

/// Log into the website.
pub async fn log_in(user: &mut GooseUser) -> TransactionResult {
    // Use ADMIN_USERNAME= to set custom admin username.
    let admin_username = match env::var("ADMIN_USERNAME") {
        Ok(username) => username,
        Err(_) => "admin".to_string(),
    };
    // Use ADMIN_PASSWORD= to set custom admin username.
    let admin_password = match env::var("ADMIN_PASSWORD") {
        Ok(password) => password,
        Err(_) => "P@ssw0rd1234".to_string(),
    };

    // Load the log in page.
    let mut goose = user.get("/en/user/login").await?;

    // We can't invoke common::validate_and_load_static_assets as while it's important
    // to validate the page and load static elements, we then need to extract form elements
    // from the HTML of the page. So we duplicate some of the logic, enhancing it for form
    // processing.
    let mut logged_in_user;
    match goose.response {
        Ok(response) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &response.headers().clone();
            match response.text().await {
                Ok(html) => {
                    // Be sure we've properly loaded the log in page.
                    let title = "Log in";
                    if !common::valid_title(&html, title) {
                        return user.set_failure(
                            &format!("{}: title not found: {}", &goose.request.raw.url, title),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }

                    // Load all static elements on the page, as a real user would.
                    common::load_static_elements(user, &html).await;

                    // Scrape the HTML to get the values needed in order to POST to the
                    // log in form.
                    let form_build_id = common::get_form_value(&html, "form_build_id");
                    if form_build_id.is_none() {
                        return user.set_failure(
                            &format!("{}: no form_build_id on page", goose.request.raw.url),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }

                    // Build log in form with username and password from environment.
                    let params = [
                        ("name", &admin_username),
                        ("pass", &admin_password),
                        ("form_build_id", &form_build_id.unwrap()),
                        ("form_id", &"user_login_form".to_string()),
                        ("op", &"Log+in".to_string()),
                    ];
                    logged_in_user = user.post_form("/en/user/login", &params).await?;

                    // A successful log in is redirected.
                    if !logged_in_user.request.redirected {
                        return user.set_failure(
                            &format!(
                                "{}: login failed (check ADMIN_USERNAME and ADMIN_PASSWORD)",
                                logged_in_user.request.final_url
                            ),
                            &mut logged_in_user.request,
                            Some(headers),
                            None,
                        );
                    }
                }
                Err(e) => {
                    return user.set_failure(
                        &format!("{}: failed to parse page: {}", goose.request.raw.url, e),
                        &mut goose.request,
                        Some(headers),
                        None,
                    );
                }
            }
        }
        Err(e) => {
            return user.set_failure(
                &format!("{}: no response from server: {}", goose.request.raw.url, e),
                &mut goose.request,
                None,
                None,
            );
        }
    }
    // Check the title to verify that the user is logged in.
    common::validate_and_load_static_assets(user, logged_in_user, &admin_username).await?;

    Ok(())
}

/// Load and edit a random article.
pub async fn edit_article(user: &mut GooseUser) -> TransactionResult {
    // First, load a random article.
    let nodes = common::get_nodes(&common::ContentType::Article);
    let article = nodes.choose(&mut rand::thread_rng());
    let goose = user.get(article.unwrap().url_en).await?;
    common::validate_and_load_static_assets(user, goose, article.unwrap().title_en).await?;

    // Next, load the edit link for the chosen article.
    let mut goose = user
        .get(&format!("/en/node/{}/edit", article.unwrap().nid))
        .await?;

    let mut saved_article;
    match goose.response {
        Ok(response) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &response.headers().clone();
            match response.text().await {
                Ok(html) => {
                    // Be sure we've properly loaded the edit page.
                    let title = "Edit Article";
                    if !common::valid_title(&html, title) {
                        return user.set_failure(
                            &format!("{}: title not found: {}", &goose.request.raw.url, title),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }

                    // Load all static elements on the page, as a real user would.
                    common::load_static_elements(user, &html).await;

                    // Scrape the HTML to get the values needed in order to POST to the
                    // log in form.
                    let form_build_id = common::get_form_value(&html, "form_build_id");
                    if form_build_id.is_none() {
                        return user.set_failure(
                            &format!("{}: no form_build_id on page", goose.request.raw.url),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }
                    let form_token = common::get_form_value(&html, "form_token");
                    if form_token.is_none() {
                        return user.set_failure(
                            &format!("{}: no form_token on page", goose.request.raw.url),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }

                    // Build node form with random word from title.
                    let params = [
                        ("form_build_id", &form_build_id.unwrap()),
                        ("form_token", &form_token.unwrap()),
                        ("form_id", &"node_article_edit_form".to_string()),
                        ("op", &"Save (this translation)".to_string()),
                    ];
                    saved_article = user
                        .post_form(&format!("/en/node/{}/edit", article.unwrap().nid), &params)
                        .await?;

                    // A successful node save is redirected.
                    if !saved_article.request.redirected {
                        return user.set_failure(
                            &format!("{}: saving article failed", saved_article.request.final_url),
                            &mut saved_article.request,
                            Some(headers),
                            None,
                        );
                    }
                }
                Err(e) => {
                    return user.set_failure(
                        &format!("{}: failed to parse page: {}", goose.request.raw.url, e),
                        &mut goose.request,
                        Some(headers),
                        None,
                    );
                }
            }
        }
        Err(e) => {
            return user.set_failure(
                &format!("{}: no response from server: {}", goose.request.raw.url, e),
                &mut goose.request,
                None,
                None,
            );
        }
    }
    // Be sure we're viewing the same article after editing it.
    common::validate_and_load_static_assets(user, saved_article, article.unwrap().title_en).await?;

    Ok(())
}
