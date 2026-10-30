use crate::SearchError;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::{Host, Url};

pub(crate) fn uuid(value: &str) -> Result<(),SearchError> {
    uuid::Uuid::parse_str(value).map(|_| ()).map_err(|_| SearchError::new(400,"invalid_request_or_service_id"))
}
pub(crate) fn public_url(raw: &str) -> Result<Url,SearchError> {
    let bad = || SearchError::new(400,"public_http_url_required");
    if raw.len() > 8192 || raw.chars().any(|c| c.is_control() || c.is_whitespace()) { return Err(bad()); }
    let url = Url::parse(raw).map_err(|_| bad())?;
    if !matches!(url.scheme(),"http"|"https") || !url.username().is_empty() || url.password().is_some() || url.port() == Some(0) { return Err(bad()); }
    match url.host().ok_or_else(bad)? {
        Host::Ipv4(ip) if public_ip(IpAddr::V4(ip)) => {},
        Host::Ipv6(ip) if public_ip(IpAddr::V6(ip)) => {},
        Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            if !name.contains('.') || ["localhost","local","internal","lan","home","onion","invalid","test","arpa"].iter().any(|suffix| name == *suffix || name.ends_with(&format!(".{suffix}"))) { return Err(bad()); }
        },
        _ => return Err(bad()),
    }
    Ok(url)
}
pub(crate) fn public_ip(ip: IpAddr) -> bool {
    match ip { IpAddr::V4(ip) => public_v4(ip), IpAddr::V6(ip) => public_v6(ip) }
}
fn public_v4(ip: Ipv4Addr) -> bool {
    let [a,b,c,_] = ip.octets();
    !(a == 0 || a == 10 || a == 127 || a >= 224 || (a == 100 && (64..=127).contains(&b)) || (a == 169 && b == 254) || (a == 172 && (16..=31).contains(&b)) || (a == 192 && (b == 168 || (b == 0 && (c == 0 || c == 2)))) || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100))) || (a == 203 && b == 0 && c == 113))
}
fn public_v6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    // Only global unicast, excluding documentation and transition/tunneling ranges.
    segments[0] & 0xe000 == 0x2000 && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8)) && segments[0] != 0x2002 && !(segments[0] == 0x3fff && segments[1] < 0x1000)
}
pub(crate) fn bounded(text: &str, max: usize) -> String { text.chars().take(max).collect() }
