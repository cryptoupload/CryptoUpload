// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::percent::decode as percent_decode;
use cu_backend::{BackendError, EntryKind, ListEntry};

struct RawEntry {
    href: String,
    kind: EntryKind,
    size: Option<u64>,
    modified: Option<i64>,
}

fn find_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case(local_name))
}

fn parse_multistatus(body: &str) -> Result<Vec<RawEntry>, BackendError> {
    let doc = roxmltree::Document::parse(body).map_err(|e| {
        BackendError::Other(format!("webdav: PROPFIND response is not valid XML: {e}"))
    })?;
    let mut out = Vec::new();
    for response in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case("response"))
    {
        let Some(href) = find_child(response, "href").and_then(|n| n.text()) else {
            continue;
        };
        let href = percent_decode(href);
        if href.is_empty() {
            continue;
        }

        let mut kind = EntryKind::File;
        let mut size = None;
        let mut modified = None;
        let mut found_ok = false;
        for propstat in response
            .children()
            .filter(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case("propstat"))
        {
            let status_ok = find_child(propstat, "status")
                .and_then(|n| n.text())
                .is_some_and(|t| t.contains(" 200 ") || t.trim_end().ends_with(" 200"));
            if !status_ok {
                continue;
            }
            let Some(prop) = find_child(propstat, "prop") else {
                continue;
            };
            found_ok = true;
            if let Some(resourcetype) = find_child(prop, "resourcetype") {
                let is_collection = resourcetype.children().any(|c| {
                    c.is_element() && c.tag_name().name().eq_ignore_ascii_case("collection")
                });
                if is_collection {
                    kind = EntryKind::Directory;
                }
            }
            if let Some(len_node) = find_child(prop, "getcontentlength") {
                size = len_node.text().and_then(|t| t.trim().parse::<u64>().ok());
            }
            if let Some(date_node) = find_child(prop, "getlastmodified") {
                modified = date_node
                    .text()
                    .and_then(cu_backend::timefmt::parse_http_date);
            }
        }
        if found_ok {
            out.push(RawEntry {
                href,
                kind,
                size,
                modified,
            });
        }
    }
    Ok(out)
}

pub(crate) fn stat_from_multistatus(body: &str) -> Result<(EntryKind, Option<u64>), BackendError> {
    parse_multistatus(body)?
        .into_iter()
        .next()
        .map(|e| (e.kind, e.size))
        .ok_or_else(|| {
            BackendError::Other(
                "webdav: PROPFIND response without any recognizable entry".to_string(),
            )
        })
}

pub(crate) fn children_from_multistatus(
    body: &str,
    self_path: &str,
) -> Result<Vec<ListEntry>, BackendError> {
    let entries = parse_multistatus(body)?;
    let self_norm = self_path.trim_end_matches('/');
    let mut out = Vec::new();
    for entry in entries {
        let href_norm = entry.href.trim_end_matches('/');
        if href_norm == self_norm {
            continue;
        }
        let Some(rest) = href_norm.strip_prefix(self_norm) else {
            continue;
        };
        let name = rest.trim_start_matches('/');
        if name.is_empty() || name.contains('/') {
            continue;
        }
        out.push(ListEntry {
            name: name.to_string(),
            kind: entry.kind,
            size: entry.size,
            modified: entry.modified,
        });
    }
    Ok(out)
}
