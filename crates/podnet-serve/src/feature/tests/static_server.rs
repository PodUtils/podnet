use super::*;

#[test]
fn static_serving_returns_the_file_with_security_headers() {
    let d = tmpdir("static-file");
    std::fs::write(d.join("index.html"), b"<h1>hello</h1>").expect("write");

    let origin = crate::origin::Origin::parse(&format!("static:{}", d.display())).expect("parse");
    let server = StaticServer::from_origin(&origin).expect("from origin");

    let resp = server.respond(&req("GET", "/")).expect("respond");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.header_value("x-content-type-options"), Some("nosniff"));
    assert_eq!(resp.header_value("x-frame-options"), Some("SAMEORIGIN"));
    assert!(resp
        .header_value("content-type")
        .expect("type")
        .starts_with("text/html"));

    // The response must render as a real HTTP response.
    let mut wire = Vec::new();
    assert_eq!(resp.write_to(&mut wire).expect("write"), 14);
    let bytes = String::from_utf8(wire).expect("utf8");
    assert!(bytes.starts_with("HTTP/1.1 200 OK\r\n"), "{bytes}");
    // The declared length must match the body actually sent, or a client
    // either hangs or truncates.
    let body = "<h1>hello</h1>";
    assert!(
        bytes.ends_with(&format!("Content-Length: {}\r\n\r\n{body}", body.len())),
        "declared length must match the {} byte body: {bytes}",
        body.len()
    );
}

/// The whole body of a response.
fn body_of(resp: StaticResponse) -> Vec<u8> {
    let mut body = resp.body;
    let mut out = Vec::new();
    io::Read::read_to_end(&mut body, &mut out).expect("read the body");
    out
}

// Issue 296: a file of any size is streamed. The response keeps the
// open file, and the body is read from the file when it is read: a
// byte written after `respond` is in the body. Each read gives at most
// the buffer of the reader.
#[test]
fn a_large_file_is_streamed_completely() {
    use std::io::{Read, Seek, Write};
    let d = tmpdir("static-big");
    let size: u64 = 200 * 1024 * 1024;
    let mut big = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(d.join("big.bin"))
        .expect("create");
    big.set_len(size).expect("set_len");
    let origin = crate::origin::Origin::parse(&format!("static:{}", d.display())).expect("parse");
    let server = StaticServer::from_origin(&origin).expect("from origin");
    let mut resp = server.respond(&req("GET", "/big.bin")).expect("respond");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.len(), size);
    assert!(resp.body.is_file());
    assert_eq!(resp.header_value("x-content-type-options"), Some("nosniff"));
    let head = String::from_utf8(resp.head_bytes()).expect("utf8");
    assert!(
        head.contains(&format!("Content-Length: {size}\r\n")),
        "{head}"
    );
    // A response that holds a copy of the file does not see this byte.
    big.seek(std::io::SeekFrom::Start(size - 1)).expect("seek");
    big.write_all(b"Z").expect("write the last byte");
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    let mut last = 0u8;
    loop {
        let n = resp.body.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        assert!(n <= buf.len());
        total += n as u64;
        last = buf[n - 1];
    }
    assert_eq!(total, size);
    assert_eq!(last, b'Z', "the body was not read from the file");

    // write_to sends the head and the whole body.
    struct Count(u64);
    impl std::io::Write for Count {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            assert!(b.len() <= podnet_core::util::http::BODY_CHUNK);
            self.0 += b.len() as u64;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let resp = server.respond(&req("GET", "/big.bin")).expect("respond");
    let head = resp.head_bytes().len() as u64;
    let mut count = Count(0);
    assert_eq!(resp.write_to(&mut count).expect("write"), size);
    assert_eq!(count.0, head + size);
    let _ = std::fs::remove_dir_all(&d);
}

// A file that becomes shorter after `respond` gives an error, not a
// short body with a long Content-Length.
#[test]
fn a_file_that_shrinks_is_an_error() {
    let d = tmpdir("static-shrink");
    std::fs::write(d.join("a.bin"), vec![1u8; 1000]).expect("write");
    let origin = crate::origin::Origin::parse(&format!("static:{}", d.display())).expect("parse");
    let server = StaticServer::from_origin(&origin).expect("from origin");
    let resp = server.respond(&req("GET", "/a.bin")).expect("respond");
    std::fs::OpenOptions::new()
        .write(true)
        .open(d.join("a.bin"))
        .expect("open")
        .set_len(10)
        .expect("truncate");
    let err = resp.write_to(&mut Vec::new()).expect_err("short file");
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn security_headers_are_present_on_an_error_response_too() {
    // A 404 body is still rendered by a browser, so it needs the same
    // protection as the file it failed to serve.
    let d = tmpdir("static-missing");
    std::fs::write(d.join("index.html"), b"x").expect("write");
    let origin = crate::origin::Origin::parse(&format!("static:{}", d.display())).expect("parse");
    let resp = StaticServer::from_origin(&origin)
        .expect("from origin")
        .respond(&req("GET", "/nope"))
        .expect("respond");

    assert_eq!(resp.status, 404);
    assert_eq!(resp.header_value("x-content-type-options"), Some("nosniff"));
}

#[test]
fn traversal_outside_the_root_is_still_refused_by_the_wrapper() {
    // The wrapper must not weaken the guarantee StaticDir already makes.
    let d = tmpdir("static-traverse");
    std::fs::write(d.join("index.html"), b"index").expect("write");
    let secret = d.join("..").join("podnet-feature-secret");
    std::fs::write(&secret, b"SECRET").expect("write secret");

    let origin = crate::origin::Origin::parse(&format!("static:{}", d.display())).expect("parse");
    let server = StaticServer::from_origin(&origin).expect("from origin");
    for attack in ["/../podnet-feature-secret", "/%2e%2e/podnet-feature-secret"] {
        let resp = server.respond(&req("GET", attack)).expect("respond");
        assert_eq!(resp.status, 404, "{attack} must not be served");
        assert!(
            !String::from_utf8_lossy(&body_of(resp)).contains("SECRET"),
            "{attack} leaked content"
        );
    }
    let _ = std::fs::remove_file(&secret);
}

// A conditional request that matches gets 304 with no body and one
// Content-Length.
#[test]
fn a_matched_etag_gets_304_with_no_body() {
    let d = tmpdir("static-304");
    std::fs::write(d.join("a.txt"), b"small").expect("write");
    let origin = crate::origin::Origin::parse(&format!("static:{}", d.display())).expect("parse");
    let server = StaticServer::from_origin(&origin).expect("from origin");
    let mtime = std::fs::metadata(d.join("a.txt"))
        .and_then(|m| m.modified())
        .expect("mtime")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("epoch")
        .as_secs();
    let mut request = req("GET", "/a.txt");
    request.set_header("If-None-Match", &format!("\"{mtime}\""));
    let resp = server.respond(&request).expect("respond");
    assert_eq!(resp.status, 304);
    let mut wire = Vec::new();
    assert_eq!(resp.write_to(&mut wire).expect("write"), 0);
    let wire = String::from_utf8(wire).expect("utf8");
    assert!(wire.starts_with("HTTP/1.1 304 Not Modified\r\n"), "{wire}");
    assert_eq!(wire.matches("Content-Length").count(), 1, "{wire}");
    assert!(wire.ends_with("Content-Length: 0\r\n\r\n"), "{wire}");
}

#[test]
fn a_write_method_is_refused_by_the_static_server() {
    let d = tmpdir("static-write");
    std::fs::write(d.join("index.html"), b"index").expect("write");
    let origin = crate::origin::Origin::parse(&format!("static:{}", d.display())).expect("parse");
    let server = StaticServer::from_origin(&origin).expect("from origin");

    let resp = server.respond(&req("POST", "/")).expect("respond");
    assert_eq!(resp.status, 405);
    assert_eq!(resp.header_value("allow"), Some("GET, HEAD"));
}

#[test]
fn from_origin_refuses_an_origin_that_is_not_static() {
    let err = StaticServer::from_origin(&crate::origin::Origin::HelloWorld)
        .expect_err("hello_world is not a directory");
    match err {
        FeatureError::Invalid(m) => assert!(m.contains("not a static origin"), "{m}"),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn an_spa_origin_falls_back_and_never_caches_the_shell() {
    let d = tmpdir("static-spa");
    std::fs::write(d.join("index.html"), b"shell").expect("write");
    let origin = crate::origin::Origin::parse(&format!("spa:{}", d.display())).expect("parse");
    let server = StaticServer::from_origin(&origin).expect("from origin");

    let resp = server.respond(&req("GET", "/deep/route")).expect("respond");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.len(), 5);
    // The shell is the same for every route, so caching it would pin a
    // visitor to an old build.
    assert_eq!(resp.header_value("cache-control"), Some("no-store"));
    assert_eq!(body_of(resp), b"shell");
}

#[test]
fn a_buffered_response_records_what_was_written() {
    use podnet_core::util::http::ResponseWriter;

    let mut b = BufferedResponse::new();
    assert!(!b.is_started(), "a fresh buffer has no status yet");
    b.status(201).header("X-Test", "1");
    b.text("hi").expect("write body");
    assert!(b.is_started());
    assert_eq!(b.status, 201);
    assert_eq!(
        b.header_value("x-test"),
        Some("1"),
        "lookup is case-insensitive"
    );
    assert_eq!(b.body, b"hi");
    assert!(String::from_utf8(b.to_bytes())
        .expect("utf8")
        .starts_with("HTTP/1.1 201 Created"));
}
