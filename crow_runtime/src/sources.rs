//! Where a program's data comes from and goes (spec, Files, URLs and S3):
//! a path is a file on this machine, an `http://` or `https://` URL, or an
//! `s3://bucket/key` object wherever the program reads one — `read`,
//! `lines`, `csv`, `frame.load`, `math.load` — and a file or an S3 object
//! wherever it writes one (`write`, `frame.save`). One reader and one
//! writer, which both modes call, so every builtin reaches a URL alike. A
//! table's format is its extension (`.csv`, `.json`, `.parquet`), or, for a
//! URL with none, the media type its server names.

/// Whether a path names something not on this machine.
pub fn is_remote(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://") || path.starts_with("s3://")
}

/// What a remote source answered: its bytes, and the media type its server
/// named.
pub struct Fetched {
    pub bytes: Vec<u8>,
    pub media: Option<String>,
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

/// A remote source's bytes: an HTTP GET, or an S3 object read with the
/// environment's AWS credentials (without them, unsigned, as a public
/// bucket is read).
pub fn fetch(path: &str) -> Result<Fetched, String> {
    #[cfg(feature = "net")]
    {
        if path.starts_with("s3://") {
            net::s3(path, "GET", None).map(|(bytes, media)| Fetched { bytes, media })
        } else {
            net::http_get(path)
        }
    }
    #[cfg(not(feature = "net"))]
    {
        Err(format!("cannot read '{}': this program was built without network sources", path))
    }
}

/// A remote source's text.
pub fn fetch_text(path: &str) -> Result<String, String> {
    let fetched = fetch(path)?;
    String::from_utf8(fetched.bytes).map_err(|_| format!("cannot read '{}': it is not text (UTF-8)", path))
}

/// Bytes written to a remote destination: an S3 object. A URL is read, not
/// written.
pub fn put(path: &str, bytes: &[u8]) -> Result<(), String> {
    if !path.starts_with("s3://") {
        return Err(format!("cannot write '{}': a URL is read, not written — write a file, or an s3:// object", path));
    }
    #[cfg(feature = "net")]
    {
        net::s3(path, "PUT", Some(bytes)).map(|_| ())
    }
    #[cfg(not(feature = "net"))]
    {
        let _ = bytes;
        Err(format!("cannot write '{}': this program was built without network sources", path))
    }
}

/// Lowercase hex, as SigV4 spells a digest.
#[cfg(feature = "net")]
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// A SHA-256 digest in hex.
#[cfg(feature = "net")]
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex(&sha2::Sha256::digest(bytes))
}

/// An S3 key as SigV4's canonical URI spells it: every byte but the
/// unreserved ones and `/` percent-encoded.
#[cfg(feature = "net")]
pub(crate) fn uri_encode(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// The civil date and time of a moment, as SigV4's `x-amz-date` spells it
/// (`20130524T000000Z`).
#[cfg(feature = "net")]
pub(crate) fn amz_date(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // days since the epoch to a civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}Z", y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// What SigV4 signs a request with.
#[cfg(feature = "net")]
pub(crate) struct Credentials {
    pub key_id: String,
    pub secret: String,
    pub token: Option<String>,
}

/// A request's SigV4 headers for S3 (AWS's Signature Version 4): the
/// request's own headers — `host`, what `extra` adds, the payload's digest
/// and the date — signed with a key derived from the secret, the day, the
/// region and the service. The headers to send, `authorization` last.
#[cfg(feature = "net")]
pub(crate) fn sign_s3(method: &str, host: &str, uri: &str, extra: &[(&str, &str)], payload_hash: &str, date: &str, region: &str, creds: &Credentials) -> Result<Vec<(String, String)>, String> {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mac = |key: &[u8], data: &str| -> Result<Vec<u8>, String> {
        let mut m = <HmacSha256 as Mac>::new_from_slice(key).map_err(|e| format!("signing: {}", e))?;
        m.update(data.as_bytes());
        Ok(m.finalize().into_bytes().to_vec())
    };
    let mut headers: Vec<(String, String)> = vec![
        ("host".to_string(), host.to_string()),
        ("x-amz-content-sha256".to_string(), payload_hash.to_string()),
        ("x-amz-date".to_string(), date.to_string()),
    ];
    headers.extend(extra.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string())));
    if let Some(token) = &creds.token {
        headers.push(("x-amz-security-token".to_string(), token.clone()));
    }
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{}:{}\n", k, v)).collect();
    let signed: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    let signed = signed.join(";");
    let canonical = format!("{}\n{}\n\n{}\n{}\n{}", method, uri, canonical_headers, signed, payload_hash);
    let day = &date[..8];
    let scope = format!("{}/{}/s3/aws4_request", day, region);
    let to_sign = format!("AWS4-HMAC-SHA256\n{}\n{}\n{}", date, scope, sha256_hex(canonical.as_bytes()));
    let k_date = mac(format!("AWS4{}", creds.secret).as_bytes(), day)?;
    let k_region = mac(&k_date, region)?;
    let k_service = mac(&k_region, "s3")?;
    let k_signing = mac(&k_service, "aws4_request")?;
    let signature = hex(&mac(&k_signing, &to_sign)?);
    headers.retain(|(k, _)| k != "host");
    headers.push(("authorization".to_string(), format!("AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}", creds.key_id, scope, signed, signature)));
    Ok(headers)
}

#[cfg(feature = "net")]
mod net {
    use super::*;
    use std::time::Duration;

    /// The most a source may answer: a frame is held in memory whole.
    const LIMIT: u64 = 4 << 30;

    /// The client every source is read with: a server that does not
    /// answer in 30 seconds, or a transfer longer than ten minutes, is the
    /// read's failure, not a program that hangs.
    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(600)))
            .http_status_as_error(false)
            .build()
            .into()
    }

    fn answered(path: &str, mut resp: ureq::http::Response<ureq::Body>) -> Result<(Vec<u8>, Option<String>), String> {
        let status = resp.status();
        let media = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_string);
        let bytes = resp.body_mut().with_config().limit(LIMIT).read_to_vec().map_err(|e| format!("cannot read '{}': {}", path, e))?;
        if !status.is_success() {
            // S3 says why in an XML body's <Message>
            let text = String::from_utf8_lossy(&bytes);
            let why = text.split("<Message>").nth(1).and_then(|m| m.split("</Message>").next()).map(|m| format!(" — {}", m)).unwrap_or_default();
            return Err(format!("cannot read '{}': the server answered {}{}", path, status, why));
        }
        Ok((bytes, media))
    }

    pub fn http_get(url: &str) -> Result<Fetched, String> {
        let resp = agent().get(url).call().map_err(|e| format!("cannot read '{}': {}", url, e))?;
        let (bytes, media) = answered(url, resp)?;
        Ok(Fetched { bytes, media })
    }

    fn env(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    /// An S3 object read or written: virtual-hosted on AWS
    /// (`bucket.s3.region.amazonaws.com`), path-style at an endpoint the
    /// environment names for an S3-compatible store (`AWS_ENDPOINT_URL`:
    /// MinIO, R2, …); signed with the environment's credentials, unsigned
    /// without them.
    pub fn s3(path: &str, method: &str, body: Option<&[u8]>) -> Result<(Vec<u8>, Option<String>), String> {
        s3_with(path, method, body, &env)
    }

    /// `s3`, with the environment it reads handed in (a test's own).
    pub(crate) fn s3_with(path: &str, method: &str, body: Option<&[u8]>, env: &dyn Fn(&str) -> Option<String>) -> Result<(Vec<u8>, Option<String>), String> {
        let verb = if method == "GET" { "read" } else { "write" };
        let rest = &path["s3://".len()..];
        let (bucket, key) = rest.split_once('/').filter(|(b, k)| !b.is_empty() && !k.is_empty()).ok_or_else(|| format!("cannot {} '{}': an S3 path is s3://bucket/key", verb, path))?;
        let region = env("AWS_REGION").or_else(|| env("AWS_DEFAULT_REGION")).unwrap_or_else(|| "us-east-1".to_string());
        let (url, host, uri) = match env("AWS_ENDPOINT_URL_S3").or_else(|| env("AWS_ENDPOINT_URL")) {
            Some(endpoint) => {
                let endpoint = endpoint.trim_end_matches('/').to_string();
                let host = endpoint.split("://").nth(1).unwrap_or(&endpoint).split('/').next().unwrap_or("").to_string();
                let uri = format!("/{}/{}", uri_encode(bucket), uri_encode(key));
                (format!("{}{}", endpoint, uri), host, uri)
            }
            None => {
                let host = format!("{}.s3.{}.amazonaws.com", bucket, region);
                let uri = format!("/{}", uri_encode(key));
                (format!("https://{}{}", host, uri), host, uri)
            }
        };
        let payload = body.unwrap_or(&[]);
        let payload_hash = sha256_hex(payload);
        let creds = match (env("AWS_ACCESS_KEY_ID"), env("AWS_SECRET_ACCESS_KEY")) {
            (Some(key_id), Some(secret)) => Some(Credentials { key_id, secret, token: env("AWS_SESSION_TOKEN") }),
            _ => None,
        };
        let headers = match &creds {
            Some(c) => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                sign_s3(method, &host, &uri, &[], &payload_hash, &amz_date(now), &region, c).map_err(|e| format!("cannot {} '{}': {}", verb, path, e))?
            }
            None => Vec::new(),
        };
        let agent = agent();
        let resp = if method == "PUT" {
            let mut req = agent.put(&url);
            for (k, v) in &headers {
                req = req.header(k.as_str(), v.as_str());
            }
            req.send(payload)
        } else {
            let mut req = agent.get(&url);
            for (k, v) in &headers {
                req = req.header(k.as_str(), v.as_str());
            }
            req.call()
        }
        .map_err(|e| format!("cannot {} '{}': {}", verb, path, e))?;
        answered(path, resp).map_err(|e| if verb == "write" { e.replacen("cannot read", "cannot write", 1) } else { e })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server on this machine that answers each request it is sent, in
    /// order, with the next of `answers`, and keeps what it was sent.
    #[cfg(feature = "net")]
    fn serve(answers: Vec<String>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let kept = seen.clone();
        std::thread::spawn(move || {
            for answer in answers {
                let Ok((mut conn, _)) = listener.accept() else { return };
                let mut got = Vec::new();
                let mut buf = [0u8; 4096];
                // the head, then as much body as it says
                loop {
                    let n = conn.read(&mut buf).unwrap_or(0);
                    got.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&got).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text[..end].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if got.len() >= end + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                kept.lock().unwrap().push(String::from_utf8_lossy(&got).to_string());
                let _ = conn.write_all(answer.as_bytes());
            }
        });
        (addr, seen)
    }

    #[cfg(feature = "net")]
    fn answer(status: &str, media: &str, body: &str) -> String {
        format!("HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", status, media, body.len(), body)
    }

    /// A URL's body, with the media type its server named; an error status
    /// is the read's failure, the server's own words after it
    #[cfg(feature = "net")]
    #[test]
    fn a_url_is_read_as_its_server_answers() {
        let (addr, seen) = serve(vec![
            answer("200 OK", "application/json", "[{\"a\": 1}]"),
            answer("404 Not Found", "application/xml", "<Error><Message>The specified key does not exist.</Message></Error>"),
        ]);
        let got = fetch(&format!("{}/v2/aggs?apiKey=x", addr)).unwrap_or_else(|e| panic!("{}", e));
        assert_eq!(got.bytes, b"[{\"a\": 1}]");
        assert_eq!(format_sniffed(got.media.as_deref(), &got.bytes), Format::Json);
        let missing = fetch(&format!("{}/nope.csv", addr)).err().unwrap_or_default();
        assert!(missing.starts_with(&format!("cannot read '{}/nope.csv': the server answered 404 Not Found — The specified key does not exist.", addr)), "{}", missing);
        assert!(seen.lock().unwrap()[0].starts_with("GET /v2/aggs?apiKey=x HTTP/1.1"), "{:?}", seen.lock().unwrap());
    }

    /// An S3 object at an S3-compatible endpoint is reached by path and
    /// signed with the credentials the environment holds; written, its
    /// bytes are the request's body; with no credentials it is read
    /// unsigned
    #[cfg(feature = "net")]
    #[test]
    fn an_s3_object_is_signed_and_reached_by_path_at_an_endpoint() {
        let (addr, seen) = serve(vec![answer("200 OK", "text/csv", "a,b\n1,2\n"), answer("200 OK", "application/xml", ""), answer("200 OK", "text/csv", "x\n")]);
        let signed = |name: &str| match name {
            "AWS_ENDPOINT_URL" => Some(addr.clone()),
            "AWS_ACCESS_KEY_ID" => Some("AKIDEXAMPLE".to_string()),
            "AWS_SECRET_ACCESS_KEY" => Some("secret".to_string()),
            "AWS_REGION" => Some("eu-west-2".to_string()),
            _ => None,
        };
        let (bytes, _) = net::s3_with("s3://bucket/2026/07/trades one.csv", "GET", None, &signed).unwrap_or_else(|e| panic!("{}", e));
        assert_eq!(bytes, b"a,b\n1,2\n");
        net::s3_with("s3://bucket/out.parquet", "PUT", Some(b"PAR1"), &signed).unwrap_or_else(|e| panic!("{}", e));
        let public = |name: &str| if name == "AWS_ENDPOINT_URL" { Some(addr.clone()) } else { None };
        net::s3_with("s3://open/x.csv", "GET", None, &public).unwrap_or_else(|e| panic!("{}", e));
        let seen = seen.lock().unwrap();
        assert!(seen[0].starts_with("GET /bucket/2026/07/trades%20one.csv HTTP/1.1"), "{}", seen[0]);
        let lower = seen[0].to_ascii_lowercase();
        assert!(lower.contains("authorization: aws4-hmac-sha256 credential=akidexample/"), "{}", seen[0]);
        assert!(lower.contains("/eu-west-2/s3/aws4_request, signedheaders=host;x-amz-content-sha256;x-amz-date, signature="), "{}", seen[0]);
        assert!(seen[1].starts_with("PUT /bucket/out.parquet HTTP/1.1") && seen[1].ends_with("PAR1"), "{}", seen[1]);
        assert!(!seen[2].to_ascii_lowercase().contains("authorization:"), "{}", seen[2]);
        assert!(net::s3_with("s3://bucket-only", "GET", None, &public).err().unwrap_or_default().contains("an S3 path is s3://bucket/key"));
    }

    /// A URL is read, not written
    #[test]
    fn a_url_is_not_written() {
        assert!(put("https://example.com/x.csv", b"a").err().unwrap_or_default().contains("a URL is read, not written"));
    }

    /// AWS's own example of a signed S3 GET (Signature Version 4, "GET
    /// Object"): the same request signs to the same signature.
    #[cfg(feature = "net")]
    #[test]
    fn a_request_signs_as_aws_documents() {
        let creds = Credentials { key_id: "AKIAIOSFODNN7EXAMPLE".into(), secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(), token: None };
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let headers = sign_s3("GET", "examplebucket.s3.amazonaws.com", "/test.txt", &[("Range", "bytes=0-9")], empty, "20130524T000000Z", "us-east-1", &creds).unwrap_or_default();
        let auth = headers.iter().find(|(k, _)| k == "authorization").map(|(_, v)| v.clone()).unwrap_or_default();
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        assert_eq!(sha256_hex(b""), empty);
    }

    #[cfg(feature = "net")]
    #[test]
    fn a_moment_is_spelled_as_sigv4_spells_it() {
        assert_eq!(amz_date(1_369_353_600), "20130524T000000Z");
        assert_eq!(amz_date(951_782_400), "20000229T000000Z");
        assert_eq!(amz_date(0), "19700101T000000Z");
    }

    #[test]
    fn a_format_is_its_extension_or_what_the_server_says() {
        assert_eq!(format_of("data/bars.parquet"), Some(Format::Parquet));
        assert_eq!(format_of("https://x.test/a.JSON?key=1"), Some(Format::Json));
        assert_eq!(format_of("s3://b/k/trades.csv"), Some(Format::Csv));
        assert_eq!(format_of("https://api.test/v2/aggs"), None);
        assert_eq!(format_sniffed(Some("application/json; charset=utf-8"), b""), Format::Json);
        assert_eq!(format_sniffed(None, b"PAR1...."), Format::Parquet);
        assert_eq!(format_sniffed(None, b"  [{\"a\": 1}]"), Format::Json);
        assert_eq!(format_sniffed(Some("text/plain"), b"a,b\n1,2\n"), Format::Csv);
    }

    #[cfg(feature = "net")]
    #[test]
    fn an_s3_key_is_encoded_as_sigv4_reads_it() {
        assert_eq!(uri_encode("dir/a b+c.csv"), "dir/a%20b%2Bc.csv");
    }
}
