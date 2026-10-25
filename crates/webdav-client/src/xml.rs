//! Reads a `207 Multi-Status` body. Elements are matched by local name,
//! which is enough for the `DAV:` properties a listing asks for and keeps
//! working when a server picks unusual namespace prefixes.

use std::time::SystemTime;

use quick_xml::{Reader, escape::resolve_xml_entity, events::Event};

/// The properties a listing asks for.
pub(crate) const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:"><D:prop><D:resourcetype/><D:getcontentlength/><D:getlastmodified/><D:getetag/></D:prop></D:propfind>"#;

/// One `response` element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Member {
    /// As written, still percent-encoded.
    pub href: String,
    /// The response-level status, which some servers send instead of
    /// propstats for a member they cannot describe.
    pub status: Option<u16>,
    /// The first failed propstat, except 404 for absent properties.
    pub propstat_error: Option<u16>,
    /// Whether a successful propstat was present.
    pub described: bool,
    pub collection: bool,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub etag: Option<String>,
}

#[derive(Default)]
struct PropStat {
    status: Option<u16>,
    collection: bool,
    size: Option<u64>,
    modified: Option<SystemTime>,
    etag: Option<String>,
}

pub(crate) fn parse(body: &[u8]) -> Result<Vec<Member>, String> {
    let mut reader = Reader::from_reader(body);
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut members = Vec::new();
    let mut member: Option<Member> = None;
    let mut propstat: Option<PropStat> = None;
    let mut text = String::new();
    let mut saw_multistatus = false;
    let mut closed = false;
    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("not XML: {e}"))?;
        match event {
            Event::Start(e) => {
                let name = e.local_name().as_ref().as_bytes().to_vec();
                match name.as_slice() {
                    b"multistatus" => saw_multistatus = true,
                    b"response" => member = Some(Member::default()),
                    b"propstat" => propstat = Some(PropStat::default()),
                    b"collection" if parent_is(&stack, b"resourcetype") => {
                        if let Some(ps) = propstat.as_mut() {
                            ps.collection = true;
                        }
                    }
                    _ => {}
                }
                stack.push(name);
                text.clear();
            }
            Event::Text(t) => text.push_str(&t.xml10_content()),
            // References arrive as their own events, between the text
            // around them: `&amp;` in an href, or `&#x6587;` for a name.
            Event::GeneralRef(r) => {
                let char = r
                    .resolve_char_ref()
                    .map_err(|e| format!("bad reference: {e}"))?;
                match char {
                    Some(c) => text.push(c),
                    None => text.push_str(
                        resolve_xml_entity(&r.into_inner())
                            .ok_or("unknown entity in the listing")?,
                    ),
                }
            }
            Event::CData(c) => text.push_str(&c.into_inner()),
            Event::End(_) => {
                let Some(name) = stack.pop() else {
                    return Err("unbalanced XML".into());
                };
                let value = text.trim();
                if name == b"multistatus" && stack.is_empty() {
                    closed = true;
                }
                match name.as_slice() {
                    b"href" if parent_is(&stack, b"response") => {
                        if let Some(m) = member.as_mut() {
                            m.href = value.to_owned();
                        }
                    }
                    b"status" if parent_is(&stack, b"propstat") => {
                        if let Some(ps) = propstat.as_mut() {
                            ps.status = status_code(value);
                        }
                    }
                    b"status" if parent_is(&stack, b"response") => {
                        if let Some(m) = member.as_mut() {
                            m.status = status_code(value);
                        }
                    }
                    b"getcontentlength" => {
                        if let Some(ps) = propstat.as_mut() {
                            ps.size = value.parse().ok();
                        }
                    }
                    b"getlastmodified" => {
                        if let Some(ps) = propstat.as_mut() {
                            ps.modified = httpdate::parse_http_date(value).ok();
                        }
                    }
                    b"getetag" => {
                        if let Some(ps) = propstat.as_mut() {
                            ps.etag = (!value.is_empty()).then(|| value.to_owned());
                        }
                    }
                    b"propstat" => {
                        if let (Some(m), Some(ps)) = (member.as_mut(), propstat.take()) {
                            merge(m, ps);
                        }
                    }
                    b"response" => {
                        if let Some(m) = member.take() {
                            if m.href.is_empty() {
                                return Err("a response without href".into());
                            }
                            members.push(m);
                        }
                    }
                    _ => {}
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if !saw_multistatus {
        return Err("not a multistatus response".into());
    }
    // A body cut off mid-way would otherwise read as a shorter listing,
    // which a scanner would take as deleted files.
    if !closed || !stack.is_empty() {
        return Err("the multistatus response is incomplete".into());
    }
    Ok(members)
}

fn parent_is(stack: &[Vec<u8>], name: &[u8]) -> bool {
    stack.last().is_some_and(|top| top == name)
}

/// The code in a status line such as `HTTP/1.1 200 OK`.
fn status_code(line: &str) -> Option<u16> {
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Only a successful propstat describes the member. A 404 propstat lists
/// properties the server does not have, which is not an error.
fn merge(member: &mut Member, ps: PropStat) {
    if !ps.status.is_some_and(|s| (200..300).contains(&s)) {
        if ps.status != Some(404) {
            member
                .propstat_error
                .get_or_insert(ps.status.unwrap_or(500));
        }
        return;
    }
    member.described = true;
    member.collection |= ps.collection;
    member.size = member.size.or(ps.size);
    member.modified = member.modified.or(ps.modified);
    member.etag = member.etag.take().or(ps.etag);
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:" xmlns:x="urn:example">
  <d:response>
    <d:href>/dav/%E9%9F%B3%E5%A3%B0/</d:href>
    <d:propstat>
      <d:prop><d:resourcetype><d:collection/></d:resourcetype><d:getetag>"dir1"</d:getetag></d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
    <d:propstat>
      <d:prop><d:getcontentlength/></d:prop>
      <d:status>HTTP/1.1 404 Not Found</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>https://nas.lan/dav/%E9%9F%B3%E5%A3%B0/01%20a&amp;b.mp3</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype/>
        <d:getcontentlength>1234</d:getcontentlength>
        <d:getlastmodified>Sun, 11 Oct 2026 10:00:00 GMT</d:getlastmodified>
        <d:getetag><![CDATA["f1"]]></d:getetag>
      </d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/%E9%9F%B3%E5%A3%B0/gone</d:href>
    <d:status>HTTP/1.1 404 Not Found</d:status>
  </d:response>
</d:multistatus>"#;

    #[test]
    fn members_carry_their_own_properties_and_statuses() {
        let members = parse(LISTING.as_bytes()).unwrap();
        assert_eq!(members.len(), 3);
        assert!(members[0].collection && members[0].described);
        assert_eq!(members[0].size, None, "a 404 propstat is ignored");
        assert_eq!(members[0].etag.as_deref(), Some("\"dir1\""));
        let file = &members[1];
        assert_eq!(
            file.href,
            "https://nas.lan/dav/%E9%9F%B3%E5%A3%B0/01%20a&b.mp3"
        );
        assert!(!file.collection);
        assert_eq!(file.size, Some(1234));
        assert_eq!(
            file.modified,
            Some(httpdate::parse_http_date("Sun, 11 Oct 2026 10:00:00 GMT").unwrap())
        );
        assert_eq!(file.etag.as_deref(), Some("\"f1\""));
        assert_eq!(members[2].status, Some(404));
        assert!(!members[2].described);
    }

    #[test]
    fn a_truncated_listing_is_rejected() {
        let cut = &LISTING[..LISTING.find("<d:response>\n    <d:href>https").unwrap()];
        assert!(parse(cut.as_bytes()).is_err());
        let mid = &LISTING[..LISTING.find("<d:getcontentlength>1234").unwrap()];
        assert!(parse(mid.as_bytes()).is_err());
    }

    #[test]
    fn references_and_cdata_become_text() {
        let body = r#"<d:multistatus xmlns:d="DAV:"><d:response>
            <d:href>/dav/&#x97F3;&#22768;/a&amp;b&apos;c/<![CDATA[x<y]]></d:href>
            <d:propstat><d:prop><d:resourcetype/></d:prop>
            <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
        </d:response></d:multistatus>"#;
        let members = parse(body.as_bytes()).unwrap();
        assert_eq!(members[0].href, "/dav/音声/a&b'c/x<y");
        let unknown = body.replace("&amp;", "&nbsp;");
        assert!(parse(unknown.as_bytes()).is_err(), "not an XML entity");
    }

    #[test]
    fn a_page_that_is_not_multistatus_is_rejected() {
        assert!(parse(b"<html><body>login</body></html>").is_err());
        assert!(parse(b"not xml <<<").is_err());
    }
}
