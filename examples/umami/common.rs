use goose::goose::GooseResponse;
use goose::prelude::*;

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

/// Returns a vector of all nodes of a specified content type. The URLs and titles
/// were checked against a stock Drupal 11.4 Umami site.
pub fn get_nodes(content_type: &ContentType) -> Vec<Node<'_>> {
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
                    url_en: "/en/articles/the-umami-guide-to-our-favorite-mushrooms",
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
                    url_es: "/es/articles/percances-al-hornear-nuestros-consejos-para-solucionar-problemas",
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

/// Returns a vector of all taxonomy terms. The URLs and titles were checked against
/// a stock Drupal 11.4 Umami site.
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
            url_es: "/es/tags/sin-lactosa",
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

/// A valid title on this website starts with "<title>foo", where "foo" is the expected
/// title text. Returns true if the expected title is set, otherwise returns false.
pub fn valid_title(html: &str, title: &str) -> bool {
    html.contains(&("<title>".to_string() + title))
}

/// The kinds of static asset a page names, each requested under its own name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AssetKind {
    Css,
    Js,
    Image,
}

impl AssetKind {
    /// The request name the asset is loaded under.
    pub fn request_name(self) -> &'static str {
        match self {
            AssetKind::Css => "static css",
            AssetKind::Js => "static js",
            AssetKind::Image => "static image",
        }
    }
}

/// Decodes the HTML entities Drupal writes in attribute values, such as the `&amp;`
/// between the query parameters of an aggregated CSS or JS URL.
pub fn decode_entities(value: &str) -> String {
    let entity = Regex::new(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|[a-zA-Z]+);").unwrap();
    entity
        .replace_all(value, |caps: &regex::Captures| {
            let name = &caps[1];
            let decoded = match name {
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                _ if name.starts_with("#x") || name.starts_with("#X") => {
                    u32::from_str_radix(&name[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                }
                _ if name.starts_with('#') => name[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            decoded.map_or_else(|| caps[0].to_string(), |c| c.to_string())
        })
        .into_owned()
}

/// Returns the local static assets a browser loads for this page, in page order and
/// each once, with HTML entities decoded: stylesheets from `<link rel="stylesheet">`,
/// scripts from `<script src>` and images from `<img src>`. Drupal 11.4 serves CSS
/// and JS from `/sites/default/files/css/` and `/sites/default/files/js/` with
/// aggregation on, and from `/core/` and the theme's folder with it off; any local
/// path is taken, so both work.
pub fn static_asset_urls(html: &str) -> Vec<(AssetKind, String)> {
    // @TODO: parse HTML5 srcset= also
    let tag = Regex::new(r"(?is)<(link|script|img)\b([^>]*)>").unwrap();
    let attribute = Regex::new(r#"(?is)\s([a-z-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap();
    let mut assets: Vec<(AssetKind, String)> = Vec::new();
    for found in tag.captures_iter(html) {
        let mut rel = None;
        let mut url = None;
        for attr in attribute.captures_iter(&found[2]) {
            let value = attr.get(2).or_else(|| attr.get(3)).unwrap().as_str();
            match attr[1].to_ascii_lowercase().as_str() {
                "rel" => rel = Some(value.to_ascii_lowercase()),
                "href" | "src" => url = Some(decode_entities(value)),
                _ => {}
            }
        }
        let kind = match found[1].to_ascii_lowercase().as_str() {
            "link" if rel.as_deref() == Some("stylesheet") => AssetKind::Css,
            "script" => AssetKind::Js,
            "img" => AssetKind::Image,
            _ => continue,
        };
        // Only local assets: a path on this site, not another host's.
        if let Some(url) = url.filter(|u| u.starts_with('/') && !u.starts_with("//")) {
            if !assets.iter().any(|(_, seen)| *seen == url) {
                assets.push((kind, url));
            }
        }
    }
    assets
}

/// Finds all local static elements on the page and loads them, as a browser would.
pub async fn load_static_elements(user: &mut GooseUser, html: &str) {
    for (kind, asset) in static_asset_urls(html) {
        let _ = user.get_named(&asset, kind.request_name()).await;
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
    let re = Regex::new(&format!(r#"name="{name}" value=['"](.*?)['"]"#)).unwrap();
    re.captures(html).map(|value| value[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The English front page of a stock Drupal 11.4 Umami site, saved with CSS and JS
    // aggregation on and then off.
    const AGGREGATED: &str = include_str!("fixtures/front-aggregated.html");
    const UNAGGREGATED: &str = include_str!("fixtures/front-unaggregated.html");

    fn urls(assets: &[(AssetKind, String)], kind: AssetKind) -> Vec<&str> {
        assets
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, url)| url.as_str())
            .collect()
    }

    /// Counts the tags that name a local asset of this kind, independently of the parser.
    fn tag_count(html: &str, pattern: &str) -> usize {
        Regex::new(pattern).unwrap().find_iter(html).count()
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            decode_entities("/a.css?delta=0&amp;language=en&#38;theme=umami&#x26;x=&lt;&gt;"),
            "/a.css?delta=0&language=en&theme=umami&x=<>"
        );
        // An unknown entity is left as it is.
        assert_eq!(decode_entities("a&bogus;b"), "a&bogus;b");
    }

    #[test]
    fn aggregated_assets_are_found_and_decoded() {
        let assets = static_asset_urls(AGGREGATED);
        let css = urls(&assets, AssetKind::Css);
        let js = urls(&assets, AssetKind::Js);
        assert_eq!(
            css.len(),
            tag_count(AGGREGATED, r#"<link rel="stylesheet"[^>]*href="/"#)
        );
        assert_eq!(js.len(), tag_count(AGGREGATED, r#"<script[^>]*src="/"#));
        assert!(!css.is_empty() && !js.is_empty());
        for url in css.iter().chain(js.iter()) {
            assert!(!url.contains("&amp;"), "not decoded: {}", url);
            assert!(url.contains("&language=en&theme=umami&include="), "{}", url);
        }
        assert!(css
            .iter()
            .all(|url| url.starts_with("/sites/default/files/css/css_")));
        assert!(js
            .iter()
            .all(|url| url.starts_with("/sites/default/files/js/js_")));
    }

    #[test]
    fn unaggregated_assets_are_found() {
        let assets = static_asset_urls(UNAGGREGATED);
        let css = urls(&assets, AssetKind::Css);
        let js = urls(&assets, AssetKind::Js);
        assert_eq!(
            css.len(),
            tag_count(UNAGGREGATED, r#"<link rel="stylesheet"[^>]*href="/"#)
        );
        assert_eq!(js.len(), tag_count(UNAGGREGATED, r#"<script[^>]*src="/"#));
        assert!(!css.is_empty() && !js.is_empty());
        assert!(css
            .iter()
            .any(|url| url.starts_with("/core/modules/system/css/components/align.module.css?")));
        assert!(css
            .iter()
            .any(|url| url.starts_with("/core/profiles/demo_umami/themes/umami/css/")));
        assert!(js
            .iter()
            .any(|url| url.starts_with("/core/misc/drupal.js?")));
        for url in css.iter().chain(js.iter()) {
            assert!(!url.contains("&amp;"), "not decoded: {}", url);
        }
    }

    #[test]
    fn images_are_found_on_both_pages() {
        for html in [AGGREGATED, UNAGGREGATED] {
            let assets = static_asset_urls(html);
            let images = urls(&assets, AssetKind::Image);
            assert!(images.contains(&"/core/profiles/demo_umami/themes/umami/logo.svg"));
            assert!(images
                .iter()
                .any(|url| url.starts_with("/sites/default/files/styles/")));
        }
    }
}
