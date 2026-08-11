use std::io::{Read, Write};
use std::time::Duration;
use serialport::SerialPort;

fn main() {
    let port = serialport::new("/dev/ttyUSB0", 115200)
        .timeout(Duration::from_millis(1000))
        .open()
        .unwrap();
    let mut port = port;
    let frames = [
        vec![0xFE,0xFE,0x94,0xE0,0x03,0x00,0xFD],
        vec![0xFE,0xFE,0x94,0xE0,0x04,0x00,0xFD],
        vec![0xFE,0xFE,0x94,0xE0,0x03,0x03,0xFD],
        vec![0xFE,0xFE,0x94,0xE0,0x04,0x03,0xFD],
    ];
    for frame in frames {
        port.write_all(&frame).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let mut buf = [0u8; 256];
        match port.read(&mut buf) {
            Ok(n) => println!("frame {:?} -> {}", frame, String::from_utf8_lossy(&buf[..n])),
            Err(e) => println!("err {:?}", e),
        }
    }
}
