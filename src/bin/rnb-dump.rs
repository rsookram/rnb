use std::{char::decode_utf16, env::args_os, fs};

// Assuming that there are < 30k paragraphs per book
// First, metadata on paragraphs:
// - u16 of number of paragraphs in the book
//
// Then paragraphs... Paragraph format is:
// - length prefix (u16 for bytes of paragraph text)
//   - the highest three bits are flags; from highest to lowest:
//   - isImage: interpret the remaining bits as an image index
//   - isBold: display the paragraph with bold text
//   - isLarge: display the paragraph with larger text
//
// - text of paragraph (UTF-16LE)
// - list of spans
//   - num furigana spans (u8)
//   - each span has 4 fields
//   - the start offset of where it applies to the paragraph (u16)
//   - the number of chars it applies to in the paragraph (u8)
//   - number of bytes for the reading (u8)
//   - UTF-16LE encoded bytes for reading
//
// After paragraphs come the images, one image after the next.
// Then the image metadata:
// For each image, start offset within entire file (u32), then size of image in bytes
// (u32)
// Finally the number of images (u8)
fn main() {
    let path = args_os().nth(1).unwrap();

    let bytes = fs::read(path).unwrap();
    let mut bytes = bytes.as_slice();

    let num_paragraphs = u16::from_le_bytes([bytes[0], bytes[1]]);
    bytes = &bytes[2..];
    println!("{num_paragraphs}");

    for i in 0..num_paragraphs {
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
                "zero length paragraph: idx={i}, bold={is_bold}, is_large={is_large}, prefix={prefix}",
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

        println!("paragraph meta: idx={i}, bold={is_bold}, is_large={is_large}");
        println!("{text}");

        let num_ruby = bytes[0];
        bytes = &bytes[1..];
        if num_ruby == 0 {
            continue;
        }

        for j in 0..num_ruby {
            let start_offset = u16::from_le_bytes([bytes[0], bytes[1]]);
            bytes = &bytes[2..];

            let num_chars_in_paragraph = bytes[0];
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
                "ruby meta: idx={j}, start_offset={start_offset}, num_chars_in_paragraph={num_chars_in_paragraph}"
            );
            println!("{reading}");
        }
    }
}
