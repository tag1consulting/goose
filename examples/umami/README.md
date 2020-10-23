# Overview

This is a load test for Drupal's Umami Profile, which is included as part of Drupal 9 core. In order to use this load test, you must first install the Umami Profile as documented here:
https://www.drupal.org/docs/umami-drupal-demonstration-installation-profile

The load test was developed using a locally hosted Drupal 9 install hosted in a DDEV container:
https://www.ddev.com/

By default it will try and load pages from https://drupal-9.0.7.ddev.site/.

## Load Test Implementation

The load test is split into the following files:
 - `main.rs`: This file contains the main() function and defines the actual load test;
 - `common.rs`: This file contains helper functions used by the task functions;
 - `english.rs`: This files contains all task functions loading pages in English;
 - `spanish.rs`: This files contains all task functions loading pages in Spanish.

## Load Test Features

The load test defines the following users:
 - Anonymous English user: this user performs all tasks in English, it runs 3 times as often as the Spanish user;
 - Anonymous Spanish user: this user performs all tasks in Spanish, it runs 1/3 as often as the English user.

Each load test user runs the following tasks in their own language, and also loads all static elements on any pages loaded:
 - Loads the front page;
 - Loads a "basic page";
 - Loads the article listing page;
 - Loads an "article";
 - Loads the recipe listing page;
 - Loads a "recipe";
 - Loads a random node by nid;
 - Loads the term listing page filtered by a random term;
 - Performs a search using a random word from a random node's title;
 - Submits website feedback through the contact form;
