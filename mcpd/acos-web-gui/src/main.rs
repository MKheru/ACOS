use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";

fn main() -> io::Result<()> {
    let bind_addr = env::var("ACOS_WEB_GUI_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());
    let listener = TcpListener::bind(&bind_addr)?;
    eprintln!("acos-web-gui listening on http://{bind_addr}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(err) = handle_stream(stream) {
                    eprintln!("acos-web-gui request failed: {err}");
                }
            }
            Err(err) => eprintln!("acos-web-gui accept failed: {err}"),
        }
    }

    Ok(())
}

fn handle_stream(mut stream: TcpStream) -> io::Result<()> {
    let request_line = {
        let mut reader = BufReader::new(&mut stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        line
    };

    let response = response_for_request_line(&request_line);
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn response_for_request_line(request_line: &str) -> String {
    match parse_request_target(request_line) {
        Some(("GET", "/health")) => http_response("200 OK", "text/plain; charset=utf-8", "ok\n"),
        Some(("GET", _)) => http_response("404 Not Found", "text/plain; charset=utf-8", "not found\n"),
        Some(_) => http_response(
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            "method not allowed\n",
        ),
        None => http_response("400 Bad Request", "text/plain; charset=utf-8", "bad request\n"),
    }
}

fn parse_request_target(request_line: &str) -> Option<(&str, &str)> {
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    let version = parts.next()?;

    if !version.starts_with("HTTP/") || parts.next().is_some() {
        return None;
    }

    Some((method, target))
}

fn http_response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::response_for_request_line;

    #[test]
    fn health_endpoint_returns_200() {
        let response = response_for_request_line("GET /health HTTP/1.1\r\n");

        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("\r\n\r\nok\n"));
    }

    #[test]
    fn only_health_endpoint_is_exposed() {
        let response = response_for_request_line("GET / HTTP/1.1\r\n");

        assert!(response.starts_with("HTTP/1.1 404 Not Found\r\n"));
    }

    #[test]
    fn non_get_methods_are_rejected() {
        let response = response_for_request_line("POST /health HTTP/1.1\r\n");

        assert!(response.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"));
    }
}
