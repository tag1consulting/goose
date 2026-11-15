//! The regex functions of `common.rs` as they were before their regexes were compiled
//! once, kept only for the tests in `common.rs`, which check that the current functions
//! return exactly what these return. Everything below the imports is copied unchanged.

use crate::common::AssetKind;
use regex::Regex;

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

/// Use regular expression to get the value of a named form element.
pub fn get_form_value(html: &str, name: &str) -> Option<String> {
    let re = Regex::new(&format!(r#"name="{name}" value=['"](.*?)['"]"#)).unwrap();
    re.captures(html).map(|value| value[1].to_string())
}
