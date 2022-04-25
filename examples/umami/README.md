# Overview

This is a load test for Drupal's Umami Profile, which is included as part of Drupal 9 core. In order to use this load test, you must first install the Umami Profile as documented here:
https://www.drupal.org/docs/umami-drupal-demonstration-installation-profile

The load test was developed using a locally hosted Drupal 9 install hosted in a DDEV container:
https://www.ddev.com/

By default it will try and load pages from https://drupal-9.ddev.site/. Use the `--host` flag to specify a different domain to load test.

## Load Test Implementation

The load test is split into the following files:
 - `main.rs`: This file contains the main() function and defines the actual load test;
 - `common.rs`: This file contains helper functions used by the transaction functions;
 - `english.rs`: This file contains all transaction functions loading pages in English;
 - `spanish.rs`: This file contains all transaction functions loading pages in Spanish;
 - `admin.rs`: This file contains all transaction functions specific to simulating an admin user.

## Load Test Features

The load test defines the following users:
 - Anonymous English user: this user performs all transactions in English, it has a weight of 40, and randomly pauses for 0 to 3 seconds after each transaction;
 - Anonymous Spanish user: this user performs all transactions in Spanish, it has a weight of 9, and randomly pauses for 0 to 3 seconds after each transaction;
 - Admin user: this user logs into the website, it has a weight of 1, and randomly pauses for 3 to 10 seconds after each transaction.

Due to user weighting, the load test should simulate at least 50 users when it runs. If you simulate 100 users (with the `-u 100` run time option) then 80 anonymous English users, 18 anonymous Spanish users, and 2 admin users will be simulated.

Each anonymous load test user runs the following transactions in their own language, and also loads all static elements on any pages loaded:
 - Loads the front page;
 - Loads a "basic page";
 - Loads the article listing page;
 - Loads an "article";
 - Loads the recipe listing page;
 - Loads a "recipe";
 - Loads a random node by nid;
 - Loads the term listing page filtered by a random term;
 - Performs a search using a random word from a random node's title;
 - Submits website feedback through the contact form.

Each admin load test user logs in one time in English, and then runs the following transactions and also loads all static elements on any pages loaded:
 - Loads the front page;
 - Loads the article listing page;
 - Loads an "article", edits (not making any actual changes), and saves it (flushing all caches).

 ## Configuring The Admin User

 The load test needs to know what username and password to use to log in. By default it will attempt to log in with the username `admin` and the password `P@ssw0rd1234`. However, you can use the ADMIN_USERNAME and/or ADMIN_PASSWORD environment variables to log in with different values. In the following example, the load test will attempt to log in with the username `foo` and the password `bar`:

 ```
 ADMIN_USERNAME=foo ADMIN_PASSWORD=bar cargo run --release --example umami -- -H https://drupal-9.ddev.site -v -u150
 ```
