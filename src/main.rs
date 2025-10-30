use std::env;

// use futures_util::sink::SinkExt;
use tokio_serial::SerialPortBuilderExt;
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;

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

    let mut framed = Framed::new(serial, esp_codec::EspCodec::new());

    tokio::spawn(async move {
        loop {
            match framed.try_next().await {
                Ok(Some(pkt)) => match postcard::from_bytes::<SensorMessage>(&pkt) {
                    Ok(data) => println!(">> Data: {:?}", data),
                    Err(e) => eprintln!(">> Postcard Error: {e:?}"),
                },
                Ok(None) => {}
                Err(e) => eprintln!("ERR: {e}"),
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    Ok(())
}

mod esp_codec {

    use bytes::{Buf, BytesMut};
    use tokio_util::codec::Decoder;

    pub const HEADER_LEN: usize = 6; // [0x00, 0xff, 0x00, 0xff, len0, len1]
    pub static START: [u8; 4] = [0x00_u8, 0xff_u8, 0x00_u8, 0xff_u8];

    #[derive(Debug)]
    pub struct EspCodec {
        pkt_start: bool,
        pkt_len: usize,
    }

    impl EspCodec {
        pub fn new() -> Self {
            Self {
                pkt_start: false,
                pkt_len: 0,
            }
        }
    }

    impl Decoder for EspCodec {
        type Item = Vec<u8>;
        type Error = std::io::Error;

        fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
            match self.pkt_start {
                true => {
                    // Binary message - check if we have enough bytes
                    if src.len() >= self.pkt_len {
                        let item = src[..self.pkt_len].to_vec();
                        // println!("PKT: {item:?}");
                        src.advance(self.pkt_len);
                        self.pkt_start = false;
                        self.pkt_len = 0;
                        Ok(Some(item))
                    } else {
                        Ok(None)
                    }
                }
                false => match src.windows(4).position(|w| w == START) {
                    // Look for START flag
                    Some(i) => {
                        // Advance to START
                        src.advance(i);
                        // Check if have full header
                        if src.len() >= HEADER_LEN {
                            self.pkt_len = u16::from_le_bytes([src[4], src[5]]) as usize;
                            // Advance past header and set pkt_start flag
                            src.advance(HEADER_LEN);
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
