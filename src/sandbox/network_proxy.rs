//! HTTP CONNECT proxy for domain-based network restrictions.
//!
//! Runs as a host-resident proxy alongside the RPC server. Containers set
//! `HTTPS_PROXY` / `HTTP_PROXY` env vars to route all outbound HTTPS through
//! this proxy. The proxy verifies auth, checks domain allowlist, resolves DNS
//! on the host side (rejecting private IPs), and tunnels traffic.
//!
//! Combined with iptables inside the container (default-deny egress, only
//! allow proxy and RPC ports), this prevents the sandbox from accessing
//! unapproved destinations even if it ignores the proxy env vars.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;
use tracing::{debug, warn};

use crate::sandbox::rpc::generate_token;

/// Maximum concurrent proxy connections. Matches RPC server cap.
const MAX_CONNECTIONS: usize = 16;

/// Maximum size of the CONNECT request (line + headers).
/// Prevents memory exhaustion from oversized requests.
const MAX_REQUEST_SIZE: usize = 8 * 1024;

/// Timeout for reading the initial CONNECT request (prevents slowloris).
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout for connecting to the upstream target.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The only destination port allowed via CONNECT.
const ALLOWED_PORT: u16 = 443;

/// HTTP CONNECT proxy server with domain allowlist and private IP rejection.
pub struct NetworkProxy {
    listener: TcpListener,
    port: u16,
    token: String,
    allowed_domains: Vec<String>,
}

/// Handle to a running proxy server thread.
pub struct ProxyHandle {
    _handle: thread::JoinHandle<()>,
}

impl NetworkProxy {
    /// Bind to a random port on all interfaces (same as RPC server).
    pub fn bind(allowed_domains: &[String]) -> Result<Self> {
        let listener =
            TcpListener::bind("0.0.0.0:0").context("Failed to bind network proxy listener")?;
        let port = listener.local_addr()?.port();
        let token = generate_token();
        debug!(port, "network proxy bound");
        Ok(Self {
            listener,
            port,
            token,
            allowed_domains: allowed_domains.to_vec(),
        })
    }

    /// Get the port the proxy is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get the auth token for this proxy session.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Spawn the proxy accept loop in a background thread.
    pub fn spawn(self) -> ProxyHandle {
        let ctx = Arc::new(ProxyContext {
            token: self.token,
            allowed_domains: self.allowed_domains,
        });
        let active = Arc::new(AtomicUsize::new(0));

        let handle = thread::spawn(move || {
            for stream in self.listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let current = active.load(Ordering::Relaxed);
                        if current >= MAX_CONNECTIONS {
                            warn!(current, "proxy connection limit reached, dropping");
                            drop(stream);
                            continue;
                        }
                        active.fetch_add(1, Ordering::Relaxed);
                        let ctx = Arc::clone(&ctx);
                        let active = Arc::clone(&active);
                        thread::spawn(move || {
                            if let Err(e) = handle_proxy_connection(stream, &ctx) {
                                debug!(error = %e, "proxy connection ended");
                            }
                            active.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(e) => {
                        debug!(error = %e, "proxy accept error, shutting down");
                        break;
                    }
                }
            }
        });

        ProxyHandle { _handle: handle }
    }
}

/// Shared context for proxy connection handlers.
struct ProxyContext {
    token: String,
    allowed_domains: Vec<String>,
}

/// Check if a domain matches a pattern (case-insensitive).
///
/// Supports exact match and wildcard prefix (`*.example.com` matches
/// `foo.example.com` and `example.com` itself).
fn domain_matches(domain: &str, pattern: &str) -> bool {
    let domain = domain.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    if let Some(suffix) = pattern.strip_prefix('*') {
        // suffix is ".example.com"
        domain.ends_with(&suffix) || domain == suffix.strip_prefix('.').unwrap_or(suffix)
    } else {
        domain == pattern
    }
}

/// Check if an IP address is private/reserved and should be blocked.
fn is_private_ip(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.is_multicast()
                // CGNAT range 100.64.0.0/10
                || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xC0) == 64)
        }
        IpAddr::V6(ip) => {
            // Check IPv4-mapped addresses (::ffff:x.x.x.x)
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_private_ip(&IpAddr::V4(mapped));
            }
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                // ULA fc00::/7
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                // Link-local fe80::/10
                || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Constant-time byte comparison to prevent timing side-channel attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Parse and handle a single proxy connection.
fn handle_proxy_connection(stream: TcpStream, ctx: &ProxyContext) -> Result<()> {
    let peer = stream.peer_addr().ok();
    debug!(?peer, "proxy connection accepted");

    // Set auth timeout to prevent slowloris
    stream.set_read_timeout(Some(AUTH_TIMEOUT))?;

    let mut reader = BufReader::new(&stream);
    let mut writer = stream.try_clone().context("Failed to clone proxy stream")?;

    // Read all headers (bounded)
    let mut total_read = 0usize;
    let mut request_line = String::new();
    let mut proxy_auth: Option<String> = None;

    // Read request line
    let n = reader.read_line(&mut request_line)?;
    total_read += n;
    if total_read > MAX_REQUEST_SIZE {
        write_error(&mut writer, 400, "Request too large")?;
        return Ok(());
    }

    // Read headers until empty line
    loop {
        let mut header_line = String::new();
        let n = reader.read_line(&mut header_line)?;
        total_read += n;
        if total_read > MAX_REQUEST_SIZE {
            write_error(&mut writer, 400, "Request too large")?;
            return Ok(());
        }

        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break;
        }

        // Parse Proxy-Authorization header
        if let Some(value) = trimmed.strip_prefix("Proxy-Authorization:") {
            proxy_auth = Some(value.trim().to_string());
        }
    }

    // Parse CONNECT request line: "CONNECT host:port HTTP/1.1\r\n"
    let request_line = request_line.trim();
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 || parts[0] != "CONNECT" {
        write_error(&mut writer, 400, "Expected CONNECT method")?;
        return Ok(());
    }

    let target = parts[1];
    let (hostname, port) = parse_host_port(target)?;

    debug!(hostname, port, "CONNECT request");

    // Verify auth token
    let expected = format!("Basic {}", base64_encode(&format!("workmux:{}", ctx.token)));
    match proxy_auth {
        None => {
            debug!(hostname, "proxy auth missing");
            write_error(&mut writer, 407, "Proxy authentication required")?;
            return Ok(());
        }
        Some(ref auth) if !constant_time_eq(auth.as_bytes(), expected.as_bytes()) => {
            debug!(hostname, "proxy auth failed");
            write_error(&mut writer, 407, "Invalid proxy credentials")?;
            return Ok(());
        }
        _ => {}
    }

    // Clear read timeout for authenticated tunneling
    stream.set_read_timeout(None)?;

    // Reject non-443 ports
    if port != ALLOWED_PORT {
        debug!(hostname, port, "rejected: non-443 port");
        write_error(&mut writer, 403, "Only port 443 is allowed")?;
        return Ok(());
    }

    // Normalize hostname
    let hostname = hostname.to_ascii_lowercase();
    let hostname = hostname.strip_suffix('.').unwrap_or(&hostname);

    // Reject IP literal hostnames
    if hostname.parse::<IpAddr>().is_ok() {
        debug!(hostname, "rejected: IP literal hostname");
        write_error(&mut writer, 403, "IP literal hostnames not allowed")?;
        return Ok(());
    }

    // Check domain allowlist
    let allowed = ctx
        .allowed_domains
        .iter()
        .any(|pattern| domain_matches(hostname, pattern));
    if !allowed {
        debug!(hostname, "rejected: domain not in allowlist");
        write_error(&mut writer, 403, "Domain not allowed")?;
        return Ok(());
    }

    // Resolve DNS on host side
    let addrs: Vec<SocketAddr> = match format!("{}:{}", hostname, port).to_socket_addrs() {
        Ok(addrs) => addrs.collect(),
        Err(e) => {
            debug!(hostname, error = %e, "DNS resolution failed");
            write_error(&mut writer, 502, "DNS resolution failed")?;
            return Ok(());
        }
    };

    // Filter out private IPs
    let public_addrs: Vec<&SocketAddr> = addrs.iter().filter(|a| !is_private_ip(&a.ip())).collect();

    if public_addrs.is_empty() {
        debug!(hostname, "rejected: all resolved IPs are private");
        write_error(&mut writer, 403, "All resolved IPs are private")?;
        return Ok(());
    }

    // Connect to first public IP (TOCTOU-safe: use validated SocketAddr directly)
    let target_addr = *public_addrs[0];
    let target_stream = match TcpStream::connect_timeout(&target_addr, CONNECT_TIMEOUT) {
        Ok(s) => s,
        Err(e) => {
            debug!(hostname, addr = %target_addr, error = %e, "connect failed");
            write_error(&mut writer, 502, "Connection to target failed")?;
            return Ok(());
        }
    };

    debug!(hostname, addr = %target_addr, "tunnel established");

    // Send 200 Connection Established
    writer.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    writer.flush()?;

    // Bidirectional tunnel
    tunnel(reader.into_inner(), &target_stream)?;

    Ok(())
}

/// Parse "host:port" from CONNECT target.
fn parse_host_port(target: &str) -> Result<(&str, u16)> {
    // Handle IPv6 literals like [::1]:443
    if let Some(bracket_end) = target.find(']') {
        let host = &target[..=bracket_end];
        let port_str = target[bracket_end + 1..].strip_prefix(':').unwrap_or("443");
        let port: u16 = port_str.parse().context("Invalid port")?;
        return Ok((host, port));
    }

    match target.rsplit_once(':') {
        Some((host, port_str)) => {
            let port: u16 = port_str.parse().context("Invalid port")?;
            Ok((host, port))
        }
        None => Ok((target, 443)),
    }
}

/// Write an HTTP error response.
fn write_error(writer: &mut impl Write, code: u16, reason: &str) -> Result<()> {
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\n\r\n{}",
        code,
        reason,
        reason.len(),
        reason,
    );
    writer.write_all(response.as_bytes())?;
    writer.flush()?;
    Ok(())
}

/// Simple base64 encoding (avoids adding a dependency for this one use).
fn base64_encode(input: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        result.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}

/// Bidirectional byte tunnel between two TCP streams.
fn tunnel(client: &TcpStream, target: &TcpStream) -> Result<()> {
    let mut client_read = client.try_clone()?;
    let mut target_write = target.try_clone()?;
    let mut target_read = target.try_clone()?;
    let mut client_write = client.try_clone()?;

    // client -> target
    let t1 = thread::spawn(move || {
        let _ = std::io::copy(&mut client_read, &mut target_write);
        let _ = target_write.shutdown(std::net::Shutdown::Write);
    });

    // target -> client
    let t2 = thread::spawn(move || {
        let _ = std::io::copy(&mut target_read, &mut client_write);
        let _ = client_write.shutdown(std::net::Shutdown::Write);
    });

    t1.join().ok();
    t2.join().ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── domain_matches tests ────────────────────────────────────────────

    #[test]
    fn domain_exact_match() {
        assert!(domain_matches("example.com", "example.com"));
        assert!(domain_matches("api.anthropic.com", "api.anthropic.com"));
    }

    #[test]
    fn domain_case_insensitive() {
        assert!(domain_matches("Example.COM", "example.com"));
        assert!(domain_matches("example.com", "Example.COM"));
    }

    #[test]
    fn domain_wildcard_match() {
        assert!(domain_matches("foo.googleapis.com", "*.googleapis.com"));
        assert!(domain_matches("bar.baz.googleapis.com", "*.googleapis.com"));
    }

    #[test]
    fn domain_wildcard_matches_base() {
        // *.example.com should also match example.com itself
        assert!(domain_matches("example.com", "*.example.com"));
    }

    #[test]
    fn domain_no_match() {
        assert!(!domain_matches("evil.com", "example.com"));
        assert!(!domain_matches("notexample.com", "example.com"));
        assert!(!domain_matches("evil.com", "*.example.com"));
    }

    // ── is_private_ip tests ─────────────────────────────────────────────

    #[test]
    fn private_ip_rfc1918() {
        assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_private_ip(&"192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn private_ip_loopback() {
        assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"::1".parse().unwrap()));
    }

    #[test]
    fn private_ip_link_local() {
        assert!(is_private_ip(&"169.254.1.1".parse().unwrap()));
    }

    #[test]
    fn private_ip_cgnat() {
        assert!(is_private_ip(&"100.64.0.1".parse().unwrap()));
        assert!(is_private_ip(&"100.127.255.255".parse().unwrap()));
    }

    #[test]
    fn private_ip_multicast() {
        assert!(is_private_ip(&"224.0.0.1".parse().unwrap()));
    }

    #[test]
    fn private_ip_ipv6_ula() {
        assert!(is_private_ip(&"fc00::1".parse().unwrap()));
        assert!(is_private_ip(&"fd12::1".parse().unwrap()));
    }

    #[test]
    fn private_ip_ipv6_link_local() {
        assert!(is_private_ip(&"fe80::1".parse().unwrap()));
    }

    #[test]
    fn private_ip_v4_mapped_v6() {
        // ::ffff:127.0.0.1 is IPv4-mapped IPv6 for loopback
        assert!(is_private_ip(&"::ffff:127.0.0.1".parse().unwrap()));
        // ::ffff:10.0.0.1 is IPv4-mapped IPv6 for RFC1918
        assert!(is_private_ip(&"::ffff:10.0.0.1".parse().unwrap()));
        // ::ffff:8.8.8.8 is IPv4-mapped IPv6 for public IP
        assert!(!is_private_ip(&"::ffff:8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn public_ip_allowed() {
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip(&"1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip(&"2607:f8b0:4004:800::200e".parse().unwrap()));
    }

    #[test]
    fn not_private_ip_100_non_cgnat() {
        // 100.0.0.1 is NOT in CGNAT range (100.64.0.0/10)
        assert!(!is_private_ip(&"100.0.0.1".parse().unwrap()));
    }

    // ── parse_host_port tests ───────────────────────────────────────────

    #[test]
    fn parse_host_port_standard() {
        let (host, port) = parse_host_port("example.com:443").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
    }

    #[test]
    fn parse_host_port_non_standard() {
        let (host, port) = parse_host_port("example.com:8443").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 8443);
    }

    #[test]
    fn parse_host_port_no_port() {
        let (host, port) = parse_host_port("example.com").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
    }

    // ── base64 encoding test ────────────────────────────────────────────

    #[test]
    fn base64_encode_basic_auth() {
        assert_eq!(base64_encode("workmux:mytoken"), "d29ya211eDpteXRva2Vu");
        assert_eq!(base64_encode(""), "");
        assert_eq!(base64_encode("a"), "YQ==");
        assert_eq!(base64_encode("ab"), "YWI=");
        assert_eq!(base64_encode("abc"), "YWJj");
    }

    // ── proxy server lifecycle tests ────────────────────────────────────

    #[test]
    fn proxy_binds_to_random_port() {
        let proxy = NetworkProxy::bind(&["example.com".to_string()]).unwrap();
        assert!(proxy.port() > 0);
    }

    #[test]
    fn proxy_token_is_nonempty() {
        let proxy = NetworkProxy::bind(&[]).unwrap();
        assert!(!proxy.token().is_empty());
    }

    #[test]
    fn proxy_rejects_missing_auth() {
        let proxy = NetworkProxy::bind(&["example.com".to_string()]).unwrap();
        let port = proxy.port();
        let _handle = proxy.spawn();

        std::thread::sleep(Duration::from_millis(50));

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream
            .write_all(b"CONNECT example.com:443 HTTP/1.1\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();

        let mut response = String::new();
        let mut reader = BufReader::new(&stream);
        reader.read_line(&mut response).unwrap();
        assert!(response.contains("407"));
    }

    #[test]
    fn proxy_rejects_wrong_auth() {
        let proxy = NetworkProxy::bind(&["example.com".to_string()]).unwrap();
        let port = proxy.port();
        let _handle = proxy.spawn();

        std::thread::sleep(Duration::from_millis(50));

        let auth = format!("Basic {}", base64_encode("workmux:wrong-token"));
        let request = format!(
            "CONNECT example.com:443 HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
            auth
        );

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();

        let mut response = String::new();
        let mut reader = BufReader::new(&stream);
        reader.read_line(&mut response).unwrap();
        assert!(response.contains("407"));
    }

    #[test]
    fn proxy_rejects_non_443_port() {
        let proxy = NetworkProxy::bind(&["example.com".to_string()]).unwrap();
        let port = proxy.port();
        let token = proxy.token().to_string();
        let _handle = proxy.spawn();

        std::thread::sleep(Duration::from_millis(50));

        let auth = format!("Basic {}", base64_encode(&format!("workmux:{}", token)));
        let request = format!(
            "CONNECT example.com:80 HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
            auth
        );

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();

        let mut response = String::new();
        let mut reader = BufReader::new(&stream);
        reader.read_line(&mut response).unwrap();
        assert!(response.contains("403"));
    }

    #[test]
    fn proxy_rejects_unlisted_domain() {
        let proxy = NetworkProxy::bind(&["allowed.com".to_string()]).unwrap();
        let port = proxy.port();
        let token = proxy.token().to_string();
        let _handle = proxy.spawn();

        std::thread::sleep(Duration::from_millis(50));

        let auth = format!("Basic {}", base64_encode(&format!("workmux:{}", token)));
        let request = format!(
            "CONNECT denied.com:443 HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
            auth
        );

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();

        let mut response = String::new();
        let mut reader = BufReader::new(&stream);
        reader.read_line(&mut response).unwrap();
        assert!(response.contains("403"));
    }

    #[test]
    fn proxy_rejects_ip_literal_hostname() {
        let proxy = NetworkProxy::bind(&["8.8.8.8".to_string()]).unwrap();
        let port = proxy.port();
        let token = proxy.token().to_string();
        let _handle = proxy.spawn();

        std::thread::sleep(Duration::from_millis(50));

        let auth = format!("Basic {}", base64_encode(&format!("workmux:{}", token)));
        let request = format!(
            "CONNECT 8.8.8.8:443 HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
            auth
        );

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();

        let mut response = String::new();
        let mut reader = BufReader::new(&stream);
        reader.read_line(&mut response).unwrap();
        assert!(response.contains("403"));
    }
}
