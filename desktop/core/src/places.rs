//! A reply's place cards, read off its places block (companion PROTOCOL.md, "Blocks under a
//! reply") as the macOS app's `ChatPlaceFacts` reads them, without its map: what a card says,
//! the website it may open, and what Open in Maps launches. Nothing here opens anything.
//!
//! Every place is already a link in the reply's words; a card only adds to them.

use crate::chat_wire::protocol::{Block, Distance, Place, Rating};

/// The most places a block holds (the contract's bound); a longer one shows its first ten.
pub const MAX_PLACES: usize = 10;

/// One place as its card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub name: String,
    /// None, one or two lines: the rating, price and kind; then today's hours (or the street)
    /// and the distance. A line with nothing to say is left out.
    pub lines: Vec<String>,
    /// What a screen reader hears: the name, then every part.
    pub spoken: String,
    /// The place in a maps app: its coordinates, labelled with its name.
    pub maps: String,
    /// The place on OpenStreetMap's web map, for a computer with no maps app.
    pub web_map: String,
    /// The first of the place's own site and its source page that is a web address.
    pub website: Option<String>,
    /// As the source wrote it, for Copy address.
    pub address: Option<String>,
}

/// What Open in Maps launches: `first`, and `fallback` when that launch fails for any reason
/// but the person turning it down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapsLaunch {
    pub first: String,
    pub fallback: Option<String>,
}

/// The cards of a reply's places, in the order its words first link them.
pub fn cards(blocks: &[Block]) -> Vec<Card> {
    blocks
        .iter()
        .flat_map(|block| match block {
            Block::Places(places) => places.iter(),
        })
        .take(MAX_PLACES)
        .map(card)
        .collect()
}

pub fn card(place: &Place) -> Card {
    let sort = place.categories.first().and_then(|category| kind(category));
    let summary: Vec<String> = [
        rating(place.rating.as_ref()),
        place.price_range.clone(),
        sort,
    ]
    .into_iter()
    .filter_map(said)
    .collect();
    let visit: Vec<String> = [visit_when_where(place), distance(place.distance.as_ref())]
        .into_iter()
        .filter_map(said)
        .collect();
    let spoken = std::iter::once(place.name.as_str())
        .chain(summary.iter().chain(&visit).map(String::as_str))
        .collect::<Vec<_>>()
        .join(", ");
    let lines = [summary, visit]
        .into_iter()
        .filter(|parts| !parts.is_empty())
        .map(|parts| parts.join(" · "))
        .collect();
    Card {
        name: place.name.clone(),
        lines,
        spoken,
        maps: geo_uri(place),
        web_map: osm_link(place),
        website: website(place),
        address: place.address.clone(),
    }
}

/// With an app for `geo:` on this computer the place opens there, and on the web map only when
/// that launch fails; with none, on the web map at once, so nobody meets a chooser that offers
/// no app.
pub fn maps_launch(card: &Card, geo_handler: bool) -> MapsLaunch {
    if geo_handler {
        return MapsLaunch {
            first: card.maps.clone(),
            fallback: Some(card.web_map.clone()),
        };
    }
    MapsLaunch {
        first: card.web_map.clone(),
        fallback: None,
    }
}

/// A part as the card says it: trimmed, and nothing when only spaces are left, so no line of
/// spaces is drawn and a screen reader hears no empty part.
fn said(part: Option<String>) -> Option<String> {
    let part = part?;
    let trimmed = part.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// `4.6 (321)`: the value to one decimal, the count whole. A count without a value says nothing.
fn rating(rating: Option<&Rating>) -> Option<String> {
    let rating = rating?;
    let value = format!("{:.1}", rating.value?);
    Some(match rating.count {
        Some(count) => format!("{value} ({})", grouped(&format!("{count:.0}"))),
        None => value,
    })
}

/// A source's category in sentence case: `coffee_shop` is `Coffee shop`.
fn kind(category: &str) -> Option<String> {
    let words = category.replace('_', " ").to_lowercase();
    let mut chars = words.trim().chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

/// Today's hours exactly as the source wrote them, never worked out into open or closed; where
/// it gave none, the street: the address before its first comma.
fn visit_when_where(place: &Place) -> Option<String> {
    let hours = place
        .opening_hours
        .as_ref()
        .and_then(|hours| hours.current_day.as_ref());
    if let Some(hours) = hours.filter(|hours| !hours.trim().is_empty()) {
        return Some(hours.clone());
    }
    let address = place.address.as_deref()?;
    let street = address.split(',').next().unwrap_or_default().trim();
    (!street.is_empty()).then(|| street.to_owned())
}

/// `0.4 km`: the value to at most two decimals, then its units as given.
fn distance(distance: Option<&Distance>) -> Option<String> {
    let distance = distance?;
    let fixed = format!("{:.2}", distance.value);
    let shown = fixed.trim_end_matches('0').trim_end_matches('.');
    Some(format!("{} {}", grouped(shown), distance.units.trim()))
}

/// `1234.5` as `1,234.5`: the whole part in groups of three.
fn grouped(number: &str) -> String {
    let (sign, unsigned) = match number.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", number),
    };
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let mut out = String::with_capacity(number.len() + whole.len() / 3);
    out.push_str(sign);
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

/// The first of the place's own site and its source page that is a web address. Open website
/// hands it to the desktop, which would open any other scheme in the app that owns it, so a
/// record can never open anything but a browser (Mac 7066656).
fn website(place: &Place) -> Option<String> {
    [&place.canonical_url, &place.source_url]
        .into_iter()
        .flatten()
        .find(|url| is_web_address(url))
        .cloned()
}

/// Whether a link is one a place in `blocks` stands for, which its card opens or offers, so the
/// strip draws no second card for it (Mac fbd2880): a place's page at its source or its own
/// website, as the engine matches them (scheme and host in any case, without the fragment and a
/// trailing slash), or a page inside that website: its host with or without `www.`, and its path
/// or one under it, so a site many places share gives each only its own pages.
pub fn opened_by(address: &str, blocks: &[Block]) -> bool {
    let Some(link) = WebAddress::parse(address) else {
        return false;
    };
    blocks
        .iter()
        .flat_map(|block| match block {
            Block::Places(places) => places.iter(),
        })
        .any(|place| stands_for(place, &link))
}

fn stands_for(place: &Place, link: &WebAddress) -> bool {
    let page = link.page();
    let mut pages = [&place.canonical_url, &place.source_url]
        .into_iter()
        .flatten()
        .filter_map(|url| WebAddress::parse(url));
    if pages.any(|known| known.page() == page) {
        return true;
    }
    let site = place.canonical_url.as_deref().and_then(WebAddress::parse);
    site.is_some_and(|site| site.holds(link))
}

/// A web address in its parts as written. Only what `is_web_address` accepts parses.
pub(crate) struct WebAddress<'a> {
    scheme: &'a str,
    /// The host with any user and port.
    authority: &'a str,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

impl<'a> WebAddress<'a> {
    pub(crate) fn parse(url: &'a str) -> Option<WebAddress<'a>> {
        if !is_web_address(url) {
            return None;
        }
        let (scheme, rest) = url.split_once("://")?;
        let (rest, fragment) = match rest.split_once('#') {
            Some((rest, fragment)) => (rest, Some(fragment)),
            None => (rest, None),
        };
        let (rest, query) = match rest.split_once('?') {
            Some((rest, query)) => (rest, Some(query)),
            None => (rest, None),
        };
        let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        Some(WebAddress {
            scheme,
            authority,
            path,
            query,
            fragment,
        })
    }

    /// The host alone, lowercased: no user, no port.
    pub(crate) fn host(&self) -> String {
        let host = self.authority.rsplit('@').next().unwrap_or_default();
        let host = match host.strip_prefix('[') {
            Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
            None => host.split(':').next().unwrap_or_default(),
        };
        host.to_lowercase()
    }

    /// Two addresses are one destination when this is equal: the scheme and host lowercased,
    /// the path, query and fragment as written (Mac `ChatDestination`).
    pub(crate) fn destination(&self) -> String {
        let mut key = format!("{}{}", self.lowered(), self.path);
        if let Some(query) = self.query {
            key.push('?');
            key.push_str(query);
        }
        if let Some(fragment) = self.fragment {
            key.push('#');
            key.push_str(fragment);
        }
        key
    }

    /// The page a place's link matches (Mac `ChatPlaceLink`): no fragment, no trailing slash.
    fn page(&self) -> String {
        let mut key = format!("{}{}", self.lowered(), self.path.trim_end_matches('/'));
        if let Some(query) = self.query {
            key.push('?');
            key.push_str(query);
        }
        key
    }

    /// Whether `link` is this website or a page under it (Mac `ChatPlaceSite`).
    fn holds(&self, link: &WebAddress) -> bool {
        let bare = |address: &WebAddress| {
            let host = address.host();
            host.strip_prefix("www.").map(str::to_owned).unwrap_or(host)
        };
        if bare(self) != bare(link) {
            return false;
        }
        let site = self.path.trim_end_matches('/');
        let page = link.path.trim_end_matches('/');
        page == site || page.starts_with(&format!("{site}/"))
    }

    /// `scheme://authority`, lowercased.
    fn lowered(&self) -> String {
        format!(
            "{}://{}",
            self.scheme.to_ascii_lowercase(),
            self.authority.to_lowercase()
        )
    }
}

/// `http` or `https` in any case, then `://` and a host, with no space or control anywhere.
pub fn is_web_address(url: &str) -> bool {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    let web = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    web && !host.is_empty()
}

/// `geo:` with the place's coordinates and its name as the label (the `?q=lat,lon(label)` form
/// GNOME Maps reads). The name is percent-encoded but for RFC 3986's unreserved characters, so
/// no `(`, `)`, `,`, `&` or `#` in it can end the label early.
fn geo_uri(place: &Place) -> String {
    let (lat, lon) = (place.location.latitude, place.location.longitude);
    let name = encoded(&place.name);
    format!("geo:{lat},{lon}?q={lat},{lon}({name})")
}

/// The place on OpenStreetMap, marked and at street level. The web map takes no name.
fn osm_link(place: &Place) -> String {
    let (lat, lon) = (place.location.latitude, place.location.longitude);
    format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=17/{lat}/{lon}")
}

fn encoded(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        let kept = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if kept {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
