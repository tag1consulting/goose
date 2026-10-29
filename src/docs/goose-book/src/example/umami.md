# Umami Example

The [`examples/umami`](https://github.com/tag1consulting/goose/tree/main/examples/umami) example load tests the [Umami demonstration profile](https://www.drupal.org/docs/umami-drupal-demonstration-installation-profile) included with Drupal core.

<img src="umami-transactions.png" width="576" alt="The dashboard's Scenarios table with Anonymous English user at 40 users, Anonymous Spanish user at 9 and Admin user at 1, cut after its runs/s column, above the Transactions table's first six rows, anon / through anon /en/recipes/%, all for the English user, with 0 fails, cut after its #fails column">

*`cargo run --release --features dashboard --example umami -- --host http://localhost:18888 -u 50 -r 5 -t 3m --no-reset-metrics --dashboard -q`, against a Drupal 11 site installed with the Umami demo profile, with CSS and JS aggregation turned off and the admin account `admin` with the password `P@ssw0rd1234`, 120 seconds in.*

The Scenarios table shows that with 50 users the weights 40, 9 and 1 give exactly 40, 9 and 1 users. The Transactions table goes on past the six English rows shown here to 23 rows in all: the rest of the English user's, the Spanish user's and the admin user's. In about the first two minutes of the run, the 50 users made 61,652 requests with none failed; 57,772 of them were the images and scripts the pages name, which the example requests under the one name `static asset`. With `-q` Goose does not print the dashboard's address when it starts: open `http://127.0.0.1:5118` while the test runs. If that tab is still open when the run ends, Goose prints the address and waits for Ctrl-C.

Turn CSS and JS aggregation off on the site: the example does not decode `&amp;` in the aggregated asset links it parses, so the URLs it requests still hold `&amp;` where the page meant `&`. With aggregation off it loads no CSS, because it looks for stylesheets only under the aggregated path.

## Overview

The Umami profile installs a sample food magazine site in English and Spanish, with articles, recipes, basic pages, several taxonomies, search and a contact form.

The included example simulates three different types of users: an anonymous user browsing the site in English (scenario `Anonymous English user`, weight 40), an anonymous user browsing the site in Spanish (`Anonymous Spanish user`, weight 9), and an administrative user that logs into the site (`Admin user`, weight 1). The anonymous users load most of the site's page types. The anonymous user browsing the site in English loads the front page, the About Umami basic page, the article listing and a random article, the recipe listing and a random recipe, a random node by node id, a random term listing, runs searches using words from the site's content, and fills out the site's contact form. The Spanish user does the same in Spanish, except loading a node by node id. The admin user logs in once, then loads the front page, the article listing, and edits and saves a random article. Goose counts any response that is not a 2xx as a failure. On each page the example also checks the HTML for the expected title and loads the images and scripts the page references.

The example load tests `https://drupal-9.ddev.site/` unless `--host` names another site. The admin user logs in as `admin` with the password `P@ssw0rd1234`; set the `ADMIN_USERNAME` or `ADMIN_PASSWORD` environment variable to change either.

Read the blog [A Goose In The Clouds: Load Testing At Scale](https://www.tag1consulting.com/blog/goose-clouds-load-testing-scale) for a demonstration of using this example, and learn more about the testplan from the [README](https://github.com/tag1consulting/goose/blob/main/examples/umami/README.md).

## Alternative

The [Goose Eggs library](https://docs.rs/goose-eggs/) contains [a variation of the Umami example](https://github.com/tag1consulting/goose-eggs/tree/main/examples/umami).

## Complete Source Code

This example is more complex than the other examples, and is split into multiple files, all of which can be found within [`examples/umami`](https://github.com/tag1consulting/goose/tree/main/examples/umami).