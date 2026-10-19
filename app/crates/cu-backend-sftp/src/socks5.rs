// SPDX-License-Identifier: GPL-3.0-or-later

use std::io;
use std::net::IpAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const VERSION: u8 = 0x05;
const NO_AUTH: u8 = 0x00;
const CMD_CONNECT: u8 = 0x01;
const ATYP_V4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_V6: u8 = 0x04;

fn refused(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionRefused, what.to_string())
}

pub(crate) async fn connect(
    proxy_host: &str,
    proxy_port: u16,
    host: &str,
    port: u16,
) -> io::Result<TcpStream> {
    let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;
    handshake(&mut stream, host, port).await?;
    Ok(stream)
}

async fn handshake<S>(stream: &mut S, host: &str, port: u16) -> io::Result<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    stream.write_all(&[VERSION, 1, NO_AUTH]).await?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await?;
    if choice != [VERSION, NO_AUTH] {
        return Err(refused(
            "the proxy does not speak SOCKS5 or requires authentication",
        ));
    }

    let mut request = vec![VERSION, CMD_CONNECT, 0x00];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            request.push(ATYP_V4);
            request.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            request.push(ATYP_V6);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            let len = u8::try_from(host.len())
                .ok()
                .filter(|&len| len > 0)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid host name"))?;
            request.push(ATYP_DOMAIN);
            request.push(len);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;

    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await?;
    if reply[0] != VERSION {
        return Err(refused("invalid proxy response"));
    }
    if reply[1] != 0x00 {
        return Err(refused(&format!(
            "the proxy refused the destination (code {})",
            reply[1]
        )));
    }
    let bound_len = match reply[3] {
        ATYP_V4 => 4,
        ATYP_V6 => 16,
        ATYP_DOMAIN => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            usize::from(len[0])
        }
        _ => return Err(refused("invalid proxy response")),
    };
    let mut bound = vec![0u8; bound_len + 2];
    stream.read_exact(&mut bound).await?;
    Ok(())
}
