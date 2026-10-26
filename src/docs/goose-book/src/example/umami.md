# Umami Example

The [`examples/umami`](https://github.com/tag1consulting/goose/tree/main/examples/umami) example load tests the [Umami demonstration profile](https://www.drupal.org/docs/umami-drupal-demonstration-installation-profile) included with [Drupal 9](https://www.drupal.org/blog/drupal-9-released).

## Overview

The Drupal Umami demonstration profile generates an attractive and realistic website simulating a food magazine, offering a practical example of what Drupal is capable of. The demo site is multi-lingual and has quite a bit of content, multiple taxonomies, and much of the rich functionality you'd expect from a real website, making it a good load test target.

The included example simulates three different types of users: an anonymous user browsing the site in English (scenario `Anonymous English user`, weight 40), an anonymous user browsing the site in Spanish (`Anonymous Spanish user`, weight 9), and an administrative user that logs into the site (`Admin user`, weight 1). The anonymous users load every kind of page on the site. For example, the anonymous user browsing the site in English loads the front page, a random basic page, the article listing and a random article, the recipe listing and a random recipe, a random node by node id, a random term listing, performs searches using words pulled from actual site content, and fills out the site's contact form; the Spanish user does the same in Spanish, except loading a node by node id. The admin user logs in once, then loads the front page, the article listing, and edits and saves a random article. With each action performed, Goose validates the HTTP response code and inspects the HTML returned to confirm that it contains the elements we expect, and loads the static assets the page references.

The example load tests `https://drupal-9.ddev.site/` unless `--host` names another site. The admin user logs in as `admin` with the password `P@ssw0rd1234`; set the `ADMIN_USERNAME` or `ADMIN_PASSWORD` environment variable to change either.

Read the blog [A Goose In The Clouds: Load Testing At Scale](https://www.tag1consulting.com/blog/goose-clouds-load-testing-scale) for a demonstration of using this example, and learn more about the testplan from the [README](https://github.com/tag1consulting/goose/blob/main/examples/umami/README.md).

## Alternative

The [Goose Eggs library](https://docs.rs/goose-eggs/) contains [a variation of the Umami example](https://github.com/tag1consulting/goose-eggs/tree/main/examples/umami).

## Complete Source Code

This example is more complex than the other examples, and is split into multiple files, all of which can be found within [`examples/umami`](https://github.com/tag1consulting/goose/tree/main/examples/umami).