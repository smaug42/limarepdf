//! A small IPP client of LimarePDF's own, which asks the printing service (CUPS) about its
//! printers as the system's print dialogs do: Get-Printer-Attributes, and CUPS's own
//! CUPS-Get-Printers and CUPS-Get-Default, sent over HTTP to CUPS's own socket or its port,
//! encoded and read here (RFC 8010). Used on Linux and macOS for the printer's paper sizes
//! and margins, which `lpoptions` doesn't give, and to list the printers at once, where
//! `lpstat` waits for those the network announces.
#![cfg_attr(windows, allow(dead_code))]

use crate::Result;
use anyhow::{anyhow, bail};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where the printing service listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Server {
    /// A local socket, as CUPS's own.
    Socket(PathBuf),
    /// A host and its port.
    Host(String, u16),
}

/// An attribute's name and values.
pub(crate) type Attribute = (String, Vec<Value>);

/// An attribute's value, as far as LimarePDF reads them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    /// An integer, an enum or a boolean.
    Int(i32),
    /// A keyword, name, text, URI and the like.
    Text(String),
    /// A collection: its members, each with its values.
    Collection(Vec<Attribute>),
    /// Anything else: a range, a resolution, a date, an out-of-band value.
    Other,
}

impl Value {
    pub(crate) fn int(&self) -> Option<i32> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
    pub(crate) fn members(&self) -> Option<&[Attribute]> {
        match self {
            Value::Collection(m) => Some(m),
            _ => None,
        }
    }
}

/// The values of the attribute `name` in `attributes`, none when it isn't there.
pub(crate) fn values<'a>(attributes: &'a [Attribute], name: &str) -> &'a [Value] {
    attributes
        .iter()
        .find(|(n, _)| n == name)
        .map_or(&[], |(_, v)| v.as_slice())
}

// Delimiter and value tags (RFC 8010, section 3.5).
const OPERATION_GROUP: u8 = 0x01;
const END_OF_ATTRIBUTES: u8 = 0x03;
const PRINTER_GROUP: u8 = 0x04;
const INTEGER: u8 = 0x21;
const BOOLEAN: u8 = 0x22;
const ENUM: u8 = 0x23;
const BEGIN_COLLECTION: u8 = 0x34;
const TEXT_WITH_LANGUAGE: u8 = 0x35;
const NAME_WITH_LANGUAGE: u8 = 0x36;
const END_COLLECTION: u8 = 0x37;
const MEMBER_NAME: u8 = 0x4A;

/// The status of a printer the printing service doesn't know.
const NOT_FOUND: u16 = 0x0406;

/// How deep collections may nest in an answer: the printers' own go two deep.
const MAX_DEPTH: usize = 8;

/// The most of an answer read: a printer's whole description is tens of kilobytes.
const MAX_ANSWER: usize = 16 << 20;

// Operations: IPP's own (RFC 8011) and CUPS's.
const GET_PRINTER_ATTRIBUTES: u16 = 0x000B;
const CUPS_GET_DEFAULT: u16 = 0x4001;
const CUPS_GET_PRINTERS: u16 = 0x4002;

/// A Get-Printer-Attributes request for the printer at `uri`, asking for `wanted`.
pub(crate) fn request(uri: &str, wanted: &[&str]) -> Vec<u8> {
    request_for(GET_PRINTER_ATTRIBUTES, Some(uri), wanted)
}

/// A request for `operation`, about the printer at `uri` when it names one, asking for
/// `wanted`.
fn request_for(operation: u16, uri: Option<&str>, wanted: &[&str]) -> Vec<u8> {
    // IPP/1.1, which every CUPS answers; request 1.
    let mut b = vec![1, 1];
    b.extend_from_slice(&operation.to_be_bytes());
    b.extend_from_slice(&[0, 0, 0, 1, OPERATION_GROUP]);
    let mut put = |tag: u8, name: &str, value: &str| {
        b.push(tag);
        for part in [name.as_bytes(), value.as_bytes()] {
            b.extend_from_slice(&(part.len() as u16).to_be_bytes());
            b.extend_from_slice(part);
        }
    };
    put(0x47, "attributes-charset", "utf-8");
    put(0x48, "attributes-natural-language", "en");
    if let Some(uri) = uri {
        put(0x45, "printer-uri", uri);
    }
    for (i, w) in wanted.iter().enumerate() {
        // The values after the first carry no name.
        put(0x44, if i == 0 { "requested-attributes" } else { "" }, w);
    }
    b.push(END_OF_ATTRIBUTES);
    b
}

/// An answer: its status, the message that came with it, and the printer's attributes.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Answer {
    pub status: u16,
    pub message: Option<String>,
    pub printer: Vec<Attribute>,
}

/// Reads an answer, refusing one cut short or malformed.
pub(crate) fn read_answer(data: &[u8]) -> Result<Answer> {
    let mut r = Reader { data, at: 0 };
    r.take(2)?;
    let mut answer = Answer {
        status: r.u16()?,
        ..Answer::default()
    };
    r.take(4)?;
    let mut group = 0;
    let mut operation: Vec<Attribute> = Vec::new();
    loop {
        let tag = r.u8()?;
        if tag == END_OF_ATTRIBUTES {
            break;
        }
        if tag < 0x10 {
            group = tag;
            continue;
        }
        let name = r.string()?;
        let bytes = r.string_bytes()?;
        let value = r.value(tag, bytes, 0)?;
        let list = match group {
            OPERATION_GROUP => &mut operation,
            PRINTER_GROUP => &mut answer.printer,
            // Other groups (unsupported attributes, jobs) are read and left.
            _ => continue,
        };
        match (name.is_empty(), list.last_mut()) {
            (false, _) => list.push((name, vec![value])),
            (true, Some(last)) => last.1.push(value),
            (true, None) => bail!("the printing service's answer has a value without a name"),
        }
    }
    answer.message = values(&operation, "status-message")
        .first()
        .and_then(Value::text)
        .map(str::to_string);
    Ok(answer)
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or_else(|| anyhow!("the printing service's answer was cut short"))?;
        let part = &self.data[self.at..end];
        self.at = end;
        Ok(part)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    /// A length and that many bytes.
    fn string_bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.u16()?;
        self.take(n.into())
    }
    fn string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.string_bytes()?).into_owned())
    }

    /// The value of `tag` written as `bytes`; a collection's members follow it.
    fn value(&mut self, tag: u8, bytes: &[u8], depth: usize) -> Result<Value> {
        Ok(match tag {
            BEGIN_COLLECTION => Value::Collection(self.collection(depth + 1)?),
            INTEGER | ENUM if bytes.len() == 4 => {
                Value::Int(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            BOOLEAN if bytes.len() == 1 => Value::Int(bytes[0].into()),
            TEXT_WITH_LANGUAGE | NAME_WITH_LANGUAGE => {
                // A language, then the text, each with its length.
                let mut r = Reader { data: bytes, at: 0 };
                r.string_bytes()?;
                Value::Text(r.string()?)
            }
            0x30 | 0x41..=0x49 => Value::Text(String::from_utf8_lossy(bytes).into_owned()),
            _ => Value::Other,
        })
    }

    /// A collection's members, up to its end.
    fn collection(&mut self, depth: usize) -> Result<Vec<Attribute>> {
        if depth > MAX_DEPTH {
            bail!("the printing service's answer nests too deeply");
        }
        let mut members: Vec<Attribute> = Vec::new();
        loop {
            let tag = self.u8()?;
            // Members' names come as values; their own names are empty.
            self.string_bytes()?;
            let bytes = self.string_bytes()?;
            match tag {
                END_COLLECTION => return Ok(members),
                MEMBER_NAME => {
                    members.push((String::from_utf8_lossy(bytes).into_owned(), Vec::new()));
                }
                t if t < 0x10 => bail!("the printing service's answer ends inside a collection"),
                t => {
                    let value = self.value(t, bytes, depth)?;
                    match members.last_mut() {
                        Some(m) => m.1.push(value),
                        None => bail!("the printing service's answer has a member without a name"),
                    }
                }
            }
        }
    }
}

/// The printing service LimarePDF asks, found as CUPS's own commands find it: `CUPS_SERVER`,
/// else the `ServerName` of the user's and then the system's `client.conf`, else CUPS's
/// local socket, else this computer's port 631.
pub(crate) fn server() -> Server {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let confs = [
        home.map(|h| h.join(".cups/client.conf")),
        Some(PathBuf::from("/etc/cups/client.conf")),
    ];
    let named = std::env::var("CUPS_SERVER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            confs
                .iter()
                .flatten()
                .find_map(|p| server_name(&std::fs::read_to_string(p).ok()?))
        });
    server_from(named.as_deref(), |p| p.exists())
}

/// CUPS's local sockets: Linux's, an older Linux's, and macOS's.
const SOCKETS: [&str; 3] = [
    "/run/cups/cups.sock",
    "/var/run/cups/cups.sock",
    "/private/var/run/cupsd",
];

/// The `ServerName` set in a `client.conf`.
fn server_name(conf: &str) -> Option<String> {
    conf.lines().find_map(|l| {
        let (key, value) = l.trim().split_once(char::is_whitespace)?;
        let value = value.trim();
        (key.eq_ignore_ascii_case("ServerName") && !value.is_empty()).then(|| value.to_string())
    })
}

/// The server `named` (a socket's path, or a host with its port), else the first of
/// [`SOCKETS`] that `exists`, else port 631 here.
fn server_from(named: Option<&str>, exists: impl Fn(&Path) -> bool) -> Server {
    if let Some(n) = named.map(str::trim).filter(|n| !n.is_empty()) {
        if n.starts_with('/') {
            return Server::Socket(n.into());
        }
        // "host", "host:port", "[v6 address]:port", with "/version=1.1" after it at times.
        let n = n.split('/').next().unwrap_or(n);
        let (host, port) = match n.strip_prefix('[') {
            Some(rest) => match rest.split_once(']') {
                Some((h, p)) => (h, p.strip_prefix(':')),
                None => (rest, None),
            },
            None => match n.rsplit_once(':') {
                Some((h, p)) if !h.contains(':') => (h, Some(p)),
                _ => (n, None),
            },
        };
        let port = port.and_then(|p| p.parse().ok()).unwrap_or(631);
        return Server::Host(host.to_string(), port);
    }
    match SOCKETS.iter().map(Path::new).find(|p| exists(p)) {
        Some(p) => Server::Socket(p.into()),
        None => Server::Host("localhost".into(), 631),
    }
}

/// `queue`'s attributes `wanted`, asked of `server` and given up on after `limit`; `None`
/// when the service doesn't know the printer.
pub(crate) fn printer_attributes(
    server: &Server,
    queue: &str,
    wanted: &[&str],
    limit: Duration,
) -> Result<Option<Vec<Attribute>>> {
    let path = format!("/printers/{}", escaped(queue));
    let uri = match server {
        Server::Socket(_) => format!("ipp://localhost{path}"),
        Server::Host(h, p) => format!("ipp://{}:{p}{path}", bracketed(h)),
    };
    let reply = post(server, &path, &request(&uri, wanted), limit)?;
    let answer = read_answer(&reply)?;
    match answer.status {
        // Successful, perhaps with some attributes left out or changed.
        0..=0x00FF => Ok(Some(answer.printer)),
        NOT_FOUND => Ok(None),
        s => Err(match answer.message {
            Some(m) => anyhow!("the printing service refused to describe the printer: {m}"),
            None => {
                anyhow!("the printing service refused to describe the printer (status {s:#06x})")
            }
        }),
    }
}

/// The printers `server` has queues for, by name, and its default if it has one, asked with
/// CUPS's own CUPS-Get-Printers and CUPS-Get-Default, each given up on after `limit`. These
/// are the printers set up on this computer and those CUPS has already made a queue for;
/// others the network announces aren't among them (`lpstat -e` waits for those).
pub(crate) fn cups_printers(
    server: &Server,
    limit: Duration,
) -> Result<(Vec<String>, Option<String>)> {
    let ask = |operation| -> Result<Answer> {
        let body = request_for(operation, None, &["printer-name"]);
        read_answer(&post(server, "/", &body, limit)?)
    };
    // Each printer is a group of its own; their names come one after another.
    let names = |a: &Answer| -> Vec<String> {
        let named = a.printer.iter().filter(|(n, _)| n == "printer-name");
        named
            .filter_map(|(_, v)| v.first()?.text().map(str::to_string))
            .collect()
    };
    let all = ask(CUPS_GET_PRINTERS)?;
    let listed = match all.status {
        0..=0x00FF => names(&all),
        // No printers.
        NOT_FOUND => Vec::new(),
        s => bail!(
            "the printing service refused to list the printers: {}",
            all.message.unwrap_or_else(|| format!("status {s:#06x}"))
        ),
    };
    let default = ask(CUPS_GET_DEFAULT)?;
    let default = match default.status {
        0..=0x00FF => names(&default).into_iter().next(),
        _ => None,
    };
    Ok((listed, default))
}

/// `h` as it goes in a URI or a Host header: an IPv6 address in brackets.
fn bracketed(h: &str) -> String {
    if h.contains(':') {
        format!("[{h}]")
    } else {
        h.to_string()
    }
}

/// `s` as it goes in a URI's path: letters, digits and `-._~` as they are, the rest as
/// `%XX` bytes.
fn escaped(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A connection to the printing service.
enum Conn {
    #[cfg(unix)]
    Socket(std::os::unix::net::UnixStream),
    Tcp(std::net::TcpStream),
}

impl Conn {
    fn open(server: &Server, limit: Duration) -> Result<Conn> {
        let cannot = |e: std::io::Error| anyhow!("cannot reach the printing service: {e}");
        match server {
            #[cfg(unix)]
            Server::Socket(p) => Ok(Conn::Socket(
                std::os::unix::net::UnixStream::connect(p).map_err(cannot)?,
            )),
            #[cfg(not(unix))]
            Server::Socket(_) => bail!("cannot reach the printing service: no local sockets here"),
            Server::Host(h, port) => {
                use std::net::ToSocketAddrs;
                let addrs = (h.as_str(), *port).to_socket_addrs().map_err(cannot)?;
                let mut last = None;
                for a in addrs {
                    match std::net::TcpStream::connect_timeout(&a, limit) {
                        Ok(s) => return Ok(Conn::Tcp(s)),
                        Err(e) => last = Some(e),
                    }
                }
                Err(cannot(last.unwrap_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "no address")
                })))
            }
        }
    }
    /// Waits on each read and write for at most `d`.
    fn wait_at_most(&self, d: Duration) -> std::io::Result<()> {
        let d = Some(d.max(Duration::from_millis(1)));
        match self {
            #[cfg(unix)]
            Conn::Socket(s) => s.set_read_timeout(d).and(s.set_write_timeout(d)),
            Conn::Tcp(s) => s.set_read_timeout(d).and(s.set_write_timeout(d)),
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Conn::Socket(s) => s.read(buf),
            Conn::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Conn::Socket(s) => s.write(buf),
            Conn::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Conn::Socket(s) => s.flush(),
            Conn::Tcp(s) => s.flush(),
        }
    }
}

/// Posts the IPP request `body` to `path` on `server` and returns the IPP answer, all
/// within `limit`.
fn post(server: &Server, path: &str, body: &[u8], limit: Duration) -> Result<Vec<u8>> {
    let deadline = Instant::now() + limit;
    let left = || deadline.saturating_duration_since(Instant::now());
    let too_slow = || {
        anyhow!(
            "the printing service didn't answer within {} seconds",
            limit.as_secs()
        )
    };
    let host = match server {
        Server::Socket(_) => "localhost".to_string(),
        Server::Host(h, p) => format!("{}:{p}", bracketed(h)),
    };
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/ipp\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut conn = Conn::open(server, limit)?;
    let lost = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => too_slow(),
        _ => anyhow!("the printing service stopped answering: {e}"),
    };
    conn.wait_at_most(left()).map_err(lost)?;
    conn.write_all(head.as_bytes()).map_err(lost)?;
    conn.write_all(body).map_err(lost)?;
    let mut reply = Vec::new();
    let mut buf = [0u8; 16 << 10];
    loop {
        if left().is_zero() {
            return Err(too_slow());
        }
        // macOS refuses to change the waits of a socket whose other end has closed, as CUPS
        // closes it after answering (BUG-48): the wait set before still ends by the deadline,
        // so what is left of the answer is read on.
        match conn.wait_at_most(left()) {
            Err(e) if e.kind() != std::io::ErrorKind::InvalidInput => return Err(lost(e)),
            _ => {}
        }
        let n = match conn.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(lost(e)),
        };
        if n == 0 {
            break;
        }
        reply.extend_from_slice(&buf[..n]);
        if reply.len() > MAX_ANSWER {
            bail!("the printing service's answer is too long");
        }
    }
    http_body(&reply)
}

/// The body of the HTTP answer `reply`, read whole: refused unless it is a success.
fn http_body(reply: &[u8]) -> Result<Vec<u8>> {
    let split = reply.windows(4).position(|w| w == b"\r\n\r\n");
    let split = split.ok_or_else(|| anyhow!("the printing service's answer was cut short"))?;
    let head = String::from_utf8_lossy(&reply[..split]);
    let rest = &reply[split + 4..];
    let mut lines = head.lines();
    let status = lines.next().unwrap_or_default();
    let code = status.split_whitespace().nth(1).unwrap_or_default();
    if code != "200" {
        bail!(
            "the printing service answered “{}”",
            crate::display_line(status, 80)
        );
    }
    let header = |name: &str| {
        head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(name)
                .then(|| v.trim().to_string())
        })
    };
    if header("Transfer-Encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        return dechunked(rest);
    }
    match header("Content-Length").and_then(|v| v.parse::<usize>().ok()) {
        Some(n) if rest.len() >= n => Ok(rest[..n].to_vec()),
        Some(_) => bail!("the printing service's answer was cut short"),
        None => Ok(rest.to_vec()),
    }
}

/// A body sent in chunks, put together.
fn dechunked(mut rest: &[u8]) -> Result<Vec<u8>> {
    let short = || anyhow!("the printing service's answer was cut short");
    let mut out = Vec::new();
    loop {
        let end = rest
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(short)?;
        let size = String::from_utf8_lossy(&rest[..end]);
        let size = size.split(';').next().unwrap_or_default().trim();
        let n = usize::from_str_radix(size, 16).map_err(|_| short())?;
        rest = &rest[end + 2..];
        if n == 0 {
            return Ok(out);
        }
        let chunk = rest.get(..n).ok_or_else(short)?;
        out.extend_from_slice(chunk);
        rest = rest.get(n + 2..).ok_or_else(short)?;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A value written into a made-up answer.
    pub(crate) enum V {
        Kw(&'static str),
        Int(i32),
        Coll(Vec<(&'static str, Vec<V>)>),
    }

    fn put(b: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) {
        b.push(tag);
        b.extend_from_slice(&(name.len() as u16).to_be_bytes());
        b.extend_from_slice(name.as_bytes());
        b.extend_from_slice(&(value.len() as u16).to_be_bytes());
        b.extend_from_slice(value);
    }

    fn put_value(b: &mut Vec<u8>, name: &str, v: &V) {
        match v {
            V::Kw(s) => put(b, 0x44, name, s.as_bytes()),
            V::Int(n) => put(b, INTEGER, name, &n.to_be_bytes()),
            V::Coll(members) => {
                put(b, BEGIN_COLLECTION, name, &[]);
                for (m, vs) in members {
                    put(b, MEMBER_NAME, "", m.as_bytes());
                    for v in vs {
                        put_value(b, "", v);
                    }
                }
                put(b, END_COLLECTION, "", &[]);
            }
        }
    }

    /// A made-up answer with `status` and the printer's `attributes`, as CUPS writes them.
    pub(crate) fn answer(status: u16, attributes: &[(&str, Vec<V>)]) -> Vec<u8> {
        answer_for(status, &[attributes])
    }

    /// A made-up answer with `status` and a group of attributes for each printer.
    fn answer_for(status: u16, printers: &[&[(&str, Vec<V>)]]) -> Vec<u8> {
        let mut b = vec![1, 1];
        b.extend_from_slice(&status.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.push(OPERATION_GROUP);
        put(&mut b, 0x47, "attributes-charset", b"utf-8");
        put(&mut b, 0x48, "attributes-natural-language", b"en");
        put(&mut b, 0x41, "status-message", b"successful-ok");
        for attributes in printers {
            b.push(PRINTER_GROUP);
            for (name, vs) in attributes.iter() {
                for (i, v) in vs.iter().enumerate() {
                    put_value(&mut b, if i == 0 { name } else { "" }, v);
                }
            }
        }
        b.push(END_OF_ATTRIBUTES);
        b
    }

    /// A media size with its margins, as `media-col-database` lists them (hundredths of a
    /// millimetre).
    pub(crate) fn media_col(w: i32, h: i32, margins: [i32; 4]) -> V {
        let size = V::Coll(vec![
            ("x-dimension", vec![V::Int(w)]),
            ("y-dimension", vec![V::Int(h)]),
        ]);
        let [t, r, b, l] = margins;
        V::Coll(vec![
            ("media-size", vec![size]),
            ("media-top-margin", vec![V::Int(t)]),
            ("media-right-margin", vec![V::Int(r)]),
            ("media-bottom-margin", vec![V::Int(b)]),
            ("media-left-margin", vec![V::Int(l)]),
        ])
    }

    #[test]
    fn the_request_names_the_printer_and_what_is_wanted() {
        let b = request(
            "ipp://localhost/printers/Office",
            &["media-supported", "media-default"],
        );
        // IPP/1.1, Get-Printer-Attributes, request 1, the operation group.
        assert_eq!(b[..9], [1, 1, 0, 0x0B, 0, 0, 0, 1, 1]);
        assert_eq!(*b.last().unwrap(), END_OF_ATTRIBUTES);
        let text = String::from_utf8_lossy(&b);
        for part in [
            "attributes-charset",
            "utf-8",
            "printer-uri",
            "ipp://localhost/printers/Office",
        ] {
            assert!(text.contains(part), "{part}");
        }
        // The second value of requested-attributes: a keyword with an empty name.
        let second = b"\x44\0\0\0\x0dmedia-default";
        assert!(b.windows(second.len()).any(|w| w == second), "{b:?}");
    }

    #[test]
    fn an_answer_is_read_with_its_collections() {
        let bytes = answer(
            0,
            &[
                (
                    "media-supported",
                    vec![V::Kw("na_letter_8.5x11in"), V::Kw("iso_a4_210x297mm")],
                ),
                ("media-default", vec![V::Kw("na_letter_8.5x11in")]),
                (
                    "media-col-database",
                    vec![media_col(21590, 27940, [423, 423, 423, 423])],
                ),
            ],
        );
        let a = read_answer(&bytes).unwrap();
        assert_eq!(a.status, 0);
        assert_eq!(a.message.as_deref(), Some("successful-ok"));
        let media: Vec<_> = values(&a.printer, "media-supported")
            .iter()
            .filter_map(Value::text)
            .collect();
        assert_eq!(media, ["na_letter_8.5x11in", "iso_a4_210x297mm"]);
        let col = values(&a.printer, "media-col-database")[0]
            .members()
            .unwrap();
        let size = values(col, "media-size")[0].members().unwrap();
        assert_eq!(values(size, "x-dimension")[0].int(), Some(21590));
        assert_eq!(values(col, "media-left-margin")[0].int(), Some(423));
        assert!(values(&a.printer, "printer-name").is_empty());
    }

    #[test]
    fn a_broken_answer_is_refused_without_panicking() {
        let good = answer(0, &[("media-col-database", vec![media_col(1, 2, [0; 4])])]);
        // Every cut is refused.
        for n in 0..good.len() {
            assert!(read_answer(&good[..n]).is_err(), "cut at {n}");
        }
        // Collections nested past the limit.
        let mut deep = V::Int(1);
        for _ in 0..20 {
            deep = V::Coll(vec![("m", vec![deep])]);
        }
        let e = read_answer(&answer(0, &[("x", vec![deep])])).unwrap_err();
        assert!(e.to_string().contains("nests too deeply"), "{e}");
        // A value without a name at the start of a group.
        let mut b = answer(0, &[]);
        b.pop();
        put(&mut b, 0x44, "", b"loose");
        b.push(END_OF_ATTRIBUTES);
        assert!(read_answer(&b).is_err());
    }

    #[test]
    fn the_service_is_found_as_cups_finds_it() {
        let none = |_: &Path| false;
        let socket = |p: &Path| p == Path::new("/var/run/cups/cups.sock");
        assert_eq!(
            server_from(None, none),
            Server::Host("localhost".into(), 631)
        );
        assert_eq!(
            server_from(None, socket),
            Server::Socket("/var/run/cups/cups.sock".into())
        );
        assert_eq!(
            server_from(Some("/tmp/cups.sock"), none),
            Server::Socket("/tmp/cups.sock".into())
        );
        assert_eq!(
            server_from(Some("print.example:8631"), socket),
            Server::Host("print.example".into(), 8631)
        );
        assert_eq!(
            server_from(Some("print.example/version=1.1"), none),
            Server::Host("print.example".into(), 631)
        );
        assert_eq!(
            server_from(Some("[::1]:632"), none),
            Server::Host("::1".into(), 632)
        );
        assert_eq!(server_from(Some("  "), socket), server_from(None, socket));
        let conf = "# a comment\nEncryption IfRequested\nservername  print.example:631\n";
        assert_eq!(server_name(conf).as_deref(), Some("print.example:631"));
        assert_eq!(server_name("ServerName\n"), None);
    }

    #[test]
    fn a_queue_name_is_escaped_in_its_uri() {
        assert_eq!(escaped("Office Printer"), "Office%20Printer");
        assert_eq!(escaped("HP_LaserJet-1.0~x"), "HP_LaserJet-1.0~x");
        assert_eq!(escaped("Büro"), "B%C3%BCro");
        assert_eq!(bracketed("::1"), "[::1]");
    }

    #[test]
    fn http_bodies_are_read_by_length_or_in_chunks() {
        let by_length = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcXYZ";
        assert_eq!(http_body(by_length).unwrap(), b"abc");
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n";
        assert_eq!(http_body(chunked).unwrap(), b"abcde");
        let e = http_body(b"HTTP/1.1 403 Forbidden\r\n\r\n").unwrap_err();
        assert!(e.to_string().contains("403 Forbidden"), "{e}");
        assert!(http_body(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nabc").is_err());
        assert!(
            http_body(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n9\r\nabc").is_err()
        );
        assert!(http_body(b"HTTP/1.1 200 OK\r\n").is_err());
    }

    /// A stand-in printing service on a socket of its own, answering one request with
    /// `reply` after `delay`; it gives back the request it was sent.
    #[cfg(unix)]
    fn stand_in(
        name: &str,
        reply: Vec<u8>,
        delay: Duration,
    ) -> (Server, std::thread::JoinHandle<Vec<u8>>) {
        let (server, serve) = stand_in_for(name, vec![reply], delay);
        let one = std::thread::spawn(move || serve.join().unwrap().remove(0));
        (server, one)
    }

    /// A stand-in printing service answering a request with each of `replies` in turn, each
    /// after `delay`; it gives back the requests it was sent.
    #[cfg(unix)]
    fn stand_in_for(
        name: &str,
        replies: Vec<Vec<u8>>,
        delay: Duration,
    ) -> (Server, std::thread::JoinHandle<Vec<Vec<u8>>>) {
        use std::os::unix::net::UnixListener;
        let path =
            std::env::temp_dir().join(format!("limarepdf-ipp-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let done = path.clone();
        let serve = std::thread::spawn(move || {
            let mut got = Vec::new();
            for reply in replies {
                let (mut s, _) = listener.accept().unwrap();
                got.push(take_request(&mut s));
                std::thread::sleep(delay);
                let _ = s.write_all(&reply);
            }
            let _ = std::fs::remove_file(&done);
            got
        });
        (Server::Socket(path), serve)
    }

    /// The printers are listed with CUPS-Get-Printers, a group for each, and the default
    /// asked with CUPS-Get-Default, neither naming a printer; no printers, and no default,
    /// are said so.
    #[cfg(unix)]
    #[test]
    fn the_printers_are_listed_with_their_default() {
        let name = |n: &'static str| [("printer-name", vec![V::Kw(n)])];
        let (office, hall) = (name("Office_laser"), name("Hall inkjet"));
        let listed = answer_for(0, &[&office, &hall]);
        let default = answer(0, &[("printer-name", vec![V::Kw("Hall inkjet")])]);
        let (server, serve) =
            stand_in_for("list", vec![http(&listed), http(&default)], Duration::ZERO);
        let got = cups_printers(&server, Duration::from_secs(5)).unwrap();
        assert_eq!(got.0, ["Office_laser", "Hall inkjet"]);
        assert_eq!(got.1.as_deref(), Some("Hall inkjet"));
        let sent = serve.join().unwrap();
        for (request, operation) in sent.iter().zip([CUPS_GET_PRINTERS, CUPS_GET_DEFAULT]) {
            let text = String::from_utf8_lossy(request);
            assert!(text.starts_with("POST / HTTP/1.1\r\n"), "{text}");
            assert!(!text.contains("printer-uri"), "{text}");
            let at = text.find("\r\n\r\n").unwrap() + 4;
            assert_eq!(request[at + 2..at + 4], operation.to_be_bytes());
        }
        let none = http(&answer(NOT_FOUND, &[]));
        let (server, serve) = stand_in_for("none", vec![none.clone(), none], Duration::ZERO);
        let got = cups_printers(&server, Duration::from_secs(5)).unwrap();
        assert_eq!(got, (vec![], None));
        serve.join().unwrap();
    }

    /// A request read from `s`: the head, then as much body as it says.
    #[cfg(unix)]
    fn take_request(s: &mut std::os::unix::net::UnixStream) -> Vec<u8> {
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = s.read(&mut buf) {
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&got).to_string();
            if let Some(at) = text.find("\r\n\r\n") {
                let len = text
                    .lines()
                    .find_map(|l| l.strip_prefix("Content-Length: "))
                    .and_then(|n| n.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if got.len() >= at + 4 + len {
                    break;
                }
            }
        }
        got
    }

    /// An answer longer than a socket holds, sent a piece at a time, the service closing its
    /// end the moment the last piece is sent, as CUPS does after answering: read whole. macOS
    /// refuses to change the waits of a socket whose other end has closed, so the client gave
    /// up on CUPS's answers there part way (BUG-48, found by PR #79's macOS CI).
    #[cfg(unix)]
    #[test]
    fn an_answer_the_service_closes_after_is_read_whole() {
        use std::os::unix::net::UnixListener;
        let sizes: Vec<V> = (0..5000).map(|_| V::Kw("iso_a4_210x297mm")).collect();
        let reply = http(&answer(0, &[("media-supported", sizes)]));
        let path =
            std::env::temp_dir().join(format!("limarepdf-ipp-close-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let serve = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            take_request(&mut s);
            for piece in reply.chunks(1000) {
                if s.write_all(piece).is_err() {
                    break;
                }
            }
            // Closed at once.
            drop(s);
        });
        let got = printer_attributes(
            &Server::Socket(path.clone()),
            "Office",
            &["media-supported"],
            Duration::from_secs(5),
        );
        serve.join().unwrap();
        let _ = std::fs::remove_file(&path);
        let attrs = got.unwrap().unwrap();
        assert_eq!(values(&attrs, "media-supported").len(), 5000);
    }

    #[cfg(unix)]
    fn http(body: &[u8]) -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(body);
        r
    }

    #[cfg(unix)]
    #[test]
    fn a_printer_is_asked_about_over_the_socket() {
        let body = answer(0, &[("media-default", vec![V::Kw("iso_a4_210x297mm")])]);
        let (server, serve) = stand_in("ask", http(&body), Duration::ZERO);
        let got = printer_attributes(
            &server,
            "Office Printer",
            &["media-default"],
            Duration::from_secs(5),
        );
        let attrs = got.unwrap().unwrap();
        assert_eq!(
            values(&attrs, "media-default")[0].text(),
            Some("iso_a4_210x297mm")
        );
        let sent = serve.join().unwrap();
        let sent = String::from_utf8_lossy(&sent);
        assert!(
            sent.starts_with("POST /printers/Office%20Printer HTTP/1.1\r\n"),
            "{sent}"
        );
        assert!(sent.contains("Host: localhost\r\n") && sent.contains("application/ipp"));
        assert!(
            sent.contains("ipp://localhost/printers/Office%20Printer"),
            "{sent}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_printer_the_service_doesnt_know_is_none() {
        let (server, serve) = stand_in("unknown", http(&answer(NOT_FOUND, &[])), Duration::ZERO);
        let got = printer_attributes(&server, "Gone", &["media-default"], Duration::from_secs(5));
        assert_eq!(got.unwrap(), None);
        serve.join().unwrap();
        let (server, serve) = stand_in("refused", http(&answer(0x0401, &[])), Duration::ZERO);
        let e = printer_attributes(
            &server,
            "Locked",
            &["media-default"],
            Duration::from_secs(5),
        );
        let e = e.unwrap_err().to_string();
        assert!(e.contains("refused to describe the printer"), "{e}");
        serve.join().unwrap();
    }

    /// A service that takes the request and never answers is given up on at the limit.
    #[cfg(unix)]
    #[test]
    fn a_service_that_never_answers_is_given_up_on() {
        let (server, serve) = stand_in("slow", http(&answer(0, &[])), Duration::from_millis(1500));
        let start = Instant::now();
        let e = printer_attributes(
            &server,
            "Slow",
            &["media-default"],
            Duration::from_millis(300),
        );
        let took = start.elapsed();
        let e = e.unwrap_err().to_string();
        assert!(e.contains("didn't answer within"), "{e}");
        assert!(took < Duration::from_millis(1200), "{took:?}");
        serve.join().unwrap();
        // A socket no service listens on.
        let gone = Server::Socket(std::env::temp_dir().join("limarepdf-ipp-no-such.sock"));
        let e = printer_attributes(&gone, "X", &["media-default"], Duration::from_secs(1));
        let e = e.unwrap_err().to_string();
        assert!(e.contains("cannot reach the printing service"), "{e}");
    }
}
