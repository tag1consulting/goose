# Overview

This is a load test for Drupal's Umami demonstration profile, as Drupal 11.4 installs it. It targets Drupal 11.4 only: earlier releases install different content and modules. Install a stock site and leave it as the installer sets it up, with CSS and JS aggregation on; the load test needs nothing changed or enabled.

By default it loads pages from `https://drupal-9.ddev.site/`. Use the `--host` flag to name the site to load test.

## Installing The Site

One way to install the site is with two containers, both pinned by digest: the database, `mariadb:11.4@sha256:d1891ede5a559f1086f55053bb8cc1569b0298338c4d72d71725a7f74f6ba620`, with a database `drupal` and a user `drupal`, and the web server, `drupal:11-apache@sha256:f101028b47c9a39cb07869d7af298e14702d43d8ac3c0c92b632765fbeae0d0e` (Drupal 11.4.11). In the web container, with `DB_PASSWORD` and `ADMIN_PASSWORD` set and the database reachable as `db`:

```
cd /opt/drupal
composer require drush/drush
php -d memory_limit=-1 vendor/drush/drush/drush.php site:install demo_umami -y \
  --db-url="mysql://drupal:${DB_PASSWORD}@db:3306/drupal" \
  --account-name=admin --account-pass="$ADMIN_PASSWORD"
php -d memory_limit=-1 vendor/drush/drush/drush.php config:set system.mail interface.default test_mail_collector -y
php -d memory_limit=-1 vendor/drush/drush/drush.php cron
chown -R www-data:www-data web/sites/default/files
apache2-foreground
```

`site:install demo_umami` needs more memory than PHP's command line default of 128M, so drush runs with no limit. The mail setting keeps the site from trying to send mail, and the `chown` lets Apache write CSS and JS aggregates and image derivatives, as the install ran as root.

The first request for each image derivative makes Drupal generate it, and while one request generates a derivative, Drupal answers every other request for it with `503 Service Unavailable` and `Retry-After: 3`. A load test against a new site counts those 503s as failures in its first seconds; a second run, with every derivative generated, has none.

## Load Test Implementation

The load test is split into the following files:
 - `main.rs`: This file contains the main() function and defines the actual load test;
 - `common.rs`: This file contains helper functions used by the transaction functions, the lists of the site's nodes and terms, and the tests of its asset parsing;
 - `english.rs`: This file contains all transaction functions loading pages in English;
 - `spanish.rs`: This file contains all transaction functions loading pages in Spanish;
 - `admin.rs`: This file contains all transaction functions specific to simulating an admin user;
 - `fixtures/`: the English front page of a stock Drupal 11.4 Umami site, saved with CSS and JS aggregation on and with it off, for the tests.

## Load Test Features

The load test defines the following users:
 - Anonymous English user: this user performs all transactions in English, it has a weight of 40, and randomly pauses for 0 to 3 seconds after each transaction;
 - Anonymous Spanish user: this user performs all transactions in Spanish, it has a weight of 9, and randomly pauses for 0 to 3 seconds after each transaction;
 - Admin user: this user logs into the website, it has a weight of 1, and randomly pauses for 3 to 10 seconds after each transaction.

Due to user weighting, the load test should simulate at least 50 users when it runs. If you simulate 100 users (with the `-u 100` run time option) then 80 anonymous English users, 18 anonymous Spanish users, and 2 admin users will be simulated.

Each anonymous load test user runs the following transactions in their own language:
 - Loads the front page;
 - Loads a "basic page";
 - Loads the article listing page;
 - Loads an "article";
 - Loads the recipe listing page;
 - Loads a "recipe";
 - Loads a random node by nid (the English user only);
 - Loads the term listing page filtered by a random term.

The contact form and search transactions were removed, as Drupal 11.4's Umami installs neither the contact module nor node search.

Each admin load test user logs in one time in English, and then runs the following transactions:
 - Loads the front page;
 - Loads the article listing page;
 - Loads an "article", edits (not making any actual changes), and saves it, which invalidates every cached page that shows the article or a list of nodes.

Every page is checked for its expected title, and then the load test loads the stylesheets, scripts and images the page names, as a browser would, each once: stylesheets under the request name `static css`, scripts under `static js` and images under `static image`. It finds them whether Drupal aggregates CSS and JS or not, and decodes the `&amp;` in aggregated asset URLs. It does not load the images named only in `srcset`.

## Configuring The Admin User

The load test needs to know what username and password to use to log in. By default it will attempt to log in with the username `admin` and the password `P@ssw0rd1234`. However, you can use the ADMIN_USERNAME and/or ADMIN_PASSWORD environment variables to log in with different values. In the following example, the load test will attempt to log in with the username `foo` and the password `bar`:

```
ADMIN_USERNAME=foo ADMIN_PASSWORD=bar cargo run --release --example umami -- -H http://umami.example.com -v -u 50
```
