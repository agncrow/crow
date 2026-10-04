//! Where a program's data comes from and goes (spec, Files, URLs and S3):
//! a path is a file on this machine, an `http://` or `https://` URL, or an
//! `s3://bucket/key` object wherever the program reads one — `read`,
//! `lines`, `csv`, `frame.load`, `math.load` — and a file or an S3 object
//! wherever it writes one (`write`, `frame.save`). One reader and one
//! writer, which both modes call, so every builtin reaches a URL alike. A
//! table's format is its extension (`.csv`, `.json`, `.parquet`), or, for a
//! URL with none, the media type its server names.

use std::collections::BTreeMap;

/// Whether a path names something not on this machine (the emitter asks,
/// to build a program that may read Parquet from a server; the readers
/// themselves are written in Crow, src/modules/sys.co).
pub fn is_remote(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://") || path.starts_with("s3://")
}

/// The formats a table is read and written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Parquet,
}

/// A table's format by its path's extension (a URL's query left out), or
/// none.
pub fn format_of(path: &str) -> Option<Format> {
    let end = path.find(['?', '#']).unwrap_or(path.len());
    let name = path[..end].rsplit('/').next().unwrap_or("");
    let (_, ext) = name.rsplit_once('.')?;
    match ext.to_ascii_lowercase().as_str() {
        "csv" => Some(Format::Csv),
        "json" => Some(Format::Json),
        "parquet" | "pq" => Some(Format::Parquet),
        _ => None,
    }
}

/// The format of what a URL with no extension answered: the media type its
/// server named, then the bytes themselves — Parquet's `PAR1`, JSON's
/// opening bracket — and CSV otherwise.
pub fn format_sniffed(media: Option<&str>, bytes: &[u8]) -> Format {
    let media = media.unwrap_or("").to_ascii_lowercase();
    if media.contains("json") {
        return Format::Json;
    }
    if media.contains("parquet") {
        return Format::Parquet;
    }
    if media.contains("csv") {
        return Format::Csv;
    }
    if bytes.starts_with(b"PAR1") {
        return Format::Parquet;
    }
    match bytes.iter().find(|b| !b.is_ascii_whitespace()) {
        Some(b'[') | Some(b'{') => Format::Json,
        _ => Format::Csv,
    }
}

/// A map's values as text, each as the one display shows it: what a
/// request's headers, query, form and files are sent as.
pub fn text_map<T: crate::display::CrowDisplay>(m: &BTreeMap<String, T>) -> BTreeMap<String, String> {
    m.iter().map(|(k, v)| (k.clone(), v.crow_fmt())).collect()
}

/// What a request was answered (`request[url, method, body]`): its status,
/// its body as text, and its headers, their names in lower case. A status
/// that is no success is an answer like any other — `.status` says it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Reply {
    pub status: i64,
    pub body: String,
    pub headers: std::collections::BTreeMap<String, String>,
}

impl Reply {
    /// The record a failed call holds — a request never answered — as a
    /// compiled program's own structs have theirs (`crow_default`).
    pub fn crow_default() -> Self {
        Reply::default()
    }
}

/// As the interpreter's record shows: its fields in name order.
impl crate::display::CrowDisplay for Reply {
    fn crow_repr(&self) -> String {
        format!("[body = {}, headers = {}, status = {}]", self.body.crow_repr(), self.headers.crow_repr(), self.status.crow_repr())
    }
}

impl crate::json::ToJson for Reply {
    fn to_json(&self) -> crate::json::Json {
        crate::json::object(vec![("body", self.body.to_json()), ("headers", self.headers.to_json()), ("status", self.status.to_json())])
    }
}

/// `sys.reply[status, body, headers]`: a request's answer from its parts,
/// the record `request` answers — made by the client written in Crow.
pub fn reply(status: i64, body: &str, headers: &std::collections::BTreeMap<String, String>) -> Reply {
    Reply { status, body: body.to_string(), headers: headers.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_format_is_its_extension() {
        assert_eq!(format_of("data/bars.parquet"), Some(Format::Parquet));
        assert_eq!(format_of("https://x.test/a.JSON?key=1"), Some(Format::Json));
        assert_eq!(format_of("s3://b/trades.csv"), Some(Format::Csv));
        assert_eq!(format_of("https://x.test/table#top"), None);
        assert!(is_remote("s3://b/k") && is_remote("https://x") && !is_remote("data/x.csv"));
    }
}
