# Umami Example

The [`examples/umami`](https://github.com/tag1consulting/goose/tree/main/examples/umami) example load tests the [Umami demonstration profile](https://www.drupal.org/docs/umami-drupal-demonstration-installation-profile) included with [Drupal 9](https://www.drupal.org/blog/drupal-9-released).

## Overview

The Drupal Umami demonstration profile generates an attractive and realistic website simulating a food magazine, offering a practical example of what Drupal is capable of. The demo site is multi-lingual and has quite a bit of content, multiple taxonomies, and much of the rich functionality you'd expect from a real website, making it a good load test target.

The included example simulates three different types of users: an anonymous user browsing the site in English, an anonymous user browsing the site in Spanish, and an administrative user that logs into the site. The two anonymous users visit every page on the site. For example, the anonymous user browsing the site in English loads the front page, browses all the articles and the article listings, views all the recipes and recipe listings, accesses all nodes directly by node id, performs searches using terms pulled from actual site content, and fills out the site's contact form. With each action performed, Goose validates the HTTP response code and inspects the HTML returned to confirm that it contains the elements we expect.

Read the blog [A Goose In The Clouds: Load Testing At Scale](https://www.tag1consulting.com/blog/goose-clouds-load-testing-scale) for a demonstration of using this example, and learn more about the testplan from the [README](https://github.com/tag1consulting/goose/blob/main/examples/umami/README.md).

## Alternative

The [Goose Eggs library](https://docs.rs/goose-eggs/) contains [a variation of the Umami example](https://github.com/tag1consulting/goose-eggs/tree/main/examples/umami).

## Complete Source Code

This example is more complex than the other examples, and is split into multiple files, all of which can be found within [`examples/umami`](https://github.com/tag1consulting/goose/tree/main/examples/umami).