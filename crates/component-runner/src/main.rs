use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let bind_addr =
        std::env::var("MASONWING_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    let listener = TcpListener::bind(&bind_addr).await?;

    loop {
        let (mut socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut request = [0_u8; 1024];
            let Ok(read) = socket.read(&mut request).await else {
                return;
            };
            let request = &request[..read];
            let healthy = request.starts_with(b"GET /health/live ")
                || request.starts_with(b"GET /health/ready ");
            let (status, body) = if healthy {
                ("200 OK", b"OK".as_slice())
            } else {
                ("404 Not Found", b"NOT_FOUND".as_slice())
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            if socket.write_all(response.as_bytes()).await.is_ok() {
                let _ = socket.write_all(body).await;
            }
        });
    }
}
