use bytes::{Buf, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

//
// Frame Layout
//
// Frame Header ----------------------> |
// Start               | Data Len (u16) | Data        | CRC (u16)
//                     | (excludes CRC) | (len bytes) |
// 0xAA 0xCC 0xAA 0xCC | LEN_L LEN_H    | DATA...     | CRC_L CRC_H
//
pub const FRAME_HEADER_LEN: usize = 6;
pub const FRAME_START: [u8; 4] = [0xaa, 0xcc, 0xaa, 0xcc];
pub const FRAME_LENGTH_OFFSET: usize = FRAME_START.len();
pub const CRC_LEN: usize = 2;
pub const MAX_FRAME_LEN: usize = 2048; // Header (6 bytes), body (... bytes), crc (2 bytes)
pub const MAX_PAYLOAD_LEN: usize = MAX_FRAME_LEN - FRAME_HEADER_LEN - CRC_LEN;

#[derive(Debug)]
pub struct FrameHeader {
    hdr: [u8; 6],
}

#[derive(Debug, Clone)]
pub enum FrameError {
    InvalidFrame,
    InvalidLength,
    InvalidCRC,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::InvalidFrame => write!(f, "Invalid frame structure"),
            FrameError::InvalidLength => write!(f, "Invalid frame length"),
            FrameError::InvalidCRC => write!(f, "Invalid frame CRC"),
        }
    }
}

impl From<FrameError> for std::io::Error {
    fn from(err: FrameError) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::InvalidData, err)
    }
}

impl std::error::Error for FrameError {}

impl FrameHeader {
    pub fn new_from_buf(buf: &[u8]) -> Result<Self, FrameError> {
        if buf.len() == FRAME_HEADER_LEN && buf.starts_with(&FRAME_START) {
            let len = u16::from_le_bytes([buf[FRAME_LENGTH_OFFSET], buf[FRAME_LENGTH_OFFSET + 1]])
                as usize;
            if len > MAX_PAYLOAD_LEN {
                Err(FrameError::InvalidLength)
            } else {
                let mut hdr = [0_u8; 6];
                hdr.copy_from_slice(buf);
                Ok(Self { hdr })
            }
        } else {
            Err(FrameError::InvalidFrame)
        }
    }
    pub fn new(len: usize) -> Result<Self, FrameError> {
        if len > MAX_PAYLOAD_LEN {
            Err(FrameError::InvalidLength)
        } else {
            let [b1, b2] = u16::to_be_bytes(len as u16);
            let hdr = [0xaa, 0xcc, 0xaa, 0xcc, b1, b2];
            Ok(Self { hdr })
        }
    }

    pub fn frame_len(&self) -> usize {
        u16::from_le_bytes([
            self.hdr[FRAME_LENGTH_OFFSET],
            self.hdr[FRAME_LENGTH_OFFSET + 1],
        ]) as usize
    }
}

#[derive(Debug, PartialEq, Eq)]
enum FrameState {
    Wait,
    Frame(usize),
}

#[derive(Debug)]
pub struct FramedBinaryCodec {
    state: FrameState,
}

impl FramedBinaryCodec {
    pub fn new() -> Self {
        Self {
            state: FrameState::Wait,
        }
    }
}

impl Decoder for FramedBinaryCodec {
    type Item = Result<Vec<u8>, FrameError>;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.state {
            FrameState::Frame(frame_len) => {
                // Check if we have full frame (including CRC)
                if src.len() >= frame_len + CRC_LEN {
                    let data = src[..frame_len].to_vec();
                    if crc16(&data) == u16::from_le_bytes([src[frame_len], src[frame_len + 1]]) {
                        // CRC OK
                        src.advance(frame_len + CRC_LEN);
                        self.state = FrameState::Wait;
                        Ok(Some(Ok(data)))
                    } else {
                        eprintln!(">> Invalid CRC");
                        src.advance(frame_len + CRC_LEN);
                        self.state = FrameState::Wait;
                        Ok(Some(Err(FrameError::InvalidCRC)))
                    }
                } else {
                    Ok(None)
                }
            }
            FrameState::Wait => match src
                .windows(FRAME_START.len())
                .position(|w| w == FRAME_START)
            {
                // Look for START flag
                Some(i) => {
                    // println!(">> Found START: {i}");
                    // Advance to START
                    src.advance(i);
                    // Check if have full header
                    if src.len() >= FRAME_HEADER_LEN {
                        match FrameHeader::new_from_buf(&src[..FRAME_HEADER_LEN]) {
                            Ok(hdr) => {
                                src.advance(FRAME_HEADER_LEN);
                                self.state = FrameState::Frame(hdr.frame_len());
                                Ok(None)
                            }
                            Err(e) => {
                                src.advance(FRAME_HEADER_LEN);
                                Ok(Some(Err(e)))
                            }
                        }
                    } else {
                        Ok(None)
                    }
                }
                None => {
                    // Check for UTF-8 lines
                    while let Some(i) = src.iter().position(|&b| b == b'\n') {
                        if let Ok(msg) = std::str::from_utf8(&src[..i]) {
                            println!("DEBUG: {}", msg.trim());
                        }
                        src.advance(i + 1);
                    }
                    Ok(None)
                }
            },
        }
    }
}

impl Encoder<&[u8]> for FramedBinaryCodec {
    type Error = std::io::Error;

    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.len() > MAX_PAYLOAD_LEN {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Frame of length {} is too large.", item.len()),
            ));
        }

        dst.reserve(FRAME_HEADER_LEN + item.len() + CRC_LEN);
        let start = [0xaa, 0xcc, 0xaa, 0xcc];
        let len = u16::to_le_bytes(item.len() as u16);
        let crc = u16::to_le_bytes(crc16(item));

        dst.extend_from_slice(&start);
        dst.extend_from_slice(&len);
        dst.extend_from_slice(item);
        dst.extend_from_slice(&crc);

        Ok(())
    }
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/*

// Binary data header is [start0, start1, start2, start3, len0, len1]
// (start is 4-byte pattern, len is 2-byte u16 le encoded)
pub const FRAME_HEADER_LEN: usize = 3;
pub static START_PATTERN: [u8; 2] = [0xAA, 0xCC];
pub static MAX_DATA_LEN: usize = 255;

struct FrameHeader([u8; FRAME_HEADER_LEN]);

impl FrameHeader {
    pub fn new(len: u8) -> Self {
        Self([0xAA, 0xCC, len])
    }
}

#[derive(Debug)]
pub struct FramedBinaryCodec {
    pkt_start: bool,
    pkt_len: usize,
}

impl FramedBinaryCodec {
    pub fn new() -> Self {
        Self {
            pkt_start: false,
            pkt_len: 0,
        }
    }
}

impl Encoder<&[u8]> for FramedBinaryCodec {
    type Error = std::io::Error;

    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> Result<(), Self::Error> {
        if item.len() > MAX_DATA_LEN {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Frame of length {} is too large.", item.len()),
            ));
        }

        dst.reserve(FRAME_HEADER_LEN + item.len());
        let hdr = FrameHeader::new(item.len() as u8);
        dst.extend_from_slice(&hdr.0);
        dst.extend_from_slice(item);

        Ok(())
    }
}

impl Decoder for FramedBinaryCodec {
    type Item = Vec<u8>;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // println!(
        //     ">> {:?}\n>> pkt_start={} pkt_len={}",
        //     src, self.pkt_start, self.pkt_len
        // );
        match self.pkt_start {
            true => {
                // Binary message - check if we have enough bytes
                if src.len() >= self.pkt_len {
                    let item = src[..self.pkt_len].to_vec();
                    src.advance(self.pkt_len);
                    self.pkt_start = false;
                    self.pkt_len = 0;
                    Ok(Some(item))
                } else {
                    Ok(None)
                }
            }
            false => match src
                .windows(START_PATTERN.len())
                .position(|w| w == START_PATTERN)
            {
                // Look for START flag
                Some(i) => {
                    // println!("++ Found START: {i}");
                    // Advance to START
                    src.advance(i);
                    // Check if have full header
                    if src.len() >= FRAME_HEADER_LEN {
                        self.pkt_len = src[FRAME_HEADER_LEN - 1] as usize;
                        // Advance past header and set pkt_start flag
                        src.advance(FRAME_HEADER_LEN);
                        self.pkt_start = true;
                    }
                    Ok(None)
                }
                None => {
                    // Look UTF-8 line
                    if let Some(i) = src.iter().position(|&b| b == b'\n') {
                        let msg = std::str::from_utf8(&src[..i])
                            .unwrap_or("<Invalid UTF8>")
                            .trim();
                        println!("DEBUG: {msg}",);
                        // Advance past string + NL
                        src.advance(i + 1);
                    }
                    Ok(None)
                }
            },
        }
    }
}
*/
