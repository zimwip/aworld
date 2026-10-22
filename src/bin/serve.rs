//! A tiny static web server for the web build: `aworld-serve [folder] [port]`.
//! No dependencies; it only has to hand a few files to a browser on this machine or network.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::thread;

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// Maps a request path onto a file under `root`, refusing anything that climbs out of it.
fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let clean = url_path.split(['?', '#']).next().unwrap_or("/");
    let mut out = root.to_path_buf();
    for part in Path::new(clean.trim_start_matches('/')).components() {
        match part {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if out.is_dir() {
        out.push("index.html");
    }
    Some(out)
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn handle(mut stream: TcpStream, root: &Path) {
    let mut buf = [0u8; 4096];
    let Ok(n) = stream.read(&mut buf) else { return };
    let request = String::from_utf8_lossy(&buf[..n]);
    let mut words = request.split_whitespace();
    let (method, target) = (words.next().unwrap_or(""), words.next().unwrap_or("/"));
    if method != "GET" && method != "HEAD" {
        return respond(&mut stream, "405 Method Not Allowed", "text/plain", b"GET only");
    }
    match resolve(root, target).and_then(|p| fs::read(&p).ok().map(|b| (p, b))) {
        Some((path, body)) => respond(&mut stream, "200 OK", mime(&path), if method == "HEAD" { &[] } else { &body }),
        None => respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.get(1).map_or(".", |s| s.as_str()));
    let port: u16 = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(8765);
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Impossible d'ouvrir le port {port} : {e}");
            std::process::exit(1);
        }
    };
    println!("AWorld : http://localhost:{port}/   (dossier : {})   Ctrl+C pour arrêter", root.display());
    for stream in listener.incoming().flatten() {
        let root = root.clone();
        thread::spawn(move || handle(stream, &root));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_cannot_escape_the_folder() {
        let root = Path::new("/srv/site");
        assert_eq!(resolve(root, "/aworld.wasm?x=1"), Some(PathBuf::from("/srv/site/aworld.wasm")));
        assert!(resolve(root, "/../etc/passwd").is_none());
        assert!(resolve(root, "/a/../../b").is_none());
    }

    #[test]
    fn wasm_has_its_own_type() {
        assert_eq!(mime(Path::new("aworld.wasm")), "application/wasm");
        assert_eq!(mime(Path::new("index.html")), "text/html; charset=utf-8");
    }
}
