use std::env;

use futures_util::sink::SinkExt;

use tokio::sync::mpsc;
use tokio_serial::SerialPortBuilderExt;
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;

use esp_now_server::framed_codec::FramedBinaryCodec;
//use sensor_message::SensorMessage;

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

    let (tx_queue, mut rx_queue) = mpsc::channel::<Vec<u8>>(10);

    // Stdin reader task
    tokio::spawn(async move {
        use tokio::io::{stdin, AsyncBufReadExt, BufReader};
        let mut stdin = BufReader::new(stdin());
        let mut buffer = String::new();
        loop {
            buffer.clear();
            match stdin.read_line(&mut buffer).await {
                Ok(0) => break,
                Ok(_) => {
                    let msg = buffer.trim().as_bytes().to_vec();
                    if !msg.is_empty() && tx_queue.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("STDIN error: {}", e);
                    break;
                }
            }
        }
    });

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(10));

    let mut counter = 0_u32;

    loop {
        tokio::select! {
            rx = framed.try_next() => {
                match rx {
                    Ok(Some(Ok(pkt))) => {
                        println!(">> RX Packet: {pkt:?} [{}]", String::from_utf8_lossy(&pkt));
                        let reply = counter.to_le_bytes();
                        match framed.send(&reply).await {
                            Ok(_) => println!(">> Sent Reply: {reply:?}"),
                            Err(e) => eprintln!(">> Error Sending Data: {e:?}"),
                        }
                    }
                    Ok(Some(Err(e))) => eprintln!(">> RX Error: {e:?}"),
                    Ok(None) => {}
                    Err(e) => eprintln!("ERR: {e}"),
                }
                counter += 1;
            }
            msg = rx_queue.recv() => {
                if let Some(data) = msg {
                    println!(">> TX from stdin: {:?}", String::from_utf8_lossy(&data));
                    if let Err(e) = framed.send(&data).await {
                        eprintln!(">> Error Sending stdin message: {e:?}");
                    }
                } else {
                    break; // stdin task dropped tx
                }

            }
            _ = ticker.tick() => {
                println!("+++ TICK");
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nReceived Ctrl-C! Shutting down...\n");
                break;
            }
        }
    }
    println!("BYE");
    Ok(())
}
