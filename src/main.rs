use std::env;
use std::error::Error;
use std::process::Stdio;

use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7878".to_owned());
    let mut listener = TcpListener::bind(&addr).await?;
    loop {
        if let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                handle_connection(socket).await;
            });
        } else {
            break;
        }
    }
    eprintln!("Shutting down.");
    Ok(())
}

fn ascii_to_num(a: u8) -> Option<u8> {
    if a.is_ascii_digit() {
        Some(a - b'0')
    } else if a.is_ascii_hexdigit() {
        Some(a.to_ascii_lowercase() - b'W')
    } else {
        None
    }
}

// Not sure about correctness
fn decode_percent(buf: &[u8]) -> Option<String> {
    let mut iter = buf.iter();
    let mut s = String::new();

    while let Some(c) = iter.next() {
        match c {
            b'+' => s.push(' '),
            b'%' => {
                let mut a: u8 = *iter.next()?;
                let mut b: u8 = *iter.next()?;

                a = ascii_to_num(a)?;
                b = ascii_to_num(b)?;

                s.push(char::from((a << 4) + b));
            }
            b => s.push(char::from(*b)),
        }
    }
    Some(s)
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

async fn handle_connection(mut stream: TcpStream) -> Option<()> {
    let mut in_buffer = [0; 1024];
    let mut out_buffer = [0; 1024];
    let mut bytes = 0;
    //stream.set_read_timeout(Some(Duration::new(5, 0))).ok()?;

    loop {
        let buf = &mut in_buffer[bytes..];
        let len = stream.read(buf).await.ok()?;

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
        let status_404 = b"HTTP/1.1 404 NOT FOUND\r\nContent-Length: 5\r\n\r\nError";
        stream.write(status_404).await.ok()?;
        stream.flush().await.ok()?;
        return None;
    };

    let cmd = decode_percent(cmd)?;

    eprintln!("Run: '{}'", cmd);

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout().take()?;

    let headers: [&[u8]; 3] = [
        b"HTTP/1.1 200 OK\r\n",
        b"Transfer-Encoding: chunked\r\n",
        b"\r\n",
    ];

    for h in &headers {
        stream.write(h).await.ok()?;
    }

    loop {
        let len = stdout.read(&mut out_buffer).await.ok()?;
        if len == 0 {
            break;
        }
        let tmp_str = format!("{:x}\r\n", len);
        stream.write(tmp_str.as_bytes()).await.ok()?;
        // Is this a bug somewhere?
        //stream
        //.write(format!("{:x}\r\n", len).as_bytes())
        //.await
        //.ok()?;
        //
        stream.write(&out_buffer[0..len]).await.ok()?;
        stream.write(b"\r\n").await.ok()?;
        stream.flush().await.ok()?;
    }

    stream.write(b"0\r\n\r\n").await.ok()?;
    stream.flush().await.ok()?;

    Some(())
}
