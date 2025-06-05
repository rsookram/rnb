use std::{char::decode_utf16, env::args_os, fs};

fn main() {
    let path = args_os().nth(1).unwrap();

    let bytes = fs::read(path).unwrap();
    let mut bytes = bytes.as_slice();

    let num_blocks = u16::from_le_bytes([bytes[0], bytes[1]]);
    bytes = &bytes[2..];
    println!("num blocks {num_blocks}");

    let num_images = bytes[0];
    bytes = &bytes[1..];
    println!("num images {num_images}");

    for i in 0..num_images {
        let offset = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        bytes = &bytes[4..];

        let uncompressed_length = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        bytes = &bytes[4..];

        println!("image meta {i}: offset={offset}, uncompressed_length={uncompressed_length}");
    }

    for i in 0..num_blocks {
        let prefix = u16::from_le_bytes([bytes[0], bytes[1]]);
        bytes = &bytes[2..];

        let is_image = (prefix & (1 << 15)) != 0;
        let is_bold = (prefix & (1 << 14)) != 0;
        let is_large = (prefix & (1 << 13)) != 0;

        if is_image {
            println!("image {}", prefix & !(1 << 15));
            continue;
        }

        let length = prefix & !(0b111 << 13);
        if length == 0 {
            println!(
                "zero length block: idx={i}, bold={is_bold}, is_large={is_large}, prefix={prefix}",
            );
            continue;
        }

        assert!(length % 2 == 0, "{length}");

        let text = &bytes[..usize::from(length)];
        bytes = &bytes[usize::from(length)..];
        let text = decode_utf16(
            text.chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]])),
        )
        .map(|ch| ch.unwrap())
        .collect::<String>();

        println!("text block meta: idx={i}, bold={is_bold}, is_large={is_large}");
        println!("{text}");

        let num_ruby = bytes[0];
        bytes = &bytes[1..];
        if num_ruby == 0 {
            continue;
        }

        for j in 0..num_ruby {
            let start_offset = u16::from_le_bytes([bytes[0], bytes[1]]);
            bytes = &bytes[2..];

            let num_chars_in_text = bytes[0];
            bytes = &bytes[1..];

            let reading_len = bytes[0];
            assert!(reading_len % 2 == 0, "{reading_len}");
            bytes = &bytes[1..];

            let reading = &bytes[..usize::from(reading_len)];
            bytes = &bytes[usize::from(reading_len)..];
            let reading = decode_utf16(
                reading
                    .chunks_exact(2)
                    .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]])),
            )
            .map(|ch| ch.unwrap())
            .collect::<String>();

            println!(
                "ruby meta: idx={j}, start_offset={start_offset}, num_chars_in_text={num_chars_in_text}"
            );
            println!("{reading}");
        }
    }
}
