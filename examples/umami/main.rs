mod admin;
mod common;
mod english;
mod spanish;

use goose::prelude::*;
use std::time::Duration;

use crate::admin::*;
use crate::english::*;
use crate::spanish::*;

/// Defines the actual load test. Each scneario simulates a type of user.
///  - Anonymous English user: loads the English version of all pages
///  - Anonymous Spanish user: loads the Spanish version of all pages
#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let _goose_metrics = GooseAttack::initialize()?
        .register_scenario(
            scenario!("Anonymous English user")
                .set_weight(40)?
                .set_wait_time(Duration::from_secs(0), Duration::from_secs(3))?
                .register_transaction(
                    transaction!(front_page_en)
                        .set_name("anon /")
                        .set_weight(2)?,
                )
                .register_transaction(transaction!(basic_page_en).set_name("anon /en/basicpage"))
                .register_transaction(
                    transaction!(article_listing_en).set_name("anon /en/articles/"),
                )
                .register_transaction(
                    transaction!(article_en)
                        .set_name("anon /en/articles/%")
                        .set_weight(2)?,
                )
                .register_transaction(transaction!(recipe_listing_en).set_name("anon /en/recipes/"))
                .register_transaction(
                    transaction!(recipe_en)
                        .set_name("anon /en/recipes/%")
                        .set_weight(4)?,
                )
                .register_transaction(transaction!(page_by_nid).set_name("anon /node/%nid"))
                .register_transaction(
                    transaction!(term_listing_en)
                        .set_name("anon /en term")
                        .set_weight(2)?,
                )
                .register_transaction(transaction!(search_en).set_name("anon /en/search"))
                .register_transaction(
                    transaction!(anonymous_contact_form_en).set_name("anon /en/contact"),
                ),
        )
        .register_scenario(
            scenario!("Anonymous Spanish user")
                .set_weight(9)?
                .set_wait_time(Duration::from_secs(0), Duration::from_secs(3))?
                .register_transaction(
                    transaction!(front_page_es)
                        .set_name("anon /es/")
                        .set_weight(2)?,
                )
                .register_transaction(transaction!(basic_page_es).set_name("anon /es/basicpage"))
                .register_transaction(
                    transaction!(article_listing_es).set_name("anon /es/articles/"),
                )
                .register_transaction(
                    transaction!(article_es)
                        .set_name("anon /es/articles/%")
                        .set_weight(2)?,
                )
                .register_transaction(transaction!(recipe_listing_es).set_name("anon /es/recipes/"))
                .register_transaction(
                    transaction!(recipe_es)
                        .set_name("anon /es/recipes/%")
                        .set_weight(4)?,
                )
                .register_transaction(
                    transaction!(term_listing_es)
                        .set_name("anon /es term")
                        .set_weight(2)?,
                )
                .register_transaction(transaction!(search_es).set_name("anon /es/search"))
                .register_transaction(
                    transaction!(anonymous_contact_form_es).set_name("anon /es/contact"),
                ),
        )
        .register_scenario(
            scenario!("Admin user")
                .set_weight(1)?
                .set_wait_time(Duration::from_secs(3), Duration::from_secs(10))?
                .register_transaction(
                    transaction!(log_in)
                        .set_on_start()
                        .set_name("auth /en/user/login"),
                )
                .register_transaction(
                    transaction!(front_page_en)
                        .set_name("auth /")
                        .set_weight(2)?,
                )
                .register_transaction(
                    transaction!(article_listing_en).set_name("auth /en/articles/"),
                )
                .register_transaction(
                    transaction!(edit_article)
                        .set_name("auth /en/node/%/edit")
                        .set_weight(2)?,
                ),
        )
        .set_default(GooseDefault::Host, "https://drupal-9.ddev.site/")?
        .execute()
        .await?;

    Ok(())
}
