//! Asking the host's router to send the invite's port to this machine, so
//! people outside its network can join without a VPN.
//!
//! This is UPnP, which most home routers answer. The router is found on the
//! local network, asked for the address it has on the internet, and asked to
//! pass one TCP port on for an hour at a time. Nothing beyond the local
//! network is spoken to, and nothing is asked of a router that does not hold
//! a public address itself: behind an internet provider's shared address, or
//! a second router, the port would open onto nobody.
//!
//! Everything a router says is read with a limit and believed no further
//! than this needs. Runs on the host's reach thread.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Where routers listen to be found.
const SEARCH: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);

/// What a router that can pass a port on calls itself, in both versions.
const ROUTERS: [&str; 2] = [
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
    "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
];

/// The two services that pass ports on. Which one a router has depends on
/// how it reaches the internet.
const SERVICES: [&str; 2] = [
    "urn:schemas-upnp-org:service:WANIPConnection:",
    "urn:schemas-upnp-org:service:WANPPPConnection:",
];

/// How long routers have to answer the search, and how long after the first
/// answer a second one is waited for.
const SEARCHING: Duration = Duration::from_secs(2);
const STRAGGLERS: Duration = Duration::from_millis(300);

/// How long a router has to answer one question.
const PATIENCE: Duration = Duration::from_secs(2);

/// The most of a router's answer that is read.
const ANSWER_LIMIT: u64 = 256 << 10;

/// How long a port is asked for at a time, in seconds. Asked for again well
/// before it runs out, so a Lumit that went down leaves it open an hour at
/// most.
const LEASE: u32 = 3600;

/// How often an open port is asked for again.
pub(crate) const RENEW: Duration = Duration::from_secs(20 * 60);

/// What a router says when it only passes ports on for good.
const ONLY_FOR_GOOD: u16 = 725;

/// Why no port was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Closed {
    /// No router answered, or the one that did would not.
    Refused,
    /// The router's own address is not on the internet.
    Behind,
}

/// A router that is passing a port on to this machine.
#[derive(Debug)]
pub(crate) struct Mapping {
    router: SocketAddr,
    /// Where on the router its port service is asked, and what it is called.
    control: String,
    service: String,
    port: u16,
    /// This machine's address on the router's network.
    here: IpAddr,
    lease: u32,
}

/// Find a router and have it pass `port` on to this machine. Answers the
/// mapping, to renew and to close, and the router's address on the internet.
/// Gives up early once `stop` is set.
pub(crate) fn open(port: u16, stop: &AtomicBool) -> Result<(Mapping, Ipv4Addr), Closed> {
    let mut why = Closed::Refused;
    for (router, description) in search(stop) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match open_at(router, &description, port) {
            Ok(open) => return Ok(open),
            Err(Closed::Behind) => why = Closed::Behind,
            Err(Closed::Refused) => {}
        }
    }
    Err(why)
}

/// Ask the router at `router`, which describes itself at `description`.
fn open_at(
    router: SocketAddr,
    description: &str,
    port: u16,
) -> Result<(Mapping, Ipv4Addr), Closed> {
    let (control, service) = describe(router, description).ok_or(Closed::Refused)?;
    // The address this machine has towards the router. Connecting a UDP
    // socket sends nothing: it asks the system which interface it would use.
    let here = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|socket| {
            socket.connect(router)?;
            socket.local_addr()
        })
        .map_err(|_| Closed::Refused)?
        .ip();
    let mut mapping = Mapping {
        router,
        control,
        service,
        port,
        here,
        lease: LEASE,
    };
    let answer = mapping.ask("GetExternalIPAddress", &[]);
    let outside = answer
        .ok()
        .and_then(|answer| field(&answer, "NewExternalIPAddress"))
        .and_then(|address| address.trim().parse::<Ipv4Addr>().ok())
        .ok_or(Closed::Refused)?;
    if !public(outside) {
        return Err(Closed::Behind);
    }
    match mapping.add() {
        Ok(()) => {}
        Err(ONLY_FOR_GOOD) => {
            mapping.lease = 0;
            mapping.add().map_err(|_| Closed::Refused)?;
        }
        Err(_) => return Err(Closed::Refused),
    }
    Ok((mapping, outside))
}

impl Mapping {
    /// Ask the router to pass the port on, for the first time or again.
    fn add(&self) -> Result<(), u16> {
        let (port, here, lease) = (
            self.port.to_string(),
            self.here.to_string(),
            self.lease.to_string(),
        );
        let asked = [
            ("NewRemoteHost", ""),
            ("NewExternalPort", port.as_str()),
            ("NewProtocol", "TCP"),
            ("NewInternalPort", port.as_str()),
            ("NewInternalClient", here.as_str()),
            ("NewEnabled", "1"),
            ("NewPortMappingDescription", "Lumit shared project"),
            ("NewLeaseDuration", lease.as_str()),
        ];
        self.ask("AddPortMapping", &asked).map(|_| ())
    }

    /// Keep the port open for another stretch. False when the router would
    /// not.
    pub(crate) fn renew(&self) -> bool {
        self.add().is_ok()
    }

    /// Have the router stop passing the port on.
    pub(crate) fn close(&self) {
        let port = self.port.to_string();
        let asked = [
            ("NewRemoteHost", ""),
            ("NewExternalPort", port.as_str()),
            ("NewProtocol", "TCP"),
        ];
        let _ = self.ask("DeletePortMapping", &asked);
    }

    /// Ask the router's port service one thing. Answers what it said, or the
    /// number of its refusal, 0 when it gave none.
    fn ask(&self, action: &str, asked: &[(&str, &str)]) -> Result<String, u16> {
        let service = &self.service;
        let fields: String = asked
            .iter()
            .map(|(name, value)| format!("<{name}>{value}</{name}>"))
            .collect();
        let body = format!(
            "<?xml version=\"1.0\"?>\
             <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
             s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
             <u:{action} xmlns:u=\"{service}\">{fields}</u:{action}>\
             </s:Body></s:Envelope>"
        );
        let headers = format!(
            "Content-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{service}#{action}\"\r\n"
        );
        let (status, answer) =
            http(self.router, "POST", &self.control, &headers, &body).ok_or(0u16)?;
        if status == 200 {
            return Ok(answer);
        }
        let refusal = field(&answer, "errorCode").and_then(|code| code.trim().parse().ok());
        Err(refusal.unwrap_or(0))
    }
}

/// Whether an address is one the internet can reach. A router that holds
/// anything else is behind another, or behind a provider's shared address.
fn public(address: Ipv4Addr) -> bool {
    let [a, b, ..] = address.octets();
    let shared = a == 100 && (64..128).contains(&b);
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_broadcast()
        || shared)
}

/// Call out for routers on the local network. Answers each one's address and
/// where it describes itself, in the order they answered.
fn search(stop: &AtomicBool) -> Vec<(SocketAddr, String)> {
    let mut found: Vec<(SocketAddr, String)> = Vec::new();
    // From the interface the default route leaves by, which is the one the
    // router is on.
    let here = crate::local_address()
        .parse()
        .unwrap_or(Ipv4Addr::UNSPECIFIED);
    let Ok(socket) = UdpSocket::bind((here, 0)) else {
        return found;
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
    for router in ROUTERS {
        let call = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: {SEARCH}\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: {router}\r\n\r\n"
        );
        let _ = socket.send_to(call.as_bytes(), SEARCH);
    }
    let mut until = Instant::now() + SEARCHING;
    let mut answer = [0u8; 2048];
    while Instant::now() < until && !stop.load(Ordering::Relaxed) {
        let Ok((len, from)) = socket.recv_from(&mut answer) else {
            continue;
        };
        let text = String::from_utf8_lossy(answer.get(..len).unwrap_or_default());
        let location = text.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("location")
                .then(|| value.trim())
        });
        let Some((router, description)) = location.and_then(place) else {
            continue;
        };
        // A router describes itself, and nothing else on the network gets to
        // send this machine asking somewhere of its choosing.
        if router.ip() != from.ip() || found.iter().any(|(known, _)| *known == router) {
            continue;
        }
        if found.is_empty() {
            until = until.min(Instant::now() + STRAGGLERS);
        }
        found.push((router, description));
    }
    found
}

/// The address and the path of an `http://address:port/path`.
fn place(url: &str) -> Option<(SocketAddr, String)> {
    let rest = url.strip_prefix("http://")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let address = host
        .parse::<SocketAddr>()
        .or_else(|_| host.parse::<IpAddr>().map(|ip| SocketAddr::new(ip, 80)))
        .ok()?;
    Some((address, format!("/{path}")))
}

/// Read how a router describes itself, for where its port service is asked
/// and what that service is called.
fn describe(router: SocketAddr, description: &str) -> Option<(String, String)> {
    let (status, text) = http(router, "GET", description, "", "")?;
    if status != 200 {
        return None;
    }
    let document = roxmltree::Document::parse(&text).ok()?;
    let child = |node: roxmltree::Node<'_, '_>, name: &str| {
        let found = node.children().find(|c| c.tag_name().name() == name);
        found
            .and_then(|c| c.text())
            .map(str::trim)
            .map(str::to_owned)
    };
    let services = document
        .descendants()
        .filter(|node| node.tag_name().name() == "service");
    for service in services {
        let Some(kind) = child(service, "serviceType") else {
            continue;
        };
        if !SERVICES.iter().any(|known| kind.starts_with(known)) {
            continue;
        }
        let Some(control) = child(service, "controlURL") else {
            continue;
        };
        // Written whole or from the router's root. Whole, it has to be this
        // router.
        let control = match place(&control) {
            Some((at, path)) if at == router => path,
            Some(_) => continue,
            None if control.starts_with('/') => control,
            None => format!("/{control}"),
        };
        return Some((control, kind));
    }
    None
}

/// The words inside the first `<name>` anywhere in `xml`.
fn field(xml: &str, name: &str) -> Option<String> {
    let Ok(document) = roxmltree::Document::parse(xml) else {
        // Some routers answer in XML that is not quite XML. The one plain
        // value wanted from it can still be read.
        let (_, rest) = xml.split_once(&format!("<{name}>"))?;
        let (value, _) = rest.split_once(&format!("</{name}>"))?;
        return Some(value.to_owned());
    };
    let found = document
        .descendants()
        .find(|node| node.tag_name().name() == name)?;
    found.text().map(str::to_owned)
}

/// One question to a router and its answer: the status and the body.
fn http(
    router: SocketAddr,
    method: &str,
    path: &str,
    headers: &str,
    body: &str,
) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect_timeout(&router, PATIENCE).ok()?;
    stream.set_read_timeout(Some(PATIENCE)).ok()?;
    stream.set_write_timeout(Some(PATIENCE)).ok()?;
    let length = body.len();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {router}\r\nConnection: close\r\n\
         Content-Length: {length}\r\n{headers}\r\n{body}"
    );
    stream.write_all(request.as_bytes()).ok()?;

    let mut reader = BufReader::new(stream.take(ANSWER_LIMIT));
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let status = line.split_whitespace().nth(1)?.parse().ok()?;
    let (mut length, mut chunked) = (None, false);
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 || line.trim().is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        match name.as_str() {
            "content-length" => length = value.parse::<u64>().ok(),
            "transfer-encoding" => chunked = value.eq_ignore_ascii_case("chunked"),
            _ => {}
        }
    }
    let mut answer = Vec::new();
    if chunked {
        loop {
            line.clear();
            if reader.read_line(&mut line).ok()? == 0 {
                break;
            }
            let size = line.trim().split(';').next().unwrap_or_default();
            let Ok(size) = u64::from_str_radix(size, 16) else {
                break;
            };
            if size == 0 {
                break;
            }
            (&mut reader).take(size).read_to_end(&mut answer).ok()?;
            // The line end after each piece.
            line.clear();
            let _ = reader.read_line(&mut line);
        }
    } else {
        // A router that keeps the connection open past what it promised is
        // waited on no longer than its patience, and what came is kept.
        let _ = match length {
            Some(length) => (&mut reader).take(length).read_to_end(&mut answer),
            None => reader.read_to_end(&mut answer),
        };
    }
    Some((status, String::from_utf8_lossy(&answer).into_owned()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A router as this needs one: it describes itself, says what address
    /// it has outside, and answers the two things asked of its ports. What
    /// it was asked is kept.
    fn router(
        outside: &'static str,
        refuses_leases: bool,
    ) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let at = listener.local_addr().unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let heard = asked.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut head, mut line, mut length) = (String::new(), String::new(), 0usize);
                while reader.read_line(&mut line).unwrap_or(0) > 0 && line.trim() != "" {
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    head.push_str(&line);
                    line.clear();
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).unwrap();
                let body = String::from_utf8(body).unwrap();
                heard.lock().push(format!("{head}{body}"));

                let (status, answer) = if head.starts_with("GET /desc.xml") {
                    let said = format!(
                        "<root><device><deviceList><device><serviceList><service>\
                         <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>\
                         <controlURL>http://{at}/ctl/ip</controlURL>\
                         </service></serviceList></device></deviceList></device></root>"
                    );
                    ("200 OK", said)
                } else if body.contains("GetExternalIPAddress") {
                    let said = format!(
                        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:GetExternalIPAddressResponse xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">\
                         <NewExternalIPAddress>{outside}</NewExternalIPAddress>\
                         </u:GetExternalIPAddressResponse></s:Body></s:Envelope>"
                    );
                    ("200 OK", said)
                } else if refuses_leases && body.contains("<NewLeaseDuration>3600<") {
                    let said = "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><detail><UPnPError>\
                                <errorCode>725</errorCode></UPnPError></detail></s:Fault>\
                                </s:Body></s:Envelope>";
                    ("500 Internal Server Error", said.to_owned())
                } else {
                    ("200 OK", "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body/></s:Envelope>".to_owned())
                };
                // In pieces, as some routers send it.
                let (first, rest) = answer.split_at(answer.len() / 2);
                let said = format!(
                    "HTTP/1.1 {status}\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{first}\r\n{:x}\r\n{rest}\r\n0\r\n\r\n",
                    first.len(),
                    rest.len()
                );
                let _ = stream.write_all(said.as_bytes());
            }
        });
        (at, asked)
    }

    /// A router with a public address is asked for the port, to this
    /// machine, and to close it again afterwards. One that only passes
    /// ports on for good is asked again that way.
    #[test]
    fn a_router_is_asked_to_pass_the_port_on_and_to_stop() {
        for refuses_leases in [false, true] {
            let (at, asked) = router("203.0.113.7", refuses_leases);
            let (mapping, outside) = open_at(at, "/desc.xml", 47856).unwrap();
            assert_eq!(outside, Ipv4Addr::new(203, 0, 113, 7));
            assert_eq!(mapping.lease, if refuses_leases { 0 } else { LEASE });
            assert!(mapping.renew());
            mapping.close();

            let asked = asked.lock();
            let added = asked
                .iter()
                .rfind(|a| a.contains("#AddPortMapping"))
                .unwrap();
            assert!(added.starts_with("POST /ctl/ip "));
            for part in [
                "<NewExternalPort>47856</NewExternalPort>",
                "<NewInternalPort>47856</NewInternalPort>",
                "<NewInternalClient>127.0.0.1</NewInternalClient>",
                "<NewProtocol>TCP</NewProtocol>",
            ] {
                assert!(added.contains(part), "{part}");
            }
            assert!(asked.last().unwrap().contains("#DeletePortMapping"));
        }
    }

    /// A router that is itself behind another address is not asked for a
    /// port at all: it would open onto nobody.
    #[test]
    fn a_router_with_no_public_address_is_left_alone() {
        for inside in ["192.168.0.2", "10.1.2.3", "100.64.0.9"] {
            let (at, asked) = router(inside, false);
            assert_eq!(open_at(at, "/desc.xml", 47856).unwrap_err(), Closed::Behind);
            assert!(!asked.lock().iter().any(|a| a.contains("AddPortMapping")));
        }
    }
}
