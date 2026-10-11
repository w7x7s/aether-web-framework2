//! # Aether WebSocket RFC 6455 Fast Frame Parser

use aether_core::error::AetherError;

#[derive(Debug, PartialEq, Eq)]
pub enum WsOpcode {
    Continuation = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

pub struct WsFrameView<'a> {
    pub fin: bool,
    pub opcode: WsOpcode,
    pub payload: &'a [u8],
}

pub fn parse_frame(buf: &[u8]) -> Result<Option<(WsFrameView<'_>, usize)>, AetherError> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];

    let fin = (b0 & 0x80) != 0;
    let opcode = match b0 & 0x0F {
        0x1 => WsOpcode::Text,
        0x2 => WsOpcode::Binary,
        0x8 => WsOpcode::Close,
        0x9 => WsOpcode::Ping,
        0xA => WsOpcode::Pong,
        _ => WsOpcode::Continuation,
    };

    let masked = (b1 & 0x80) != 0;
    let mut payload_len = (b1 & 0x7F) as usize;
    let mut offset = 2;

    if payload_len == 126 {
        if buf.len() < 4 {
            return Ok(None);
        }
        payload_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        offset = 4;
    } else if payload_len == 127 {
        if buf.len() < 10 {
            return Ok(None);
        }
        payload_len = u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize;
        offset = 10;
    }

    if masked {
        offset += 4;
    }

    let total_len = offset + payload_len;
    if buf.len() < total_len {
        return Ok(None);
    }

    let payload = &buf[offset..total_len];
    Ok(Some((WsFrameView { fin, opcode, payload }, total_len)))
}
