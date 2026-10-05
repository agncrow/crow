//! `use sys`: the standard library's own layer (spec, "The standard
//! library's layer") — bytes, and a connection to a host, plain or behind
//! TLS, read and written as bytes. A program may not import it: the
//! modules written in Crow build `request`, `socket` and the readers on
//! it, and a program reaches those. Both backends call these functions.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// Bytes: what a connection reads and writes, and text encoded as UTF-8.
/// Its own type, not `Vec<u8>`, so it shows as bytes do and is told apart
/// from a list of ints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    pub fn len(&self) -> i64 {
        self.0.len() as i64
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl crate::display::CrowDisplay for Bytes {
    fn crow_repr(&self) -> String {
        format!("<{} bytes>", self.0.len())
    }
}

impl std::fmt::Display for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} bytes>", self.0.len())
    }
}

/// `sys.bytes[text]`: the text's UTF-8.
pub fn bytes_of_text(s: &str) -> Bytes {
    Bytes(s.as_bytes().to_vec())
}

/// `sys.text[bytes]`: the text the bytes spell in UTF-8, or the reason
/// they spell none.
pub fn text_of_bytes(b: &Bytes) -> Result<String, String> {
    String::from_utf8(b.0.clone()).map_err(|e| format!("not UTF-8 text: invalid byte at {}", e.utf8_error().valid_up_to()))
}

/// `sys.slice[bytes, from, to]`: the bytes from `from` to `to` (excluded),
/// counted from the end when negative, as a text's slice is.
pub fn slice(b: &Bytes, from: i64, to: i64) -> Result<Bytes, String> {
    let n = b.0.len() as i64;
    let at = |i: i64| if i < 0 { n + i } else { i };
    let (lo, hi) = (at(from), at(to));
    if lo < 0 || hi > n || lo > hi {
        return Err(format!("slice [{}, {}) is outside {} bytes", from, to, n));
    }
    Ok(Bytes(b.0[lo as usize..hi as usize].to_vec()))
}

/// `sys.find[bytes, needle]`: where the needle first occurs, or -1.
pub fn find(b: &Bytes, needle: &Bytes) -> i64 {
    if needle.0.is_empty() {
        return 0;
    }
    b.0.windows(needle.0.len()).position(|w| w == needle.0.as_slice()).map_or(-1, |i| i as i64)
}

/// `sys.concat[a, b]`: the two in order.
pub fn concat(a: &Bytes, b: &Bytes) -> Bytes {
    let mut out = a.0.clone();
    out.extend_from_slice(&b.0);
    Bytes(out)
}

/// `sys.byte[bytes, i]`: the byte at `i` as an int, counted from the end
/// when negative.
pub fn byte(b: &Bytes, i: i64) -> Result<i64, String> {
    let n = b.0.len();
    let at = crate::stops::item(i, n)?;
    Ok(b.0[at] as i64)
}

/// `len[bytes]`: how many.
pub fn len_bytes(b: &Bytes) -> i64 {
    b.len()
}

/// `sys.gather[chunks]`: the chunks joined in order — what a connection
/// read, piece by piece, as one (a `concat` per piece would copy the whole
/// each time).
pub fn gather(chunks: &[Bytes]) -> Bytes {
    let mut out = Vec::with_capacity(chunks.iter().map(|c| c.0.len()).sum());
    for c in chunks {
        out.extend_from_slice(&c.0);
    }
    Bytes(out)
}

/// `sys.file[path]`: a file's bytes, as they are (the words `read`'s
/// reason has, since `read` is written on it).
pub fn file(path: &str) -> Result<Bytes, String> {
    std::fs::read(path).map(Bytes).map_err(|e| format!("cannot read '{}': {}", path, e))
}

/// A connection's answer, as the HTTP client written in Crow reads one
/// (`sys.exchange`, `sys.fetch`): the status and its phrase, the headers
/// (names in lower case, repeated ones joined) and the body's bytes. The
/// record both modes hold, as `request`'s reply is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    pub status: i64,
    pub phrase: String,
    pub headers: std::collections::BTreeMap<String, String>,
    pub body: Bytes,
}

impl Answer {
    /// The record a failed call holds, as a compiled program's own structs
    /// have theirs (`crow_default`).
    pub fn crow_default() -> Self {
        Answer::default()
    }
}

/// As the interpreter's record shows: its fields in name order.
impl crate::display::CrowDisplay for Answer {
    fn crow_repr(&self) -> String {
        format!("[body = {}, headers = {}, phrase = {}, status = {}]", self.body.crow_repr(), self.headers.crow_repr(), self.phrase.crow_repr(), self.status.crow_repr())
    }
}

impl crate::json::ToJson for Answer {
    fn to_json(&self) -> crate::json::Json {
        crate::json::object(vec![
            ("body", String::from_utf8_lossy(&self.body.0).to_string().to_json()),
            ("headers", self.headers.to_json()),
            ("phrase", self.phrase.to_json()),
            ("status", self.status.to_json()),
        ])
    }
}

/// `sys.answer[status, phrase, headers, body]`: an answer from its parts.
pub fn answer(status: i64, phrase: &str, headers: &std::collections::BTreeMap<String, String>, body: &Bytes) -> Answer {
    Answer { status, phrase: phrase.to_string(), headers: headers.clone(), body: body.clone() }
}

/// `sys.csv_rows[text]`: CSV text as rows, each a map from its column's
/// name to its text (`io.csv` is written on it).
pub fn csv_rows(text: &str) -> Result<Vec<std::collections::BTreeMap<String, String>>, String> {
    crate::io::parse_csv_string(text)
}

/// `sys.frame_of_csv[text]`: a frame from CSV text, each column's type
/// read from its cells (`frame.load` is written on it).
pub fn frame_of_csv(text: &str) -> Result<crate::frame::CrowFrame, String> {
    crate::frame::csv_frame(text)
}

/// `sys.frame_of_json[text]`: a frame from JSON records.
pub fn frame_of_json(text: &str) -> Result<crate::frame::CrowFrame, String> {
    crate::frame::from_json(text)
}

/// `sys.frame_of_parquet[bytes]`: a frame from a Parquet file's bytes,
/// where the program was built to read Parquet (the interpreter always
/// is; a compiled program whose paths may name one).
pub fn frame_of_parquet(bytes: &Bytes) -> Result<crate::frame::CrowFrame, String> {
    crate::frames::parquet_frame(bytes.0.clone())
}

/// `sys.frame_csv[frame]`: a frame as CSV text.
pub fn frame_csv(f: &crate::frame::CrowFrame) -> Result<String, String> {
    f.to_csv()
}

/// `sys.frame_json[frame]`: a frame as JSON records.
pub fn frame_json(f: &crate::frame::CrowFrame) -> String {
    crate::frame::to_json(f)
}

/// `sys.frame_parquet[frame]`: a frame as a Parquet file's bytes.
pub fn frame_parquet(f: &crate::frame::CrowFrame) -> Result<Bytes, String> {
    crate::frames::parquet_bytes(f).map(Bytes)
}

/// `sys.tensor_of_frame[frame]`: a frame's cells as a matrix of numbers
/// (`math.load` is written on it).
pub fn tensor_of_frame(f: &crate::frame::CrowFrame) -> Result<crate::tensor::CrowTensor, String> {
    // a table with no rows is a matrix with none, whatever its columns
    // would have held (a header alone types them as text)
    if f.nrows() == 0 && f.ncols() > 0 {
        return Ok(crate::tensor::CrowTensor::from_vec(vec![], vec![0, f.ncols()]));
    }
    let (data, nrows, ncols) = crate::frame::frame_to_tensor_data(f, None)?;
    Ok(crate::tensor::CrowTensor::from_vec(data, vec![nrows, ncols]))
}

/// `sys.store[path, bytes]`: the bytes written to a file, as they are.
pub fn store(path: &str, b: &Bytes) -> Result<(), String> {
    std::fs::write(path, &b.0).map_err(|e| format!("cannot write '{}': {}", path, e))
}

/// `sys.random[n]`: n bytes from the operating system's randomness.
pub fn random(n: i64) -> Result<Bytes, String> {
    let n = usize::try_from(n).map_err(|_| format!("random: a count cannot be negative (got {})", n))?;
    crate::signin::random_bytes(n).map(Bytes)
}

/// `sys.hex[bytes]`: the bytes as lowercase hex, two digits each.
pub fn hex(b: &Bytes) -> String {
    b.0.iter().map(|x| format!("{:02x}", x)).collect()
}

/// `sys.text[bytes, fill]`: the text the bytes spell, each run that spells
/// none replaced by `fill` (an answer's body shown whatever it holds: the
/// replacement character U+FFFD, as a browser shows it, or any text).
pub fn text_with(b: &Bytes, fill: &str) -> String {
    let lossy = String::from_utf8_lossy(&b.0);
    if fill == "\u{FFFD}" { lossy.to_string() } else { lossy.replace('\u{FFFD}', fill) }
}

/// `sys.hash[bytes, algo]`: the digest as lowercase hex — `'sha256'`,
/// `'sha512'`, `'md5'`.
pub fn hash(b: &Bytes, algo: &str) -> Result<String, String> {
    use sha2::Digest;
    match algo {
        "sha256" => Ok(format!("{:x}", sha2::Sha256::digest(&b.0))),
        "sha512" => Ok(format!("{:x}", sha2::Sha512::digest(&b.0))),
        "sha1" => Ok(format!("{:x}", sha1::Sha1::digest(&b.0))),
        "md5" => Ok(format!("{:x}", md5::Md5::digest(&b.0))),
        other => Err(format!("hash: '{}' is not a digest of bytes — sha256, sha512, sha1 or md5", other)),
    }
}

/// `sys.digest[bytes, algo]`: the digest as bytes — `'sha1'` (a socket's
/// accept key), `'sha256'`, `'sha512'` or `'md5'`.
pub fn digest(b: &Bytes, algo: &str) -> Result<Bytes, String> {
    use sha2::Digest;
    match algo {
        "sha1" => Ok(Bytes(sha1::Sha1::digest(&b.0).to_vec())),
        "sha256" => Ok(Bytes(sha2::Sha256::digest(&b.0).to_vec())),
        "sha512" => Ok(Bytes(sha2::Sha512::digest(&b.0).to_vec())),
        "md5" => Ok(Bytes(md5::Md5::digest(&b.0).to_vec())),
        other => Err(format!("digest: '{}' is not a digest of bytes — sha1, sha256, sha512 or md5", other)),
    }
}

/// `sys.mask[bytes, key]`: the bytes XORed with the 4-byte key, cycling —
/// a client's frame masked, a masked frame read (RFC 6455). A key of
/// another length stops.
pub fn mask(b: &Bytes, key: &Bytes) -> Result<Bytes, String> {
    if key.0.len() != 4 {
        return Err(format!("mask: a key is 4 bytes, got {}", key.0.len()));
    }
    Ok(Bytes(b.0.iter().enumerate().map(|(i, x)| x ^ key.0[i % 4]).collect()))
}

/// A WebSocket client's connection (`socket[url]`, written in Crow over
/// this layer): the stream after its handshake, the URL it was opened to,
/// and the bytes read past what the socket last needed — the handshake's
/// head, a frame's end — held until it reads on. Shared by every value
/// that holds it.
#[derive(Clone)]
pub struct Socket(Arc<SocketInner>);

struct SocketInner {
    stream: CrowStream,
    url: String,
    unread: Mutex<Vec<u8>>,
}

impl std::fmt::Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "socket {}", self.0.url)
    }
}

impl crate::display::CrowDisplay for Socket {
    fn crow_repr(&self) -> String {
        format!("<socket {}>", self.0.url)
    }
}

impl Socket {
    /// A socket that is closed: what a socket that failed to open holds,
    /// so every call on it answers that it is closed.
    pub fn closed() -> Self {
        Socket(Arc::new(SocketInner { stream: CrowStream::closed(), url: String::new(), unread: Mutex::new(Vec::new()) }))
    }
}

/// `sys.socket_of[stream, url, unread]`: a socket over a stream the
/// handshake upgraded, holding what came after the handshake's head.
pub fn socket_of(stream: &CrowStream, url: &str, unread: &Bytes) -> Socket {
    Socket(Arc::new(SocketInner { stream: stream.clone(), url: url.to_string(), unread: Mutex::new(unread.0.clone()) }))
}

/// `sys.stream_of[socket]`: the stream a socket talks over.
pub fn stream_of(s: &Socket) -> CrowStream {
    s.0.stream.clone()
}

/// `sys.leftover[socket]`: the bytes a socket holds unread, taken — none
/// stay with it.
pub fn leftover(s: &Socket) -> Bytes {
    Bytes(std::mem::take(&mut *crate::lock(&s.0.unread)))
}

/// `sys.unread[socket, bytes]`: bytes put back for the socket's next read,
/// before anything it holds (what a read took past a frame's end).
pub fn unread(s: &Socket, b: &Bytes) {
    let mut held = crate::lock(&s.0.unread);
    let mut again = b.0.clone();
    again.extend_from_slice(&held);
    *held = again;
}

/// `sys.base64[bytes]`: the bytes as base64, the standard alphabet padded
/// with `=`, as HTTP's Basic authorization and SMTP's AUTH PLAIN want it.
pub fn base64(b: &Bytes) -> String {
    crate::text::base64_encode(&b.0)
}

/// `sys.base64url[bytes]`: the bytes as base64 in the URL-safe alphabet
/// (`-` and `_`), unpadded, as a token's parts are written (RFC 7515).
pub fn base64url(b: &Bytes) -> String {
    crate::text::base64_encode(&b.0).replace('+', "-").replace('/', "_").trim_end_matches('=').to_string()
}

/// `sys.unbase64[text]`: base64's bytes, either alphabet, padded or not;
/// an error value of kind `'parse'` where the text is no base64.
pub fn unbase64(text: &str) -> Result<Bytes, String> {
    crate::text::base64_decode(text).map(Bytes)
}

/// `sys.same[a, b]`: whether two byte strings are equal, decided in time
/// that does not depend on where they differ (a signature checked against
/// the one computed gives nothing away by how long the check took).
pub fn same(a: &Bytes, b: &Bytes) -> bool {
    if a.0.len() != b.0.len() {
        return false;
    }
    a.0.iter().zip(&b.0).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `sys.pack[n, width]`: an int as `width` bytes, big-endian — 1, 2, 4 or
/// 8, the low bytes of its two's complement (a length in a frame's header,
/// a MessagePack number). Another width stops.
pub fn pack(n: i64, width: i64) -> Result<Bytes, String> {
    match width {
        1 | 2 | 4 | 8 => Ok(Bytes(n.to_be_bytes()[(8 - width as usize)..].to_vec())),
        other => Err(format!("pack: a width is 1, 2, 4 or 8 bytes, not {}", other)),
    }
}

/// `sys.ieee[x]`: a number as the 8 bytes of its IEEE 754 double,
/// big-endian (a MessagePack float).
pub fn ieee(x: f64) -> Bytes {
    Bytes(x.to_be_bytes().to_vec())
}

/// A mailer `smtp[host, port, user, pass]` made: the server to talk to and
/// the credentials, which the module written in Crow (src/modules/mail.co)
/// reads through `mailer_field`; it shows as its host, never its password.
#[derive(Clone, Default)]
pub struct Mailer(Arc<MailerInner>);

#[derive(Default)]
struct MailerInner {
    host: String,
    port: i64,
    user: String,
    pass: String,
}

impl std::fmt::Debug for Mailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mailer:{}", self.0.host)
    }
}

impl crate::display::CrowDisplay for Mailer {
    fn crow_repr(&self) -> String {
        format!("<mailer:{}>", self.0.host)
    }
}

impl std::fmt::Display for Mailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<mailer:{}>", self.0.host)
    }
}

/// `smtp[host, port, user, pass]`: a mailer; its messages come from `user`.
pub fn mailer(host: &str, port: i64, user: &str, pass: &str) -> Mailer {
    Mailer(Arc::new(MailerInner { host: host.to_string(), port, user: user.to_string(), pass: pass.to_string() }))
}

/// `sys.mailer_field[mailer, name]`: what a mailer holds — its `host`,
/// `port` (as text), `user` or `pass`; another name stops.
pub fn mailer_field(m: &Mailer, name: &str) -> Result<String, String> {
    match name {
        "host" => Ok(m.0.host.clone()),
        "port" => Ok(m.0.port.to_string()),
        "user" => Ok(m.0.user.clone()),
        "pass" => Ok(m.0.pass.clone()),
        other => Err(format!("mailer_field: a mailer has a host, a port, a user and a pass, not '{}'", other)),
    }
}

/// `sys.secure[stream, host]`: a plain connection put behind TLS from here
/// on (SMTP's STARTTLS), the certificate checked against the host's name;
/// the same stream, for every value that holds it. An error value of kind
/// `'sys'` where the stream is closed, already secured, or the name is
/// none a certificate can carry.
pub fn secure(s: &CrowStream, host: &str) -> Result<CrowStream, String> {
    let mut guard = s.0.conn.lock().map_err(|_| "the stream is held by a thread that stopped".to_string())?;
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| format!("{} is no name a certificate can carry", host))?;
    let secured = match guard.take() {
        None => return Err("the stream is closed".to_string()),
        Some(Conn::Tls(t)) => {
            *guard = Some(Conn::Tls(t));
            return Err("the stream is already behind TLS".to_string());
        }
        Some(Conn::Plain(tcp)) => {
            let client = match rustls::ClientConnection::new(tls_config(), name) {
                Ok(c) => c,
                Err(e) => {
                    *guard = Some(Conn::Plain(tcp));
                    return Err(format!("TLS to {}: {}", host, e));
                }
            };
            Conn::Tls(Box::new(rustls::StreamOwned::new(client, tcp)))
        }
    };
    *guard = Some(secured);
    Ok(s.clone())
}

/// `sys.hmac[key, message, algo]`: the keyed digest, as bytes — `'sha256'`
/// (an S3 signature's chain) or `'sha512'`.
pub fn hmac(key: &Bytes, message: &Bytes, algo: &str) -> Result<Bytes, String> {
    use hmac::{Hmac, Mac};
    Ok(Bytes(match algo {
        "sha256" => {
            let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&key.0).map_err(|e| format!("hmac: {}", e))?;
            mac.update(&message.0);
            mac.finalize().into_bytes().to_vec()
        }
        "sha512" => {
            let mut mac = Hmac::<sha2::Sha512>::new_from_slice(&key.0).map_err(|e| format!("hmac: {}", e))?;
            mac.update(&message.0);
            mac.finalize().into_bytes().to_vec()
        }
        other => return Err(format!("hmac: '{}' is not a keyed digest — sha256 or sha512", other)),
    }))
}

enum Conn {
    Plain(std::net::TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>),
}

impl Conn {
    fn tcp(&self) -> &std::net::TcpStream {
        match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => s.get_ref(),
        }
    }
}

/// A connection to a host: one, behind a lock, however many values hold
/// it; closed by `close` or when it is let go.
#[derive(Clone)]
pub struct CrowStream(Arc<StreamInner>);

struct StreamInner {
    conn: Mutex<Option<Conn>>,
    name: String,
    /// The wait `open` gave it, which a read given no seconds goes back to.
    wait: Option<std::time::Duration>,
}

impl std::fmt::Debug for CrowStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stream {}", self.0.name)
    }
}

impl crate::display::CrowDisplay for CrowStream {
    fn crow_repr(&self) -> String {
        format!("<stream {}>", self.0.name)
    }
}

impl CrowStream {
    /// What a failed `open` holds: a stream that is closed.
    pub fn closed() -> Self {
        CrowStream(Arc::new(StreamInner { conn: Mutex::new(None), name: "closed".to_string(), wait: None }))
    }
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
        })
        .clone()
}

/// `sys.open[host, port, tls, seconds]`: a connection to the host's port,
/// behind TLS when asked, each read and write giving up after the seconds
/// (none when they are 0), or the reason none could be made.
pub fn open(host: &str, port: i64, tls: bool, seconds: f64) -> Result<CrowStream, String> {
    use std::net::ToSocketAddrs;
    if !(0..=65535).contains(&port) {
        return Err(format!("port {} is outside 0 to 65535", port));
    }
    let timeout = (seconds > 0.0).then(|| std::time::Duration::from_secs_f64(seconds));
    let addrs: Vec<std::net::SocketAddr> = (host, port as u16).to_socket_addrs().map_err(|e| format!("cannot resolve {}: {}", host, e))?.collect();
    if addrs.is_empty() {
        return Err(format!("cannot resolve {}", host));
    }
    // a name's addresses in turn (`localhost` is ::1 and 127.0.0.1, and a
    // server may listen on one), the last refusal the reason when none answers
    let mut tcp = None;
    let mut refused = String::new();
    for addr in &addrs {
        let tried = match timeout {
            Some(t) => std::net::TcpStream::connect_timeout(addr, t),
            None => std::net::TcpStream::connect(addr),
        };
        match tried {
            Ok(t) => {
                tcp = Some(t);
                break;
            }
            Err(e) => refused = e.to_string(),
        }
    }
    let Some(tcp) = tcp else { return Err(format!("cannot connect to {}:{}: {}", host, port, refused)) };
    tcp.set_read_timeout(timeout).map_err(|e| e.to_string())?;
    tcp.set_write_timeout(timeout).map_err(|e| e.to_string())?;
    let conn = if tls {
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| format!("{} is no name a certificate can carry", host))?;
        let client = rustls::ClientConnection::new(tls_config(), name).map_err(|e| format!("TLS to {}: {}", host, e))?;
        Conn::Tls(Box::new(rustls::StreamOwned::new(client, tcp)))
    } else {
        Conn::Plain(tcp)
    };
    Ok(CrowStream(Arc::new(StreamInner { conn: Mutex::new(Some(conn)), name: format!("{}:{}", host, port), wait: timeout })))
}

/// `sys.send[stream, bytes]`: the bytes written, all of them, or the reason.
pub fn send(s: &CrowStream, b: &Bytes) -> Result<i64, String> {
    let mut guard = s.0.conn.lock().map_err(|_| "the stream is held by a thread that stopped".to_string())?;
    let Some(conn) = guard.as_mut() else { return Err("the stream is closed".to_string()) };
    let r = match conn {
        Conn::Plain(t) => t.write_all(&b.0).and_then(|_| t.flush()),
        Conn::Tls(t) => t.write_all(&b.0).and_then(|_| t.flush()),
    };
    r.map_err(|e| format!("cannot send to {}: {}", s.0.name, e))?;
    Ok(b.len())
}

/// `sys.recv[stream, most, seconds]`: up to `most` bytes as they arrive —
/// none when the other side closed — waiting at most the seconds (the
/// connection's own wait when they are 0), or the reason: a wait that
/// passed its seconds, a stream that is closed.
pub fn recv(s: &CrowStream, most: i64, seconds: f64) -> Result<Bytes, String> {
    if most <= 0 {
        return Err(format!("recv[] reads at most a positive number of bytes, got {}", most));
    }
    let mut guard = s.0.conn.lock().map_err(|_| "the stream is held by a thread that stopped".to_string())?;
    let Some(conn) = guard.as_mut() else { return Err("the stream is closed".to_string()) };
    // (a read given no seconds waits as the connection was opened to,
    // whatever the read before it waited)
    let wait = if seconds > 0.0 { Some(std::time::Duration::from_secs_f64(seconds)) } else { s.0.wait };
    conn.tcp().set_read_timeout(wait).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; most as usize];
    let r = match conn {
        Conn::Plain(t) => t.read(&mut buf),
        Conn::Tls(t) => t.read(&mut buf),
    };
    match r {
        Ok(n) => {
            buf.truncate(n);
            Ok(Bytes(buf))
        }
        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Err(format!("no data from {} in {} s", s.0.name, seconds)),
        Err(e) => Err(format!("cannot read from {}: {}", s.0.name, e)),
    }
}

/// `sys.close[stream]`: the connection let go; every call after is refused.
pub fn close(s: &CrowStream) {
    if let Ok(mut guard) = s.0.conn.lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    /// A name with several addresses connects to the one that answers:
    /// `localhost` is ::1 before 127.0.0.1 on macOS, and a server listening
    /// on 127.0.0.1 alone was refused as "connection refused" by the first
    #[test]
    fn a_name_with_several_addresses_connects_to_the_one_that_answers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port() as i64;
        let accepting = std::thread::spawn(move || listener.accept().map(|_| ()));
        let opened = super::open("localhost", port, false, 2.0);
        assert!(opened.is_ok(), "{:?}", opened.err());
        drop(opened);
        assert!(accepting.join().unwrap().is_ok());
    }

    use super::*;

    /// A table's cells are a matrix, a row each; a cell that is no number
    /// is the reason, naming its column; a header alone is a matrix with
    /// no rows; CSV rows are maps from the column's name to its text.
    #[test]
    fn a_frame_of_numbers_is_a_matrix_and_csv_rows_are_maps() {
        let f = frame_of_csv("a,b\n1,2\n3,4.5\n").unwrap();
        let t = tensor_of_frame(&f).unwrap();
        assert_eq!(t.shape(), &[2, 2]);
        assert_eq!(t.to_vec(), vec![1.0, 2.0, 3.0, 4.5]);
        let neg = tensor_of_frame(&frame_of_csv("x\n-1\n-2.5\n").unwrap()).unwrap();
        assert_eq!((neg.shape(), neg.to_vec()), (&[2usize, 1][..], vec![-1.0, -2.5]));
        let text = tensor_of_frame(&frame_of_csv("sym,qty\nAAPL,1\n").unwrap()).unwrap_err();
        assert!(text.contains("sym"), "{}", text);
        let header = tensor_of_frame(&frame_of_csv("a,b\n").unwrap()).unwrap();
        assert_eq!(header.shape(), &[0, 2]);
        let rows = csv_rows("name,age\nalice,30\nbob,25\n").unwrap();
        assert_eq!((rows.len(), rows[1]["age"].as_str()), (2, "25"));
        assert!(csv_rows("").is_err());
    }

    /// A read given no seconds waits as the connection was opened to,
    /// whatever the read before it waited (a timed read had left its wait
    /// on the connection, and a socket's untimed read gave up after it)
    #[test]
    fn a_read_given_no_seconds_waits_as_the_connection_does() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((s, _)) = listener.accept() {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                drop(s);
            }
        });
        let s = open("127.0.0.1", port as i64, false, 0.6).unwrap();
        let quick = std::time::Instant::now();
        assert!(recv(&s, 1, 0.1).unwrap_err().contains("in 0.1 s"));
        assert!(quick.elapsed() < std::time::Duration::from_millis(500));
        let own = std::time::Instant::now();
        assert!(recv(&s, 1, 0.0).unwrap_err().contains("no data"));
        assert!(own.elapsed() >= std::time::Duration::from_millis(500), "waited {:?}", own.elapsed());
    }

    /// A server that echoes what it is sent, once, then closes.
    fn echo_once() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 64];
                if let Ok(n) = s.read(&mut buf) {
                    let _ = s.write_all(&buf[..n]);
                }
            }
        });
        port
    }

    #[test]
    fn bytes_are_text_sliced_found_and_joined() {
        let b = bytes_of_text("héllo");
        assert_eq!(b.len(), 6);
        assert_eq!(text_of_bytes(&b).unwrap(), "héllo");
        assert_eq!(text_of_bytes(&slice(&b, 1, 2).unwrap()).unwrap_err().contains("not UTF-8"), true);
        assert_eq!(find(&b, &bytes_of_text("llo")), 3);
        assert_eq!(find(&b, &bytes_of_text("x")), -1);
        assert_eq!(text_of_bytes(&concat(&slice(&b, 0, 1).unwrap(), &slice(&b, -2, 6).unwrap())).unwrap(), "hlo");
        assert_eq!(byte(&b, -1).unwrap(), b'o' as i64);
        assert!(slice(&b, 2, 1).is_err());
        assert_eq!(format!("{}", b), "<6 bytes>");
    }

    #[test]
    fn a_connection_sends_receives_and_closes() {
        let port = echo_once();
        let s = open("127.0.0.1", port as i64, false, 2.0).unwrap();
        assert_eq!(send(&s, &bytes_of_text("ping")).unwrap(), 4);
        assert_eq!(text_of_bytes(&recv(&s, 64, 2.0).unwrap()).unwrap(), "ping");
        // the server closed: nothing more comes
        assert!(recv(&s, 64, 2.0).unwrap().is_empty());
        close(&s);
        assert!(send(&s, &bytes_of_text("x")).unwrap_err().contains("closed"));
        assert!(open("127.0.0.1", 1, false, 0.2).unwrap_err().contains("cannot connect"));
        assert!(open("127.0.0.1", 70000, false, 0.2).unwrap_err().contains("outside"));
    }
}
