use goose::goose::GooseResponse;
use goose::prelude::*;

use rand::prelude::IteratorRandom;
use rand::seq::SliceRandom;
use regex::Regex;

/// The Umami website defines three content types.
pub enum ContentType {
    Article,
    BasicPage,
    Recipe,
}

/// Details tracked about individual nodes used to run load test and validate
/// that pages are being correctly loaded.
pub struct Node<'a> {
    pub nid: u32,
    pub url_en: &'a str,
    pub url_es: &'a str,
    pub title_en: &'a str,
    pub title_es: &'a str,
}

/// Vocabulary term details.
pub struct Term<'a> {
    pub url_en: &'a str,
    pub url_es: &'a str,
    pub title_en: &'a str,
    pub title_es: &'a str,
}

/// Returns a vector of all nodes of a specified content type.
pub fn get_nodes(content_type: &ContentType) -> Vec<Node> {
    match content_type {
        ContentType::Article => {
            vec![
                Node {
                    nid: 11,
                    url_en: "/en/articles/give-it-a-go-and-grow-your-own-herbs",
                    url_es: "/es/articles/prueba-y-cultiva-tus-propias-hierbas",
                    title_en: "Give it a go and grow your own herbs",
                    title_es: "Prueba y cultiva tus propias hierbas",
                },
                Node {
                    nid: 12,
                    url_en: "/en/articles/dairy-free-and-delicious-milk-chocolate",
                    url_es: "/es/articles/delicioso-chocolate-sin-lactosa",
                    title_en: "Dairy-free and delicious milk chocolate",
                    title_es: "Delicioso chocolate sin lactosa",
                },
                Node {
                    nid: 13,
                    url_en: "/en/articles/the-real-deal-for-supermarket-savvy-shopping",
                    url_es: "/es/articles/el-verdadeo-negocio-para-comprar-en-el-supermercado",
                    title_en: "The real deal for supermarket savvy shopping",
                    title_es: "El verdadero negocio para comprar en el supermercado",
                },
                Node {
                    nid: 14,
                    url_en: "/en/articles/the-umami-guide-to-our-favourite-mushrooms",
                    url_es: "/es/articles/guia-umami-de-nuestras-setas-preferidas",
                    title_en: "The Umami guide to our favorite mushrooms",
                    title_es: "Guía Umami de nuestras setas preferidas",
                },
                Node {
                    nid: 15,
                    url_en: "/en/articles/lets-hear-it-for-carrots",
                    url_es: "/es/articles/un-aplauso-para-las-zanahorias",
                    title_en: "Let&#039;s hear it for carrots",
                    title_es: "Un aplauso para las zanahorias",
                },
                Node {
                    nid: 16,
                    url_en: "/en/articles/baking-mishaps-our-troubleshooting-tips",
                    url_es: "es/articles/percances-al-hornear-nuestros-consejos-para-solucionar-problemas",
                    title_en: "Baking mishaps - our troubleshooting tips",
                    title_es: "Percances al hornear - nuestros consejos para solucionar los problemas",
                },
                Node {
                    nid: 17,
                    url_en: "/en/articles/skip-the-spirits-with-delicious-mocktails",
                    url_es: "/es/articles/salta-los-espiritus-con-deliciosos-cocteles-sin-alcohol",
                    title_en: "Skip the spirits with delicious mocktails",
                    title_es: "Salta los espíritus con deliciosos cócteles sin alcohol",
                },
            ]
        }
        ContentType::BasicPage => {
            vec![Node {
                nid: 19,
                url_en: "/en/about-umami",
                url_es: "/es/acerca-de-umami",
                title_en: "About Umami",
                title_es: "Acerca de Umami",
            }]
        }
        ContentType::Recipe => {
            vec![
                Node {
                    nid: 1,
                    url_en: "/en/recipes/deep-mediterranean-quiche",
                    url_es: "/es/recipes/quiche-mediterráneo-profundo",
                    title_en: "Deep mediterranean quiche",
                    title_es: "Quiche mediterráneo profundo",
                },
                Node {
                    nid: 2,
                    url_en: "/en/recipes/vegan-chocolate-and-nut-brownies",
                    url_es: "/es/recipes/bizcochos-veganos-de-chocolate-y-nueces",
                    title_en: "Vegan chocolate and nut brownies",
                    title_es: "Bizcochos veganos de chocolate y nueces",
                },
                Node {
                    nid: 3,
                    url_en: "/en/recipes/super-easy-vegetarian-pasta-bake",
                    url_es: "/es/recipes/pasta-vegetariana-horno-super-facil",
                    title_en: "Super easy vegetarian pasta bake",
                    title_es: "Pasta vegetariana al horno súper fácil",
                },
                Node {
                    nid: 4,
                    url_en: "/en/recipes/watercress-soup",
                    url_es: "/es/recipes/sopa-de-berro",
                    title_en: "Watercress soup",
                    title_es: "Sopa de berro",
                },
                Node {
                    nid: 5,
                    url_en: "/en/recipes/victoria-sponge-cake",
                    url_es: "/es/recipes/pastel-victoria",
                    title_en: "Victoria sponge cake",
                    title_es: "Pastel Victoria",
                },
                Node {
                    nid: 6,
                    url_en: "/en/recipes/gluten-free-pizza",
                    url_es: "/es/recipes/pizza-sin-gluten",
                    title_en: "Gluten free pizza",
                    title_es: "Pizza sin gluten",
                },
                Node {
                    nid: 7,
                    url_en: "/en/recipes/thai-green-curry",
                    url_es: "/es/recipes/curry-verde-tailandes",
                    title_en: "Thai green curry",
                    title_es: "Curry verde tailandés",
                },
                Node {
                    nid: 8,
                    url_en: "/en/recipes/crema-catalana",
                    url_es: "/es/recipes/crema-catalana",
                    title_en: "Crema catalana",
                    title_es: "Crema catalana",
                },
                Node {
                    nid: 9,
                    url_en: "/en/recipes/fiery-chili-sauce",
                    url_es: "/es/recipes/salsa-de-chile-ardiente",
                    title_en: "Fiery chili sauce",
                    title_es: "Salsa de chile ardiente",
                },
                Node {
                    nid: 10,
                    url_en: "/en/recipes/borscht-with-pork-ribs",
                    url_es: "/es/recipes/borscht-con-costillas-de-cerdo",
                    title_en: "Borscht with pork ribs",
                    title_es: "Borscht con costillas de cerdo",
                },
            ]
        }
    }
}

/// Returns a vector of all taxonomy terms.
pub fn get_terms() -> Vec<Term<'static>> {
    vec![
        Term {
            url_en: "/en/recipe-category/accompaniments",
            url_es: "/es/recipe-category/acompañamientos",
            title_en: "Accompaniments",
            title_es: "Acompañamientos",
        },
        Term {
            url_en: "/en/recipe-category/desserts",
            url_es: "/es/recipe-category/postres",
            title_en: "Desserts",
            title_es: "Postres",
        },
        Term {
            url_en: "/en/recipe-category/main-courses",
            url_es: "/es/recipe-category/platos-principales",
            title_en: "Main courses",
            title_es: "Platos principales",
        },
        Term {
            url_en: "/en/recipe-category/snacks",
            url_es: "/es/recipe-category/tentempiés",
            title_en: "Snacks",
            title_es: "Tentempiés",
        },
        Term {
            url_en: "/en/recipe-category/starters",
            url_es: "/es/recipe-category/entrantes",
            title_en: "Starters",
            title_es: "Entrantes",
        },
        Term {
            url_en: "/en/tags/alcohol-free",
            url_es: "/es/tags/sin-alcohol",
            title_en: "Alcohol free",
            title_es: "Sin alcohol",
        },
        Term {
            url_en: "/en/tags/baked",
            url_es: "/es/tags/horneado",
            title_en: "Baked",
            title_es: "Horneado",
        },
        Term {
            url_en: "/en/tags/baking",
            url_es: "/es/tags/cocción",
            title_en: "Baking",
            title_es: "Cocción",
        },
        Term {
            url_en: "/en/tags/breakfast",
            url_es: "/es/tags/desayuno",
            title_en: "Breakfast",
            title_es: "Desayuno",
        },
        Term {
            url_en: "/en/tags/cake",
            url_es: "/es/tags/pastel",
            title_en: "Cake",
            title_es: "Pastel",
        },
        Term {
            url_en: "/en/tags/carrots",
            url_es: "/es/tags/zanahorias",
            title_en: "Carrots",
            title_es: "Zanahorias",
        },
        Term {
            url_en: "/en/tags/chocolate",
            url_es: "/es/tags/chocolate",
            title_en: "Chocolate",
            title_es: "Chocolate",
        },
        Term {
            url_en: "/en/tags/cocktail-party",
            url_es: "/es/tags/fiesta-de-coctel",
            title_en: "Cocktail party",
            title_es: "Fiesta de coctel",
        },
        Term {
            url_en: "/en/tags/dairy-free",
            url_es: "/es/tags/sin-Lactosa",
            title_en: "Dairy-free",
            title_es: "Sin Lactosa",
        },
        Term {
            url_en: "/en/tags/dessert",
            url_es: "/es/tags/postre",
            title_en: "Dessert",
            title_es: "Postre",
        },
        Term {
            url_en: "/en/tags/dinner-party",
            url_es: "/es/tags/fiesta-de-cena",
            title_en: "Dinner party",
            title_es: "Fiesta de cena",
        },
        Term {
            url_en: "/en/tags/drinks",
            url_es: "/es/tags/bebidas",
            title_en: "Drinks",
            title_es: "Bebidas",
        },
        Term {
            url_en: "/en/tags/egg",
            url_es: "/es/tags/huevo",
            title_en: "Egg",
            title_es: "Huevo",
        },
        Term {
            url_en: "/en/tags/grow-your-own",
            url_es: "/es/tags/cultiva-los-tuyos",
            title_en: "Grow your own",
            title_es: "Cultiva los tuyos",
        },
        Term {
            url_en: "/en/tags/healthy",
            url_es: "/es/tags/saludable",
            title_en: "Healthy",
            title_es: "Saludable",
        },
        Term {
            url_en: "/en/tags/herbs",
            url_es: "/es/tags/hierbas",
            title_en: "Herbs",
            title_es: "Hierbas",
        },
        Term {
            url_en: "/en/tags/learn-to-cook",
            url_es: "/es/tags/aprender-a-cocinar",
            title_en: "Learn to cook",
            title_es: "Aprender a cocinar",
        },
        Term {
            url_en: "/en/tags/mushrooms",
            url_es: "/es/tags/champiñones",
            title_en: "Mushrooms",
            title_es: "Champiñones",
        },
        Term {
            url_en: "/en/tags/oats",
            url_es: "/es/tags/avena",
            title_en: "Oats",
            title_es: "Avena",
        },
        Term {
            url_en: "/en/tags/party",
            url_es: "/es/tags/fiesta",
            title_en: "Party",
            title_es: "Fiesta",
        },
        Term {
            url_en: "/en/tags/pasta",
            url_es: "/es/tags/pastas",
            title_en: "Pasta",
            title_es: "Pastas",
        },
        Term {
            url_en: "/en/tags/pastry",
            url_es: "/es/tags/repostería",
            title_en: "Pastry",
            title_es: "Repostería",
        },
        Term {
            url_en: "/en/tags/seasonal",
            url_es: "/es/tags/estacional",
            title_en: "Seasonal",
            title_es: "Estacional",
        },
        Term {
            url_en: "/en/tags/shopping",
            url_es: "/es/tags/compras",
            title_en: "Shopping",
            title_es: "Compras",
        },
        Term {
            url_en: "/en/tags/soup",
            url_es: "/es/tags/sopa",
            title_en: "Soup",
            title_es: "Sopa",
        },
        Term {
            url_en: "/en/tags/supermarkets",
            url_es: "/es/tags/supermercados",
            title_en: "Supermarkets",
            title_es: "Supermercados",
        },
        Term {
            url_en: "/en/tags/vegan",
            url_es: "/es/tags/vegano",
            title_en: "Vegan",
            title_es: "Vegano",
        },
        Term {
            url_en: "/en/tags/vegetarian",
            url_es: "/es/tags/vegetariano",
            title_en: "Vegetarian",
            title_es: "Vegetariano",
        },
    ]
}

/// Return a vector of random words taken from node titles in the specified
/// language.
pub fn random_words(count: usize, english: bool) -> Vec<String> {
    let mut random_words: Vec<String> = Vec::new();

    for _ in 0..count {
        // Randomly select a content type, favoring articles and recipes.
        let content_types = [
            ContentType::Article,
            ContentType::Article,
            ContentType::Article,
            ContentType::BasicPage,
            ContentType::Recipe,
            ContentType::Recipe,
            ContentType::Recipe,
        ];
        let content_type = content_types.choose(&mut rand::thread_rng());
        // Then randomly select a node of this content type.
        let nodes = get_nodes(content_type.unwrap());
        let page = nodes.choose(&mut rand::thread_rng());
        // Randomly select a word from the title to use in our search.
        let title = if english {
            page.unwrap().title_en
        } else {
            page.unwrap().title_es
        };
        let words = title.split_whitespace();
        let word = words.choose(&mut rand::thread_rng()).unwrap();
        // Remove ' to avoid encoding/decoding issues when validating later.
        let cleaned_word = word.replace("&#039;", "");
        random_words.push(cleaned_word.to_string());
    }

    // Return a vector of words in the specified language.
    random_words
}

/// A valid title on this website starts with "<title>foo", where "foo" is the expected
/// title text. Returns true if the expected title is set, otherwise returns false.
pub fn valid_title(html: &str, title: &str) -> bool {
    html.contains(&("<title>".to_string() + title))
}

/// Finds all local static elements on the page and loads them asynchronously.
/// This default profile only has local assets, so we can use simple patterns.
pub async fn load_static_elements(user: &mut GooseUser, html: &str) {
    // Use a regular expression to find all src=<foo> in the HTML, where foo
    // is the URL to image and js assets.
    // @TODO: parse HTML5 srcset= also
    let image = Regex::new(r#"src="(.*?)""#).unwrap();
    let mut urls = Vec::new();
    for url in image.captures_iter(html) {
        if url[1].starts_with("/sites") || url[1].starts_with("/core") {
            urls.push(url[1].to_string());
        }
    }

    // Use a regular expression to find all href=<foo> in the HTML, where foo
    // is the URL to css assets.
    let css = Regex::new(r#"href="(/sites/default/files/css/.*?)""#).unwrap();
    for url in css.captures_iter(html) {
        urls.push(url[1].to_string());
    }

    // Load all the static assets found on the page.
    for asset in &urls {
        let _ = user.get_named(asset, "static asset").await;
    }
}

/// Validate the HTML response, confirming the expected title was returned, then load
/// all static assets found on the page.
pub async fn validate_and_load_static_assets(
    user: &mut GooseUser,
    mut goose: GooseResponse,
    title: &str,
) -> TransactionResult {
    match goose.response {
        Ok(response) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &response.headers().clone();
            match response.text().await {
                Ok(html) => {
                    if !valid_title(&html, title) {
                        return user.set_failure(
                            &format!("{}: title not found: {}", goose.request.raw.url, title),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }

                    load_static_elements(user, &html).await;
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

    Ok(())
}

/// Use regular expression to get the value of a named form element.
pub fn get_form_value(html: &str, name: &str) -> Option<String> {
    let re = Regex::new(&format!(r#"name="{}" value=['"](.*?)['"]"#, name)).unwrap();
    re.captures(html).map(|value| value[1].to_string())
}

/// Anonymously load the contact form and POST feedback. The english boolean flag indicates
/// whether to load the English form or the Spanish form.
pub async fn anonymous_contact_form(user: &mut GooseUser, english: bool) -> TransactionResult {
    let contact_form_url = if english {
        "/en/contact"
    } else {
        "/es/contact"
    };
    let mut goose = user.get(contact_form_url).await?;

    // We can't invoke common::validate_and_load_static_assets as while it's important
    // to validate the page and load static elements, we then need to extra form elements
    // from the HTML of the page. So we duplicate some of the logic, enhancing it for form
    // processing.
    let contact_form;
    match goose.response {
        Ok(response) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &response.headers().clone();
            match response.text().await {
                Ok(html) => {
                    // Be sure we've properly loaded the Contact form.
                    let title = if english {
                        "Website feedback"
                    } else {
                        "Comentarios sobre el sitio web"
                    };
                    if !valid_title(&html, title) {
                        return user.set_failure(
                            &format!("{}: title not found: {}", goose.request.raw.url, title),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }

                    // Load all static elements on the page, as a real user would.
                    load_static_elements(user, &html).await;

                    // Scrape the HTML to get the values needed in order to POST to the
                    // contact form.
                    let form_build_id = get_form_value(&html, "form_build_id");
                    if form_build_id.is_none() {
                        return user.set_failure(
                            &format!("{}: no form_build_id on page", goose.request.raw.url),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }

                    // Build contact form parameters.
                    let name = random_words(2, english).join(" ");
                    let email = format!("{}@example.com", random_words(1, english).pop().unwrap());
                    let subject = random_words(8, english).join(" ");
                    let message = random_words(12, english).join(" ");
                    let params = [
                        ("name", name.as_str()),
                        ("mail", email.as_str()),
                        ("subject[0][value]", subject.as_str()),
                        ("message[0][value]", message.as_str()),
                        ("form_build_id", &form_build_id.unwrap()),
                        ("form_id", "contact_message_feedback_form"),
                        ("op", "Send+message"),
                    ];
                    contact_form = user.post_form(contact_form_url, &params).await?;
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

    // Drupal 9 throttles how many times an IP address can submit the contact form, so we
    // need special handling.
    match contact_form.response {
        Ok(response) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &response.headers().clone();
            match response.text().await {
                Ok(html) => {
                    // Drupal 9 will throttle how many times our IP address can actually
                    // submit the contact form. We can detect this, but it happens a lot
                    // so there's nothing useful to do.
                    let error_text = if english {
                        "You cannot send more than"
                    } else {
                        "No le está permitido enviar más"
                    };
                    if html.contains(error_text) {
                        // The contact form was throttled, safely ignore this.
                    }

                    // Either way, a "real" user would still load all static elements on
                    // the returned page.
                    load_static_elements(user, &html).await;
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

    Ok(())
}

/// Load the search page and perform a search using one word from one of the node titles
/// on the site.
pub async fn search(user: &mut GooseUser, english: bool) -> TransactionResult {
    let search_form_url = if english {
        "/en/search/node"
    } else {
        "/es/search/node"
    };
    let mut goose = user.get(search_form_url).await?;

    // We can't invoke common::validate_and_load_static_assets as while it's important
    // to validate the page and load static elements, we then need to extra form elements
    // from the HTML of the page. So we duplicate some of the logic, enhancing it for form
    // processing.
    let search_phrase;
    let mut search_form;
    match goose.response {
        Ok(response) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &response.headers().clone();
            match response.text().await {
                Ok(html) => {
                    // Be sure we've properly loaded the Search page.
                    let title = if english { "Search" } else { "Buscar" };
                    if !valid_title(&html, title) {
                        return user.set_failure(
                            &format!("{}: title not found: {}", goose.request.raw.url, title),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }

                    // Load all static elements on the page, as a real user would.
                    load_static_elements(user, &html).await;

                    // Scrape the HTML to get the values needed in order to POST to the
                    // search form.
                    let form_build_id = get_form_value(&html, "form_build_id");
                    if form_build_id.is_none() {
                        return user.set_failure(
                            &format!("{}: no form_build_id on page", goose.request.raw.url),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }

                    // Build a random three-word phrase, save to validate the results later.
                    let search_words = random_words(3, english);
                    search_phrase = search_words.join(" ");

                    // Build search form with random word from title.
                    let params = [
                        ("keys", search_phrase.as_str()),
                        ("form_build_id", &form_build_id.unwrap()),
                        ("form_id", "search_form"),
                        ("op", "Search"),
                    ];
                    search_form = user.post_form(search_form_url, &params).await?;

                    // A successful search is redirected.
                    if !search_form.request.redirected {
                        return user.set_failure(
                            &format!("{}: search didn't redirect", search_form.request.final_url),
                            &mut search_form.request,
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

    match search_form.response {
        Ok(response) => {
            // Copy the headers so we have them for logging if there are errors.
            let headers = &response.headers().clone();
            match response.text().await {
                Ok(html) => {
                    if !html.contains(&search_phrase) {
                        return user.set_failure(
                            &format!(
                                "{}: search terms ({}) not on page",
                                goose.request.raw.url, &search_phrase
                            ),
                            &mut goose.request,
                            Some(headers),
                            Some(&html),
                        );
                    }
                    load_static_elements(user, &html).await;

                    // @TODO: get all href="" inside class="search-result__title" and load random node
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

    Ok(())
}
