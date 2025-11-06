use std::env;

use futures_util::sink::SinkExt;
use tokio_serial::SerialPortBuilderExt;
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;

use esp_now_server::framed_codec::FramedBinaryCodec;
use sensor_message::SensorMessage;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Get the current executable name
    let exe_path = env::current_exe().expect("Failed to get current executable path");
    let exe_name = exe_path.file_name().expect("Failed to get executable name");

    let mut args = env::args();
    let tty_path = args
        .nth(1)
        .expect(&format!("Usage: {} <tty_path>", exe_name.to_string_lossy()));

    let mut serial = tokio_serial::new(tty_path, 115200).open_native_async()?;

    #[cfg(unix)]
    serial
        .set_exclusive(false)
        .expect("Unable to set serial port exclusive to false");

    let mut framed = Framed::new(serial, FramedBinaryCodec::new());

    tokio::spawn(async move {
        let mut counter = 0_u32;
        loop {
            match framed.try_next().await {
                Ok(Some(pkt)) => {
                    match postcard::from_bytes::<SensorMessage>(&pkt) {
                        Ok(data) => println!(">> Data: {:?}", data),
                        Err(e) => eprintln!(">> Postcard Error: {e:?}"),
                    }
                    let reply = counter.to_be_bytes();
                    match framed.send(&reply).await {
                        Ok(_) => println!(">> Sent Reply: {reply:?}"),
                        Err(e) => eprintln!(">> Error Sending Data: {e:?}"),
                    }
                }
                Ok(None) => {}
                Err(e) => eprintln!("ERR: {e}"),
            }
            counter += 1;
        }
    });

    tokio::signal::ctrl_c().await?;
    Ok(())
}

/*
mod framed_codec {

    use bytes::{Buf, BytesMut};
    use tokio_util::codec::{Decoder, Encoder};

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
}
*/
